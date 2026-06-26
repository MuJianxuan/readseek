// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Image detection, metadata, and OCR.

use anyhow::{Context as _, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

const DETECTION_MODEL: (&str, &str) = (
    "text-detection.rten",
    "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten",
);

const RECOGNITION_MODEL: (&str, &str) = (
    "text-recognition.rten",
    "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten",
);

/// A recognized image format.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
    Tiff,
    Avif,
    Heic,
    Ico,
}

impl ImageFormat {
    fn from_image_type(kind: imagesize::ImageType) -> Option<Self> {
        use imagesize::{Compression, ImageType};
        Some(match kind {
            ImageType::Png => Self::Png,
            ImageType::Jpeg => Self::Jpeg,
            ImageType::Gif => Self::Gif,
            ImageType::Webp => Self::WebP,
            ImageType::Bmp => Self::Bmp,
            ImageType::Tiff => Self::Tiff,
            ImageType::Ico => Self::Ico,
            ImageType::Heif(Compression::Av1) => Self::Avif,
            ImageType::Heif(Compression::Hevc) => Self::Heic,
            _ => return None,
        })
    }
}

/// Structural metadata for a recognized image.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct ImageInfo {
    pub(crate) format: ImageFormat,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) animated: bool,
}

/// Text extracted from an image by OCR.
#[derive(Debug, Serialize)]
pub(crate) struct OcrText {
    text: String,
    lines: Vec<OcrLine>,
}

/// A recognized line of text with its bounding box `[x, y, width, height]`.
#[derive(Debug, Serialize)]
pub(crate) struct OcrLine {
    text: String,
    bbox: [i32; 4],
}

/// Identify `bytes` as an image, reporting its format, pixel dimensions, and a
/// best-effort animation flag. Returns `None` when the bytes are not a
/// supported image.
pub(crate) fn probe(bytes: &[u8]) -> Option<ImageInfo> {
    let format = ImageFormat::from_image_type(imagesize::image_type(bytes).ok()?)?;
    let size = imagesize::blob_size(bytes).ok()?;
    let animated = match format {
        ImageFormat::Png => png_animated(bytes),
        ImageFormat::Gif => gif_animated(bytes),
        ImageFormat::WebP => webp_animated(bytes),
        _ => false,
    };
    Some(ImageInfo {
        format,
        width: size.width,
        height: size.height,
        animated,
    })
}

/// Whether a PNG carries an `acTL` chunk (APNG) ahead of its first `IDAT`.
fn png_animated(bytes: &[u8]) -> bool {
    let mut pos = 8;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let tag = &bytes[pos + 4..pos + 8];
        if tag == b"acTL" {
            return true;
        }
        if tag == b"IDAT" {
            return false;
        }
        pos = pos.saturating_add(12).saturating_add(len);
    }
    false
}

/// Whether a GIF contains more than one image frame.
fn gif_animated(bytes: &[u8]) -> bool {
    if bytes.len() < 13 {
        return false;
    }
    let screen_flags = bytes[10];
    let mut pos = 13;
    if screen_flags & 0x80 != 0 {
        pos += 3 * (1usize << ((screen_flags & 0x07) + 1));
    }
    let mut frames = 0u32;
    while let Some(&block) = bytes.get(pos) {
        match block {
            0x2c => {
                frames += 1;
                if frames > 1 {
                    return true;
                }
                let Some(&local_flags) = bytes.get(pos + 9) else {
                    return false;
                };
                pos += 10;
                if local_flags & 0x80 != 0 {
                    pos += 3 * (1usize << ((local_flags & 0x07) + 1));
                }
                pos = skip_sub_blocks(bytes, pos + 1);
            }
            0x21 => pos = skip_sub_blocks(bytes, pos + 2),
            _ => break,
        }
    }
    false
}

/// Whether a RIFF/WebP container declares animation via an `ANIM` chunk.
fn webp_animated(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return false;
    }
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        if &bytes[pos..pos + 4] == b"ANIM" {
            return true;
        }
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        pos = pos
            .saturating_add(8)
            .saturating_add(size)
            .saturating_add(size & 1);
    }
    false
}

/// Advance past a GIF sub-block chain, returning the offset after the
/// terminating zero-length block.
fn skip_sub_blocks(bytes: &[u8], mut pos: usize) -> usize {
    while let Some(&size) = bytes.get(pos) {
        pos += 1;
        if size == 0 {
            break;
        }
        pos += usize::from(size);
    }
    pos
}

/// Recognize text in `bytes`, downloading the OCR models into the user cache
/// directory on first use.
pub(crate) fn run_ocr(bytes: &[u8]) -> Result<OcrText> {
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};

    let dir = cache_dir()?;
    let detection = ensure_model(&dir, DETECTION_MODEL.0, DETECTION_MODEL.1)?;
    let recognition = ensure_model(&dir, RECOGNITION_MODEL.0, RECOGNITION_MODEL.1)?;
    let detection_model = rten::Model::load_file(&detection)
        .map_err(|err| anyhow::anyhow!("load {}: {err}", detection.display()))?;
    let recognition_model = rten::Model::load_file(&recognition)
        .map_err(|err| anyhow::anyhow!("load {}: {err}", recognition.display()))?;
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })?;

    let decoded = decode(bytes)?;
    let source = ImageSource::from_bytes(&decoded.rgb, (decoded.width, decoded.height))
        .map_err(|err| anyhow::anyhow!("ocr image source: {err}"))?;
    let input = engine.prepare_input(source)?;
    let words = engine.detect_words(&input)?;
    let line_rects = engine.find_text_lines(&input, &words);
    let recognized = engine.recognize_text(&input, &line_rects)?;

    let mut lines = Vec::new();
    for line in recognized.into_iter().flatten() {
        let text = line.to_string();
        if text.trim().is_empty() {
            continue;
        }
        let rect = line.bounding_rect();
        lines.push(OcrLine {
            text,
            bbox: [rect.left(), rect.top(), rect.width(), rect.height()],
        });
    }
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(OcrText { text, lines })
}

struct Decoded {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Result<Decoded> {
    let img = image::load_from_memory(bytes)
        .context("decode image")?
        .into_rgb8();
    let (width, height) = img.dimensions();
    Ok(Decoded {
        width,
        height,
        rgb: img.into_raw(),
    })
}

fn cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir().context("no cache directory available")?;
    Ok(base.join("readseek"))
}

/// Return the cached path for `name`, downloading it from `url` on first use.
fn ensure_model(dir: &Path, name: &str, url: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    log::info!("downloading OCR model {name}");
    let partial = dir.join(format!("{name}.part"));
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| anyhow::anyhow!("download {url}: {err}"))?;
    let mut file =
        std::fs::File::create(&partial).with_context(|| format!("create {}", partial.display()))?;
    std::io::copy(&mut response.body_mut().as_reader(), &mut file)
        .with_context(|| format!("download {url}"))?;
    file.sync_all().ok();
    drop(file);
    std::fs::rename(&partial, &path).with_context(|| format!("install {}", path.display()))?;
    Ok(path)
}
