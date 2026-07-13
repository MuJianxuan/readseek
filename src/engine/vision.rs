// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Image vision analysis: captioning (BLIP), object detection
//! (YOLOv8-nano), and OCR (`TrOCR`). Models run on the best available device
//! (Metal on macOS, CPU elsewhere) through pure-Rust inference stacks and
//! are fetched lazily into the user cache directory (see
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
use crate::engine::quantized_blip;
use crate::engine::quantized_trocr;
use crate::engine::yolo::{COCO_CLASSES, Multiples, YoloV8};
use anyhow::{Context as _, Result, anyhow};
use candle::{DType, Device, IndexOp, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::{
    models::{blip, trocr, vit},
    object_detection::{Bbox, KeyPoint, non_maximum_suppression},
    quantized_var_builder,
};
use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};
use std::io::IsTerminal as _;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

const CAPTION_MAX_TOKENS: usize = 256;
/// Maximum tokens generated for OCR text; bounded by the `TrOCR` decoder
/// `max_position_embeddings` (512).
const OCR_MAX_TOKENS: usize = 512;
/// Long-edge cap
const IMAGE_MAX_LONG_EDGE: u32 = 1280;
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

struct CaptionRuntime {
    device: Device,
    tokenizer: Tokenizer,
    model: quantized_blip::BlipForConditionalGeneration,
}

struct ObjectRuntime {
    device: Device,
    yolo: YoloV8,
}

struct OcrRuntime {
    device: Device,
    tokenizer: Tokenizer,
    config: TrOcrConfig,
    model: quantized_trocr::TrOCRModel,
}

fn init_caption_runtime() -> Result<CaptionRuntime> {
    let device = best_device();
    let model_path = model::file("blip-image-captioning-large-q4k.gguf")?;
    let tokenizer_path = model::file("blip-tokenizer.json")?;
    let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!(e))?;
    let config = blip::Config::image_captioning_large();
    let vb = quantized_var_builder::VarBuilder::from_gguf(&model_path, &device)?;
    let model = quantized_blip::BlipForConditionalGeneration::new(&config, vb)?;
    Ok(CaptionRuntime {
        device,
        tokenizer,
        model,
    })
}

fn caption_runtime() -> Result<&'static Mutex<CaptionRuntime>> {
    static RUNTIME: OnceLock<Result<Mutex<CaptionRuntime>, String>> = OnceLock::new();
    match RUNTIME.get_or_init(|| {
        init_caption_runtime()
            .map(Mutex::new)
            .map_err(|e| e.to_string())
    }) {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

fn init_object_runtime() -> Result<ObjectRuntime> {
    let device = best_device();
    let model_path = model::file("yolov8n.safetensors")?;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)? };
    let yolo = YoloV8::load(vb, Multiples::n(), 80)?;
    Ok(ObjectRuntime { device, yolo })
}

fn object_runtime() -> Result<&'static Mutex<ObjectRuntime>> {
    static RUNTIME: OnceLock<Result<Mutex<ObjectRuntime>, String>> = OnceLock::new();
    match RUNTIME.get_or_init(|| {
        init_object_runtime()
            .map(Mutex::new)
            .map_err(|e| e.to_string())
    }) {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

fn init_ocr_runtime() -> Result<OcrRuntime> {
    let device = best_device();
    let model_path = model::file("trocr-base-printed.safetensors")?;
    let tokenizer_path = model::file("trocr-tokenizer.json")?;
    let config_path = model::file("trocr-config.json")?;
    let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!(e))?;
    let config: TrOcrConfig =
        serde_json::from_slice(&std::fs::read(&config_path).context("read trocr config")?)?;
    let gguf_path = model::local_cache_path(quantized_trocr::Q4K_GGUF_NAME)?;
    if !gguf_path.exists() {
        quantized_trocr::build_q4k_gguf(&model_path, &gguf_path)?;
    }
    let vb = quantized_var_builder::VarBuilder::from_gguf(&gguf_path, &device)?;
    let model = quantized_trocr::TrOCRModel::new(&config.encoder, &config.decoder, vb)?;
    Ok(OcrRuntime {
        device,
        tokenizer,
        config,
        model,
    })
}

