// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Fixed scalar decoder for the Qwen3-VL-2B `Q8_0` text model.

#![allow(clippy::cast_precision_loss)]

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use rayon::prelude::*;

use super::gguf::{Gguf, TensorType};
use super::kernels::{
    f32_to_fp16, fp16_to_f32, matrix_vector, matrix_vector_argmax, matrix_vector_pair,
    matrix_vector_triple, rms_norm, silu, softmax, vector_add, vector_multiply,
};
use super::tokenizer::{Tokenizer, Utf8Decoder};
use super::vision::VisionEmbedding;

const LAYER_COUNT: usize = 28;
const EMBEDDING_SIZE: usize = 2_048;
const FEED_FORWARD_SIZE: usize = 6_144;
const QUERY_HEAD_COUNT: usize = 16;
const KEY_VALUE_HEAD_COUNT: usize = 8;
const HEAD_SIZE: usize = 128;
const KEY_VALUE_SIZE: usize = KEY_VALUE_HEAD_COUNT * HEAD_SIZE;
const QUERY_GROUP_SIZE: usize = QUERY_HEAD_COUNT / KEY_VALUE_HEAD_COUNT;
const DEEPSTACK_LAYER_COUNT: usize = 3;
const IMAGE_EMBEDDING_SIZE: usize = EMBEDDING_SIZE * (DEEPSTACK_LAYER_COUNT + 1);
const RMS_NORM_EPSILON: f32 = 1.0e-6;
const ROPE_BASE: f32 = 5_000_000.0;
const ROPE_SECTIONS: [u32; 3] = [24, 20, 20];

const IM_START: &str = "<|im_start|>";
const IM_END: &str = "<|im_end|>";
const VISION_START: &str = "<|vision_start|>";
const VISION_END: &str = "<|vision_end|>";

/// Text and timing information from one greedy generation request.
#[derive(Debug)]
pub struct Generation {
    pub text: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    pub prefill_duration: Duration,
    pub decode_duration: Duration,
}

/// Loaded, immutable Qwen3-VL-2B text decoder.
pub struct TextModel {
    gguf: Gguf,
    tokenizer: Tokenizer,
    layers: Vec<TextLayerNames>,
}

struct TextLayerNames {
    attention_norm: String,
    query: String,
    key: String,
    value: String,
    query_norm: String,
    key_norm: String,
    attention_output: String,
    feed_forward_norm: String,
    feed_forward_gate: String,
    feed_forward_up: String,
    feed_forward_down: String,
}

impl TextLayerNames {
    fn new(layer: usize) -> Self {
        let prefix = format!("blk.{layer}");
        Self {
            attention_norm: format!("{prefix}.attn_norm.weight"),
            query: format!("{prefix}.attn_q.weight"),
            key: format!("{prefix}.attn_k.weight"),
            value: format!("{prefix}.attn_v.weight"),
            query_norm: format!("{prefix}.attn_q_norm.weight"),
            key_norm: format!("{prefix}.attn_k_norm.weight"),
            attention_output: format!("{prefix}.attn_output.weight"),
            feed_forward_norm: format!("{prefix}.ffn_norm.weight"),
            feed_forward_gate: format!("{prefix}.ffn_gate.weight"),
            feed_forward_up: format!("{prefix}.ffn_up.weight"),
            feed_forward_down: format!("{prefix}.ffn_down.weight"),
        }
    }
}

impl TextModel {
    /// Load and fully validate the pinned Qwen3-VL-2B `Q8_0` decoder.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let gguf = Gguf::load(path).context("parse text GGUF")?;
        validate_metadata(&gguf).context("validate text GGUF metadata")?;
        validate_tensors(&gguf).context("validate text GGUF tensors")?;

