// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Qwen3-VL image analysis through llama.cpp. One multimodal model handles
//! captioning, object detection, and OCR; its GGUF files are fetched lazily into
//! the user cache directory (see [`crate::engine::model`]).

// Coordinate conversion from normalized model output to pixels is bounded by
// the image dimensions.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::cell::{OnceCell, RefCell};
use std::ffi::CString;
use std::io::IsTerminal as _;
use std::num::NonZeroU32;
use std::os::raw::{c_char, c_int, c_void};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use encoding_rs::UTF_8;
use indicatif::ProgressBar;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::mtmd::{MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;
use serde::{Deserialize, Serialize};

const MODEL_FILE: &str = "Qwen3VL-2B-Instruct-Q4_K_M.gguf";
const MMPROJ_FILE: &str = "mmproj-Qwen3VL-2B-Instruct-F16.gguf";
const CONTEXT_SIZE: u32 = 8192;
const CONTEXT_SIZE_NONZERO: NonZeroU32 =
    NonZeroU32::new(CONTEXT_SIZE).expect("context size is nonzero");
const BATCH_SIZE: i32 = 512;
const MAX_NEW_TOKENS: i32 = 4096;
/// Keep enough context cells for the prompt and dense JSON output.
const IMAGE_MAX_TOKENS: i32 = 2048;
const LOCATION_BINS: f32 = 1000.0;
const PROGRESS_DEADLINE: Duration = Duration::from_secs(2);
const PROGRESS_TICK: Duration = Duration::from_millis(100);
const PROGRESS_MSG: &str = "Analyzing image...";

const FIELD_CAPTION: &str = "\"caption\": a single paragraph describing the image";
const FIELD_OBJECTS: &str = "\"objects\": an array of {\"label\": string, \"bbox\": [x1,y1,x2,y2]} for the salient objects, where bbox is an axis-aligned box with integer coordinates normalized to 0-1000 relative to image width and height";
const FIELD_OCR: &str =
    "\"ocr\": a string containing all visible text in reading order, preserving line breaks";

// llama.cpp's mtmd helper has a log sink separate from the backend logger. The
// callback setter is exported by libmtmd but not wrapped by llama-cpp-sys-2.
type GgmlLogCallback = Option<unsafe extern "C" fn(c_int, *const c_char, *mut c_void)>;

unsafe extern "C" {
    fn mtmd_helper_log_set(callback: GgmlLogCallback, user_data: *mut c_void);
}

unsafe extern "C" fn noop_log(_level: c_int, _text: *const c_char, _user_data: *mut c_void) {}

/// A detected object with its category label and bounding box `[x1,y1,x2,y2]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DetectedObject {
    label: String,
    bbox: [i32; 4],
}

/// Which vision tasks to run against an image.
#[derive(Clone, Copy)]
pub(crate) struct Request {
    pub(crate) caption: bool,
    pub(crate) objects: bool,
    pub(crate) ocr: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Input<'a> {
    Encoded(&'a [u8]),
    Rgb {
        width: u32,
        height: u32,
        pixels: &'a [u8],
    },
}

impl Input<'_> {
    pub(crate) fn cache_hash(self) -> String {
        match self {
            Self::Encoded(bytes) => crate::engine::hash::hash_bytes(bytes),
            Self::Rgb {
                width,
                height,
                pixels,
            } => {
                let mut hasher = blake3::Hasher::new();
                hasher.update(b"readseek-rgb\0");
                hasher.update(&width.to_le_bytes());
                hasher.update(&height.to_le_bytes());
                hasher.update(pixels);
                hasher.finalize().to_string()
            }
        }
    }
}

/// Results of the requested vision tasks.
#[derive(Default)]
pub(crate) struct Analysis {
    pub(crate) caption: Option<String>,
    pub(crate) objects: Option<Vec<DetectedObject>>,
    pub(crate) ocr: Option<String>,
}

/// Loaded model state shared across image analyses in one process.
///
/// Fields are declared in dependency drop order. The model is process-lifetime
/// storage so the reusable context can safely borrow it.
struct VisionRuntime {
    context: LlamaContext<'static>,
    mtmd: MtmdContext,
    chat_template: LlamaChatTemplate,
    model: &'static LlamaModel,
    _backend: LlamaBackend,
}