fn ocr_runtime() -> Result<&'static Mutex<OcrRuntime>> {
    static RUNTIME: OnceLock<Result<Mutex<OcrRuntime>, String>> = OnceLock::new();
    match RUNTIME.get_or_init(|| {
        init_ocr_runtime()
            .map(Mutex::new)
            .map_err(|e| e.to_string())
    }) {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

fn lock_runtime<T>(
    runtime: &'static Mutex<T>,
    name: &str,
) -> Result<std::sync::MutexGuard<'static, T>> {
    runtime
        .lock()
        .map_err(|_| anyhow!("{name} runtime mutex poisoned"))
}

/// Run the requested tasks against `image_bytes`. Each task runs independently;
/// a task that fails is logged and left `None` so it is recomputed on a later
/// run instead of being cached as final-empty.
pub(crate) fn analyze(image_bytes: &[u8], request: Request) -> Result<Analysis> {
    let mut image = image::load_from_memory(image_bytes).context("decode image")?;
    let long = image.width().max(image.height());
    if long > IMAGE_MAX_LONG_EDGE {
        let scale = f64::from(IMAGE_MAX_LONG_EDGE) / f64::from(long);
        let target_w = (f64::from(image.width()) * scale).round() as u32;
        let target_h = (f64::from(image.height()) * scale).round() as u32;
        image = image.resize_exact(target_w, target_h, image::imageops::FilterType::Triangle);
    }
    let mut analysis = Analysis::default();
    let progress = InferenceProgress::new();

    std::thread::scope(|scope| {
        let caption_handle = request.caption.then(|| {
            let image = &image;
            let progress = progress.clone();
            scope.spawn(move || caption(image, &progress))
        });
        let objects_handle = request.objects.then(|| {
            let image = &image;
            let progress = progress.clone();
            scope.spawn(move || detect_objects(image, &progress))
        });
        let ocr_handle = request.ocr.then(|| {
            let image = &image;
            let progress = progress.clone();
            scope.spawn(move || ocr_text(image, &progress))
        });

        if let Some(handle) = caption_handle {
            match handle.join().expect("caption task panicked") {
                Ok(text) => analysis.caption = Some(text),
                Err(error) => log::warn!("vision caption skipped: {error:#}"),
            }
        }
        if let Some(handle) = objects_handle {
            match handle.join().expect("objects task panicked") {
                Ok(objects) => analysis.objects = Some(objects),
                Err(error) => log::warn!("vision objects skipped: {error:#}"),
            }
        }
        if let Some(handle) = ocr_handle {
            match handle.join().expect("OCR task panicked") {
                Ok(text) => analysis.ocr = Some(text),
                Err(error) => log::warn!("vision OCR skipped: {error:#}"),
            }
        }
    });

    Ok(analysis)
}

/// Pick the best available inference [`Device`]: Metal on macOS, CPU
/// elsewhere. Metal selection is best-effort — if the GPU is unavailable we
/// fall back to CPU so headless CI keeps working.
fn best_device() -> Device {
    #[cfg(target_os = "macos")]
    {
        let device = match Device::new_metal(0) {
            Ok(device) => device,
            Err(err) => {
                log::warn!("Metal unavailable, falling back to CPU: {err}");
                Device::Cpu
            }
        };
        log::info!("inference device: {device:?}");
        device
    }
    #[cfg(not(target_os = "macos"))]
    {
        let device = Device::Cpu;
        log::info!("inference device: {device:?}");
        device
    }
}

