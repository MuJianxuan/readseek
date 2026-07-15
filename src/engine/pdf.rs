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
                caption: mode == ImageMode::Caption,
                objects: mode == ImageMode::Objects,
                ocr: mode == ImageMode::Ocr,
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

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn extracts_page_text_and_prepared_images() {
        let bytes = sample_pdf();
        let info = probe(&bytes).unwrap();
        assert_eq!(info.pages, 1);

        let output = read(&bytes, ImageMode::None, |_, _| {
            panic!("none mode must not run vision analysis")
        })
        .unwrap();
        let json = serde_json::to_value(output).unwrap();
        assert_eq!(json["format"], "pdf");
        assert_eq!(json["pages"], 1);
        assert!(
            json["markdown"]
                .as_str()
                .unwrap()
                .starts_with("<!-- readseek:page 1 -->\n")
        );
        assert!(json["markdown"].as_str().unwrap().contains("Hello PDF"));
        assert_eq!(json["images"][0]["page"], 1);
        assert_eq!(json["images"][0]["width"], 1);
        assert_eq!(json["images"][0]["height"], 1);
        assert_eq!(json["images"][0]["mode"], "none");
        assert_eq!(json["images"][0]["encoding"], "base64");
        assert!(json["images"][0]["data"].as_str().is_some());
    }

    #[test]
    fn analyzes_only_the_selected_image_mode() {
        let output = read(&sample_pdf(), ImageMode::Caption, |_, request| {
            assert!(request.caption);
            assert!(!request.objects);
            assert!(!request.ocr);
            Analysis {
                caption: Some("red square".to_owned()),
                objects: None,
                ocr: None,
            }
        })
        .unwrap();
        let json = serde_json::to_value(output).unwrap();
        assert_eq!(json["images"][0]["caption"], "red square");
        assert_eq!(json["images"][0]["mode"], "caption");
        assert_eq!(json["images"][0]["data"], Value::Null);
    }

    fn sample_pdf() -> Vec<u8> {
        let content = b"q 10 0 0 10 0 0 cm /Im1 Do Q\nBT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            stream_object("", content),
            stream_object(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 ",
                &[255, 0, 0],
            ),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn stream_object(dictionary: &str, data: &[u8]) -> Vec<u8> {
        let mut object = format!("<< {dictionary}/Length {} >>\nstream\n", data.len()).into_bytes();
        object.extend_from_slice(data);
        object.extend_from_slice(b"\nendstream");
        object
    }
}