impl VisionRuntime {
    fn load() -> Result<Self> {
        let mut backend = LlamaBackend::init().context("initialize llama.cpp")?;
        backend.void_logs();
        // Safe: this installs a no-op logger and does not retain user data.
        unsafe { mtmd_helper_log_set(Some(noop_log), std::ptr::null_mut()) };

        let model_path = crate::engine::model::file(MODEL_FILE)?;
        let mmproj_path = crate::engine::model::file(MMPROJ_FILE)?;
        let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
            .context("load Qwen3-VL model")?;
        let model = Box::leak(Box::new(model));
        let threads = inference_threads()?;
        let context_params = LlamaContextParams::default()
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            .with_n_batch(BATCH_SIZE.try_into()?)
            .with_n_ctx(Some(CONTEXT_SIZE_NONZERO));
        let context = model
            .new_context(&backend, context_params)
            .context("create Qwen3-VL context")?;
        let params = MtmdContextParams {
            use_gpu: backend.supports_gpu_offload(),
            print_timings: false,
            n_threads: inference_threads()?,
            media_marker: CString::new(llama_cpp_2::mtmd::mtmd_default_marker())
                .context("media marker contains null")?,
            image_min_tokens: -1,
            image_max_tokens: IMAGE_MAX_TOKENS,
        };
        let mtmd = MtmdContext::init_from_file(&mmproj_path.to_string_lossy(), model, &params)
            .context("load Qwen3-VL multimodal projector")?;
        let chat_template = model.chat_template(None)?;
        Ok(Self {
            context,
            mtmd,
            chat_template,
            model,
            _backend: backend,
        })
    }
}

thread_local! {
    static RUNTIME: OnceCell<Result<RefCell<VisionRuntime>, String>> = const { OnceCell::new() };
}

fn with_runtime<T>(run: impl FnOnce(&mut VisionRuntime) -> Result<T>) -> Result<T> {
    RUNTIME.with(|slot| {
        let runtime = slot.get_or_init(|| {
            VisionRuntime::load()
                .map(RefCell::new)
                .map_err(|error| error.to_string())
        });
        let runtime = runtime.as_ref().map_err(|error| anyhow!(error.clone()))?;
        let mut runtime = runtime
            .try_borrow_mut()
            .map_err(|_| anyhow!("vision runtime is already in use"))?;
        run(&mut runtime)
    })
}

/// Run the selected tasks in one multimodal generation pass. The loaded model
/// is reused by later images, which matters for PDFs containing several images.
pub(crate) fn analyze(input: Input<'_>, request: Request) -> Result<Analysis> {
    if !request.caption && !request.objects && !request.ocr {
        return Ok(Analysis::default());
    }

    let embedded_ocr = match input {
        Input::Encoded(bytes) if request.ocr => crate::engine::image::embedded_drawio_text(bytes),
        _ => None,
    };
    let model_request = Request {
        caption: request.caption,
        objects: request.objects,
        ocr: request.ocr && embedded_ocr.is_none(),
    };
    if !model_request.caption && !model_request.objects && !model_request.ocr {
        return Ok(Analysis {
            ocr: embedded_ocr,
            ..Analysis::default()
        });
    }

    let (raw, width, height) = with_runtime(|runtime| {
        let bitmap = match input {
            Input::Encoded(bytes) => MtmdBitmap::from_buffer(&runtime.mtmd, bytes, false)
                .context("decode image for Qwen3-VL")?,
            Input::Rgb {
                width,
                height,
                pixels,
            } => MtmdBitmap::from_image_data(width, height, pixels)
                .context("load RGB image for Qwen3-VL")?,
        };
        let width = bitmap.nx();
        let height = bitmap.ny();
        let raw = generate(runtime, &bitmap, &build_prompt(model_request))?;
        Ok((raw, width, height))
    })?;
    let mut analysis = parse_analysis(&raw, model_request, width, height);
    if embedded_ocr.is_some() {
        analysis.ocr = embedded_ocr;
    }
    Ok(analysis)
}

fn build_prompt(request: Request) -> String {
    let mut fields = Vec::new();
    if request.caption {
        fields.push(FIELD_CAPTION);
    }
    if request.objects {
        fields.push(FIELD_OBJECTS);
    }
    if request.ocr {
        fields.push(FIELD_OCR);
    }
    format!(
        "Analyze the image and respond with a single JSON object containing {}. Output only the JSON object.",
        fields.join(", ")
    )
}

/// Greedily decode one response while keeping image, prompt, and output within
/// the context window.
fn generate(runtime: &mut VisionRuntime, bitmap: &MtmdBitmap, prompt: &str) -> Result<String> {
    let marker = llama_cpp_2::mtmd::mtmd_default_marker();
    let message = LlamaChatMessage::new("user".to_owned(), format!("{prompt}{marker}"))?;
    let formatted = runtime
        .model
        .apply_chat_template(&runtime.chat_template, &[message], true)?;
    let input = MtmdInputText {
        text: formatted,
        add_special: true,
        parse_special: true,
    };
    let chunks = runtime.mtmd.tokenize(input, &[bitmap])?;

    let mut progress = InferenceProgress::new();
    runtime.context.clear_kv_cache();
    let n_past = chunks.eval_chunks(&runtime.mtmd, &runtime.context, 0, 0, BATCH_SIZE, true)?;
    progress.maybe_reveal();
    let budget = (CONTEXT_SIZE as i32 - chunks.total_tokens() as i32).clamp(0, MAX_NEW_TOKENS);
    decode_tokens(
        runtime.model,
        &mut runtime.context,
        n_past,
        budget,
        &mut progress,
    )
}