        let tokenizer = Tokenizer::from_gguf(&gguf).context("load text tokenizer")?;
        validate_special_tokens(&tokenizer).context("validate text special tokens")?;
        let layers = (0..LAYER_COUNT).map(TextLayerNames::new).collect();
        Ok(Self {
            gguf,
            tokenizer,
            layers,
        })
    }

    /// Generate greedily from the fixed `ReadSeek` one-image chat template.
    #[allow(clippy::too_many_lines)]
    pub fn generate(
        &self,
        prompt: &str,
        image: &VisionEmbedding,
        max_new_tokens: usize,
    ) -> Result<Generation> {
        let image_tokens = validate_image(image)?;
        let prefix = format!("{IM_START}user\n{VISION_START}");
        let suffix = format!("{VISION_END}{prompt}{IM_END}\n{IM_START}assistant\n");
        let prefix_tokens = self
            .tokenizer
            .encode(&prefix, true)
            .context("tokenize chat prefix")?;
        let suffix_tokens = self
            .tokenizer
            .encode(&suffix, true)
            .context("tokenize chat suffix")?;
        let prompt_tokens = prefix_tokens
            .len()
            .checked_add(image_tokens)
            .and_then(|count| count.checked_add(suffix_tokens.len()))
            .context("prompt token count overflow")?;

        let cache_tokens = prompt_tokens
            .checked_add(max_new_tokens)
            .context("key/value cache token count overflow")?;
        let mut cache = KvCache::new(cache_tokens)?;
        let mut attention_scratch = TextAttentionScratch::new(cache_tokens)?;
        let mut scalar_position = 0_usize;
        let mut last_hidden = None;
        let prefill_started = Instant::now();

        for token in prefix_tokens {
            let embedding = self.token_embedding(token)?;
            let position = text_position(scalar_position)?;
            last_hidden = Some(self.forward_token(
                &embedding,
                position,
                None,
                &mut cache,
                &mut attention_scratch,
            )?);
            scalar_position = scalar_position
                .checked_add(1)
                .context("text position overflow")?;
        }

        for (index, row) in image.values.chunks_exact(IMAGE_EMBEDDING_SIZE).enumerate() {
            let position =
                image_position(scalar_position, index, image.grid_width, image.grid_height)?;
            let base = &row[..EMBEDDING_SIZE];
            let auxiliaries = &row[EMBEDDING_SIZE..];
            last_hidden = Some(self.forward_token(
                base,
                position,
                Some(auxiliaries),
                &mut cache,
                &mut attention_scratch,
            )?);
        }
        scalar_position = scalar_position
            .checked_add(image.grid_width.max(image.grid_height))
            .context("image position overflow")?;

        for token in suffix_tokens {
            let embedding = self.token_embedding(token)?;
            let position = text_position(scalar_position)?;
            last_hidden = Some(self.forward_token(
                &embedding,
                position,
                None,
                &mut cache,
                &mut attention_scratch,
            )?);
            scalar_position = scalar_position
                .checked_add(1)
                .context("text position overflow")?;
        }

        let hidden = last_hidden.context("chat prompt produced no decoder input")?;
        let mut token = self.greedy_token(&hidden)?;
        let prefill_duration = prefill_started.elapsed();

        let decode_started = Instant::now();
        let mut decoder = Utf8Decoder::new();
        let mut json = JsonObjectTracker::default();
        let mut text = String::new();
        let mut generated_tokens = 0_usize;

        while generated_tokens < max_new_tokens {
            if self.tokenizer.is_eos(token) {
                break;
            }
            generated_tokens += 1;

            let mut json_complete = false;
            if let Some(decoded) = decoder.push(self.tokenizer.token_piece(token)?)? {
                if let Some(end) = json.push(&decoded) {
                    text.push_str(&decoded[..end]);
                    json_complete = true;
                } else {
                    text.push_str(&decoded);
                }
            }
            if json_complete || generated_tokens == max_new_tokens {
                break;
            }

            let embedding = self.token_embedding(token)?;
            let position = text_position(scalar_position)?;
            let hidden = self.forward_token(
                &embedding,
                position,
                None,
                &mut cache,
                &mut attention_scratch,
            )?;
            scalar_position = scalar_position
                .checked_add(1)
                .context("text position overflow")?;
            token = self.greedy_token(&hidden)?;
        }

        text.push_str(&decoder.finish()?);
        let decode_duration = decode_started.elapsed();
        Ok(Generation {
            text,
            prompt_tokens,
            generated_tokens,
            prefill_duration,
            decode_duration,
        })
    }

    fn token_embedding(&self, token: u32) -> Result<Vec<f32>> {
        let tensor = self.gguf.tensor("token_embd.weight")?;
        let row = tensor
            .q8_row_bytes(token as usize)
            .with_context(|| format!("read token embedding {token}"))?;
        let mut embedding = vec![0.0; EMBEDDING_SIZE];
        super::kernels::dequantize_q8_0_row(row, &mut embedding)?;
        Ok(embedding)
    }

    fn forward_token(
        &self,
        input: &[f32],
        position: Position,
        deepstack: Option<&[f32]>,
        cache: &mut KvCache,
        attention_scratch: &mut TextAttentionScratch,
    ) -> Result<Vec<f32>> {
        ensure!(
            input.len() == EMBEDDING_SIZE,
            "decoder input has {} values, expected {EMBEDDING_SIZE}",
            input.len()
        );
        if let Some(deepstack) = deepstack {
            ensure!(
                deepstack.len() == DEEPSTACK_LAYER_COUNT * EMBEDDING_SIZE,
                "DeepStack input has {} values, expected {}",
                deepstack.len(),
                DEEPSTACK_LAYER_COUNT * EMBEDDING_SIZE
            );
        }

        let rope = ImRope::new(position);

        let mut hidden = input.to_vec();
        for (layer, names) in self.layers.iter().enumerate() {
            let attention_norm = self.gguf.tensor(&names.attention_norm)?;
            let normalized = rms_norm(
                &hidden,
                EMBEDDING_SIZE,
                attention_norm.f32_slice()?,
                RMS_NORM_EPSILON,
            )?;

            let (mut query, mut key, value) = matrix_vector_triple(
                &self.gguf.tensor(&names.query)?,
                &self.gguf.tensor(&names.key)?,
                &self.gguf.tensor(&names.value)?,
                &normalized,
            )?;

            let query_norm = self.gguf.tensor(&names.query_norm)?;
            query = rms_norm(&query, HEAD_SIZE, query_norm.f32_slice()?, RMS_NORM_EPSILON)?;
            let key_norm = self.gguf.tensor(&names.key_norm)?;
            key = rms_norm(&key, HEAD_SIZE, key_norm.f32_slice()?, RMS_NORM_EPSILON)?;
            apply_im_rope(&mut query, &rope)?;
            apply_im_rope(&mut key, &rope)?;

            cache.layers[layer].append(&key, &value)?;
            causal_gqa(&query, &cache.layers[layer], attention_scratch)?;
            let projected = matrix_vector(
                &self.gguf.tensor(&names.attention_output)?,
                &attention_scratch.output,
            )?;
            vector_add(&mut hidden, &projected)?;

            let feed_forward_norm = self.gguf.tensor(&names.feed_forward_norm)?;
            let normalized = rms_norm(
                &hidden,
                EMBEDDING_SIZE,
                feed_forward_norm.f32_slice()?,
                RMS_NORM_EPSILON,
            )?;
            let (mut gate, up) = matrix_vector_pair(
                &self.gguf.tensor(&names.feed_forward_gate)?,
                &self.gguf.tensor(&names.feed_forward_up)?,
                &normalized,
            )?;
            silu(&mut gate);
            vector_multiply(&mut gate, &up)?;
            let down = matrix_vector(&self.gguf.tensor(&names.feed_forward_down)?, &gate)?;
            vector_add(&mut hidden, &down)?;

            if layer < DEEPSTACK_LAYER_COUNT
                && let Some(deepstack) = deepstack
            {
                let start = layer * EMBEDDING_SIZE;
                vector_add(&mut hidden, &deepstack[start..start + EMBEDDING_SIZE])?;
            }
        }

        let output_norm = self.gguf.tensor("output_norm.weight")?;
        rms_norm(
            &hidden,
            EMBEDDING_SIZE,
            output_norm.f32_slice()?,
            RMS_NORM_EPSILON,
        )
    }

    fn greedy_token(&self, hidden: &[f32]) -> Result<u32> {
        let index = matrix_vector_argmax(&self.gguf.tensor("token_embd.weight")?, hidden)
            .context("compute tied-embedding token")?;
        u32::try_from(index).context("sampled token ID exceeds u32")
    }
}

