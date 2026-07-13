// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

// Based on TrOCR/ViT model code from https://github.com/huggingface/candle,
// which is licensed with MIT OR Apache-2.0.

#![allow(dead_code, clippy::all, clippy::pedantic)]

use anyhow::{Context as _, Result};
use candle::quantized::gguf_file;
use candle::quantized::{GgmlDType, QTensor};
use candle::safetensors::MmapedSafetensors;
use candle::{D, DType, Device, Module, Result as CResult, Tensor};
use candle_nn::{Conv2d, Conv2dConfig, Embedding, LayerNorm};
use candle_transformers::models::{trocr, vit};
use candle_transformers::quantized_nn::{
    Linear as QLinear, layer_norm as q_layer_norm, linear as q_linear,
    linear_no_bias as q_linear_no_bias,
};
use candle_transformers::quantized_var_builder::VarBuilder as QVarBuilder;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{IsTerminal as _, Write as _};
use std::path::Path;

/// File name of the locally produced q4_K GGUF, written next to the F32
/// safetensors under the readseek model cache directory.
pub(crate) const Q4K_GGUF_NAME: &str = "trocr-base-printed-q4k.gguf";

#[derive(Debug, Clone)]
struct PatchEmbeddings {
    num_patches: usize,
    projection: Conv2d,
}

impl PatchEmbeddings {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let image_size = cfg.image_size;
        let patch_size = cfg.patch_size;
        let num_patches = (image_size / patch_size) * (image_size / patch_size);
        let conv_cfg = Conv2dConfig {
            stride: patch_size,
            ..Default::default()
        };
        let p_vb = vb.pp("projection");
        // 4-D conv weight: dequantize to F32 (conv is not on the fast QMatmul path).
        let weight = p_vb
            .get(
                (cfg.hidden_size, cfg.num_channels, patch_size, patch_size),
                "weight",
            )?
            .dequantize(vb.device())?;
        let bias = p_vb.get(cfg.hidden_size, "bias")?.dequantize(vb.device())?;
        let projection = Conv2d::new(weight, Some(bias), conv_cfg);
        Ok(Self {
            num_patches,
            projection,
        })
    }
}

impl Module for PatchEmbeddings {
    fn forward(&self, pixel_values: &Tensor) -> CResult<Tensor> {
        self.projection
            .forward(pixel_values)?
            .flatten_from(2)?
            .transpose(1, 2)
    }
}

#[derive(Debug, Clone)]
pub struct Embeddings {
    cls_token: Tensor,
    mask_token: Option<Tensor>,
    patch_embeddings: PatchEmbeddings,
    position_embeddings: Tensor,
    hidden_size: usize,
}

impl Embeddings {
    pub fn new(cfg: &vit::Config, use_mask_token: bool, vb: QVarBuilder) -> CResult<Self> {
        let hidden_size = cfg.hidden_size;
        let cls_token = vb
            .get((1, 1, hidden_size), "cls_token")?
            .dequantize(vb.device())?;
        let mask_token = if use_mask_token {
            Some(
                vb.get((1, 1, hidden_size), "mask_token")?
                    .dequantize(vb.device())?,
            )
        } else {
            None
        };
        let patch_embeddings = PatchEmbeddings::new(cfg, vb.pp("patch_embeddings"))?;
        let num_patches = patch_embeddings.num_patches;
        let position_embeddings = vb
            .get((1, num_patches + 1, hidden_size), "position_embeddings")?
            .dequantize(vb.device())?;
        Ok(Self {
            cls_token,
            mask_token,
            patch_embeddings,
            position_embeddings,
            hidden_size,
        })
    }

    pub fn forward(
        &self,
        pixel_values: &Tensor,
        bool_masked_pos: Option<&Tensor>,
        interpolate_pos_encoding: bool,
    ) -> CResult<Tensor> {
        debug_assert!(
            !interpolate_pos_encoding,
            "pos-encoding interpolation unsupported"
        );
        let (b_size, _num_channels, _height, _width) = pixel_values.dims4()?;
        let embeddings = self.patch_embeddings.forward(pixel_values)?;
        let embeddings = match (bool_masked_pos, &self.mask_token) {
            (None, _) => embeddings,
            (Some(_), None) => candle::bail!("bool_masked_pos set without mask_token"),
            (Some(bool_masked_pos), Some(mask_tokens)) => {
                let seq_len = embeddings.dim(1)?;
                let mask_tokens = mask_tokens.broadcast_as((b_size, seq_len, self.hidden_size))?;
                let mask = bool_masked_pos
                    .unsqueeze(D::Minus1)?
                    .to_dtype(mask_tokens.dtype())?;
                ((mask_tokens * &mask)? - (embeddings * (mask - 1.)?)?)?
            }
        };
        let cls_tokens = self.cls_token.broadcast_as((b_size, 1, self.hidden_size))?;
        let embeddings = Tensor::cat(&[&cls_tokens, &embeddings], 1)?;
        embeddings.broadcast_add(&self.position_embeddings)
    }
}

