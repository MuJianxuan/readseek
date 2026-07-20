// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! `Qwen3-VL` image analysis through `ReadSeek`'s fixed CPU inference engine. One
//! multimodal model handles captioning, object detection, and OCR; its GGUF
//! files are fetched lazily into the user cache directory.

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
use std::io::IsTerminal as _;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, ensure};
use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};

use crate::engine::qwen::{TextModel, VisionEmbedding, VisionInput, VisionModel};

const MODEL_FILE: &str = "Qwen3VL-2B-Instruct-Q8_0.gguf";
const MMPROJ_FILE: &str = "mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf";
const CAPTION_MAX_NEW_TOKENS: usize = 512;
const OBJECTS_MAX_NEW_TOKENS: usize = 1024;
const OCR_MAX_NEW_TOKENS: usize = 4096;
const LOCATION_BINS: i32 = 1000;
const PROGRESS_DEADLINE: Duration = Duration::from_secs(2);
const PROGRESS_TICK: Duration = Duration::from_millis(100);
const PROGRESS_MSG: &str = "Analyzing image...";

const FIELD_CAPTION: &str =
    "\"caption\": one concise paragraph of at most 100 words describing the image";
const FIELD_OBJECTS: &str = "\"objects\": an array of at most 32 {\"label\": string, \"bbox\": [x1,y1,x2,y2]} entries for the salient objects, where bbox is an axis-aligned box with integer coordinates normalized to 0-1000 relative to image width and height";
const FIELD_OCR: &str =
    "\"ocr\": a string containing all visible text in reading order, preserving line breaks";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VisionProfile {
    Fast,
    #[default]
    Balanced,
    Accurate,
}

