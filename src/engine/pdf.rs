// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Page-oriented PDF extraction.

use std::fmt::Write as _;

use anyhow::{Context, Result};
use pdf_oxide::{Destination, OutlineItem, PdfDocument, RegionRole};
use serde::Serialize;

use crate::engine::document::{
    BoundingBox, Document, DocumentFormat, Node, NodeKind, SourceAnchor,
};
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
    let mut document = document_outline(path, &pdf, id, pages)?;
    for page_index in 0..pages {
        let page = page_index + 1;
        let structured = pdf
            .extract_structured(page_index)
            .with_context(|| format!("extract structure from PDF page {page}"))?;
        let parent_id = outline_parent_for_page(&document.nodes, page).map(str::to_owned);
        append_page_nodes(
            &document.id,
            page,
            parent_id,
            &structured,
            &mut document.nodes,
        );
    }
    Ok(document)
}

pub(crate) fn extract_outline_document(
    path: &std::path::Path,
    bytes: &[u8],
    id: String,
) -> Result<Document> {
    let pdf = PdfDocument::from_bytes(bytes.to_vec()).context("parse PDF")?;
    let pages = pdf.page_count().context("read PDF page count")?;
    document_outline(path, &pdf, id, pages)
}

fn document_outline(
    path: &std::path::Path,
    pdf: &PdfDocument,
    id: String,
    pages: usize,
) -> Result<Document> {
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
        let id = node_id(document_id, &position);
        let source_anchor = item.dest.as_ref().map(|destination| match destination {
            Destination::PageIndex(page) => SourceAnchor {
                page: Some(page + 1),
                destination: None,
                bbox: None,
            },
            Destination::Named(destination) => SourceAnchor {
                page: None,
                destination: Some(destination.clone()),
                bbox: None,
            },
        });
        nodes.push(Node {
            id: id.clone(),
            parent_id: parent_id.map(str::to_owned),
            kind: NodeKind::Section,
            title: Some(item.title.clone()),
            text: None,
            level: None,
            column: None,
            source_anchor,
        });
        append_outline_nodes(document_id, &item.children, Some(&id), &position, nodes);
    }
}

fn append_page_nodes(
    document_id: &str,
    page: usize,
    parent_id: Option<String>,
    structured: &pdf_oxide::StructuredPage,
    nodes: &mut Vec<Node>,
) {
    let page_id = node_id(document_id, &format!("page:{page}"));
    nodes.push(Node {
        id: page_id.clone(),
        parent_id,
        kind: NodeKind::Page,
        title: Some(format!("Page {page}")),
        text: None,
        level: None,
        column: None,
        source_anchor: Some(SourceAnchor {
            page: Some(page),
            destination: None,
            bbox: Some(BoundingBox {
                x: 0.0,
                y: 0.0,
                width: structured.page_width,
                height: structured.page_height,
            }),
        }),
    });

    let mut headings = Vec::new();
    for (position, region) in structured.regions.iter().enumerate() {
        append_region_node(
            document_id,
            page,
            position,
            region,
            &page_id,
            &mut headings,
            nodes,
        );
    }
}

fn outline_parent_for_page(nodes: &[Node], page: usize) -> Option<&str> {
    nodes
        .iter()
        .enumerate()
        .filter_map(|(position, node)| {
            if node.kind != NodeKind::Section {
                return None;
            }
            let section_page = node.source_anchor.as_ref().and_then(|anchor| anchor.page)?;
            (section_page <= page).then_some((section_page, position, node.id.as_str()))
        })
        .max_by_key(|(section_page, position, _)| (*section_page, *position))
        .map(|(_, _, id)| id)
}

fn append_region_node(
    document_id: &str,
    page: usize,
    position: usize,
    region: &pdf_oxide::StructuredRegion,
    page_id: &str,
    headings: &mut Vec<(u8, String)>,
    nodes: &mut Vec<Node>,
) {
    let text = region.text.trim();
    if text.is_empty() {
        return;
    }
    let (kind, level) = match region.kind {
        RegionRole::BodyBlock => (NodeKind::Paragraph, None),
        RegionRole::StructuralHeading { level } => (NodeKind::Heading, Some(level)),
        RegionRole::MarginalLabel => (NodeKind::MarginalLabel, None),
        RegionRole::Header => (NodeKind::Header, None),
        RegionRole::Footer => (NodeKind::Footer, None),
        RegionRole::PageNumber => (NodeKind::PageNumber, None),
        RegionRole::Artifact => (NodeKind::Artifact, None),
    };
    let id = node_id(document_id, &format!("page:{page}:region:{position}"));
    let parent_id = if let Some(level) = level {
        while headings
            .last()
            .is_some_and(|(parent_level, _)| *parent_level >= level)
        {
            headings.pop();
        }
        headings
            .last()
            .map_or_else(|| page_id.to_owned(), |(_, id)| id.clone())
    } else if matches!(kind, NodeKind::Paragraph | NodeKind::MarginalLabel) {
        headings
            .last()
            .map_or_else(|| page_id.to_owned(), |(_, id)| id.clone())
    } else {
        page_id.to_owned()
    };
    let (title, text) = if kind == NodeKind::Heading {
        (Some(text.to_owned()), None)
    } else {
        (None, Some(text.to_owned()))
    };
    nodes.push(Node {
        id: id.clone(),
        parent_id: Some(parent_id),
        kind,
        title,
        text,
        level,
        column: region.column_index,
        source_anchor: Some(SourceAnchor {
            page: Some(page),
            destination: None,
            bbox: Some(BoundingBox {
                x: region.bbox.x,
                y: region.bbox.y,
                width: region.bbox.width,
                height: region.bbox.height,
            }),
        }),
    });
    if let Some(level) = level {
        headings.push((level, id));
    }
}

fn node_id(document_id: &str, position: &str) -> String {
    let digest = blake3::hash(format!("{document_id}\0{position}").as_bytes()).to_string();
    format!("n_{}", &digest[..16])
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
