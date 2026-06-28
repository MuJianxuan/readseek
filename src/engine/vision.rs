// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Qwen2.5-VL vision model: OCR (with regions), captioning, and object
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
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::mtmd::{MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::LogOptions;

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
const N_THREADS: i32 = 4;
const N_BATCH: i32 = 512;
const MAX_NEW_TOKENS: i32 = 1024;
const LOC_BINS: f32 = 1000.0;

const PROMPT_TRANSCRIBE: &str = "Read all text visible in the image. Output a JSON object with \"regions\": an array of {\"text\": string, \"quad\": [x1,y1,x2,y2,x3,y3,x4,y4]} where the quad is the bounding quadrilateral of each text run, coordinates normalized to the range 0-1000 relative to image width/height. If you cannot localize a region, omit the quad. Output only the JSON.";
const PROMPT_CAPTION: &str = "Describe with a paragraph what is shown in the image.";
const PROMPT_OBJECTS: &str = "Locate the objects in the image. Output a JSON array of {\"label\": string, \"bbox\": [x1,y1,x2,y2]} where bbox is the axis-aligned bounding box, coordinates normalized to 0-1000 relative to image width/height. Output only the JSON array.";

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

/// Run the requested tasks against `image_bytes`, loading the model once.
/// A fresh context is created per task so the KV cache starts clean.
pub(crate) fn analyze(image_bytes: &[u8], request: Request) -> Result<Analysis> {
    llama_cpp_2::send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
    // Safe: installs a no-op log callback; only suppresses stderr diagnostics.
    unsafe { mtmd_helper_log_set(Some(noop_log), std::ptr::null_mut()) };
    let backend = LlamaBackend::init()?;
    let model_path = crate::engine::model::file("Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf")?;
    let mmproj_path = crate::engine::model::file("mmproj-F16.gguf")?;
    let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())?;

    let mtmd_params = MtmdContextParams {
        use_gpu: false,
        print_timings: false,
        n_threads: N_THREADS,
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

    let mut analysis = Analysis::default();
    if request.transcribe {
        let raw = generate(
            &backend,
            &model,
            &mtmd_ctx,
            &chat_template,
            &bitmap,
            PROMPT_TRANSCRIBE,
        )?;
        analysis.transcribe = Some(parse_ocr(&raw, width, height));
    }
    if request.caption {
        let raw = generate(
            &backend,
            &model,
            &mtmd_ctx,
            &chat_template,
            &bitmap,
            PROMPT_CAPTION,
        )?;
        analysis.caption = Some(strip_special(&raw));
    }
    if request.objects {
        let raw = generate(
            &backend,
            &model,
            &mtmd_ctx,
            &chat_template,
            &bitmap,
            PROMPT_OBJECTS,
        )?;
        analysis.objects = Some(parse_objects(&raw, width, height));
    }
    Ok(analysis)
}

/// Greedily decode the model's answer for `prompt` given the image bitmap,
/// returning the generated text.
fn generate(
    backend: &LlamaBackend,
    model: &LlamaModel,
    mtmd_ctx: &MtmdContext,
    chat_template: &LlamaChatTemplate,
    bitmap: &MtmdBitmap,
    prompt: &str,
) -> Result<String> {
    let context_params = LlamaContextParams::default()
        .with_n_threads(N_THREADS)
        .with_n_batch(N_BATCH.try_into()?)
        .with_n_ctx(Some(NONZERO_CTX));
    let mut context = model.new_context(backend, context_params)?;

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

    let mut batch = LlamaBatch::new(CTX as usize, 1);
    let n_past_start = chunks.eval_chunks(mtmd_ctx, &context, 0, 0, N_BATCH, true)?;

    let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
    let mut output = String::new();
    let mut decoder = UTF_8.new_decoder();
    for n_past in (n_past_start..).take(MAX_NEW_TOKENS as usize) {
        let token = sampler.sample(&context, -1);
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

fn parse_ocr(raw: &str, width: u32, height: u32) -> OcrText {
    #[derive(serde::Deserialize)]
    struct RegionJson {
        text: String,
        quad: Vec<i32>,
    }
    #[derive(serde::Deserialize)]
    struct TranscribeJson {
        regions: Vec<RegionJson>,
    }

    let json_str = extract_json(raw).unwrap_or(raw);
    if let Ok(parsed) = serde_json::from_str::<TranscribeJson>(json_str) {
        let mut regions = Vec::new();
        for region in parsed.regions {
            if region.text.is_empty() || region.quad.len() != 8 {
                continue;
            }
            let mut quad = [0i32; 8];
            for (i, &loc) in region.quad.iter().enumerate() {
                quad[i] = loc_to_px(loc, if i % 2 == 0 { width } else { height });
            }
            regions.push(OcrRegion {
                text: region.text,
                quad,
            });
        }
        let text = regions
            .iter()
            .map(|region| region.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return OcrText { text, regions };
    }

    log::warn!("transcribe JSON parse failed, using raw text");
    let cleaned = strip_special(raw);
    let full_quad = [
        0,
        0,
        width as i32,
        0,
        width as i32,
        height as i32,
        0,
        height as i32,
    ];
    OcrText {
        text: cleaned.clone(),
        regions: vec![OcrRegion {
            text: cleaned,
            quad: full_quad,
        }],
    }
}

fn parse_objects(raw: &str, width: u32, height: u32) -> Vec<DetectedObject> {
    #[derive(serde::Deserialize)]
    struct ObjectJson {
        label: String,
        bbox: Vec<i32>,
    }

    let json_str = extract_json(raw).unwrap_or(raw);
    match serde_json::from_str::<Vec<ObjectJson>>(json_str) {
        Ok(parsed) => parsed
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
            .collect(),
        Err(error) => {
            log::warn!("objects JSON parse failed: {error}");
            Vec::new()
        }
    }
}

/// Remove special tokens and trim whitespace from generated text.
fn strip_special(raw: &str) -> String {
    raw.replace("<|im_end|>", "").trim().to_string()
}