#[derive(Debug, Clone)]
struct SelfAttention {
    query: QLinear,
    key: QLinear,
    value: QLinear,
    num_attention_heads: usize,
    attention_head_size: usize,
}

impl SelfAttention {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let attention_head_size = cfg.hidden_size / cfg.num_attention_heads;
        let num_attention_heads = cfg.num_attention_heads;
        let all_head_size = num_attention_heads * attention_head_size;
        let mk = |name| -> CResult<QLinear> {
            if cfg.qkv_bias {
                q_linear(cfg.hidden_size, all_head_size, vb.pp(name))
            } else {
                q_linear_no_bias(cfg.hidden_size, all_head_size, vb.pp(name))
            }
        };
        let query = mk("query")?;
        let key = mk("key")?;
        let value = mk("value")?;
        Ok(Self {
            query,
            key,
            value,
            num_attention_heads,
            attention_head_size,
        })
    }

    fn transpose_for_scores(&self, xs: &Tensor) -> CResult<Tensor> {
        let (b_size, seq_len, _) = xs.dims3()?;
        xs.reshape((
            b_size,
            seq_len,
            self.num_attention_heads,
            self.attention_head_size,
        ))?
        .permute((0, 2, 1, 3))
    }
}

impl Module for SelfAttention {
    fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        let query = self.query.forward(xs)?;
        let key = self.key.forward(xs)?;
        let value = self.value.forward(xs)?;

        let query = self.transpose_for_scores(&query)?.contiguous()?;
        let key = self.transpose_for_scores(&key)?.contiguous()?;
        let value = self.transpose_for_scores(&value)?.contiguous()?;

        let attention_scores =
            (query.matmul(&key.t()?)? / f64::sqrt(self.attention_head_size as f64))?;
        let attention_probs = candle_nn::ops::softmax_last_dim(&attention_scores)?;
        attention_probs
            .matmul(&value)?
            .permute((0, 2, 1, 3))?
            .contiguous()?
            .flatten_from(D::Minus2)
    }
}

#[derive(Debug, Clone)]
struct SelfOutput {
    dense: QLinear,
}

impl SelfOutput {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let dense = q_linear(cfg.hidden_size, cfg.hidden_size, vb.pp("dense"))?;
        Ok(Self { dense })
    }
}

impl Module for SelfOutput {
    fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        xs.apply(&self.dense)
    }
}

#[derive(Debug, Clone)]
struct Attention {
    attention: SelfAttention,
    output: SelfOutput,
}

impl Attention {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let attention = SelfAttention::new(cfg, vb.pp("attention"))?;
        let output = SelfOutput::new(cfg, vb.pp("output"))?;
        Ok(Self { attention, output })
    }
}

impl Module for Attention {
    fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        xs.apply(&self.attention)?.apply(&self.output)
    }
}

#[derive(Debug, Clone)]
struct Intermediate {
    dense: QLinear,
    intermediate_act_fn: candle_nn::Activation,
}

impl Intermediate {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let dense = q_linear(cfg.hidden_size, cfg.intermediate_size, vb.pp("dense"))?;
        Ok(Self {
            dense,
            intermediate_act_fn: cfg.hidden_act,
        })
    }
}

impl Module for Intermediate {
    fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        xs.apply(&self.dense)?.apply(&self.intermediate_act_fn)
    }
}

#[derive(Debug, Clone)]
struct Output {
    dense: QLinear,
}

impl Output {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let dense = q_linear(cfg.intermediate_size, cfg.hidden_size, vb.pp("dense"))?;
        Ok(Self { dense })
    }

    fn forward(&self, xs: &Tensor, input_tensor: &Tensor) -> CResult<Tensor> {
        xs.apply(&self.dense)? + input_tensor
    }
}

