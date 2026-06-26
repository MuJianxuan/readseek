// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Image detection and attachment-ready descriptors.

use std::path::{Path, PathBuf};

#[cfg(feature = "transform")]
use anyhow::Context as _;
use anyhow::Result;
use base64::Engine as _;
use serde::Serialize;

use crate::engine::hash::hash_bytes;

/// A recognized image format usable as a model attachment.
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
    /// The IANA media type for this format.
    pub(crate) fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
            Self::Avif => "image/avif",
            Self::Heic => "image/heic",
            Self::Ico => "image/x-icon",
        }
    }

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

/// An attachment-ready descriptor: image metadata plus the base64 payload.
#[derive(Debug, Serialize)]
pub(crate) struct ImageOutput {
    file: PathBuf,
    image: bool,
    format: ImageFormat,
    width: usize,
    height: usize,
    animated: bool,
    bytes: usize,
    hash: String,
    media_type: &'static str,
    encoding: &'static str,
    data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    original: Option<OriginalImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ocr: Option<OcrText>,
}

/// The pre-transform image, reported when `image` re-encoded or downscaled.
#[derive(Debug, Serialize)]
pub(crate) struct OriginalImage {
    format: ImageFormat,
    width: usize,
    height: usize,
    bytes: usize,
}

/// Text extracted from an image by OCR.
#[cfg_attr(not(feature = "ocr"), allow(dead_code))]
#[derive(Debug, Serialize)]
pub(crate) struct OcrText {
    text: String,
    lines: Vec<OcrLine>,
}

/// A recognized line of text with its bounding box `[x, y, width, height]`.
#[cfg_attr(not(feature = "ocr"), allow(dead_code))]
#[derive(Debug, Serialize)]
pub(crate) struct OcrLine {
    text: String,
    bbox: [i32; 4],
}

/// Options controlling optional re-encoding and downscaling.
#[derive(Debug, Default)]
pub(crate) struct TransformOptions {
    pub(crate) max_dim: Option<u32>,
    pub(crate) max_bytes: Option<usize>,
    pub(crate) format: Option<ImageFormat>,
}

impl TransformOptions {
    fn is_noop(&self) -> bool {
        self.max_dim.is_none() && self.max_bytes.is_none() && self.format.is_none()
    }
}