type Position = [u32; 4];

fn text_position(position: usize) -> Result<Position> {
    let position = u32::try_from(position).context("text position exceeds u32")?;
    Ok([position; 4])
}

fn image_position(
    scalar: usize,
    index: usize,
    grid_width: usize,
    grid_height: usize,
) -> Result<Position> {
    ensure!(grid_width != 0 && grid_height != 0, "image grid is empty");
    let row = index / grid_width;
    let column = index % grid_width;
    ensure!(row < grid_height, "image token {index} is outside the grid");

    let temporal = u32::try_from(scalar).context("image temporal position exceeds u32")?;
    let height = scalar
        .checked_add(row)
        .context("image height position overflow")?;
    let width = scalar
        .checked_add(column)
        .context("image width position overflow")?;
    Ok([
        temporal,
        u32::try_from(height).context("image height position exceeds u32")?,
        u32::try_from(width).context("image width position exceeds u32")?,
        0,
    ])
}

struct ImRope {
    cosine: [f32; HEAD_SIZE / 2],
    sine: [f32; HEAD_SIZE / 2],
}

impl ImRope {
    fn new(position: Position) -> Self {
        let temporal = position[0] as f32;
        let height = position[1] as f32;
        let width = position[2] as f32;
        let mut rope = Self {
            cosine: [0.0; HEAD_SIZE / 2],
            sine: [0.0; HEAD_SIZE / 2],
        };

        for pair in 0..HEAD_SIZE / 2 {
            let coordinate = if pair % 3 == 1 && pair < 3 * ROPE_SECTIONS[1] as usize {
                height
            } else if pair % 3 == 2 && pair < 3 * ROPE_SECTIONS[2] as usize {
                width
            } else {
                temporal
            };
            let frequency = ROPE_BASE.powf(-((2 * pair) as f32) / HEAD_SIZE as f32);
            let angle = coordinate * frequency;
            rope.cosine[pair] = angle.cos();
            rope.sine[pair] = angle.sin();
        }
        rope
    }
}

