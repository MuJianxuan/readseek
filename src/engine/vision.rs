// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! SmolVLM-500M vision model: OCR (with regions), captioning, and object
//! detection. The GGUF model and multimodal projection are fetched lazily into
//! the user cache directory (see [`crate::engine::model`]) and executed via
//! llama.cpp through the `llama-cpp-2` crate's `mtmd` (multimodal) API.

// Coordinate math converts between normalized 0-1000 bins and pixel space;
// these casts are intentional and bounded.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use anyhow::{Context as _, Result};
use encoding_rs::UTF_8;
use llama_cpp_2::LogOptions;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::mtmd::{MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;

// llama.cpp's mtmd/clip tools keep their OWN log sinks (separate from the
// global `ggml_log` that `send_logs_to_tracing` silences) and default to writing
// every model-load/tensor/warmup line straight to stderr. `mtmd_helper_log_set`
// redirects all of them (clip, mtmd, and the mtmd helper) through our callback.
// It is an `extern "C"` symbol in libmtmd (declared in mtmd-helper.h) but not
// exposed by `llama-cpp-sys-2`, so bind it directly.
type GgmlLogCallback = Option<unsafe extern "C" fn(c_int, *const c_char, *mut c_void)>;

unsafe extern "C" {
    fn mtmd_helper_log_set(callback: GgmlLogCallback, user_data: *mut c_void);
}

unsafe extern "C" fn noop_log(_level: c_int, _text: *const c_char, _user_data: *mut c_void) {}
use serde::Serialize;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::os::raw::{c_char, c_int, c_void};

const CTX: u32 = 4096;
const NONZERO_CTX: NonZeroU32 = NonZeroU32::new(CTX).expect("CTX is nonzero");
const N_BATCH: i32 = 512;
const MAX_NEW_TOKENS: i32 = 1024;
const LOC_BINS: f32 = 1000.0;

const FIELD_TRANSCRIBE: &str = "\"regions\": an array of {\"text\": string, \"quad\": [x1,y1,x2,y2,x3,y3,x4,y4]} for each run of visible text, where the quad is its bounding quadrilateral with coordinates normalized to 0-1000 relative to image width and height (omit the quad if you cannot localize the text)";
const FIELD_CAPTION: &str = "\"caption\": a single paragraph describing the image";
const FIELD_OBJECTS: &str = "\"objects\": an array of {\"label\": string, \"bbox\": [x1,y1,x2,y2]} for the salient objects, where bbox is the axis-aligned bounding box with coordinates normalized to 0-1000 relative to image width and height";

/// Text recognized in an image, with per-region bounding quads.
#[derive(Debug, Serialize)]
pub(crate) struct OcrText {
    text: String,
    regions: Vec<OcrRegion>,
}

/// One recognized text run with its bounding quad `[x1,y1,x2,y2,x3,y3,x4,y4]`.
#[derive(Debug, Serialize)]
pub(crate) struct OcrRegion {
    text: String,
    quad: [i32; 8],
}

/// A detected object with its category label and bounding box `[x1,y1,x2,y2]`.
#[derive(Debug, Serialize)]
pub(crate) struct DetectedObject {
    label: String,
    bbox: [i32; 4],
}

/// Which vision tasks to run against an image.
#[derive(Clone, Copy)]
pub(crate) struct Request {
    pub(crate) transcribe: bool,
    pub(crate) caption: bool,
    pub(crate) objects: bool,
}

/// Results of the requested vision tasks.
#[derive(Default)]
pub(crate) struct Analysis {
    pub(crate) transcribe: Option<OcrText>,
    pub(crate) caption: Option<String>,
    pub(crate) objects: Option<Vec<DetectedObject>>,
}

/// Run the requested tasks against `image_bytes` in a single pass: the image is
/// encoded once and one combined prompt requests all selected fields, which are
/// parsed back into the per-task results.
pub(crate) fn analyze(image_bytes: &[u8], request: Request) -> Result<Analysis> {
    llama_cpp_2::send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
    // Safe: installs a no-op log callback; only suppresses stderr diagnostics.
    unsafe { mtmd_helper_log_set(Some(noop_log), std::ptr::null_mut()) };
    let backend = LlamaBackend::init()?;
    let model_path = crate::engine::model::file("SmolVLM-500M-Instruct-Q8_0.gguf")?;
    let mmproj_path = crate::engine::model::file("mmproj-SmolVLM-500M-Instruct-f16.gguf")?;
    let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())?;

    let n_threads = num_cpus::get_physical() as i32;
    let mtmd_params = MtmdContextParams {
        use_gpu: false,
        print_timings: false,
        n_threads,
        media_marker: CString::new(llama_cpp_2::mtmd::mtmd_default_marker())
            .context("media marker contains null")?,
        image_min_tokens: -1,
        image_max_tokens: -1,
    };
    let mtmd_ctx =
        MtmdContext::init_from_file(&mmproj_path.to_string_lossy(), &model, &mtmd_params)?;
    let chat_template = model.chat_template(None)?;
    let bitmap = MtmdBitmap::from_buffer(&mtmd_ctx, image_bytes, false)?;
    let width = bitmap.nx();
    let height = bitmap.ny();

    let context_params = LlamaContextParams::default()
        .with_n_threads(n_threads)
        .with_n_threads_batch(n_threads)
        .with_n_batch(N_BATCH.try_into()?)
        .with_n_ctx(Some(NONZERO_CTX));
    let mut context = model.new_context(&backend, context_params)?;

    let prompt = build_prompt(request);
    let raw = generate(
        &model,
        &mtmd_ctx,
        &chat_template,
        &mut context,
        &bitmap,
        &prompt,
    )?;
    Ok(parse_analysis(&raw, request, width, height))
}

