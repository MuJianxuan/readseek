// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Image detection and metadata. Text/vision analysis lives in
//! [`crate::engine::vision`].

use serde::Serialize;

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