#[derive(Debug, Clone)]
struct Layer {
    attention: Attention,
    intermediate: Intermediate,
    output: Output,
    layernorm_before: LayerNorm,
    layernorm_after: LayerNorm,
}

impl Layer {
    fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let attention = Attention::new(cfg, vb.pp("attention"))?;
        let intermediate = Intermediate::new(cfg, vb.pp("intermediate"))?;
        let output = Output::new(cfg, vb.pp("output"))?;
        let h_sz = cfg.hidden_size;
        let layernorm_before = q_layer_norm(h_sz, cfg.layer_norm_eps, vb.pp("layernorm_before"))?;
        let layernorm_after = q_layer_norm(h_sz, cfg.layer_norm_eps, vb.pp("layernorm_after"))?;
        Ok(Self {
            attention,
            intermediate,
            output,
            layernorm_after,
            layernorm_before,
        })
    }
}

impl Module for Layer {
    fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        let xs = (xs.apply(&self.layernorm_before)?.apply(&self.attention)? + xs)?;
        let ys = xs.apply(&self.layernorm_after)?.apply(&self.intermediate)?;
        self.output.forward(&ys, &xs)
    }
}

#[derive(Debug, Clone)]
pub struct Encoder {
    layers: Vec<Layer>,
}

impl Encoder {
    pub fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let vb = vb.pp("layer");
        let mut layers = Vec::with_capacity(cfg.num_hidden_layers);
        for i in 0..cfg.num_hidden_layers {
            let layer = Layer::new(cfg, vb.pp(i))?;
            layers.push(layer)
        }
        Ok(Self { layers })
    }
}

impl Module for Encoder {
    fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        let mut xs = xs.clone();
        for layer in self.layers.iter() {
            xs = xs.apply(layer)?
        }
        Ok(xs)
    }
}

fn default_tie_word_embeddings() -> bool {
    true
}

fn default_use_learned_position_embeddings() -> bool {
    true
}

type TrOCRConfig = trocr::TrOCRConfig;

#[derive(Debug, Clone)]
struct TrOCRLearnedPositionalEmbedding {
    offset: usize,
    weights: Embedding,
}

impl TrOCRLearnedPositionalEmbedding {
    fn load(vb: QVarBuilder, cfg: &TrOCRConfig) -> CResult<Self> {
        let offset: usize = 2;
        let num_embeddings = cfg.max_position_embeddings;
        let embedding_dim = cfg.d_model;
        let weights = Embedding::new(
            vb.get((num_embeddings + offset, embedding_dim), "weight")?
                .dequantize(vb.device())?,
            embedding_dim,
        );
        Ok(Self { offset, weights })
    }

    fn new_sinusoidal(vb: QVarBuilder, cfg: &TrOCRConfig) -> CResult<Self> {
        let embedding_dim = cfg.d_model;
        let half_dim = embedding_dim / 2;
        let num_positions = cfg.max_position_embeddings + cfg.pad_token_id + 1;
        let dev = vb.device();
        let inv_freq: Vec<_> = (0..half_dim)
            .map(|i| 1f32 / 10000f32.powf(i as f32 / (half_dim - 1) as f32))
            .collect();
        let inv_freq_len = inv_freq.len();
        let inv_freq = Tensor::from_vec(inv_freq, (1, inv_freq_len), dev)?;
        let t = Tensor::arange(0u32, num_positions as u32, dev)?
            .to_dtype(DType::F32)?
            .reshape((num_positions, 1))?;
        let freqs = t.matmul(&inv_freq)?;
        let emb = Tensor::cat(&[freqs.sin()?, freqs.cos()?], 1)?;
        let emb = Tensor::cat(
            &[
                emb.narrow(0, 0, cfg.pad_token_id)?,
                Tensor::zeros((1, embedding_dim), DType::F32, dev)?,
                emb.narrow(0, cfg.pad_token_id + 1, cfg.max_position_embeddings)?,
            ],
            0,
        )?
        .contiguous()?;
        let emb = Embedding::new(emb, embedding_dim);
        Ok(Self {
            offset: cfg.pad_token_id + 1,
            weights: emb,
        })
    }

    fn forward(&mut self, input_ids: &Tensor, past_key_values_length: u32) -> CResult<Tensor> {
        let (b_sz, seq_len) = input_ids.dims2()?;
        let positions = Tensor::arange(
            past_key_values_length,
            seq_len as u32 + past_key_values_length,
            input_ids.device(),
        )?
        .expand((b_sz, seq_len))?;
        let positions =
            positions.broadcast_add(&Tensor::new(self.offset as u32, input_ids.device())?)?;
        self.weights.forward(&positions)
    }
}