fn apply_im_rope(values: &mut [f32], rope: &ImRope) -> Result<()> {
    ensure!(
        values.len().is_multiple_of(HEAD_SIZE),
        "RoPE input has {} values, not a multiple of {HEAD_SIZE}",
        values.len()
    );
    values.par_chunks_mut(HEAD_SIZE).for_each(|head| {
        let (first, second) = head.split_at_mut(HEAD_SIZE / 2);
        for pair in 0..HEAD_SIZE / 2 {
            let left = first[pair];
            let right = second[pair];
            first[pair] = left * rope.cosine[pair] - right * rope.sine[pair];
            second[pair] = left * rope.sine[pair] + right * rope.cosine[pair];
        }
    });
    Ok(())
}

#[derive(Default)]
struct LayerCache {
    keys: Vec<u16>,
    values: Vec<u16>,
}

impl LayerCache {
    fn reserve_tokens(&mut self, token_count: usize) -> Result<()> {
        let values = token_count
            .checked_mul(KEY_VALUE_SIZE)
            .context("F16 cache capacity overflow")?;
        self.keys
            .try_reserve_exact(values)
            .context("reserve F16 key cache")?;
        self.values
            .try_reserve_exact(values)
            .context("reserve F16 value cache")?;
        Ok(())
    }
    fn append(&mut self, key: &[f32], value: &[f32]) -> Result<()> {
        ensure!(
            key.len() == KEY_VALUE_SIZE,
            "cache key has {} values, expected {KEY_VALUE_SIZE}",
            key.len()
        );
        ensure!(
            value.len() == KEY_VALUE_SIZE,
            "cache value has {} values, expected {KEY_VALUE_SIZE}",
            value.len()
        );
        ensure!(
            self.keys.len() == self.values.len(),
            "key and value cache lengths differ"
        );
        self.keys
            .try_reserve(KEY_VALUE_SIZE)
            .context("grow F16 key cache")?;
        self.values
            .try_reserve(KEY_VALUE_SIZE)
            .context("grow F16 value cache")?;
        self.keys.extend(key.iter().copied().map(f32_to_fp16));
        self.values.extend(value.iter().copied().map(f32_to_fp16));
        Ok(())
    }

