// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

// Based on the Token Merging algorithm from
// https://github.com/facebookresearch/ToMe, which is licensed under CC-BY-NC.

//! Inference-only [Token Merging](https://arxiv.org/abs/2210.09461).

#![allow(clippy::pedantic)]

use candle::{D, DType, Result, Tensor};

/// Tokens merged per vision-encoder block. Conservative: 577 -> 489 over 12
/// blocks (~15% reduction). The paper's 0.2-0.3% accuracy figure is for
/// `ImageNet` classification; OCR caption accuracy at this `r` is unverified.
pub(crate) const DEFAULT_R: usize = 8;

/// Greedily merge the `r` most-similar even/odd-index token pairs by mean
/// (Bolya et al., arXiv:2210.09461). Input/output shape `(batch, tokens,
/// channels)`. When `class_token` is set, token 0 (CLS) is protected from
/// merging and returned unchanged at the front. Training-free; no unmerge.
pub(crate) fn merge(xs: &Tensor, r: usize, class_token: bool) -> Result<Tensor> {
    let (batch, tokens, ch) = xs.dims3()?;
    let (cls, body) = if class_token && tokens > 1 {
        (Some(xs.narrow(1, 0, 1)?), xs.narrow(1, 1, tokens - 1)?)
    } else {
        (None, xs.clone())
    };
    let body = body.contiguous()?;
    let body_tokens = body.dim(1)?;
    let r = r.min(body_tokens / 2);
    if r == 0 {
        return match cls {
            Some(cls) => Tensor::cat(&[&cls, &body], 1),
            None => Ok(xs.clone()),
        };
    }

    let dev = xs.device();
    let even_idx = Tensor::arange_step::<u32>(0, body_tokens as u32, 2, dev)?;
    let odd_idx = Tensor::arange_step::<u32>(1, body_tokens as u32, 2, dev)?;
    let a = body.index_select(&even_idx, 1)?;
    let dst = body.index_select(&odd_idx, 1)?;
    let na = a.dim(1)?;
    let nb = dst.dim(1)?;

    let scores = {
        let a_n = l2_normalize(&a)?;
        let b_n = l2_normalize(&dst)?;
        a_n.matmul(&b_n.transpose(1, 2)?)?
    };

    let node_idx = scores.argmax(D::Minus1)?;
    let node_max = scores
        .gather(
            &node_idx
                .unsqueeze(2)?
                .expand((batch, na, 1))?
                .contiguous()?,
            2,
        )?
        .squeeze(2)?;
    let edge_idx = node_max.sort_last_dim(false)?.1;
    let src_idx = edge_idx.narrow(1, 0, r)?.contiguous()?;
    let unm_idx = edge_idx.narrow(1, r, na - r)?.contiguous()?;
    let dst_pair = node_idx.gather(&src_idx, 1)?;

    let src_exp = src_idx.unsqueeze(2)?.expand((batch, r, ch))?.contiguous()?;
    let unm_exp = unm_idx
        .unsqueeze(2)?
        .expand((batch, na - r, ch))?
        .contiguous()?;
    let pair_exp_c = dst_pair
        .unsqueeze(2)?
        .expand((batch, r, ch))?
        .contiguous()?;
    let pair_exp_1 = dst_pair.unsqueeze(2)?.expand((batch, r, 1))?.contiguous()?;

    let unm = a.gather(&unm_exp, 1)?;
    let src = a.gather(&src_exp, 1)?;

    let dst_sum = dst.scatter_add(&pair_exp_c, &src, 1)?;
    let counts = Tensor::ones((batch, nb, 1), DType::F32, dev)?.scatter_add(
        &pair_exp_1,
        &Tensor::ones((batch, r, 1), DType::F32, dev)?,
        1,
    )?;
    let dst_out = dst_sum.broadcast_div(&counts)?;

    let body_out = Tensor::cat(&[&unm, &dst_out], 1)?;
    match cls {
        Some(cls) => Tensor::cat(&[&cls, &body_out], 1),
        None => Ok(body_out),
    }
}

fn l2_normalize(xs: &Tensor) -> Result<Tensor> {
    let denom = xs
        .sqr()?
        .sum_keepdim(D::Minus1)?
        .sqrt()?
        .clamp(1e-12f32, f32::MAX)?;
    xs.broadcast_div(&denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tensor(data: &[f32], shape: &[usize]) -> Tensor {
        Tensor::from_vec(data.to_vec(), shape, &candle::Device::Cpu).unwrap()
    }

    #[test]
    fn no_merge_when_r_zero() {
        let xs = tensor(&[1., 2., 3., 4., 5., 6.], &[1, 3, 2]);
        let out = merge(&xs, 0, false).unwrap();
        assert_eq!(out.dims(), &[1, 3, 2]);
        assert_eq!(
            out.to_vec3::<f32>().unwrap()[0],
            [[1., 2.], [3., 4.], [5., 6.]]
        );
    }

    #[test]
    fn shape_shrinks_by_r() {
        let xs = tensor(&[1., 0., 0., 1., 0., 1., 1., 0.], &[1, 4, 2]);
        let out = merge(&xs, 1, false).unwrap();
        assert_eq!(out.dims(), &[1, 3, 2]);
    }

    #[test]
    fn class_token_is_protected() {
        let xs = tensor(&[9., 9., 1., 0., 0., 1., 0., 1., 1., 0.], &[1, 5, 2]);
        let out = merge(&xs, 1, true).unwrap();
        assert_eq!(out.dims(), &[1, 4, 2]);
        let v = out.to_vec3::<f32>().unwrap();
        assert_eq!(v[0][0], [9., 9.]);
    }

    #[test]
    fn class_token_is_protected_for_multiple_batches() {
        let xs = tensor(
            &[
                9., 9., 1., 0., 0., 1., 0., 1., 1., 0., 8., 8., 1., 0., 0., 1., 0., 1., 1., 0.,
            ],
            &[2, 5, 2],
        );
        let out = merge(&xs, 1, true).unwrap();
        assert_eq!(out.dims(), &[2, 4, 2]);
        let v = out.to_vec3::<f32>().unwrap();
        assert_eq!(v[0][0], [9., 9.]);
        assert_eq!(v[1][0], [8., 8.]);
    }

    #[test]
    fn r_clamped_to_half() {
        let xs = tensor(&[1., 0., 0., 1.], &[1, 2, 2]);
        let out = merge(&xs, 10, false).unwrap();
        assert_eq!(out.dims(), &[1, 1, 2]);
    }
}