/// Generate a concise caption for `image` with the quantized BLIP model,
/// mirroring the `candle-examples/examples/blip` decoder loop.
fn caption(image: &image::DynamicImage, progress: &InferenceProgress) -> Result<String> {
    let mut runtime = lock_runtime(caption_runtime()?, "caption")?;
    let CaptionRuntime {
        device,
        tokenizer,
        model,
    } = &mut *runtime;
    model.reset_kv_cache();
    let image_embeds = blip_image(image, device)?
        .unsqueeze(0)?
        .apply(model.vision_model())?;

    let mut tokens = Vec::with_capacity(CAPTION_MAX_TOKENS + 1);
    tokens.push(BLIP_DEC_TOKEN);
    let mut generated = Vec::with_capacity(CAPTION_MAX_TOKENS);
    for index in 0..CAPTION_MAX_TOKENS {
        progress.maybe_reveal();
        let context_size = if index > 0 { 1 } else { tokens.len() };
        let start_pos = tokens.len().saturating_sub(context_size);
        let input = Tensor::new(&tokens[start_pos..], device)?.unsqueeze(0)?;
        let logits = model.text_decoder().forward(&input, &image_embeds)?;
        let logits = logits.squeeze(0)?;
        let logits = logits.get(logits.dim(0)? - 1)?;
        let next = argmax_token(&logits)?;
        if next == BLIP_SEP_TOKEN {
            break;
        }
        tokens.push(next);
        generated.push(next);
    }
    // Decode the whole sequence at once so the WordPiece decoder can join `##`
    // continuation tokens to their preceding word instead of leaving the `##`
    // markers in place (which happens when tokens are decoded one at a time).
    let output = tokenizer.decode(&generated, true).unwrap_or_default();
    Ok(output.trim().to_string())
}

/// Decode, resize to 384x384, and normalize `image` into a `(3, 384, 384)`
/// f32 tensor for the BLIP vision encoder (`OpenAI` normalization).
fn blip_image(image: &image::DynamicImage, device: &Device) -> Result<Tensor> {
    let img = image
        .resize_to_fill(384, 384, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let data = Tensor::from_vec(img.into_raw(), (384, 384, 3), device)?.permute((2, 0, 1))?;
    let mean =
        Tensor::new(&[0.481_454_66f32, 0.457_827_5, 0.408_210_73], device)?.reshape((3, 1, 1))?;
    let std =
        Tensor::new(&[0.268_629_54f32, 0.261_302_6, 0.275_777_1], device)?.reshape((3, 1, 1))?;
    Ok((data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?)
}

/// Decode, resize to 384x384, and normalize `image` into a `(3, 384, 384)`
/// f32 tensor for the `TrOCR` `ViT` encoder (mean/std 0.5 normalization).
fn trocr_image(image: &image::DynamicImage, device: &Device) -> Result<Tensor> {
    let img = image
        .resize_exact(384, 384, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let data = Tensor::from_vec(img.into_raw(), (384, 384, 3), device)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.5f32, 0.5, 0.5], device)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.5f32, 0.5, 0.5], device)?.reshape((3, 1, 1))?;
    Ok((data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?)
}

/// Greedy argmax over the last-axis logits, returning the best token id.
fn argmax_token(logits: &Tensor) -> Result<u32> {
    Ok(logits.argmax(candle::D::Minus1)?.to_scalar::<u32>()?)
}

/// Detect salient objects with YOLOv8-nano, returning labeled bounding boxes in
/// the original image's pixel space.
fn detect_objects(
    image: &image::DynamicImage,
    progress: &InferenceProgress,
) -> Result<Vec<DetectedObject>> {
    let runtime = lock_runtime(object_runtime()?, "objects")?;
    let ObjectRuntime { device, yolo } = &*runtime;
    let (input, model_w, model_h, orig_w, orig_h) = yolo_image(image, device)?;
    progress.maybe_reveal();
    let predictions = yolo.forward(&input)?.squeeze(0)?;
    progress.maybe_reveal();
    objects_from_predictions(&predictions, orig_w, orig_h, model_w, model_h)
}

/// `TrOCR` `config.json` shape: a `ViT` encoder config paired with a BART-style
/// decoder config, parsed so the model is built from the exact weights it ships.
#[derive(Deserialize)]
struct TrOcrConfig {
    encoder: vit::Config,
    decoder: trocr::TrOCRConfig,
}