/// A re-encoded image payload produced by [`maybe_transform`].
#[cfg_attr(not(feature = "transform"), allow(dead_code))]
#[derive(Debug)]
pub(crate) struct Transformed {
    format: ImageFormat,
    width: usize,
    height: usize,
    bytes: Vec<u8>,
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

/// Build an attachment-ready descriptor, preferring the transformed payload
/// when present and recording the original image alongside it.
pub(crate) fn image_output(
    path: &Path,
    bytes: &[u8],
    info: &ImageInfo,
    transformed: Option<&Transformed>,
    ocr: Option<OcrText>,
) -> ImageOutput {
    let payload: &[u8] = transformed.map_or(bytes, |t| &t.bytes);
    let (format, width, height, animated, original) = match transformed {
        Some(t) => (
            t.format,
            t.width,
            t.height,
            false,
            Some(OriginalImage {
                format: info.format,
                width: info.width,
                height: info.height,
                bytes: bytes.len(),
            }),
        ),
        None => (info.format, info.width, info.height, info.animated, None),
    };
    ImageOutput {
        file: path.to_path_buf(),
        image: true,
        format,
        width,
        height,
        animated,
        bytes: payload.len(),
        hash: hash_bytes(payload),
        media_type: format.media_type(),
        encoding: "base64",
        data: base64::engine::general_purpose::STANDARD.encode(payload),
        original,
        ocr,
    }
}

/// Apply [`TransformOptions`] when any are set, re-encoding the image to fit.
///
/// Returns `Ok(None)` when no options are requested, leaving the original
/// bytes for lossless passthrough.
#[cfg(feature = "transform")]
pub(crate) fn maybe_transform(
    bytes: &[u8],
    info: &ImageInfo,
    options: &TransformOptions,
) -> Result<Option<Transformed>> {
    if options.is_noop() {
        return Ok(None);
    }
    transform(bytes, info, options).map(Some)
}

#[cfg(not(feature = "transform"))]
pub(crate) fn maybe_transform(
    bytes: &[u8],
    info: &ImageInfo,
    options: &TransformOptions,
) -> Result<Option<Transformed>> {
    let _ = (bytes, info);
    if options.is_noop() {
        return Ok(None);
    }
    anyhow::bail!("readseek was built without image transform support")
}

#[cfg(feature = "transform")]
fn transform(bytes: &[u8], info: &ImageInfo, options: &TransformOptions) -> Result<Transformed> {
    let mut img = ::image::load_from_memory(bytes).context("decode image")?;
    if let Some(max) = options.max_dim {
        if img.width() > max || img.height() > max {
            img = img.resize(max, max, ::image::imageops::FilterType::Triangle);
        }
    }
    let format = options
        .format
        .unwrap_or_else(|| encodable_format(info.format));
    let mut encoded = encode(&img, format)?;
    if let Some(max_bytes) = options.max_bytes {
        let mut guard = 0;
        while encoded.len() > max_bytes && img.width() > 1 && img.height() > 1 && guard < 16 {
            let width = (img.width() * 17 / 20).max(1);
            let height = (img.height() * 17 / 20).max(1);
            img = img.resize(width, height, ::image::imageops::FilterType::Triangle);
            encoded = encode(&img, format)?;
            guard += 1;
        }
    }
    Ok(Transformed {
        format,
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: encoded,
    })
}

#[cfg(feature = "transform")]
fn encode(img: &::image::DynamicImage, format: ImageFormat) -> Result<Vec<u8>> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    match format {
        ImageFormat::Jpeg => ::image::DynamicImage::ImageRgb8(img.to_rgb8())
            .write_to(&mut buffer, ::image::ImageFormat::Jpeg)
            .context("encode jpeg")?,
        ImageFormat::Gif => img
            .write_to(&mut buffer, ::image::ImageFormat::Gif)
            .context("encode gif")?,
        ImageFormat::Bmp => img
            .write_to(&mut buffer, ::image::ImageFormat::Bmp)
            .context("encode bmp")?,
        ImageFormat::Tiff => img
            .write_to(&mut buffer, ::image::ImageFormat::Tiff)
            .context("encode tiff")?,
        _ => img
            .write_to(&mut buffer, ::image::ImageFormat::Png)
            .context("encode png")?,
    }
    Ok(buffer.into_inner())
}

#[cfg(feature = "transform")]
fn encodable_format(format: ImageFormat) -> ImageFormat {
    match format {
        ImageFormat::Png
        | ImageFormat::Jpeg
        | ImageFormat::Gif
        | ImageFormat::Bmp
        | ImageFormat::Tiff => format,
        _ => ImageFormat::Png,
    }
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

/// Recognize text in `bytes`, loading models from `models` or the default
/// `~/.cache/ocrs` cache.
#[cfg(feature = "ocr")]
pub(crate) fn run_ocr(bytes: &[u8], models: Option<&Path>) -> Result<OcrText> {
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};

    let dir = ocr_models_dir(models)?;
    let detection = dir.join("text-detection.rten");
    let recognition = dir.join("text-recognition.rten");
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

#[cfg(feature = "ocr")]
struct Decoded {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

#[cfg(feature = "ocr")]
fn decode(bytes: &[u8]) -> Result<Decoded> {
    let img = ::image::load_from_memory(bytes)
        .context("decode image")?
        .into_rgb8();
    let (width, height) = img.dimensions();
    Ok(Decoded {
        width,
        height,
        rgb: img.into_raw(),
    })
}

#[cfg(feature = "ocr")]
fn ocr_models_dir(models: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = models {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = std::env::var_os("READSEEK_OCR_MODELS") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("set --ocr-models or READSEEK_OCR_MODELS")?;
    Ok(PathBuf::from(home).join(".cache").join("ocrs"))
}
