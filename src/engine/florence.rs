// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Florence-2 vision model: OCR (with regions), captioning, and object
//! detection. The four INT8 ONNX graphs and the tokenizer are embedded into the
//! binary (fetched by `build.rs`) and executed with onnxruntime via `ort`.

// Tensor math intrinsically converts between integer indices/ids, pixel bins,
// and floats; these casts are intentional and bounded.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use anyhow::{Context as _, Result, anyhow};
use image::imageops::FilterType;
use ort::session::{Session, SessionInputs};
use ort::value::Tensor;
use serde::Serialize;
use tokenizers::Tokenizer;

const VISION_ENCODER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/vision_encoder_int8.onnx"));
const EMBED_TOKENS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/embed_tokens_int8.onnx"));
const ENCODER_MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/encoder_model_int8.onnx"));
const DECODER_MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/decoder_model_int8.onnx"));
const TOKENIZER_JSON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tokenizer.json"));

const IMAGE_SIZE: u32 = 768;
const HIDDEN: usize = 768;
const LOC_BINS: f32 = 1000.0;
const DECODER_START: i64 = 2;
const FORCED_BOS: i64 = 0;
const EOS: i64 = 2;
const MAX_NEW_TOKENS: usize = 1025;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

const PROMPT_OCR: &str = "What is the text in the image, with regions?";
const PROMPT_CAPTION: &str = "Describe with a paragraph what is shown in the image.";
const PROMPT_OBJECTS: &str = "Locate the objects with category name in the image.";

/// Text recognized in an image, with per-region bounding quads.
#[derive(Debug, Serialize)]
pub(crate) struct OcrText {
    text: String,
    regions: Vec<OcrRegion>,
}

/// One recognized text run with its bounding quad `[x1,y1,x2,y2,x3,y3,x4,y4]`.
#[derive(Debug, Serialize)]
pub(crate) struct OcrRegion {
    text: String,
    quad: [i32; 8],
}

/// A detected object with its category label and bounding box `[x1,y1,x2,y2]`.
#[derive(Debug, Serialize)]
pub(crate) struct DetectedObject {
    label: String,
    bbox: [i32; 4],
}

/// Which Florence-2 tasks to run against an image.
#[derive(Clone, Copy)]
pub(crate) struct Request {
    pub(crate) transcribe: bool,
    pub(crate) caption: bool,
    pub(crate) objects: bool,
}

/// Results of the requested Florence-2 tasks.
#[derive(Default)]
pub(crate) struct Analysis {
    pub(crate) transcribe: Option<OcrText>,
    pub(crate) caption: Option<String>,
    pub(crate) objects: Option<Vec<DetectedObject>>,
}

/// Run the requested tasks against `image_bytes`, loading the model once. The
/// vision encoder runs a single time and its features are reused per task.
pub(crate) fn analyze(image_bytes: &[u8], request: Request) -> Result<Analysis> {
    let mut model = Model::load()?;
    let image = decode(image_bytes)?;
    let features = model.vision_features(&image)?;

    let mut analysis = Analysis::default();
    if request.transcribe {
        let raw = model.generate(&features, PROMPT_OCR)?;
        analysis.transcribe = Some(parse_ocr(&raw, image.width, image.height));
    }
    if request.caption {
        let raw = model.generate(&features, PROMPT_CAPTION)?;
        analysis.caption = Some(strip_markers(&raw));
    }
    if request.objects {
        let raw = model.generate(&features, PROMPT_OBJECTS)?;
        analysis.objects = Some(parse_objects(&raw, image.width, image.height));
    }
    Ok(analysis)
}

struct Model {
    vision: Session,
    embed: Session,
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
}

impl Model {
    fn load() -> Result<Self> {
        Ok(Self {
            vision: session(VISION_ENCODER)?,
            embed: session(EMBED_TOKENS)?,
            encoder: session(ENCODER_MODEL)?,
            decoder: session(DECODER_MODEL)?,
            tokenizer: Tokenizer::from_bytes(TOKENIZER_JSON)
                .map_err(|err| anyhow!("load tokenizer: {err}"))?,
        })
    }

    /// `image_features [n_img, HIDDEN]` for the decoded image.
    fn vision_features(&mut self, image: &Decoded) -> Result<Vec<f32>> {
        let pixels = preprocess(image);
        let input = Tensor::from_array((
            [1usize, 3, IMAGE_SIZE as usize, IMAGE_SIZE as usize],
            pixels,
        ))?;
        let (_, features) = run(&mut self.vision, ort::inputs!["pixel_values" => input])?;
        Ok(features)
    }