/// Extract text from `image` with the `TrOCR` printed-text recognition model.
///
/// `TrOCR`-printed is a single-line recognizer (fine-tuned on SROIE receipt
/// line crops), so feeding it a full page makes every line too short to read
/// and it collapses to a single garbage token. A page is therefore split into
/// per-line crops with a horizontal ink projection; each crop is recognized
/// with one shared model and the results are joined with newlines. When no text
/// rows are found the whole image is recognized as a fallback.
fn ocr_text(image: &image::DynamicImage, progress: &InferenceProgress) -> Result<String> {
    let mut runtime = lock_runtime(ocr_runtime()?, "ocr")?;
    let OcrRuntime {
        device,
        tokenizer,
        config,
        model,
    } = &mut *runtime;
    let lines = text_lines(image);
    let crops: Vec<image::DynamicImage> = if lines.is_empty() {
        vec![image.clone()]
    } else {
        lines
            .into_iter()
            .map(|(y0, y1)| image.crop_imm(0, y0, image.width(), y1 - y0))
            .collect()
    };

    let inputs: Vec<Tensor> = crops
        .iter()
        .map(|crop| trocr_image(crop, device))
        .collect::<Result<Vec<_>>>()?;
    let batched = Tensor::stack(&inputs, 0)?;
    progress.maybe_reveal();
    let encoder_xs = model.encoder().forward(&batched)?;

    let mut text = Vec::with_capacity(crops.len());
    for index in 0..crops.len() {
        progress.maybe_reveal();
        let line_encoder_xs = encoder_xs.i(index)?.unsqueeze(0)?;
        let line = decode_line(&line_encoder_xs, model, config, tokenizer, device, progress)?;
        if !line.is_empty() {
            text.push(line);
        }
    }
    Ok(text.join("\n").trim().to_string())
}

/// Minimum foreground-pixel fraction of an image row for it to count as text.
const LINE_INK_FRACTION: f32 = 0.02;
/// Merge text-line bands whose vertical gap is at most this many pixels.
const LINE_MERGE_GAP: u32 = 8;
/// Drop bands shorter than this many pixels as noise.
const LINE_MIN_HEIGHT: u32 = 6;

/// Split `image` into text-line bands `[y0, y1)` using a horizontal ink
/// projection. Returns bands in reading order, or an empty vec when no text
/// rows are detected. This is the segmentation that lets the single-line
/// `TrOCR`-printed model read a multi-line page.
fn text_lines(image: &image::DynamicImage) -> Vec<(u32, u32)> {
    let gray = image.to_luma8();
    let (width, height) = gray.dimensions();
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let threshold = ((width as f32) * LINE_INK_FRACTION).ceil().max(1.) as usize;

    let mut dark_ink = vec![0u32; height as usize];
    let mut light_ink = vec![0u32; height as usize];
    for (_, y, pixel) in gray.enumerate_pixels() {
        if pixel[0] < 128 {
            dark_ink[y as usize] += 1;
        } else {
            light_ink[y as usize] += 1;
        }
    }
    let ink = if border_is_dark(&gray) {
        light_ink
    } else {
        dark_ink
    };

    // Raw bands of consecutive rows that meet the ink threshold.
    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut start = 0u32;
    let mut in_line = false;
    for (y, &count) in ink.iter().enumerate() {
        let is_text = (count as usize) >= threshold;
        if is_text && !in_line {
            in_line = true;
            start = y as u32;
        } else if !is_text && in_line {
            in_line = false;
            bands.push((start, y as u32));
        }
    }
    if in_line {
        bands.push((start, height));
    }

    // Merge bands split by small gaps within a single line, then drop noise.
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (start, end) in bands {
        let should_merge = merged
            .last()
            .is_some_and(|(_, prev_end)| start.saturating_sub(*prev_end) <= LINE_MERGE_GAP);
        if should_merge {
            if let Some((_, prev_end)) = merged.last_mut()
                && end > *prev_end
            {
                *prev_end = end;
            }
            continue;
        }
        merged.push((start, end));
    }
    merged.retain(|&(start, end)| end - start >= LINE_MIN_HEIGHT);
    merged
}