impl VisionProfile {
    fn image_max_tokens(self, request: Request) -> usize {
        let (caption, objects, ocr) = match self {
            Self::Fast => (512, 768, 1024),
            Self::Balanced => (768, 1024, 1536),
            Self::Accurate => return 2048,
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
    image_max_tokens: usize,
    prompt_tokens: usize,
    generated_tokens: usize,
    threads: usize,
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

impl VisionInput<'_> {
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

    /// Image dimensions without decoding the pixel data.
    fn dimensions(&self) -> Result<(u32, u32)> {
        match self {
            Self::Encoded(bytes) => {
                let size =
                    imagesize::blob_size(bytes).context("read image dimensions for Qwen3-VL")?;
                let width = u32::try_from(size.width).context("image width exceeds u32")?;
                let height = u32::try_from(size.height).context("image height exceeds u32")?;
                Ok((width, height))
            }
            Self::Rgb { width, height, .. } => Ok((*width, *height)),
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

struct VisionRuntime {
    text: TextModel,
    vision: VisionModel,
    threads: usize,
    unreported_startup_ms: Option<u128>,
}

impl VisionRuntime {
    fn load() -> Result<Self> {
        let started = Instant::now();
        let threads = configure_threads()?;
        let model_path = crate::engine::model::file(MODEL_FILE)?;
        let mmproj_path = crate::engine::model::file(MMPROJ_FILE)?;
        let text = TextModel::load(model_path).context("load Qwen3-VL text model")?;
        let vision = VisionModel::load(mmproj_path).context("load Qwen3-VL vision projector")?;
        Ok(Self {
            text,
            vision,
            threads,
            unreported_startup_ms: Some(started.elapsed().as_millis()),
        })
    }
}

thread_local! {
    static RUNTIME: OnceCell<RefCell<std::result::Result<VisionRuntime, String>>> =
        const { OnceCell::new() };
}

fn with_runtime<T>(run: impl FnOnce(&mut VisionRuntime) -> Result<T>) -> Result<T> {
    RUNTIME.with(|slot| {
        let runtime = slot.get_or_init(|| {
            RefCell::new(VisionRuntime::load().map_err(|error| format!("{error:#}")))
        });
        let mut runtime = runtime
            .try_borrow_mut()
            .map_err(|_| anyhow!("vision runtime is already in use"))?;
        let runtime = runtime.as_mut().map_err(|error| anyhow!(error.clone()))?;
        run(runtime)
    })
}

/// Run the selected tasks in one multimodal generation pass. The loaded model
/// is reused by later images, which matters for PDFs containing several images.
pub(crate) fn analyze(
    input: VisionInput<'_>,
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
        VisionInput::Encoded(bytes) if request.ocr => {
            crate::engine::image::embedded_drawio_text(bytes)
        }
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
    let (raw, width, height, metrics) = with_runtime(|runtime| {
        let total_started = Instant::now();
        let startup_ms = runtime.unreported_startup_ms.take().unwrap_or(0);
        let _progress = InferenceProgress::new();
        let vision_started = Instant::now();
        let (width, height) = input.dimensions()?;
        let embedding = runtime
            .vision
            .encode_input(input, image_max_tokens)
            .context("encode image for Qwen3-VL")?;
        let bitmap_ms = vision_started.elapsed().as_millis();
        let (raw, generation) = generate(runtime, &embedding, model_request)?;
        let metrics = InferenceMetrics {
            profile,
            image_max_tokens,
            prompt_tokens: generation.prompt_tokens,
            generated_tokens: generation.generated_tokens,
            threads: runtime.threads,
            batch_size: 1,
            micro_batch_size: 1,
            gpu_offload_supported: false,
            backend_devices: vec!["custom CPU".to_owned()],
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

fn generate(
    runtime: &VisionRuntime,
    image: &VisionEmbedding,
    request: Request,
) -> Result<(String, GenerationMetrics)> {
    let prompt = build_prompt(request);
    let token_limit = max_new_tokens(request);
    let generation_started = Instant::now();
    let generation = runtime
        .text
        .generate(&prompt, image, token_limit)
        .context("generate Qwen3-VL response")?;
    let generation_duration = generation_started.elapsed();
    let measured_duration = generation
        .prefill_duration
        .saturating_add(generation.decode_duration);
    let tokenize_ms = generation_duration
        .saturating_sub(measured_duration)
        .as_millis();
    let metrics = GenerationMetrics {
        prompt_tokens: generation.prompt_tokens,
        generated_tokens: generation.generated_tokens,
        tokenize_ms,
        prefill_ms: generation.prefill_duration.as_millis(),
        decode_ms: generation.decode_duration.as_millis(),
    };
    Ok((generation.text, metrics))
}

struct GenerationMetrics {
    prompt_tokens: usize,
    generated_tokens: usize,
    tokenize_ms: u128,
    prefill_ms: u128,
    decode_ms: u128,
}

fn max_new_tokens(request: Request) -> usize {
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

pub(crate) fn benchmark(
    input: VisionInput<'_>,
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
    (milliseconds > 0).then_some(tokens as f64 * 1000.0 / milliseconds as f64)
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
    raw.get(start..=end)
}

fn strip_special(raw: &str) -> String {
    raw.replace("<|im_end|>", "").trim().to_owned()
}

fn configure_threads() -> Result<usize> {
    if let Some(value) = env::var_os("READSEEK_VISION_THREADS") {
        let value = value
            .into_string()
            .map_err(|_| anyhow!("READSEEK_VISION_THREADS is not valid UTF-8"))?;
        let threads = value
            .parse::<usize>()
            .context("parse READSEEK_VISION_THREADS as a positive integer")?;
        ensure!(
            threads > 0,
            "READSEEK_VISION_THREADS must be greater than zero"
        );
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .context("configure vision inference thread pool")?;
    }
    Ok(rayon::current_num_threads())
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

#[derive(Default)]
struct InferenceProgress {
    completion: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl InferenceProgress {
    fn new() -> Self {
        if !std::io::stderr().is_terminal() {
            return Self::default();
        }

        let (completion, receiver) = channel();
        let worker = std::thread::spawn(move || {
            match receiver.recv_timeout(PROGRESS_DEADLINE) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            let bar = ProgressBar::new_spinner();
            bar.set_message(PROGRESS_MSG);
            bar.enable_steady_tick(PROGRESS_TICK);
            let _ = receiver.recv();
            bar.finish_and_clear();
        });
        Self {
            completion: Some(completion),
            worker: Some(worker),
        }
    }
}

impl Drop for InferenceProgress {
    fn drop(&mut self) {
        self.completion.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