#[derive(Debug, Clone)]
struct TrOCRAttention {
    head_dim: usize,
    num_heads: usize,
    is_decoder: bool,
    scaling: f64,
    k_proj: QLinear,
    v_proj: QLinear,
    q_proj: QLinear,
    out_proj: QLinear,
    kv_cache: Option<(Tensor, Tensor)>,
}

impl TrOCRAttention {
    fn load(
        vb: QVarBuilder,
        cfg: &TrOCRConfig,
        kdim: Option<usize>,
        vdim: Option<usize>,
    ) -> CResult<Self> {
        let embed_dim = cfg.d_model;
        let num_heads = cfg.decoder_attention_heads;
        let head_dim = embed_dim / num_heads;
        let kdim = kdim.unwrap_or(embed_dim);
        let vdim = vdim.unwrap_or(embed_dim);

        let k_proj = q_linear_no_bias(kdim, embed_dim, vb.pp("k_proj"))?;
        let v_proj = q_linear_no_bias(vdim, embed_dim, vb.pp("v_proj"))?;
        let q_proj = q_linear_no_bias(embed_dim, embed_dim, vb.pp("q_proj"))?;
        let out_proj = q_linear_no_bias(embed_dim, embed_dim, vb.pp("out_proj"))?;
        Ok(Self {
            head_dim,
            num_heads,
            is_decoder: true,
            scaling: 1. / (head_dim as f64).sqrt(),
            k_proj,
            v_proj,
            q_proj,
            out_proj,
            kv_cache: None,
        })
    }

    fn reset_kv_cache(&mut self) {
        self.kv_cache = None
    }

    fn _shape(&self, tensor: &Tensor, bsz: usize) -> CResult<Tensor> {
        tensor
            .reshape((bsz, (), self.num_heads, self.head_dim))?
            .transpose(1, 2)?
            .contiguous()
    }

    fn forward(
        &mut self,
        xs: &Tensor,
        kv_states: Option<&Tensor>,
        attn_mask: Option<&Tensor>,
    ) -> CResult<Tensor> {
        let (b_sz, tgt_len, _) = xs.dims3()?;
        let query_states = (xs.apply(&self.q_proj)? * self.scaling)?;
        let (key_states, value_states) = match kv_states {
            None => {
                let key_states = self._shape(&xs.apply(&self.k_proj)?, b_sz)?;
                let value_states = self._shape(&xs.apply(&self.v_proj)?, b_sz)?;
                if self.is_decoder {
                    let kv_states = match &self.kv_cache {
                        None => (key_states, value_states),
                        Some((p_key_states, p_value_states)) => {
                            let key_states = Tensor::cat(&[p_key_states, &key_states], 2)?;
                            let value_states = Tensor::cat(&[p_value_states, &value_states], 2)?;
                            (key_states, value_states)
                        }
                    };
                    self.kv_cache = Some(kv_states.clone());
                    kv_states
                } else {
                    (key_states, value_states)
                }
            }
            Some(kv_states) => {
                let key_states = self._shape(&kv_states.apply(&self.k_proj)?, b_sz)?;
                let value_states = self._shape(&kv_states.apply(&self.v_proj)?, b_sz)?;
                (key_states, value_states)
            }
        };
        let proj_shape = (b_sz * self.num_heads, (), self.head_dim);
        let query_states = self._shape(&query_states, b_sz)?.reshape(proj_shape)?;
        let key_states = key_states.reshape(proj_shape)?;
        let value_states = value_states.reshape(proj_shape)?;
        let attn_weights = query_states.matmul(&key_states.transpose(1, 2)?)?;
        let attn_weights = match attn_mask {
            None => attn_weights,
            Some(attn_mask) => attn_weights.broadcast_add(attn_mask)?,
        };
        let attn_probs = candle_nn::ops::softmax_last_dim(&attn_weights)?;
        let attn_output = attn_probs.matmul(&value_states)?;
        attn_output
            .reshape((b_sz, self.num_heads, tgt_len, self.head_dim))?
            .transpose(1, 2)?
            .reshape((b_sz, tgt_len, self.head_dim * self.num_heads))?
            .apply(&self.out_proj)
    }
}