    fn token_count(&self) -> usize {
        self.keys.len() / KEY_VALUE_SIZE
    }
}

struct KvCache {
    layers: Vec<LayerCache>,
}

impl KvCache {
    fn new(prompt_tokens: usize) -> Result<Self> {
        let mut layers: Vec<_> = (0..LAYER_COUNT).map(|_| LayerCache::default()).collect();
        for layer in &mut layers {
            layer.reserve_tokens(prompt_tokens)?;
        }
        Ok(Self { layers })
    }
}

#[derive(Default)]
struct TextAttentionScratch {
    output: Vec<f32>,
    scores: Vec<f32>,
}

impl TextAttentionScratch {
    fn new(token_capacity: usize) -> Result<Self> {
        let score_capacity = QUERY_HEAD_COUNT
            .checked_mul(token_capacity)
            .context("attention score capacity overflow")?;
        let mut scores = Vec::new();
        scores
            .try_reserve_exact(score_capacity)
            .context("reserve attention score workspace")?;
        Ok(Self {
            output: vec![0.0; EMBEDDING_SIZE],
            scores,
        })
    }
}

fn causal_gqa(query: &[f32], cache: &LayerCache, scratch: &mut TextAttentionScratch) -> Result<()> {
    ensure!(
        query.len() == EMBEDDING_SIZE,
        "attention query has {} values, expected {EMBEDDING_SIZE}",
        query.len()
    );
    ensure!(
        cache.keys.len() == cache.values.len() && cache.keys.len().is_multiple_of(KEY_VALUE_SIZE),
        "invalid key/value cache shape"
    );
    let token_count = cache.token_count();
    ensure!(token_count != 0, "attention cache is empty");
    let score_count = QUERY_HEAD_COUNT
        .checked_mul(token_count)
        .context("attention score count overflow")?;
    scratch.output.resize(EMBEDDING_SIZE, 0.0);
    scratch.output.fill(0.0);
    scratch.scores.resize(score_count, 0.0);
    let scale = (HEAD_SIZE as f32).sqrt().recip();

    scratch
        .output
        .par_chunks_mut(QUERY_GROUP_SIZE * HEAD_SIZE)
        .zip(
            scratch
                .scores
                .par_chunks_mut(QUERY_GROUP_SIZE * token_count),
        )
        .enumerate()
        .for_each(|(key_value_head, (output_heads, scores))| {
            let (output_left, output_right) = output_heads.split_at_mut(HEAD_SIZE);
            let (weights_left, weights_right) = scores.split_at_mut(token_count);
            let query_start = key_value_head * QUERY_GROUP_SIZE * HEAD_SIZE;
            let query_left = &query[query_start..query_start + HEAD_SIZE];
            let query_right = &query[query_start + HEAD_SIZE..query_start + 2 * HEAD_SIZE];
            for token in 0..token_count {
                let start = token * KEY_VALUE_SIZE + key_value_head * HEAD_SIZE;
                let keys = &cache.keys[start..start + HEAD_SIZE];
                let mut dot_left = 0.0;
                let mut dot_right = 0.0;
                for channel in 0..HEAD_SIZE {
                    let key = fp16_to_f32(keys[channel]);
                    dot_left += query_left[channel] * key;
                    dot_right += query_right[channel] * key;
                }
                weights_left[token] = dot_left * scale;
                weights_right[token] = dot_right * scale;
            }
            softmax(weights_left);
            softmax(weights_right);
            for token in 0..token_count {
                let start = token * KEY_VALUE_SIZE + key_value_head * HEAD_SIZE;
                let values = &cache.values[start..start + HEAD_SIZE];
                let left_attention = weights_left[token];
                let right_attention = weights_right[token];
                for channel in 0..HEAD_SIZE {
                    let value = fp16_to_f32(values[channel]);
                    output_left[channel] += left_attention * value;
                    output_right[channel] += right_attention * value;
                }
            }
        });
    ensure!(
        scratch.output.iter().all(|value| value.is_finite()),
        "attention produced a non-finite value"
    );
    Ok(())
}
fn validate_image(image: &VisionEmbedding) -> Result<usize> {
    ensure!(
        image.grid_width != 0 && image.grid_height != 0,
        "image decoder grid must be nonempty"
    );
    let token_count = image
        .grid_width
        .checked_mul(image.grid_height)
        .context("image decoder grid size overflow")?;
    ensure!(
        image.token_count == token_count,
        "image reports {} tokens, but its grid contains {token_count}",
        image.token_count
    );
    let expected = token_count
        .checked_mul(IMAGE_EMBEDDING_SIZE)
        .context("image embedding size overflow")?;
    ensure!(
        image.values.len() == expected,
        "image embedding has {} values, expected {expected} ({token_count} x {IMAGE_EMBEDDING_SIZE})",
        image.values.len()
    );
    ensure!(
        image.values.iter().all(|value| value.is_finite()),
        "image embedding contains a non-finite value"
    );
    Ok(token_count)
}
#[allow(clippy::cast_possible_truncation)]
fn validate_metadata(gguf: &Gguf) -> Result<()> {
    ensure!(
        gguf.architecture() == "qwen3vl",
        "model architecture is `{}`, expected `qwen3vl`",
        gguf.architecture()
    );
    validate_u32(gguf, "qwen3vl.block_count", LAYER_COUNT as u32)?;
    validate_u32(gguf, "qwen3vl.embedding_length", EMBEDDING_SIZE as u32)?;
    validate_u32(
        gguf,
        "qwen3vl.feed_forward_length",
        FEED_FORWARD_SIZE as u32,
    )?;
    validate_u32(
        gguf,
        "qwen3vl.attention.head_count",
        QUERY_HEAD_COUNT as u32,
    )?;
    validate_u32(
        gguf,
        "qwen3vl.attention.head_count_kv",
        KEY_VALUE_HEAD_COUNT as u32,
    )?;
    validate_u32(gguf, "qwen3vl.attention.key_length", HEAD_SIZE as u32)?;
    validate_u32(gguf, "qwen3vl.attention.value_length", HEAD_SIZE as u32)?;
    validate_u32(
        gguf,
        "qwen3vl.n_deepstack_layers",
        DEEPSTACK_LAYER_COUNT as u32,
    )?;

    let rope_base = gguf.f32("qwen3vl.rope.freq_base")?;
    ensure!(
        rope_base.to_bits() == ROPE_BASE.to_bits(),
        "qwen3vl.rope.freq_base is {rope_base}, expected {ROPE_BASE}"
    );
    let epsilon = gguf.f32("qwen3vl.attention.layer_norm_rms_epsilon")?;
    ensure!(
        epsilon.to_bits() == RMS_NORM_EPSILON.to_bits(),
        "qwen3vl.attention.layer_norm_rms_epsilon is {epsilon}, expected {RMS_NORM_EPSILON}"
    );
    let sections = gguf.u32_array("qwen3vl.rope.dimension_sections")?;
    let canonical = sections.as_ref() == ROPE_SECTIONS;
    let zero_padded = sections.as_ref() == [24, 20, 20, 0];
    ensure!(
        canonical || zero_padded,
        "qwen3vl.rope.dimension_sections is {sections:?}, expected {ROPE_SECTIONS:?}"
    );
    Ok(())
}

