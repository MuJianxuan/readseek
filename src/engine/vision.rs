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
use std::env;
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
const MMPROJ_FILE: &str = "mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf";
const CONTEXT_SIZE: u32 = 8192;
const CONTEXT_SIZE_NONZERO: NonZeroU32 =
    NonZeroU32::new(CONTEXT_SIZE).expect("context size is nonzero");
const DEFAULT_BATCH_SIZE: u32 = 512;
const DEFAULT_UBATCH_SIZE: u32 = 512;
const CAPTION_MAX_NEW_TOKENS: i32 = 512;
const OBJECTS_MAX_NEW_TOKENS: i32 = 1024;
const OCR_MAX_NEW_TOKENS: i32 = 4096;
const LOCATION_BINS: i32 = 1000;
const PROGRESS_DEADLINE: Duration = Duration::from_secs(2);
const PROGRESS_TICK: Duration = Duration::from_millis(100);
const PROGRESS_MSG: &str = "Analyzing image...";

const FIELD_CAPTION: &str =
    "\"caption\": one concise paragraph of at most 100 words describing the image";
const FIELD_OBJECTS: &str = "\"objects\": an array of at most 32 {\"label\": string, \"bbox\": [x1,y1,x2,y2]} entries for the salient objects, where bbox is an axis-aligned box with integer coordinates normalized to 0-1000 relative to image width and height";
const FIELD_OCR: &str =
    "\"ocr\": a string containing all visible text in reading order, preserving line breaks";

// llama.cpp's mtmd helper has a log sink separate from the backend logger. The
// callback setter is exported by libmtmd but not wrapped by llama-cpp-sys-2.
type GgmlLogCallback = Option<unsafe extern "C" fn(c_int, *const c_char, *mut c_void)>;

unsafe extern "C" {
    fn mtmd_helper_log_set(callback: GgmlLogCallback, user_data: *mut c_void);
}