#[derive(Debug, Clone)]
struct TrOCRDecoderLayer {
    self_attn: TrOCRAttention,
    activation_fn: candle_nn::Activation,
    self_attn_layer_norm: LayerNorm,
    encoder_attn: TrOCRAttention,
    encoder_attn_layer_norm: LayerNorm,
    fc1: QLinear,
    fc2: QLinear,
    final_layer_norm: LayerNorm,
}

impl TrOCRDecoderLayer {
    fn load(vb: QVarBuilder, cfg: &TrOCRConfig) -> CResult<Self> {
        let embed_dim = cfg.d_model;
        let self_attn = TrOCRAttention::load(vb.pp("self_attn"), cfg, None, None)?;
        let self_attn_layer_norm = q_layer_norm(embed_dim, 1e-5, vb.pp("self_attn_layer_norm"))?;
        let encoder_attn = TrOCRAttention::load(
            vb.pp("encoder_attn"),
            cfg,
            Some(cfg.cross_attention_hidden_size),
            Some(cfg.cross_attention_hidden_size),
        )?;
        let encoder_attn_layer_norm =
            q_layer_norm(embed_dim, 1e-5, vb.pp("encoder_attn_layer_norm"))?;
        let fc1 = q_linear_no_bias(embed_dim, cfg.decoder_ffn_dim, vb.pp("fc1"))?;
        let fc2 = q_linear_no_bias(cfg.decoder_ffn_dim, embed_dim, vb.pp("fc2"))?;
        let final_layer_norm = q_layer_norm(embed_dim, 1e-5, vb.pp("final_layer_norm"))?;
        Ok(Self {
            self_attn,
            activation_fn: cfg.activation_function,
            self_attn_layer_norm,
            encoder_attn,
            encoder_attn_layer_norm,
            fc1,
            fc2,
            final_layer_norm,
        })
    }

    fn reset_kv_cache(&mut self) {
        self.self_attn.reset_kv_cache();
    }

    fn forward(
        &mut self,
        xs: &Tensor,
        attention_mask: &Tensor,
        encoder_hidden_states: Option<&Tensor>,
    ) -> CResult<Tensor> {
        let residual = xs.clone();
        let xs = self.self_attn.forward(xs, None, Some(attention_mask))?;
        let xs = (xs + residual)?;
        let mut xs = self.self_attn_layer_norm.forward(&xs)?;

        if let Some(encoder_hidden_states) = &encoder_hidden_states {
            let residual = xs.clone();
            let encoder_attention_mask = attention_mask.clone(); // TODO
            xs = self.encoder_attn.forward(
                &xs,
                Some(encoder_hidden_states),
                Some(&encoder_attention_mask),
            )?;
            xs = (xs + residual)?;
            xs = self.encoder_attn_layer_norm.forward(&xs)?
        }

        let residual = xs.clone();
        let xs = self.fc1.forward(&xs)?;
        let xs = self.activation_fn.forward(&xs)?;
        let xs = self.fc2.forward(&xs)?;
        let xs = (xs + residual)?;
        let xs = self.final_layer_norm.forward(&xs)?;

        Ok(xs)
    }
}

#[derive(Debug, Clone)]
pub struct TrOCRDecoder {
    layers: Vec<TrOCRDecoderLayer>,
    embed_scale: Option<f64>,
    pub embed_tokens: Embedding,
    embed_positions: TrOCRLearnedPositionalEmbedding,
}

impl TrOCRDecoder {
    fn new(cfg: &TrOCRConfig, vb: QVarBuilder) -> CResult<Self> {
        let vb = vb.pp("decoder.model.decoder");

        let embed_tokens = Embedding::new(
            vb.pp("embed_tokens")
                .get((cfg.vocab_size, cfg.d_model), "weight")?
                .dequantize(vb.device())?,
            cfg.d_model,
        );
        let embed_positions = if cfg.use_learned_position_embeddings {
            TrOCRLearnedPositionalEmbedding::load(vb.pp("embed_positions"), cfg)?
        } else {
            TrOCRLearnedPositionalEmbedding::new_sinusoidal(vb.pp("embed_positions"), cfg)?
        };
        let mut layers = Vec::with_capacity(cfg.decoder_layers);
        let vb_l = vb.pp("layers");
        for idx in 0..cfg.decoder_layers {
            let layer = TrOCRDecoderLayer::load(vb_l.pp(idx), cfg)?;
            layers.push(layer)
        }
        let embed_scale = if cfg.scale_embedding {
            Some((cfg.d_model as f64).sqrt())
        } else {
            None
        };

        Ok(Self {
            layers,
            embed_scale,
            embed_tokens,
            embed_positions,
        })
    }

