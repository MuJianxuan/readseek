// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Page-oriented PDF extraction.

use std::fmt::Write as _;

use anyhow::{Context, Result};
use pdf_oxide::{Destination, OutlineItem, PdfDocument};
use serde::Serialize;

use crate::engine::document::{Document, DocumentFormat, Node, NodeKind, SourceAnchor};
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

pub(crate) fn extract_document(
    path: &std::path::Path,
    bytes: &[u8],
    id: String,
) -> Result<Document> {
    let pdf = PdfDocument::from_bytes(bytes.to_vec()).context("parse PDF")?;
    let pages = pdf.page_count().context("read PDF page count")?;
    let outline = pdf
        .get_outline()
        .context("read PDF outline")?
        .unwrap_or_default();
    let mut nodes = Vec::new();
    append_outline_nodes(&id, &outline, None, "", &mut nodes);

    let title = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
        .to_owned();
    Ok(Document {
        id,
        format: DocumentFormat::Pdf,
        source: path.to_path_buf(),
        title,
        pages,
        nodes,
        assets: Vec::new(),
    })
}

fn append_outline_nodes(
    document_id: &str,
    items: &[OutlineItem],
    parent_id: Option<&str>,
    parent_position: &str,
    nodes: &mut Vec<Node>,
) {
    for (index, item) in items.iter().enumerate() {
        let position = if parent_position.is_empty() {
            index.to_string()
        } else {
            format!("{parent_position}.{index}")
        };
        let digest = blake3::hash(format!("{document_id}\0{position}").as_bytes()).to_string();
        let id = format!("n_{}", &digest[..16]);
        let source_anchor = item.dest.as_ref().map(|destination| match destination {
            Destination::PageIndex(page) => SourceAnchor {
                page: Some(page + 1),
                destination: None,
            },
            Destination::Named(destination) => SourceAnchor {
                page: None,
                destination: Some(destination.clone()),
            },
        });
        nodes.push(Node {
            id: id.clone(),
            parent_id: parent_id.map(str::to_owned),
            kind: NodeKind::Section,
            title: Some(item.title.clone()),
            text: None,
            source_anchor,
        });
        append_outline_nodes(document_id, &item.children, Some(&id), &position, nodes);
    }
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
