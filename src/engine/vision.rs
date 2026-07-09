// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Image vision analysis: captioning (BLIP), object detection
//! (YOLOv8-nano), and OCR (ocrs). Models run on CPU through pure-Rust
//! inference stacks and are fetched lazily into the user cache directory (see
//! [`crate::engine::model`]). Tasks run independently, so a failure in one
//! leaves the other's results intact.

// Bounding-box and token-count casts are intentional and bounded by the model
// output shapes and image dimensions.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::engine::model;
use crate::engine::yolo::{COCO_CLASSES, Multiples, YoloV8};
use anyhow::{Context as _, Result, anyhow};
use candle::{DType, Device, IndexOp, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::{
    models::{blip, quantized_blip},
    object_detection::{Bbox, KeyPoint, non_maximum_suppression},
    quantized_var_builder,
};
use indicatif::ProgressBar;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use serde::{Deserialize, Serialize};
use std::io::IsTerminal as _;
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

const CAPTION_MAX_TOKENS: usize = 256;
const YOLO_CONFIDENCE: f32 = 0.25;
const YOLO_NMS: f32 = 0.45;
/// BLIP decoder start token (`[DEC]`) that seeds caption generation.
const BLIP_DEC_TOKEN: u32 = 30522;
/// BLIP separator token (`[SEP]`) that marks the end of a caption.
const BLIP_SEP_TOKEN: u32 = 102;

const PROGRESS_DEADLINE: Duration = Duration::from_secs(2);
const PROGRESS_TICK: Duration = Duration::from_millis(100);
const PROGRESS_MSG: &str = "Analyzing image...";

/// A detected object with its category label and bounding box `[x1,y1,x2,y2]`.
#[derive(Debug, Serialize, Deserialize)]
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

/// Results of the requested vision tasks.
#[derive(Default)]
pub(crate) struct Analysis {
    pub(crate) caption: Option<String>,
    pub(crate) objects: Option<Vec<DetectedObject>>,
    pub(crate) ocr: Option<String>,
}

/// Run the requested tasks against `image_bytes`. Each task runs independently;
/// a task that fails is logged and left `None` so it is recomputed on a later
/// run instead of being cached as final-empty.
pub(crate) fn analyze(image_bytes: &[u8], request: Request) -> Result<Analysis> {
    let image = image::load_from_memory(image_bytes).context("decode image")?;
    let mut analysis = Analysis::default();
    if request.caption {
        match caption(&image) {
            Ok(text) => analysis.caption = Some(text),
            Err(error) => log::warn!("vision caption skipped: {error:#}"),
        }
    }
    if request.objects {
        match detect_objects(&image) {
            Ok(objects) => analysis.objects = Some(objects),
            Err(error) => log::warn!("vision objects skipped: {error:#}"),
        }
    }
    if request.ocr {
        match ocr_text(&image) {
            Ok(text) => analysis.ocr = Some(text),
            Err(error) => log::warn!("vision OCR skipped: {error:#}"),
        }
    }
    Ok(analysis)
}

/// Generate a concise caption for `image` with the quantized BLIP model,
/// mirroring the `candle-examples/examples/blip` decoder loop.
fn caption(image: &image::DynamicImage) -> Result<String> {
    let device = Device::Cpu;
    let model_path = model::file("blip-image-captioning-large-q4k.gguf")?;
    let tokenizer_path = model::file("tokenizer.json")?;
    let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!(e))?;
    let config = blip::Config::image_captioning_large();
    let vb = quantized_var_builder::VarBuilder::from_gguf(&model_path, &device)?;
    let mut model = quantized_blip::BlipForConditionalGeneration::new(&config, vb)?;

    let image_embeds = blip_image(image, &device)?
        .unsqueeze(0)?
        .apply(model.vision_model())?;

    let mut tokens = vec![BLIP_DEC_TOKEN];
    let mut progress = InferenceProgress::new();
    let mut output = String::new();
    for index in 0..CAPTION_MAX_TOKENS {
        progress.maybe_reveal();
        let context_size = if index > 0 { 1 } else { tokens.len() };
        let start_pos = tokens.len().saturating_sub(context_size);
        let input = Tensor::new(&tokens[start_pos..], &device)?.unsqueeze(0)?;
        let logits = model.text_decoder().forward(&input, &image_embeds)?;
        let logits = logits.squeeze(0)?;
        let logits = logits.get(logits.dim(0)? - 1)?;
        let next = argmax_token(&logits)?;
        if next == BLIP_SEP_TOKEN {
            break;
        }
        tokens.push(next);
        if let Ok(piece) = tokenizer.decode(&[next], true) {
            output.push_str(&piece);
        }
    }
    Ok(output.trim().to_string())
}