fn validate_u32(gguf: &Gguf, key: &str, expected: u32) -> Result<()> {
    let value = gguf.u32(key)?;
    ensure!(value == expected, "{key} is {value}, expected {expected}");
    Ok(())
}

fn validate_tensors(gguf: &Gguf) -> Result<()> {
    validate_tensor(
        gguf,
        "token_embd.weight",
        &[EMBEDDING_SIZE, tokenizer_vocabulary_size(gguf)?],
        TensorType::Q8_0,
    )?;
    validate_tensor(
        gguf,
        "output_norm.weight",
        &[EMBEDDING_SIZE],
        TensorType::F32,
    )?;

    for layer in 0..LAYER_COUNT {
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_norm.weight"),
            &[EMBEDDING_SIZE],
            TensorType::F32,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_q.weight"),
            &[EMBEDDING_SIZE, EMBEDDING_SIZE],
            TensorType::Q8_0,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_k.weight"),
            &[EMBEDDING_SIZE, KEY_VALUE_SIZE],
            TensorType::Q8_0,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_v.weight"),
            &[EMBEDDING_SIZE, KEY_VALUE_SIZE],
            TensorType::Q8_0,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_output.weight"),
            &[EMBEDDING_SIZE, EMBEDDING_SIZE],
            TensorType::Q8_0,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_q_norm.weight"),
            &[HEAD_SIZE],
            TensorType::F32,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.attn_k_norm.weight"),
            &[HEAD_SIZE],
            TensorType::F32,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.ffn_norm.weight"),
            &[EMBEDDING_SIZE],
            TensorType::F32,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.ffn_gate.weight"),
            &[EMBEDDING_SIZE, FEED_FORWARD_SIZE],
            TensorType::Q8_0,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.ffn_up.weight"),
            &[EMBEDDING_SIZE, FEED_FORWARD_SIZE],
            TensorType::Q8_0,
        )?;
        validate_tensor(
            gguf,
            &format!("blk.{layer}.ffn_down.weight"),
            &[FEED_FORWARD_SIZE, EMBEDDING_SIZE],
            TensorType::Q8_0,
        )?;
    }
    Ok(())
}