    /// Greedily decode the answer for `prompt` given precomputed image features,
    /// returning the raw token text (with `<loc_*>` and `<s>`/`</s>` markers).
    fn generate(&mut self, features: &[f32], prompt: &str) -> Result<String> {
        let encoded = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|err| anyhow!("tokenize prompt: {err}"))?;
        let prompt_ids: Vec<i64> = encoded.get_ids().iter().map(|&id| i64::from(id)).collect();
        let prompt_embeds = self.embed(&prompt_ids)?;

        // Encoder input is [image_features ; prompt_embeds].
        let enc_len = features.len() / HIDDEN + prompt_ids.len();
        let mut enc_in = Vec::with_capacity(enc_len * HIDDEN);
        enc_in.extend_from_slice(features);
        enc_in.extend_from_slice(&prompt_embeds);
        let enc_embeds = Tensor::from_array(([1usize, enc_len, HIDDEN], enc_in))?;
        let enc_mask = Tensor::from_array(([1usize, enc_len], vec![1i64; enc_len]))?;
        let (_, encoder_hidden) = run(
            &mut self.encoder,
            ort::inputs!["attention_mask" => enc_mask, "inputs_embeds" => enc_embeds],
        )?;

        let mut ids = vec![DECODER_START];
        for step in 0..MAX_NEW_TOKENS {
            let embeds = self.embed(&ids)?;
            let dlen = ids.len();
            let dec_embeds = Tensor::from_array(([1usize, dlen, HIDDEN], embeds))?;
            let mask = Tensor::from_array(([1usize, enc_len], vec![1i64; enc_len]))?;
            let hidden = Tensor::from_array(([1usize, enc_len, HIDDEN], encoder_hidden.clone()))?;
            let (shape, logits) = run(
                &mut self.decoder,
                ort::inputs![
                    "encoder_attention_mask" => mask,
                    "encoder_hidden_states" => hidden,
                    "inputs_embeds" => dec_embeds
                ],
            )?;
            let vocab = shape[2] as usize;
            let last = &logits[(dlen - 1) * vocab..dlen * vocab];
            let next = if step == 0 { FORCED_BOS } else { argmax(last) };
            ids.push(next);
            if step > 0 && next == EOS {
                break;
            }
        }

        let out_ids: Vec<u32> = ids[1..].iter().map(|&id| id as u32).collect();
        self.tokenizer
            .decode(&out_ids, false)
            .map_err(|err| anyhow!("detokenize: {err}"))
    }

    /// `inputs_embeds` for `ids`, shaped `[len, HIDDEN]` flattened.
    fn embed(&mut self, ids: &[i64]) -> Result<Vec<f32>> {
        let input = Tensor::from_array(([1usize, ids.len()], ids.to_vec()))?;
        let (_, embeds) = run(&mut self.embed, ort::inputs!["input_ids" => input])?;
        Ok(embeds)
    }
}

fn session(bytes: &[u8]) -> Result<Session> {
    Session::builder()?
        .commit_from_memory(bytes)
        .map_err(Into::into)
}

/// Run `session` with `inputs`, returning the first output as `(shape, data)`.
fn run<'i, 'v: 'i, const N: usize>(
    session: &mut Session,
    inputs: impl Into<SessionInputs<'i, 'v, N>>,
) -> Result<(Vec<i64>, Vec<f32>)> {
    let outputs = session.run(inputs)?;
    let (key, _) = outputs.iter().next().context("model produced no output")?;
    let key = key.to_string();
    let (shape, data) = outputs[key.as_str()].try_extract_tensor::<f32>()?;
    Ok((shape.iter().copied().collect(), data.to_vec()))
}

struct Decoded {
    width: u32,
    height: u32,
    rgb: image::RgbImage,
}

fn decode(bytes: &[u8]) -> Result<Decoded> {
    let img = image::load_from_memory(bytes)
        .context("decode image")?
        .to_rgb8();
    let (width, height) = img.dimensions();
    Ok(Decoded {
        width,
        height,
        rgb: img,
    })
}