fn decode_tokens(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    n_past: i32,
    budget: i32,
    progress: &mut InferenceProgress,
) -> Result<String> {
    let mut batch = LlamaBatch::new(1, 1);
    let mut sampler = LlamaSampler::greedy();
    let mut decoder = UTF_8.new_decoder();
    let mut output = String::with_capacity(budget as usize * 4);
    let mut json = JsonCompletion::default();

    for position in (n_past..).take(budget as usize) {
        progress.maybe_reveal();
        let token = sampler.sample(context, -1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = model.token_to_piece(token, &mut decoder, true, None)?;
        output.push_str(&piece);
        if json.push(&piece) {
            return Ok(output);
        }
        batch.clear();
        batch.add(token, position, &[0], true)?;
        context.decode(&mut batch)?;
    }
    output.reserve(4);
    let (_, _, had_errors) = decoder.decode_to_string(b"", &mut output, true);
    if had_errors {
        return Err(anyhow!("vision response ended with invalid UTF-8"));
    }
    Ok(output)
}

#[derive(Default)]
struct JsonCompletion {
    depth: usize,
    escaped: bool,
    in_string: bool,
    started: bool,
}

impl JsonCompletion {
    fn push(&mut self, text: &str) -> bool {
        for character in text.chars() {
            if self.escaped {
                self.escaped = false;
                continue;
            }
            if self.in_string {
                match character {
                    '\\' => self.escaped = true,
                    '"' => self.in_string = false,
                    _ => {}
                }
                continue;
            }
            match character {
                '"' if self.started => self.in_string = true,
                '{' => {
                    self.started = true;
                    self.depth += 1;
                }
                '}' if self.started => {
                    self.depth = self.depth.saturating_sub(1);
                    if self.depth == 0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
}

#[derive(serde::Deserialize)]
struct ObjectJson {
    label: String,
    bbox: Vec<i32>,
}

#[derive(Default, serde::Deserialize)]
struct CombinedJson {
    caption: Option<String>,
    objects: Option<Vec<ObjectJson>>,
    ocr: Option<String>,
}

fn parse_analysis(raw: &str, request: Request, width: u32, height: u32) -> Analysis {
    let parsed = extract_json(raw)
        .and_then(|json| serde_json::from_str::<CombinedJson>(json).ok())
        .unwrap_or_else(|| {
            log::warn!("vision JSON parse failed, returning empty results");
            CombinedJson::default()
        });

    Analysis {
        caption: request
            .caption
            .then(|| parsed.caption.map(|value| strip_special(&value)))
            .flatten(),
        objects: request
            .objects
            .then(|| {
                parsed
                    .objects
                    .map(|objects| build_objects(objects, width, height))
            })
            .flatten(),
        ocr: request
            .ocr
            .then(|| parsed.ocr.map(|value| strip_special(&value)))
            .flatten(),
    }
}

fn build_objects(objects: Vec<ObjectJson>, width: u32, height: u32) -> Vec<DetectedObject> {
    objects
        .into_iter()
        .filter_map(|object| {
            if object.label.is_empty() || object.bbox.len() != 4 {
                return None;
            }
            let bbox = [
                location_to_pixel(object.bbox[0], width),
                location_to_pixel(object.bbox[1], height),
                location_to_pixel(object.bbox[2], width),
                location_to_pixel(object.bbox[3], height),
            ];
            (bbox[0] < bbox[2] && bbox[1] < bbox[3]).then_some(DetectedObject {
                label: object.label,
                bbox,
            })
        })
        .collect()
}

fn location_to_pixel(location: i32, dimension: u32) -> i32 {
    let location = location.clamp(0, LOCATION_BINS as i32);
    ((location as f32 + 0.5) / LOCATION_BINS * dimension as f32).round() as i32
}

fn extract_json(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    (end >= start).then_some(&raw[start..=end])
}

fn strip_special(raw: &str) -> String {
    raw.replace("<|im_end|>", "").trim().to_owned()
}

fn inference_threads() -> Result<i32> {
    let threads = std::thread::available_parallelism()
        .context("detect available parallelism")?
        .get();
    i32::try_from(threads).context("thread count exceeds i32")
}

struct InferenceProgress {
    is_tty: bool,
    started: Instant,
    bar: Option<ProgressBar>,
}

impl InferenceProgress {
    fn new() -> Self {
        Self {
            is_tty: std::io::stderr().is_terminal(),
            started: Instant::now(),
            bar: None,
        }
    }

    fn maybe_reveal(&mut self) {
        if self.bar.is_some() || !self.is_tty || self.started.elapsed() < PROGRESS_DEADLINE {
            return;
        }
        let bar = ProgressBar::new_spinner();
        bar.set_message(PROGRESS_MSG);
        bar.enable_steady_tick(PROGRESS_TICK);
        self.bar = Some(bar);
    }
}

impl Drop for InferenceProgress {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
    }
}