fn tokenizer_vocabulary_size(gguf: &Gguf) -> Result<usize> {
    Ok(gguf.string_array("tokenizer.ggml.tokens")?.len())
}

fn validate_tensor(gguf: &Gguf, name: &str, dimensions: &[usize], kind: TensorType) -> Result<()> {
    let tensor = gguf.tensor(name)?;
    ensure!(
        tensor.dimensions() == dimensions,
        "tensor `{name}` has dimensions {:?}, expected {dimensions:?}",
        tensor.dimensions()
    );
    ensure!(
        tensor.tensor_type() == kind,
        "tensor `{name}` is {:?}, expected {kind:?}",
        tensor.tensor_type()
    );
    Ok(())
}

fn validate_special_tokens(tokenizer: &Tokenizer) -> Result<()> {
    for text in [IM_START, IM_END, VISION_START, VISION_END] {
        let token = tokenizer
            .token_id(text)
            .with_context(|| format!("tokenizer is missing special token `{text}`"))?;
        ensure!(
            tokenizer.is_special(token),
            "token `{text}` ({token}) is not marked special"
        );
    }
    let im_end = tokenizer
        .token_id(IM_END)
        .with_context(|| format!("tokenizer is missing special token `{IM_END}`"))?;
    ensure!(
        tokenizer.eos_token() == im_end,
        "tokenizer EOS {} is not `{IM_END}` token {im_end}",
        tokenizer.eos_token()
    );
    Ok(())
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools)]
struct JsonObjectTracker {
    stack: Vec<char>,
    started: bool,
    in_string: bool,
    escaped: bool,
    invalid: bool,
}