fn border_is_dark(gray: &image::GrayImage) -> bool {
    let (width, height) = gray.dimensions();
    let mut sum = 0u64;
    let mut count = 0u64;
    for x in 0..width {
        sum += u64::from(gray.get_pixel(x, 0)[0]);
        count += 1;
        if height > 1 {
            sum += u64::from(gray.get_pixel(x, height - 1)[0]);
            count += 1;
        }
    }
    for y in 1..height.saturating_sub(1) {
        sum += u64::from(gray.get_pixel(0, y)[0]);
        count += 1;
        if width > 1 {
            sum += u64::from(gray.get_pixel(width - 1, y)[0]);
            count += 1;
        }
    }
    sum / count < 128
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_line_image(background: u8, foreground: u8) -> image::DynamicImage {
        let mut image = image::GrayImage::from_pixel(100, 50, image::Luma([background]));
        for y in 10..19 {
            for x in 10..90 {
                image.put_pixel(x, y, image::Luma([foreground]));
            }
        }
        for y in 30..39 {
            for x in 10..90 {
                image.put_pixel(x, y, image::Luma([foreground]));
            }
        }
        image::DynamicImage::ImageLuma8(image)
    }

    #[test]
    fn text_lines_detects_dark_text() {
        assert_eq!(text_lines(&two_line_image(255, 0)), [(10, 19), (30, 39)]);
    }

    #[test]
    fn text_lines_detects_light_text() {
        assert_eq!(text_lines(&two_line_image(0, 255)), [(10, 19), (30, 39)]);
    }
}

/// Decode one line from its encoder output, mirroring the
/// `candle-examples/examples/trocr` decoder loop.
fn decode_line(
    encoder_xs: &Tensor,
    model: &mut quantized_trocr::TrOCRModel,
    config: &TrOcrConfig,
    tokenizer: &Tokenizer,
    device: &Device,
    progress: &InferenceProgress,
) -> Result<String> {
    model.reset_kv_cache();
    let mut tokens = Vec::with_capacity(OCR_MAX_TOKENS + 1);
    tokens.push(config.decoder.decoder_start_token_id);
    let eos = config.decoder.eos_token_id;
    let mut generated = Vec::with_capacity(OCR_MAX_TOKENS);
    for index in 0..OCR_MAX_TOKENS {
        progress.maybe_reveal();
        let context_size = if index >= 1 { 1 } else { tokens.len() };
        let start_pos = tokens.len().saturating_sub(context_size);
        let input_ids = Tensor::new(&tokens[start_pos..], device)?.unsqueeze(0)?;
        let logits = model.decode(&input_ids, encoder_xs, start_pos)?;
        let logits = logits.squeeze(0)?;
        let logits = logits.get(logits.dim(0)? - 1)?;
        let next = argmax_token(&logits)?;
        if next == eos {
            break;
        }
        tokens.push(next);
        generated.push(next);
    }
    let output = tokenizer.decode(&generated, true).map_err(|e| anyhow!(e))?;
    Ok(output.trim().to_string())
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
    let predictions = predictions.to_device(&Device::Cpu)?.t()?;
    let (_, pred_size) = predictions.dims2()?;
    let nclasses = pred_size - 4;
    let flat = predictions.flatten_all()?.to_vec1::<f32>()?;
    let mut bboxes: Vec<Vec<Bbox<Vec<KeyPoint>>>> = (0..nclasses).map(|_| Vec::new()).collect();
    for pred in flat.chunks_exact(pred_size) {
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
#[derive(Clone)]
struct InferenceProgress {
    inner: Arc<Mutex<InferenceProgressState>>,
}

impl InferenceProgress {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(InferenceProgressState {
                is_tty: std::io::stderr().is_terminal(),
                started: Instant::now(),
                bar: None,
            })),
        }
    }

    /// Reveal the spinner once the deadline elapses, but only on a TTY and only
    /// once; fast runs that finish first never draw anything.
    fn maybe_reveal(&self) {
        self.inner
            .lock()
            .expect("progress mutex poisoned")
            .maybe_reveal();
    }
}

struct InferenceProgressState {
    is_tty: bool,
    started: Instant,
    bar: Option<ProgressBar>,
}

impl InferenceProgressState {
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

impl Drop for InferenceProgressState {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
    }
}
