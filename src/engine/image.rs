// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Image detection and metadata. Text/vision analysis lives in
//! [`crate::engine::vision`].

use std::io::Cursor;

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{
    DynamicImage, ImageFormat as RasterFormat, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use serde::Serialize;

const MAX_LONG_EDGE: u32 = 1568;
const JPEG_QUALITY: u8 = 80;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Return exact diagram labels from draw.io's embedded PNG `mxfile` metadata.
pub(crate) fn embedded_drawio_text(bytes: &[u8]) -> Option<String> {
    let encoded = png_text_chunk(bytes, b"mxfile")?;
    let xml = String::from_utf8(percent_decode(encoded)?).ok()?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut labels = Vec::new();

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(cell) | quick_xml::events::Event::Empty(cell))
                if cell.name().as_ref() == b"mxCell" =>
            {
                for attribute in cell.attributes().flatten() {
                    if attribute.key.as_ref() != b"value" {
                        continue;
                    }
                    let value = attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .ok()?;
                    let label = plain_drawio_label(&value);
                    if !label.is_empty() {
                        labels.push(label);
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
    }

    (!labels.is_empty()).then(|| labels.join("\n"))
}

fn png_text_chunk<'a>(bytes: &'a [u8], keyword: &[u8]) -> Option<&'a [u8]> {
    if bytes.get(..PNG_SIGNATURE.len())? != PNG_SIGNATURE {
        return None;
    }

    let mut pos = PNG_SIGNATURE.len();
    while pos.checked_add(12)? <= bytes.len() {
        let len = u32::from_be_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
        let data_start = pos.checked_add(8)?;
        let data_end = data_start.checked_add(len)?;
        let chunk_end = data_end.checked_add(4)?;
        if chunk_end > bytes.len() {
            return None;
        }
        if bytes.get(pos + 4..pos + 8)? == b"tEXt" {
            let data = bytes.get(data_start..data_end)?;
            let split = data.iter().position(|byte| *byte == 0)?;
            if data.get(..split)? == keyword {
                return data.get(split + 1..);
            }
        }
        pos = chunk_end;
    }
    None
}

fn percent_decode(value: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut pos = 0;
    while pos < value.len() {
        if value[pos] != b'%' {
            decoded.push(value[pos]);
            pos += 1;
            continue;
        }
        let high = hex_digit(*value.get(pos + 1)?)?;
        let low = hex_digit(*value.get(pos + 2)?)?;
        decoded.push(high << 4 | low);
        pos += 3;
    }
    Some(decoded)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn plain_drawio_label(value: &str) -> String {
    let value = value
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("&nbsp;", " ");
    let mut plain = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => plain.push(character),
            _ => {}
        }
    }
    plain
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

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

/// A model-ready, bounded raster image.
#[derive(Debug)]
pub(crate) struct PreparedImage {
    pub(crate) mime: &'static str,
    pub(crate) data: String,
}

/// Decode, bound, and encode an image for a model image-content block.
pub(crate) fn preprocess(bytes: &[u8]) -> Result<PreparedImage> {
    let image = image::load_from_memory(bytes).context("decode image")?;
    let image = image.resize(MAX_LONG_EDGE, MAX_LONG_EDGE, FilterType::Lanczos3);
    let (mime, bytes) = encode_image(&image)?;
    Ok(PreparedImage {
        mime,
        data: STANDARD.encode(bytes),
    })
}

fn encode_image(image: &DynamicImage) -> Result<(&'static str, Vec<u8>)> {
    let mut bytes = Vec::new();
    if image.color().has_alpha() {
        let mut cursor = Cursor::new(&mut bytes);
        image
            .write_to(&mut cursor, RasterFormat::Png)
            .context("encode PNG")?;
        Ok(("image/png", bytes))
    } else {
        JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY)
            .encode_image(image)
            .context("encode JPEG")?;
        Ok(("image/jpeg", bytes))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with_text(keyword: &[u8], value: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(keyword);
        data.push(0);
        data.extend_from_slice(value);

        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(
            &u32::try_from(data.len())
                .expect("test chunk length fits u32")
                .to_be_bytes(),
        );
        png.extend_from_slice(b"tEXt");
        png.extend_from_slice(&data);
        png.extend_from_slice(&[0; 4]);
        png
    }

    #[test]
    fn extracts_embedded_drawio_labels() {
        let encoded = b"%3Cmxfile%3E%3CmxCell%20value%3D%22Title%26%23xa%3BSecond%22%2F%3E%3CmxCell%20value%3D%22%26lt%3Bb%26gt%3BBold%26lt%3B%2Fb%26gt%3B%22%2F%3E%3C%2Fmxfile%3E";
        let png = png_with_text(b"mxfile", encoded);

        assert_eq!(
            embedded_drawio_text(&png).as_deref(),
            Some("Title\nSecond\nBold")
        );
    }

    #[test]
    fn ignores_unrelated_png_text() {
        let png = png_with_text(b"Comment", b"plain text");

        assert_eq!(embedded_drawio_text(&png), None);
    }
}