    fn reset_kv_cache(&mut self) {
        self.layers.iter_mut().for_each(|l| l.reset_kv_cache())
    }

    pub fn forward(
        &mut self,
        xs: &Tensor,
        encoder_xs: Option<&Tensor>,
        past_kv_len: usize,
        attn_mask: &Tensor,
    ) -> CResult<Tensor> {
        let embed_pos = self.embed_positions.forward(xs, past_kv_len as u32)?;
        let xs = xs.apply(&self.embed_tokens)?;

        let xs = match self.embed_scale {
            None => xs,
            Some(scale) => (xs * scale)?,
        };

        let mut xs = xs.broadcast_add(&embed_pos)?;

        for layer in self.layers.iter_mut() {
            xs = layer.forward(&xs, attn_mask, encoder_xs)?;
        }
        Ok(xs)
    }
}

#[derive(Debug, Clone)]
pub struct TrOCREncoder {
    embeddings: Embeddings,
    encoder: Encoder,
    layernorm: LayerNorm,
}

impl TrOCREncoder {
    pub fn new(cfg: &vit::Config, vb: QVarBuilder) -> CResult<Self> {
        let vb_v = vb.pp("encoder");
        let embeddings = Embeddings::new(cfg, false, vb_v.pp("embeddings"))?;
        let encoder = Encoder::new(cfg, vb_v.pp("encoder"))?;
        let layernorm = q_layer_norm(cfg.hidden_size, cfg.layer_norm_eps, vb_v.pp("layernorm"))?;
        Ok(Self {
            embeddings,
            encoder,
            layernorm,
        })
    }

    pub fn forward(&self, xs: &Tensor) -> CResult<Tensor> {
        let embedding_output = self.embeddings.forward(xs, None, false)?;
        let encoder_outputs = self.encoder.forward(&embedding_output)?;
        self.layernorm.forward(&encoder_outputs)
    }
}

#[derive(Debug, Clone)]
pub struct TrOCRForCausalLM {
    decoder: TrOCRDecoder,
    /// Tied to `decoder.embed_tokens` by default, this stays F32 (literal clone
    /// of the dequantized token-embedding weight); the untied branch loads a
    /// dequantized F32 weight too, so the head is F32 regardless of GGUF dtype.
    output_projection: candle_nn::Linear,
}

impl TrOCRForCausalLM {
    pub fn new(decoder_cfg: &TrOCRConfig, vb: QVarBuilder) -> CResult<Self> {
        let decoder = TrOCRDecoder::new(decoder_cfg, vb.clone())?;
        let output_projection = if decoder_cfg.tie_word_embeddings {
            candle_nn::Linear::new(decoder.embed_tokens.embeddings().clone(), None)
        } else {
            let op_vb = vb.pp("decoder.output_projection");
            let weight = op_vb
                .get((decoder_cfg.vocab_size, decoder_cfg.d_model), "weight")?
                .dequantize(vb.device())?;
            candle_nn::Linear::new(weight, None)
        };
        Ok(Self {
            decoder,
            output_projection,
        })
    }

    pub fn forward(
        &mut self,
        xs: &Tensor,
        encoder_xs: Option<&Tensor>,
        past_kv_len: usize,
        attn_mask: &Tensor,
    ) -> CResult<Tensor> {
        let xs = self
            .decoder
            .forward(xs, encoder_xs, past_kv_len, attn_mask)?;
        xs.apply(&self.output_projection)
    }

    fn reset_kv_cache(&mut self) {
        self.decoder.reset_kv_cache();
    }
}

#[derive(Debug, Clone)]
pub struct TrOCRModel {
    encoder: TrOCREncoder,
    decoder: TrOCRForCausalLM,
}

impl TrOCRModel {
    pub fn new(
        encoder_cfg: &vit::Config,
        decoder_cfg: &TrOCRConfig,
        vb: QVarBuilder,
    ) -> CResult<Self> {
        let encoder = TrOCREncoder::new(encoder_cfg, vb.clone())?;
        let decoder = TrOCRForCausalLM::new(decoder_cfg, vb)?;
        Ok(Self { encoder, decoder })
    }

    pub fn encoder(&mut self) -> &mut TrOCREncoder {
        &mut self.encoder
    }