/// Build a combined instruction requesting exactly the selected fields in one
/// JSON object, so the image is encoded once for all tasks.
fn build_prompt(request: Request) -> String {
    let mut fields = Vec::new();
    if request.transcribe {
        fields.push(FIELD_TRANSCRIBE);
    }
    if request.caption {
        fields.push(FIELD_CAPTION);
    }
    if request.objects {
        fields.push(FIELD_OBJECTS);
    }
    format!(
        "Analyze the image and respond with a single JSON object containing {}. Output only the JSON object.",
        fields.join(", ")
    )
}

/// Greedily decode the model's answer for `prompt` given the image bitmap,
/// returning the generated text.
fn generate(
    model: &LlamaModel,
    mtmd_ctx: &MtmdContext,
    chat_template: &LlamaChatTemplate,
    context: &mut LlamaContext,
    bitmap: &MtmdBitmap,
    prompt: &str,
) -> Result<String> {
    let marker = llama_cpp_2::mtmd::mtmd_default_marker();
    let full_prompt = if prompt.contains(marker) {
        prompt.to_string()
    } else {
        format!("{prompt}{marker}")
    };
    let msg = LlamaChatMessage::new("user".to_string(), full_prompt)?;
    let formatted = model.apply_chat_template(chat_template, &[msg], true)?;
    let input = MtmdInputText {
        text: formatted,
        add_special: true,
        parse_special: true,
    };
    let chunks = mtmd_ctx.tokenize(input, &[bitmap])?;

    let mut batch = LlamaBatch::new(1, 1);
    let n_past_start = chunks.eval_chunks(mtmd_ctx, context, 0, 0, N_BATCH, true)?;

    let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
    let mut output = String::new();
    let mut decoder = UTF_8.new_decoder();
    for n_past in (n_past_start..).take(MAX_NEW_TOKENS as usize) {
        let token = sampler.sample(context, -1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = model.token_to_piece(token, &mut decoder, true, None)?;
        output.push_str(&piece);
        batch.clear();
        batch.add(token, n_past, &[0], true)?;
        context.decode(&mut batch)?;
    }
    Ok(output)
}

/// Convert a normalized 0-1000 coordinate to a pixel coordinate.
fn loc_to_px(loc: i32, dim: u32) -> i32 {
    ((loc as f32 + 0.5) / LOC_BINS * dim as f32).round() as i32
}

/// Extract the first JSON object or array substring from `raw`, tolerating
/// prose before/after the JSON.
fn extract_json(raw: &str) -> Option<&str> {
    let (open, close) = if raw.contains('{') {
        ('{', '}')
    } else if raw.contains('[') {
        ('[', ']')
    } else {
        return None;
    };
    let start = raw.find(open)?;
    let end = raw.rfind(close)?;
    if end >= start {
        Some(&raw[start..=end])
    } else {
        None
    }
}

/// JSON shape of one OCR region in the model's response.
#[derive(serde::Deserialize)]
struct RegionJson {
    text: String,
    quad: Vec<i32>,
}

/// JSON shape of one detected object in the model's response.
#[derive(serde::Deserialize)]
struct ObjectJson {
    label: String,
    bbox: Vec<i32>,
}

/// Combined JSON object returned for a single multi-task prompt.
#[derive(Default, serde::Deserialize)]
struct CombinedJson {
    regions: Option<Vec<RegionJson>>,
    caption: Option<String>,
    objects: Option<Vec<ObjectJson>>,
}

/// Parse the combined response, filling in only the requested fields.
fn parse_analysis(raw: &str, request: Request, width: u32, height: u32) -> Analysis {
    let parsed = extract_json(raw)
        .and_then(|json| serde_json::from_str::<CombinedJson>(json).ok())
        .unwrap_or_else(|| {
            log::warn!("vision JSON parse failed, returning empty results");
            CombinedJson::default()
        });

    let mut analysis = Analysis::default();
    if request.transcribe {
        analysis.transcribe = Some(build_ocr(parsed.regions.unwrap_or_default(), width, height));
    }
    if request.caption {
        analysis.caption = Some(strip_special(&parsed.caption.unwrap_or_default()));
    }
    if request.objects {
        analysis.objects = Some(build_objects(
            parsed.objects.unwrap_or_default(),
            width,
            height,
        ));
    }
    analysis
}

/// Convert parsed OCR regions into pixel-space [`OcrText`]. A `quad` may be a
/// full quadrilateral `[x1,y1,...,x4,y4]` or an axis-aligned box `[x1,y1,x2,y2]`,
/// which is expanded to its four corners.
fn build_ocr(regions: Vec<RegionJson>, width: u32, height: u32) -> OcrText {
    let mut parsed = Vec::new();
    for region in regions {
        if region.text.is_empty() {
            continue;
        }
        let corners = match *region.quad.as_slice() {
            [x1, y1, x2, y2] => [x1, y1, x2, y1, x2, y2, x1, y2],
            [x1, y1, x2, y2, x3, y3, x4, y4] => [x1, y1, x2, y2, x3, y3, x4, y4],
            _ => continue,
        };
        let mut quad = [0i32; 8];
        for (i, &loc) in corners.iter().enumerate() {
            quad[i] = loc_to_px(loc, if i % 2 == 0 { width } else { height });
        }
        parsed.push(OcrRegion {
            text: region.text,
            quad,
        });
    }
    let text = parsed
        .iter()
        .map(|region| region.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    OcrText {
        text,
        regions: parsed,
    }
}

/// Convert parsed objects into pixel-space [`DetectedObject`]s.
fn build_objects(objects: Vec<ObjectJson>, width: u32, height: u32) -> Vec<DetectedObject> {
    objects
        .into_iter()
        .filter(|object| !object.label.is_empty() && object.bbox.len() == 4)
        .map(|object| DetectedObject {
            label: object.label,
            bbox: [
                loc_to_px(object.bbox[0], width),
                loc_to_px(object.bbox[1], height),
                loc_to_px(object.bbox[2], width),
                loc_to_px(object.bbox[3], height),
            ],
        })
        .collect()
}

/// Remove special tokens and trim whitespace from generated text.
fn strip_special(raw: &str) -> String {
    raw.replace("<|im_end|>", "").trim().to_string()
}