impl JsonObjectTracker {
    fn push(&mut self, text: &str) -> Option<usize> {
        for (offset, character) in text.char_indices() {
            if !self.started {
                if character == '{' {
                    self.started = true;
                    self.stack.push(character);
                }
                continue;
            }
            if self.invalid {
                continue;
            }
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if character == '\\' {
                    self.escaped = true;
                } else if character == '"' {
                    self.in_string = false;
                }
                continue;
            }

            match character {
                '"' => self.in_string = true,
                '{' | '[' => self.stack.push(character),
                '}' => {
                    if self.stack.pop() != Some('{') {
                        self.invalid = true;
                    } else if self.stack.is_empty() {
                        return Some(offset + character.len_utf8());
                    }
                }
                ']' => self.invalid = self.stack.pop() != Some('['),
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_rope_matches_direct_calculation() {
        let position = [17, 3, 11, 0];
        let mut expected = (0..EMBEDDING_SIZE)
            .map(|value| value as f32)
            .collect::<Vec<_>>();
        let mut actual = expected.clone();
        apply_im_rope_direct(&mut expected, position);
        apply_im_rope(&mut actual, &ImRope::new(position)).expect("apply cached RoPE");
        assert_eq!(actual, expected);
    }

    #[test]
    fn cache_reserves_the_prefill_context() {
        let mut cache = KvCache::new(2).expect("reserve cache");
        let key = vec![1.0; KEY_VALUE_SIZE];
        let value = vec![2.0; KEY_VALUE_SIZE];
        for layer in &mut cache.layers {
            assert!(layer.keys.capacity() >= 2 * KEY_VALUE_SIZE);
            layer.append(&key, &value).expect("append first token");
            layer.append(&key, &value).expect("append second token");
            assert_eq!(layer.token_count(), 2);
        }
    }

    #[test]
    fn reused_attention_workspace_matches_reference() {
        let mut cache = LayerCache::default();
        for token in 0..3 {
            let key = (0..KEY_VALUE_SIZE)
                .map(|value| (token * KEY_VALUE_SIZE + value) as f32 * 0.001)
                .collect::<Vec<_>>();
            let value = (0..KEY_VALUE_SIZE)
                .map(|value| (token * KEY_VALUE_SIZE + value) as f32 * -0.002)
                .collect::<Vec<_>>();
            cache.append(&key, &value).expect("append cache token");
        }
        let query = (0..EMBEDDING_SIZE)
            .map(|value| value as f32 * 0.003)
            .collect::<Vec<_>>();
        let expected = causal_gqa_reference(&query, &cache);
        let mut scratch = TextAttentionScratch::default();
        causal_gqa(&query, &cache, &mut scratch).expect("compute attention");
        assert_eq!(scratch.output, expected);
    }

    fn apply_im_rope_direct(values: &mut [f32], position: Position) {
        for head in values.chunks_exact_mut(HEAD_SIZE) {
            let (first, second) = head.split_at_mut(HEAD_SIZE / 2);
            for pair in 0..HEAD_SIZE / 2 {
                let coordinate = if pair % 3 == 1 && pair < 3 * ROPE_SECTIONS[1] as usize {
                    position[1] as f32
                } else if pair % 3 == 2 && pair < 3 * ROPE_SECTIONS[2] as usize {
                    position[2] as f32
                } else {
                    position[0] as f32
                };
                let angle = coordinate * ROPE_BASE.powf(-((2 * pair) as f32) / HEAD_SIZE as f32);
                let cosine = angle.cos();
                let sine = angle.sin();
                let left = first[pair];
                let right = second[pair];
                first[pair] = left * cosine - right * sine;
                second[pair] = left * sine + right * cosine;
            }
        }
    }

    fn causal_gqa_reference(query: &[f32], cache: &LayerCache) -> Vec<f32> {
        let token_count = cache.token_count();
        let scale = (HEAD_SIZE as f32).sqrt().recip();
        let mut output = vec![0.0; EMBEDDING_SIZE];
        for (query_head, output_head) in output.chunks_exact_mut(HEAD_SIZE).enumerate() {
            let key_value_head = query_head / QUERY_GROUP_SIZE;
            let query_values = &query[query_head * HEAD_SIZE..(query_head + 1) * HEAD_SIZE];
            let mut scores = Vec::with_capacity(token_count);
            for token in 0..token_count {
                let start = token * KEY_VALUE_SIZE + key_value_head * HEAD_SIZE;
                let key = &cache.keys[start..start + HEAD_SIZE];
                scores.push(
                    query_values
                        .iter()
                        .zip(key)
                        .map(|(query, key)| query * fp16_to_f32(*key))
                        .sum::<f32>()
                        * scale,
                );
            }
            softmax(&mut scores);
            for (token, score) in scores.into_iter().enumerate() {
                let start = token * KEY_VALUE_SIZE + key_value_head * HEAD_SIZE;
                let value = &cache.values[start..start + HEAD_SIZE];
                for channel in 0..HEAD_SIZE {
                    output_head[channel] += score * fp16_to_f32(value[channel]);
                }
            }
        }
        output
    }
}