    pub fn decoder(&mut self) -> &mut TrOCRForCausalLM {
        &mut self.decoder
    }

    pub fn decode(
        &mut self,
        xs: &Tensor,
        encoder_xs: &Tensor,
        past_kv_len: usize,
    ) -> CResult<Tensor> {
        let seq_len = xs.dim(1)?;
        let mask: Vec<_> = (0..seq_len)
            .flat_map(|i| (0..seq_len).map(move |j| if j > i { f32::NEG_INFINITY } else { 0f32 }))
            .collect();
        let mask = Tensor::from_vec(mask, (seq_len, seq_len), xs.device())?;

        self.decoder
            .forward(xs, Some(encoder_xs), past_kv_len, &mask)
    }

    pub fn reset_kv_cache(&mut self) {
        self.decoder.reset_kv_cache();
    }
}

/// Coercion rule for the converter: Linear weight tensors (2-D `<...>.weight`,
/// excluding the token/position embeddings and the output head, which stay F32
/// for gather/head quality and tying) are stored as q4_K; everything else
/// (rank-1 `LayerNorm`/biases, the rank-4 patch-embed conv weight, rank-3
/// `cls_token`/`position_embeddings`) stays F32. The module loads both back
/// through `quantized_var_builder::VarBuilder`, dequantizing the non-matmul
/// tensors at construction exactly as `quantized_blip` does.
fn quantize_dtype_for(name: &str, rank: usize) -> GgmlDType {
    const F32_KEEP: &[&str] = &[
        "embed_tokens.weight",
        "embed_positions.weight",
        "decoder.output_projection.weight",
    ];
    if rank == 2 && name.ends_with(".weight") && !F32_KEEP.iter().any(|keep| name.ends_with(keep)) {
        GgmlDType::Q4K
    } else {
        GgmlDType::F32
    }
}

/// Quantize the cached F32 `trocr-base-printed.safetensors` into a q4_K GGUF at
/// `out_gguf`, preserving every tensor's original name so the model loads by the
/// same keys the F32 path uses. Quantization runs on the CPU (the GGUF is a
/// device-agnostic container; `from_gguf` later loads it onto the inference
/// device). Safe to call on a verified F32 file; the produced GGUF is reused on
/// subsequent runs so the one-time cost is paid only once.
pub(crate) fn build_q4k_gguf(f32_safetensors: &Path, out_gguf: &Path) -> Result<()> {
    let device = Device::Cpu;
    let mst = unsafe { MmapedSafetensors::multi(&[f32_safetensors]) }
        .with_context(|| format!("mmap {}", f32_safetensors.display()))?;
    let named: Vec<(String, usize)> = mst
        .tensors()
        .into_iter()
        .map(|(name, view)| (name, view.shape().len()))
        .collect();

    let tty = std::io::stderr().is_terminal();
    let bar = if tty {
        let bar = ProgressBar::new(named.len() as u64);
        bar.set_style(
            ProgressStyle::with_template("{prefix:<28} {bar:30} {pos}/{len} ({percent}%) {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("=> "),
        );
        bar.set_prefix("Quantizing TrOCR");
        bar.enable_steady_tick(std::time::Duration::from_millis(100));
        bar.set_message("q4_K GGUF");
        Some(bar)
    } else {
        None
    };

    let mut qtensors: Vec<(String, QTensor)> = Vec::with_capacity(named.len());
    for (name, rank) in named {
        let tensor = mst.load(&name, &device)?;
        let dtype = quantize_dtype_for(&name, rank);
        let q = QTensor::quantize(&tensor, dtype)
            .with_context(|| format!("quantize {name} to {dtype:?}"))?;
        qtensors.push((name, q));
        if let Some(bar) = &bar {
            bar.inc(1);
        }
    }
    if let Some(bar) = &bar {
        bar.finish_with_message("q4_K GGUF done");
    }

    let file = std::fs::File::create(out_gguf)
        .with_context(|| format!("create {}", out_gguf.display()))?;
    let mut buf = std::io::BufWriter::new(file);
    let refs: Vec<(&str, &QTensor)> = qtensors.iter().map(|(n, q)| (n.as_str(), q)).collect();
    gguf_file::write(&mut buf, &[], &refs)
        .with_context(|| format!("write GGUF {}", out_gguf.display()))?;
    buf.flush().context("flush GGUF")?;
    Ok(())
}
