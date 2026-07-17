// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Page-oriented PDF extraction.

use std::fmt::Write as _;

use anyhow::{Context, Result};
use pdf_oxide::PdfDocument;
use serde::Serialize;

use crate::engine::output::ImageMode;
use crate::engine::vision::{Analysis, DetectedObject, Request};

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct PdfInfo {
    pub(crate) pages: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadPdfOutput {
    format: &'static str,
    pages: usize,
    markdown: String,
    images: Vec<PdfImageOutput>,
}

#[derive(Debug, Serialize)]
struct PdfImageOutput {
    page: usize,
    width: u32,
    height: u32,
    mime: &'static str,
    mode: ImageMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    objects: Option<Vec<DetectedObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ocr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
}

pub(crate) fn probe(bytes: &[u8]) -> Result<PdfInfo> {
    let document = PdfDocument::from_bytes(bytes.to_vec()).context("parse PDF")?;
    let pages = document.page_count().context("read PDF page count")?;
    Ok(PdfInfo { pages })
}

pub(crate) fn read(
    bytes: &[u8],
    mode: ImageMode,
    mut analyze: impl FnMut(&[u8], Request) -> Analysis,
) -> Result<ReadPdfOutput> {
    let document = PdfDocument::from_bytes(bytes.to_vec()).context("parse PDF")?;
    let pages = document.page_count().context("read PDF page count")?;
    let mut markdown = String::new();
    let mut images = Vec::new();

    for page_index in 0..pages {
        let page = page_index + 1;
        let text = document
            .extract_text(page_index)
            .with_context(|| format!("extract text from PDF page {page}"))?;
        if page_index > 0 {
            markdown.push('\n');
        }
        writeln!(markdown, "<!-- readseek:page {page} -->").unwrap();
        let text = text.trim();
        if !text.is_empty() {
            markdown.push_str(text);
            markdown.push('\n');
        }

        let handles = document
            .page_image_handles(page_index)
            .with_context(|| format!("enumerate images on PDF page {page}"))?;
        for handle in handles {
            let width = handle.width;
            let height = handle.height;
            let image = match handle.decode() {
                Ok(image) => image,
                Err(error) => {
                    log::warn!("PDF page {page} image skipped: {error}");
                    continue;
                }
            };
            let png = match image.to_png_bytes() {
                Ok(png) => png,
                Err(error) => {
                    log::warn!("PDF page {page} image skipped: {error}");
                    continue;
                }
            };
            let request = Request {
                caption: matches!(mode, ImageMode::All | ImageMode::Caption),
                objects: matches!(mode, ImageMode::All | ImageMode::Objects),
                ocr: matches!(mode, ImageMode::All | ImageMode::Ocr),
            };
            let analysis = if mode == ImageMode::None {
                Analysis::default()
            } else {
                analyze(&png, request)
            };
            let prepared = match (mode == ImageMode::None)
                .then(|| crate::engine::image::preprocess(&png))
                .transpose()
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    log::warn!("PDF page {page} image skipped: {error}");
                    continue;
                }
            };
            let (mime, encoding, data) = if let Some(prepared) = prepared {
                (prepared.mime, Some("base64"), Some(prepared.data))
            } else {
                ("image/png", None, None)
            };
            let (caption, objects, ocr) = match mode {
                ImageMode::None => (None, None, None),
                ImageMode::All => (analysis.caption, analysis.objects, analysis.ocr),
                ImageMode::Caption => (analysis.caption, None, None),
                ImageMode::Objects => (None, analysis.objects, None),
                ImageMode::Ocr => (None, None, analysis.ocr),
            };
            images.push(PdfImageOutput {
                page,
                width,
                height,
                mime,
                mode,
                caption,
                objects,
                ocr,
                encoding,
                data,
            });
        }
    }

    Ok(ReadPdfOutput {
        format: "pdf",
        pages,
        markdown,
        images,
    })
}