/// Decode, resize to 384x384, and normalize `image` into a `(3, 384, 384)`
/// f32 tensor for the BLIP vision encoder (`OpenAI` normalization).
fn blip_image(image: &image::DynamicImage, device: &Device) -> Result<Tensor> {
    let img = image
        .resize_to_fill(384, 384, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let data = Tensor::from_vec(img.into_raw(), (384, 384, 3), device)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.481_454_66f32, 0.457_827_5, 0.408_210_73], device)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.268_629_54f32, 0.261_302_6, 0.275_777_1], device)?.reshape((3, 1, 1))?;
    Ok((data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?)
}

/// Greedy argmax over the last-axis logits, returning the best token id.
fn argmax_token(logits: &Tensor) -> Result<u32> {
    let values = logits.to_vec1::<f32>()?;
    let (id, _) = values
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .expect("non-empty logits");
    Ok(id as u32)
}

/// Detect salient objects with YOLOv8-nano, returning labeled bounding boxes in
/// the original image's pixel space.
fn detect_objects(image: &image::DynamicImage) -> Result<Vec<DetectedObject>> {
    let device = Device::Cpu;
    let model_path = model::file("yolov8n.safetensors")?;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)? };
    let yolo = YoloV8::load(vb, Multiples::n(), 80)?;

    let (input, model_w, model_h, orig_w, orig_h) = yolo_image(image, &device)?;
    let mut progress = InferenceProgress::new();
    progress.maybe_reveal();
    let predictions = yolo.forward(&input)?.squeeze(0)?;
    progress.maybe_reveal();
    objects_from_predictions(&predictions, orig_w, orig_h, model_w, model_h)
}

/// Extract text from `image` with the ocrs detection and recognition models.
fn ocr_text(image: &image::DynamicImage) -> Result<String> {
    let detection_model_path = model::file("text-detection.rten")?;
    let recognition_model_path = model::file("text-recognition.rten")?;
    let detection_model = Model::load_file(detection_model_path)?;
    let recognition_model = Model::load_file(recognition_model_path)?;
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })?;

    let image = image.to_rgb8();
    let source = ImageSource::from_bytes(image.as_raw(), image.dimensions())?;
    let input = engine.prepare_input(source)?;
    let mut progress = InferenceProgress::new();
    progress.maybe_reveal();
    let text = engine.get_text(&input)?;
    progress.maybe_reveal();
    Ok(text.trim().to_string())
}

/// Resize `image` to a 32-divisible size fitting 640px on the longer side and
/// scale pixels to `[0, 1]`, returning the `(1, 3, H, W)` tensor plus the model
/// and original dimensions for mapping boxes back to pixel space.
fn yolo_image(
    image: &image::DynamicImage,
    device: &Device,
) -> Result<(Tensor, usize, usize, u32, u32)> {
    let orig_w = image.width();
    let orig_h = image.height();
    let (w, h) = {
        let w = orig_w as usize;
        let h = orig_h as usize;
        if w < h {
            let w = w * 640 / h;
            (w / 32 * 32, 640)
        } else {
            let h = h * 640 / w;
            (640, h / 32 * 32)
        }
    };
    let resized = image.resize_exact(w as u32, h as u32, image::imageops::FilterType::CatmullRom);
    let data = resized.to_rgb8().into_raw();
    let tensor = Tensor::from_vec(data, (h, w, 3), device)?
        .permute((2, 0, 1))?
        .unsqueeze(0)?
        .to_dtype(DType::F32)?;
    let tensor = (tensor * (1. / 255.))?;
    Ok((tensor, w, h, orig_w, orig_h))
}