/// CLIP preprocessing: resize to 768×768, scale to `[0,1]`, normalize, to NCHW.
fn preprocess(image: &Decoded) -> Vec<f32> {
    let resized =
        image::imageops::resize(&image.rgb, IMAGE_SIZE, IMAGE_SIZE, FilterType::CatmullRom);
    let side = IMAGE_SIZE as usize;
    let mut data = vec![0f32; 3 * side * side];
    for y in 0..side {
        for x in 0..side {
            let pixel = resized.get_pixel(x as u32, y as u32);
            for c in 0..3 {
                data[c * side * side + y * side + x] =
                    (f32::from(pixel[c]) / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    data
}

fn argmax(row: &[f32]) -> i64 {
    let mut best = 0usize;
    for (i, &value) in row.iter().enumerate() {
        if value > row[best] {
            best = i;
        }
    }
    best as i64
}

/// Convert a `<loc_n>` bin to a pixel coordinate along an axis of length `dim`.
fn loc_to_px(loc: i32, dim: u32) -> i32 {
    ((loc as f32 + 0.5) / LOC_BINS * dim as f32).round() as i32
}

/// A text run with the run of `<loc_*>` bin values that follows it.
struct Segment {
    text: String,
    locs: Vec<i32>,
}

enum Token {
    Text(String),
    Loc(i32),
}

/// Tokenize raw model output into text runs and `<loc_*>` bins, dropping
/// `<s>`/`</s>` markers.
fn raw_tokens(raw: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut rest = raw;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<loc_")
            && let Some(end) = after.find('>')
            && let Ok(loc) = after[..end].parse::<i32>()
        {
            flush_text(&mut text, &mut tokens);
            tokens.push(Token::Loc(loc));
            rest = &after[end + 1..];
            continue;
        }
        if let Some(tail) = rest
            .strip_prefix("<s>")
            .or_else(|| rest.strip_prefix("</s>"))
        {
            rest = tail;
            continue;
        }
        let mut chars = rest.chars();
        let ch = chars.next().expect("rest is non-empty");
        text.push(ch);
        rest = chars.as_str();
    }
    flush_text(&mut text, &mut tokens);
    tokens
}

fn flush_text(text: &mut String, tokens: &mut Vec<Token>) {
    let trimmed = std::mem::take(text).trim().to_string();
    if !trimmed.is_empty() {
        tokens.push(Token::Text(trimmed));
    }
}

/// Group tokens so each text run owns the `<loc_*>` bins that follow it.
fn segments(raw: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for token in raw_tokens(raw) {
        match token {
            Token::Text(text) => segments.push(Segment {
                text,
                locs: Vec::new(),
            }),
            Token::Loc(loc) => match segments.last_mut() {
                Some(last) => last.locs.push(loc),
                None => segments.push(Segment {
                    text: String::new(),
                    locs: vec![loc],
                }),
            },
        }
    }
    segments
}

fn parse_ocr(raw: &str, width: u32, height: u32) -> OcrText {
    let mut regions = Vec::new();
    for segment in segments(raw) {
        if segment.locs.len() < 8 || segment.text.is_empty() {
            continue;
        }
        for quad_locs in segment.locs.chunks_exact(8) {
            let mut quad = [0i32; 8];
            for (i, &loc) in quad_locs.iter().enumerate() {
                quad[i] = loc_to_px(loc, if i % 2 == 0 { width } else { height });
            }
            regions.push(OcrRegion {
                text: segment.text.clone(),
                quad,
            });
        }
    }
    let text = regions
        .iter()
        .map(|region| region.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    OcrText { text, regions }
}

fn parse_objects(raw: &str, width: u32, height: u32) -> Vec<DetectedObject> {
    let mut objects = Vec::new();
    for segment in segments(raw) {
        if segment.locs.len() < 4 || segment.text.is_empty() {
            continue;
        }
        for box_locs in segment.locs.chunks_exact(4) {
            objects.push(DetectedObject {
                label: segment.text.clone(),
                bbox: [
                    loc_to_px(box_locs[0], width),
                    loc_to_px(box_locs[1], height),
                    loc_to_px(box_locs[2], width),
                    loc_to_px(box_locs[3], height),
                ],
            });
        }
    }
    objects
}

/// Remove `<s>`/`</s>` and any `<loc_*>` markers, returning trimmed plain text.
fn strip_markers(raw: &str) -> String {
    segments(raw)
        .into_iter()
        .map(|segment| segment.text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}