unsafe extern "C" fn noop_log(_level: c_int, _text: *const c_char, _user_data: *mut c_void) {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VisionProfile {
    Fast,
    #[default]
    Balanced,
    Accurate,
}

impl VisionProfile {
    fn image_max_tokens(self, request: Request) -> i32 {
        if self == Self::Accurate {
            return 2048;
        }

        let (caption, objects, ocr) = match self {
            Self::Fast => (512, 768, 1024),
            Self::Balanced => (768, 1024, 1536),
            Self::Accurate => unreachable!(),
        };
        [
            request.caption.then_some(caption),
            request.objects.then_some(objects),
            request.ocr.then_some(ocr),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct InferenceMetrics {
    profile: VisionProfile,
    image_max_tokens: i32,
    prompt_tokens: usize,
    generated_tokens: usize,
    threads: i32,
    batch_size: u32,
    micro_batch_size: u32,
    gpu_offload_supported: bool,
    backend_devices: Vec<String>,
    startup_ms: u128,
    bitmap_ms: u128,
    tokenize_ms: u128,
    prefill_ms: u128,
    decode_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    prefill_tokens_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decode_tokens_per_second: Option<f64>,
    total_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_rss_bytes: Option<u64>,
}

pub(crate) struct InferenceResult {
    pub(crate) analysis: Analysis,
    pub(crate) metrics: Option<InferenceMetrics>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BenchmarkReport {
    kind: &'static str,
    profile: VisionProfile,
    request: Request,
    iterations: usize,
    warmup: InferenceMetrics,
    runs: Vec<InferenceMetrics>,
    p50_ms: u128,
    p95_ms: u128,
}

/// A detected object with its category label and bounding box `[x1,y1,x2,y2]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DetectedObject {
    label: String,
    bbox: [i32; 4],
}

/// Which vision tasks to run against an image.
#[derive(Clone, Copy, Debug, Serialize)]
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
    mmproj_path: std::path::PathBuf,
    image_max_tokens: i32,
    threads: i32,
    batch_size: u32,
    micro_batch_size: u32,
    gpu_offload_supported: bool,
    backend_devices: Vec<String>,
    unreported_startup_ms: Option<u128>,
}

impl VisionRuntime {
    fn load(image_max_tokens: i32) -> Result<Self> {
        let started = Instant::now();
        let mut backend = LlamaBackend::init().context("initialize llama.cpp")?;
        backend.void_logs();
        // Safe: this installs a no-op logger and does not retain user data.
        unsafe { mtmd_helper_log_set(Some(noop_log), std::ptr::null_mut()) };

        let gpu_offload_supported = backend.supports_gpu_offload();
        let backend_devices = llama_cpp_2::list_llama_ggml_backend_devices()
            .into_iter()
            .map(|device| {
                format!(
                    "{} ({}, {}, {} MiB free)",
                    device.name,
                    device.backend,
                    device.description,
                    device.memory_free / (1024 * 1024)
                )
            })
            .collect();
        let model_path = crate::engine::model::file(MODEL_FILE)?;
        let mmproj_path = crate::engine::model::file(MMPROJ_FILE)?;
        let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
            .context("load Qwen3-VL model")?;
        let model = Box::leak(Box::new(model));
        let threads = inference_threads()?;
        let batch_size = inference_batch_size()?;
        let micro_batch_size = inference_micro_batch_size(batch_size)?;
        let context_params = LlamaContextParams::default()
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            .with_n_batch(batch_size)
            .with_n_ubatch(micro_batch_size)
            .with_no_perf(false)
            .with_n_ctx(Some(CONTEXT_SIZE_NONZERO));
        let context = model
            .new_context(&backend, context_params)
            .context("create Qwen3-VL context")?;
        let params = mtmd_params(gpu_offload_supported, threads, image_max_tokens)?;
        let mtmd = MtmdContext::init_from_file(&mmproj_path.to_string_lossy(), model, &params)
            .context("load Qwen3-VL multimodal projector")?;
        let chat_template = model.chat_template(None)?;
        Ok(Self {
            context,
            mtmd,
            chat_template,
            model,
            _backend: backend,
            mmproj_path,
            image_max_tokens,
            threads,
            batch_size,
            micro_batch_size,
            gpu_offload_supported,
            backend_devices,
            unreported_startup_ms: Some(started.elapsed().as_millis()),
        })
    }

    fn set_image_max_tokens(&mut self, image_max_tokens: i32) -> Result<()> {
        if self.image_max_tokens == image_max_tokens {
            return Ok(());
        }

        let params = mtmd_params(self.gpu_offload_supported, self.threads, image_max_tokens)?;
        let mtmd =
            MtmdContext::init_from_file(&self.mmproj_path.to_string_lossy(), self.model, &params)
                .context("reload Qwen3-VL multimodal projector")?;
        self.mtmd = mtmd;
        self.image_max_tokens = image_max_tokens;
        Ok(())
    }
}

fn mtmd_params(use_gpu: bool, threads: i32, image_max_tokens: i32) -> Result<MtmdContextParams> {
    Ok(MtmdContextParams {
        use_gpu,
        print_timings: false,
        n_threads: threads,
        media_marker: CString::new(llama_cpp_2::mtmd::mtmd_default_marker())
            .context("media marker contains null")?,
        image_min_tokens: -1,
        image_max_tokens,
    })
}

thread_local! {
    static RUNTIME: OnceCell<Result<RefCell<VisionRuntime>, String>> = const { OnceCell::new() };
}

fn with_runtime<T>(
    image_max_tokens: i32,
    run: impl FnOnce(&mut VisionRuntime) -> Result<T>,
) -> Result<T> {
    RUNTIME.with(|slot| {
        let runtime = slot.get_or_init(|| {
            VisionRuntime::load(image_max_tokens)
                .map(RefCell::new)
                .map_err(|error| error.to_string())
        });
        let runtime = runtime.as_ref().map_err(|error| anyhow!(error.clone()))?;
        let mut runtime = runtime
            .try_borrow_mut()
            .map_err(|_| anyhow!("vision runtime is already in use"))?;
        runtime.set_image_max_tokens(image_max_tokens)?;
        run(&mut runtime)
    })
}

/// Run the selected tasks in one multimodal generation pass. The loaded model
/// is reused by later images, which matters for PDFs containing several images.
pub(crate) fn analyze(
    input: Input<'_>,
    request: Request,
    profile: VisionProfile,
) -> Result<InferenceResult> {
    if !request.caption && !request.objects && !request.ocr {
        return Ok(InferenceResult {
            analysis: Analysis::default(),
            metrics: None,
        });
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
        return Ok(InferenceResult {
            analysis: Analysis {
                ocr: embedded_ocr,
                ..Analysis::default()
            },
            metrics: None,
        });
    }

    let image_max_tokens = profile.image_max_tokens(model_request);
    let (raw, width, height, metrics) = with_runtime(image_max_tokens, |runtime| {
        let total_started = Instant::now();
        let startup_ms = runtime.unreported_startup_ms.take().unwrap_or(0);
        let bitmap_started = Instant::now();
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
        let bitmap_ms = bitmap_started.elapsed().as_millis();
        let width = bitmap.nx();
        let height = bitmap.ny();
        let (raw, generation) = generate(runtime, &bitmap, model_request)?;
        let metrics = InferenceMetrics {
            profile,
            image_max_tokens,
            prompt_tokens: generation.prompt_tokens,
            generated_tokens: generation.generated_tokens,
            threads: runtime.threads,
            batch_size: runtime.batch_size,
            micro_batch_size: runtime.micro_batch_size,
            gpu_offload_supported: runtime.gpu_offload_supported,
            backend_devices: runtime.backend_devices.clone(),
            startup_ms,
            bitmap_ms,
            tokenize_ms: generation.tokenize_ms,
            prefill_ms: generation.prefill_ms,
            decode_ms: generation.decode_ms,
            prefill_tokens_per_second: tokens_per_second(
                generation.prompt_tokens,
                generation.prefill_ms,
            ),
            decode_tokens_per_second: tokens_per_second(
                generation.generated_tokens,
                generation.decode_ms,
            ),
            total_ms: total_started.elapsed().as_millis(),
            peak_rss_bytes: peak_rss_bytes(),
        };
        Ok((raw, width, height, metrics))
    })?;
    let mut analysis = parse_analysis(&raw, model_request, width, height)?;
    if embedded_ocr.is_some() {
        analysis.ocr = embedded_ocr;
    }
    Ok(InferenceResult {
        analysis,
        metrics: Some(metrics),
    })
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
fn generate(
    runtime: &mut VisionRuntime,
    bitmap: &MtmdBitmap,
    request: Request,
) -> Result<(String, GenerationMetrics)> {
    let prompt = build_prompt(request);
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
    let tokenize_started = Instant::now();
    let chunks = runtime.mtmd.tokenize(input, &[bitmap])?;
    let tokenize_ms = tokenize_started.elapsed().as_millis();
    let prompt_tokens = chunks.total_tokens();

    let mut progress = InferenceProgress::new();
    runtime.context.clear_kv_cache();
    let prefill_started = Instant::now();
    let n_past = chunks.eval_chunks(
        &runtime.mtmd,
        &runtime.context,
        0,
        0,
        runtime.batch_size.try_into()?,
        true,
    )?;
    let prefill_ms = prefill_started.elapsed().as_millis();
    progress.maybe_reveal();
    let available_tokens = CONTEXT_SIZE as i32 - prompt_tokens as i32;
    if available_tokens <= 0 {
        return Err(anyhow!(
            "vision prompt uses {prompt_tokens} tokens, exceeding the {CONTEXT_SIZE}-token context"
        ));
    }
    let budget = available_tokens.min(max_new_tokens(request));
    let decode_started = Instant::now();
    let (output, generated_tokens) = decode_tokens(
        runtime.model,
        &mut runtime.context,
        n_past,
        budget,
        request,
        &mut progress,
    )?;
    Ok((
        output,
        GenerationMetrics {
            prompt_tokens,
            generated_tokens,
            tokenize_ms,
            prefill_ms,
            decode_ms: decode_started.elapsed().as_millis(),
        },
    ))
}

fn decode_tokens(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    n_past: i32,
    budget: i32,
    request: Request,
    progress: &mut InferenceProgress,
) -> Result<(String, usize)> {
    let schema = serde_json::to_string(&json_schema(request))?;
    let grammar = llama_cpp_2::json_schema_to_grammar(&schema)
        .context("convert vision JSON schema to grammar")?;
    let grammar = LlamaSampler::grammar(model, &grammar, "root")
        .context("create vision JSON grammar sampler")?;
    let mut sampler = LlamaSampler::chain([grammar, LlamaSampler::greedy()], false);
    let mut batch = LlamaBatch::new(1, 1);
    let mut decoder = UTF_8.new_decoder();
    let mut output = String::with_capacity(budget as usize * 4);
    let mut json = JsonCompletion::default();
    let mut generated_tokens = 0;

    for position in (n_past..).take(budget as usize) {
        progress.maybe_reveal();
        let token = sampler.sample(context, -1);
        if model.is_eog_token(token) {
            break;
        }
        generated_tokens += 1;
        let piece = model.token_to_piece(token, &mut decoder, true, None)?;
        output.push_str(&piece);
        if json.push(&piece) {
            output.reserve(4);
            let (_, _, had_errors) = decoder.decode_to_string(b"", &mut output, true);
            if had_errors {
                return Err(anyhow!("vision response ended with invalid UTF-8"));
            }
            return Ok((output, generated_tokens));
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
    Err(anyhow!(
        "vision response ended before completing JSON after {generated_tokens} tokens"
    ))
}

struct GenerationMetrics {
    prompt_tokens: usize,
    generated_tokens: usize,
    tokenize_ms: u128,
    prefill_ms: u128,
    decode_ms: u128,
}

fn max_new_tokens(request: Request) -> i32 {
    [
        request.caption.then_some(CAPTION_MAX_NEW_TOKENS),
        request.objects.then_some(OBJECTS_MAX_NEW_TOKENS),
        request.ocr.then_some(OCR_MAX_NEW_TOKENS),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0)
}

fn json_schema(request: Request) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    if request.caption {
        properties.insert(
            "caption".to_owned(),
            serde_json::json!({ "type": "string" }),
        );
        required.push("caption");
    }
    if request.objects {
        properties.insert(
            "objects".to_owned(),
            serde_json::json!({
                "type": "array",
                "maxItems": 32,
                "items": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string" },
                        "bbox": {
                            "type": "array",
                            "minItems": 4,
                            "maxItems": 4,
                            "items": { "type": "integer", "minimum": 0, "maximum": 1000 }
                        }
                    },
                    "required": ["label", "bbox"],
                    "additionalProperties": false
                }
            }),
        );
        required.push("objects");
    }
    if request.ocr {
        properties.insert("ocr".to_owned(), serde_json::json!({ "type": "string" }));
        required.push("ocr");
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

pub(crate) fn benchmark(
    input: Input<'_>,
    request: Request,
    profile: VisionProfile,
    iterations: usize,
) -> Result<(Analysis, BenchmarkReport)> {
    if iterations == 0 {
        return Err(anyhow!(
            "vision benchmark iterations must be greater than zero"
        ));
    }
    let warmup = analyze(input, request, profile)?;
    let warmup_metrics = warmup
        .metrics
        .context("vision benchmark did not execute model inference")?;
    let mut analysis = warmup.analysis;
    let mut runs = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let result = analyze(input, request, profile)?;
        let metrics = result
            .metrics
            .context("vision benchmark did not execute model inference")?;
        analysis = result.analysis;
        runs.push(metrics);
    }
    let mut totals: Vec<u128> = runs.iter().map(|metrics| metrics.total_ms).collect();
    totals.sort_unstable();
    let p50_ms = percentile(&totals, 50);
    let p95_ms = percentile(&totals, 95);
    Ok((
        analysis,
        BenchmarkReport {
            kind: "readseek_vision_benchmark",
            profile,
            request,
            iterations,
            warmup: warmup_metrics,
            runs,
            p50_ms,
            p95_ms,
        },
    ))
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index]
}

fn tokens_per_second(tokens: usize, milliseconds: u128) -> Option<f64> {
    (milliseconds > 0).then(|| tokens as f64 * 1000.0 / milliseconds as f64)
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

#[derive(serde::Deserialize)]
struct CombinedJson {
    caption: Option<String>,
    objects: Option<Vec<ObjectJson>>,
    ocr: Option<String>,
}

fn parse_analysis(raw: &str, request: Request, width: u32, height: u32) -> Result<Analysis> {
    let json = extract_json(raw).context("vision response did not contain JSON")?;
    let parsed =
        serde_json::from_str::<CombinedJson>(json).context("parse vision JSON response")?;

    Ok(Analysis {
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
    })
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
    let location = location.clamp(0, LOCATION_BINS);
    (f64::from(location) / f64::from(LOCATION_BINS) * f64::from(dimension)).round() as i32
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
    if let Some(value) = env_u32("READSEEK_VISION_THREADS")? {
        return i32::try_from(value).context("READSEEK_VISION_THREADS exceeds i32");
    }

    let threads = std::thread::available_parallelism()
        .context("detect available parallelism")?
        .get();
    i32::try_from(threads).context("thread count exceeds i32")
}

fn inference_batch_size() -> Result<u32> {
    Ok(env_u32("READSEEK_VISION_BATCH")?.unwrap_or(DEFAULT_BATCH_SIZE))
}

fn inference_micro_batch_size(batch_size: u32) -> Result<u32> {
    let value = env_u32("READSEEK_VISION_UBATCH")?.unwrap_or(DEFAULT_UBATCH_SIZE.min(batch_size));
    if value > batch_size {
        return Err(anyhow!(
            "READSEEK_VISION_UBATCH ({value}) exceeds vision batch size ({batch_size})"
        ));
    }
    Ok(value)
}

fn env_u32(name: &str) -> Result<Option<u32>> {
    let Some(value) = env::var_os(name) else {
        return Ok(None);
    };
    let value = value
        .into_string()
        .map_err(|_| anyhow!("{name} is not valid UTF-8"))?;
    let parsed = value
        .parse::<u32>()
        .with_context(|| format!("parse {name} as a positive integer"))?;
    if parsed == 0 {
        return Err(anyhow!("{name} must be greater than zero"));
    }
    Ok(Some(parsed))
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .split_ascii_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn peak_rss_bytes() -> Option<u64> {
    None
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