/// Extract confident boxes from the `YOLOv8` predictions, run per-class
/// non-maximum suppression, and scale survivors back to original pixel space.
fn objects_from_predictions(
    predictions: &Tensor,
    orig_w: u32,
    orig_h: u32,
    model_w: usize,
    model_h: usize,
) -> Result<Vec<DetectedObject>> {
    let predictions = predictions.to_device(&Device::Cpu)?;
    let (pred_size, npreds) = predictions.dims2()?;
    let nclasses = pred_size - 4;
    let mut bboxes: Vec<Vec<Bbox<Vec<KeyPoint>>>> = (0..nclasses).map(|_| Vec::new()).collect();
    for index in 0..npreds {
        let pred = Vec::<f32>::try_from(predictions.i((.., index))?)?;
        let confidence = pred[4..]
            .iter()
            .max_by(|a, b| a.total_cmp(b))
            .copied()
            .unwrap_or(0.);
        if confidence <= YOLO_CONFIDENCE {
            continue;
        }
        let mut class_index = 0;
        for class in 0..nclasses {
            if pred[4 + class] > pred[4 + class_index] {
                class_index = class;
            }
        }
        if pred[class_index + 4] > 0. {
            bboxes[class_index].push(Bbox {
                xmin: pred[0] - pred[2] / 2.,
                ymin: pred[1] - pred[3] / 2.,
                xmax: pred[0] + pred[2] / 2.,
                ymax: pred[1] + pred[3] / 2.,
                confidence,
                data: Vec::new(),
            });
        }
    }
    non_maximum_suppression(&mut bboxes, YOLO_NMS);

    let w_ratio = orig_w as f32 / model_w as f32;
    let h_ratio = orig_h as f32 / model_h as f32;
    let mut objects = Vec::new();
    for (class_index, class_bboxes) in bboxes.iter().enumerate() {
        let label = COCO_CLASSES.get(class_index).copied().unwrap_or("unknown");
        for bbox in class_bboxes {
            objects.push(DetectedObject {
                label: label.to_string(),
                bbox: [
                    (bbox.xmin * w_ratio).round().max(0.) as i32,
                    (bbox.ymin * h_ratio).round().max(0.) as i32,
                    (bbox.xmax * w_ratio).round().max(0.) as i32,
                    (bbox.ymax * h_ratio).round().max(0.) as i32,
                ],
            });
        }
    }
    Ok(objects)
}

/// Spinner that stays silent until inference exceeds `PROGRESS_DEADLINE`, then
/// shows a ticking spinner so slow runs are not silent. It draws on stderr (so
/// the JSON result on stdout stays clean) and is gated on stderr being a
/// terminal, which keeps the spinner visible even when stdout is redirected.
/// Modeled on the tpm2sh CLI progress pattern; dropping clears it so early
/// `?`-return error paths stay clean.
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

    /// Reveal the spinner once the deadline elapses, but only on a TTY and only
    /// once; fast runs that finish first never draw anything.
    fn maybe_reveal(&mut self) {
        if self.bar.is_some() || !self.is_tty {
            return;
        }
        if self.started.elapsed() >= PROGRESS_DEADLINE {
            let bar = ProgressBar::new_spinner();
            bar.set_message(PROGRESS_MSG);
            bar.enable_steady_tick(PROGRESS_TICK);
            self.bar = Some(bar);
        }
    }
}

impl Drop for InferenceProgress {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
    }
}
