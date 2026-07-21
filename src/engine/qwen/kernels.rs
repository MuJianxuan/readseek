// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Scalar CPU kernels used by the fixed Qwen inference path.

use std::sync::OnceLock;

use anyhow::{Result, bail, ensure};
use num_traits::ToPrimitive;
use rayon::prelude::*;

use super::gguf::{Tensor, TensorType};

/// Lossless `usize -> f32` for model dimensions/indexes that fit in `u16`.
/// `f32::from(u16)` is exact (24-bit mantissa holds any u16); `try_from` is the
/// clippy-recommended narrowing. Use only when the value provably fits `u16`.
pub(crate) fn dim_to_f32(value: usize) -> f32 {
    f32::from(u16::try_from(value).expect("model dimension/index exceeds u16"))
}

const Q8_0_BLOCK_VALUES: usize = 32;
const Q8_0_BLOCK_BYTES: usize = 2 + Q8_0_BLOCK_VALUES;
const PARALLEL_MIN_VALUES: usize = 16 * 1_024;
const MATRIX_ROW_TILE: usize = 4;
const MATRIX_OUTPUT_TILE: usize = 16;

/// Convert an IEEE 754 binary16 bit pattern to `f32`.
pub(crate) fn fp16_to_f32(value: u16) -> f32 {
    let sign = u32::from(value & 0x8000) << 16;
    let exponent = (value >> 10) & 0x1f;
    let fraction = value & 0x03ff;

    match exponent {
        0 if fraction == 0 => f32::from_bits(sign),
        0 => {
            let sign_multiplier = if sign == 0 { 1.0 } else { -1.0 };
            sign_multiplier * f32::from(fraction) * 2.0_f32.powi(-24)
        }
        0x1f => f32::from_bits(sign | 0x7f80_0000 | (u32::from(fraction) << 13)),
        _ => f32::from_bits(
            sign | ((u32::from(exponent) + (127 - 15)) << 23) | (u32::from(fraction) << 13),
        ),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Fp16AttentionKernel {
    Scalar,
    #[cfg(target_arch = "x86_64")]
    AvxF16c,
}

impl Fp16AttentionKernel {
    pub(crate) fn detect() -> Self {
        static KERNEL: OnceLock<Fp16AttentionKernel> = OnceLock::new();

        *KERNEL.get_or_init(|| {
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx") && std::is_x86_feature_detected!("f16c") {
                return Self::AvxF16c;
            }
            Self::Scalar
        })
    }

    pub(crate) fn dot_pair(self, values: &[u16], left: &[f32], right: &[f32]) -> (f32, f32) {
        assert_eq!(values.len(), left.len(), "FP16 dot product lengths differ");
        assert_eq!(values.len(), right.len(), "FP16 dot product lengths differ");
        match self {
            Self::Scalar => fp16_dot_pair_scalar(values, left, right),
            #[cfg(target_arch = "x86_64")]
            Self::AvxF16c => {
                // The variant is only constructed after runtime feature detection.
                unsafe { fp16_dot_pair_avx_f16c(values, left, right) }
            }
        }
    }

    pub(crate) fn accumulate_pair(
        self,
        values: &[u16],
        left_weight: f32,
        right_weight: f32,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        assert_eq!(values.len(), left.len(), "FP16 accumulation lengths differ");
        assert_eq!(
            values.len(),
            right.len(),
            "FP16 accumulation lengths differ"
        );
        match self {
            Self::Scalar => {
                fp16_accumulate_pair_scalar(values, left_weight, right_weight, left, right);
            }
            #[cfg(target_arch = "x86_64")]
            Self::AvxF16c => {
                // The variant is only constructed after runtime feature detection.
                unsafe {
                    fp16_accumulate_pair_avx_f16c(values, left_weight, right_weight, left, right);
                }
            }
        }
    }
}

fn fp16_dot_pair_scalar(values: &[u16], left: &[f32], right: &[f32]) -> (f32, f32) {
    let mut left_sum = 0.0;
    let mut right_sum = 0.0;
    for ((value, left), right) in values.iter().zip(left).zip(right) {
        let value = fp16_to_f32(*value);
        left_sum += left * value;
        right_sum += right * value;
    }
    (left_sum, right_sum)
}

fn fp16_accumulate_pair_scalar(
    values: &[u16],
    left_weight: f32,
    right_weight: f32,
    left: &mut [f32],
    right: &mut [f32],
) {
    for ((value, left), right) in values.iter().zip(left).zip(right) {
        let value = fp16_to_f32(*value);
        *left += left_weight * value;
        *right += right_weight * value;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,f16c")]
unsafe fn fp16_dot_pair_avx_f16c(values: &[u16], left: &[f32], right: &[f32]) -> (f32, f32) {
    use std::arch::x86_64::{
        __m128i, _mm256_add_ps, _mm256_cvtph_ps, _mm256_loadu_ps, _mm256_mul_ps, _mm256_setzero_ps,
        _mm256_storeu_ps,
    };

    let vectorized_len = values.len() / 8 * 8;
    let mut left_sums = _mm256_setzero_ps();
    let mut right_sums = _mm256_setzero_ps();
    for index in (0..vectorized_len).step_by(8) {
        // `read_unaligned` is the intended unaligned read; codegen is identical
        // to `_mm_loadu_si128` and does not trigger `cast_ptr_alignment`.
        let packed =
            unsafe { std::ptr::read_unaligned(values.as_ptr().add(index).cast::<__m128i>()) };
        let unpacked = _mm256_cvtph_ps(packed);
        let left_values = unsafe { _mm256_loadu_ps(left.as_ptr().add(index)) };
        let right_values = unsafe { _mm256_loadu_ps(right.as_ptr().add(index)) };
        left_sums = _mm256_add_ps(left_sums, _mm256_mul_ps(left_values, unpacked));
        right_sums = _mm256_add_ps(right_sums, _mm256_mul_ps(right_values, unpacked));
    }
    let mut left_lanes = [0.0_f32; 8];
    let mut right_lanes = [0.0_f32; 8];
    unsafe {
        _mm256_storeu_ps(left_lanes.as_mut_ptr(), left_sums);
        _mm256_storeu_ps(right_lanes.as_mut_ptr(), right_sums);
    }
    let (left_tail, right_tail) = fp16_dot_pair_scalar(
        &values[vectorized_len..],
        &left[vectorized_len..],
        &right[vectorized_len..],
    );
    (
        left_lanes.into_iter().sum::<f32>() + left_tail,
        right_lanes.into_iter().sum::<f32>() + right_tail,
    )
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,f16c")]
unsafe fn fp16_accumulate_pair_avx_f16c(
    values: &[u16],
    left_weight: f32,
    right_weight: f32,
    left: &mut [f32],
    right: &mut [f32],
) {
    use std::arch::x86_64::{
        __m128i, _mm256_add_ps, _mm256_cvtph_ps, _mm256_loadu_ps, _mm256_mul_ps, _mm256_set1_ps,
        _mm256_storeu_ps,
    };

    let vectorized_len = values.len() / 8 * 8;
    let left_weights = _mm256_set1_ps(left_weight);
    let right_weights = _mm256_set1_ps(right_weight);
    for index in (0..vectorized_len).step_by(8) {
        // `read_unaligned` is the intended unaligned read; codegen is identical
        // to `_mm_loadu_si128` and does not trigger `cast_ptr_alignment`.
        let packed =
            unsafe { std::ptr::read_unaligned(values.as_ptr().add(index).cast::<__m128i>()) };
        let unpacked = _mm256_cvtph_ps(packed);
        let left_values = unsafe { _mm256_loadu_ps(left.as_ptr().add(index)) };
        let right_values = unsafe { _mm256_loadu_ps(right.as_ptr().add(index)) };
        let left_values = _mm256_add_ps(left_values, _mm256_mul_ps(left_weights, unpacked));
        let right_values = _mm256_add_ps(right_values, _mm256_mul_ps(right_weights, unpacked));
        unsafe {
            _mm256_storeu_ps(left.as_mut_ptr().add(index), left_values);
            _mm256_storeu_ps(right.as_mut_ptr().add(index), right_values);
        }
    }
    fp16_accumulate_pair_scalar(
        &values[vectorized_len..],
        left_weight,
        right_weight,
        &mut left[vectorized_len..],
        &mut right[vectorized_len..],
    );
}

/// Convert `f32` to an IEEE 754 binary16 bit pattern using ties-to-even rounding.
pub(crate) fn f32_to_fp16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = u16::try_from(bits >> 16).expect("top 16 bits fit u16") & 0x8000;
    let exponent = i32::from(u8::try_from((bits >> 23) & 0xff).expect("exponent fits u8"));
    let fraction = bits & 0x007f_ffff;

    if exponent == 0xff {
        if fraction == 0 {
            return sign | 0x7c00;
        }
        return sign
            | 0x7e00
            | (u16::try_from(fraction >> 13).expect("fraction fits u16") & 0x01ff);
    }

    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let significand = fraction | 0x0080_0000;
        let shift = u32::try_from(14 - half_exponent).expect("shift is non-negative");
        let mut rounded = significand >> shift;
        let remainder = significand & ((1_u32 << shift) - 1);
        let halfway = 1_u32 << (shift - 1);
        if remainder > halfway || (remainder == halfway && rounded & 1 != 0) {
            rounded += 1;
        }
        return sign | u16::try_from(rounded).expect("rounded fits u16");
    }

    let mut rounded_fraction = fraction >> 13;
    let remainder = fraction & 0x1fff;
    if remainder > 0x1000 || (remainder == 0x1000 && rounded_fraction & 1 != 0) {
        rounded_fraction += 1;
    }
    let mut encoded_exponent = u16::try_from(half_exponent).expect("half_exponent in 1..=0x1e");
    if rounded_fraction == 0x400 {
        rounded_fraction = 0;
        encoded_exponent += 1;
        if encoded_exponent == 0x1f {
            return sign | 0x7c00;
        }
    }
    sign | (encoded_exponent << 10)
        | u16::try_from(rounded_fraction).expect("rounded_fraction fits u16")
}

/// Dequantize one `Q8_0` row into a caller-provided buffer.
pub(crate) fn dequantize_q8_0_row(row: &[u8], output: &mut [f32]) -> Result<()> {
    validate_q8_0_row(row, output.len())?;

    for (block, values) in row
        .chunks_exact(Q8_0_BLOCK_BYTES)
        .zip(output.chunks_exact_mut(Q8_0_BLOCK_VALUES))
    {
        let scale = fp16_to_f32(u16::from_le_bytes([block[0], block[1]]));
        for (value, quantized) in values.iter_mut().zip(&block[2..]) {
            *value = scale * f32::from(i8::from_ne_bytes([*quantized]));
        }
    }

    Ok(())
}

fn validate_q8_0_row(row: &[u8], value_count: usize) -> Result<()> {
    ensure!(
        value_count.is_multiple_of(Q8_0_BLOCK_VALUES),
        "Q8_0 row length {value_count} is not a multiple of {Q8_0_BLOCK_VALUES}"
    );
    let expected = value_count / Q8_0_BLOCK_VALUES * Q8_0_BLOCK_BYTES;
    ensure!(
        row.len() == expected,
        "Q8_0 row has {} bytes, expected {expected} for {value_count} values",
        row.len()
    );
    Ok(())
}

#[cfg(test)]
fn dot_q8_0_scalar(row: &[u8], vector: &[f32]) -> f32 {
    let mut sum = 0.0;
    for (block, values) in row
        .chunks_exact(Q8_0_BLOCK_BYTES)
        .zip(vector.chunks_exact(Q8_0_BLOCK_VALUES))
    {
        let scale = fp16_to_f32(u16::from_le_bytes([block[0], block[1]]));
        let mut block_sum = 0.0;
        for index in 0..Q8_0_BLOCK_VALUES {
            let quantized = i8::from_ne_bytes([block[index + 2]]);
            block_sum += f32::from(quantized) * values[index];
        }
        sum += scale * block_sum;
    }
    sum
}

struct Q8Activation {
    scales: Vec<f32>,
    values: Vec<i8>,
}

impl Q8Activation {
    // `round()` + `clamp(-127.0, 127.0)` keeps the value in `i8` range, so the
    // checked `to_i8()` is total here and rejects only a contract bug.
    fn new(vector: &[f32]) -> Result<Self> {
        ensure!(
            vector.len().is_multiple_of(Q8_0_BLOCK_VALUES),
            "Q8 activation length {} is not a multiple of {Q8_0_BLOCK_VALUES}",
            vector.len()
        );
        ensure!(
            vector.iter().all(|value| value.is_finite()),
            "Q8 activation contains a non-finite value"
        );
        let mut scales = Vec::with_capacity(vector.len() / Q8_0_BLOCK_VALUES);
        let mut values = Vec::with_capacity(vector.len());
        for block in vector.chunks_exact(Q8_0_BLOCK_VALUES) {
            let maximum = block.iter().copied().map(f32::abs).fold(0.0, f32::max);
            let scale = maximum / 127.0;
            let inverse = if scale == 0.0 { 0.0 } else { scale.recip() };
            scales.push(scale);
            values.extend(block.iter().map(|value| {
                (value * inverse)
                    .round()
                    .clamp(-127.0, 127.0)
                    .to_i8()
                    .expect("scale is clamped into i8 range")
            }));
        }
        Ok(Self { scales, values })
    }
}

#[derive(Clone, Copy)]
enum Q8DotKernel {
    Scalar,
    #[cfg(target_arch = "x86_64")]
    Avx2,
}

impl Q8DotKernel {
    fn detect() -> Self {
        static KERNEL: OnceLock<Q8DotKernel> = OnceLock::new();

        *KERNEL.get_or_init(|| {
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx2") {
                return Self::Avx2;
            }
            Self::Scalar
        })
    }

    fn dot(self, row: &[u8], activation: &Q8Activation) -> f32 {
        match self {
            Self::Scalar => dot_q8_0_quantized_scalar(row, activation),
            #[cfg(target_arch = "x86_64")]
            Self::Avx2 => {
                // The variant is only constructed after runtime AVX2 detection.
                unsafe { dot_q8_0_quantized_avx2(row, activation) }
            }
        }
    }
}

fn dot_q8_0_quantized_scalar(row: &[u8], activation: &Q8Activation) -> f32 {
    let mut sum = 0.0;
    for ((block, values), activation_scale) in row
        .chunks_exact(Q8_0_BLOCK_BYTES)
        .zip(activation.values.chunks_exact(Q8_0_BLOCK_VALUES))
        .zip(&activation.scales)
    {
        let weight_scale = fp16_to_f32(u16::from_le_bytes([block[0], block[1]]));
        // i8 weights and i8 activations each fit exactly in f32; the 32-wide
        // product sum stays below f32's 24-bit exact range.
        let block_sum = block[2..]
            .iter()
            .zip(values)
            .map(|(weight, value)| f32::from(i8::from_ne_bytes([*weight])) * f32::from(*value))
            .sum::<f32>();
        sum += weight_scale * activation_scale * block_sum;
    }
    sum
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dot_q8_0_quantized_avx2(row: &[u8], activation: &Q8Activation) -> f32 {
    use std::arch::x86_64::{
        __m256i, _mm_add_epi32, _mm_cvtepi32_ps, _mm_cvtss_f32, _mm_shuffle_epi32,
        _mm_unpackhi_epi64, _mm256_abs_epi8, _mm256_castsi256_si128, _mm256_extracti128_si256,
        _mm256_madd_epi16, _mm256_maddubs_epi16, _mm256_set1_epi16, _mm256_sign_epi8,
    };

    let ones = _mm256_set1_epi16(1);
    let mut sum = 0.0;
    for ((block, values), activation_scale) in row
        .chunks_exact(Q8_0_BLOCK_BYTES)
        .zip(activation.values.chunks_exact(Q8_0_BLOCK_VALUES))
        .zip(&activation.scales)
    {
        // `read_unaligned` is the intended unaligned read; codegen is identical
        // to `_mm256_loadu_si256` and does not trigger `cast_ptr_alignment`.
        let weights = unsafe { std::ptr::read_unaligned(block[2..].as_ptr().cast::<__m256i>()) };
        let activations = unsafe { std::ptr::read_unaligned(values.as_ptr().cast::<__m256i>()) };
        let signed_activations = _mm256_sign_epi8(activations, weights);
        let weight_magnitudes = _mm256_abs_epi8(weights);
        let pairs = _mm256_maddubs_epi16(weight_magnitudes, signed_activations);
        let products = _mm256_madd_epi16(pairs, ones);
        let low = _mm256_castsi256_si128(products);
        let high = _mm256_extracti128_si256::<1>(products);
        let lanes = _mm_add_epi32(low, high);
        let pairs = _mm_add_epi32(lanes, _mm_unpackhi_epi64(lanes, lanes));
        let total = _mm_add_epi32(pairs, _mm_shuffle_epi32::<0x55>(pairs));
        let weight_scale = fp16_to_f32(u16::from_le_bytes([block[0], block[1]]));
        sum += weight_scale * activation_scale * _mm_cvtss_f32(_mm_cvtepi32_ps(total));
    }
    sum
}

/// Multiply two `Q8_0` matrices by one shared activation vector.
pub(crate) fn matrix_vector_pair(
    left: &Tensor,
    right: &Tensor,
    vector: &[f32],
) -> Result<(Vec<f32>, Vec<f32>)> {
    let dimensions = matrix_dimensions(left)?;
    ensure!(
        matrix_dimensions(right)? == dimensions,
        "paired matrix dimensions differ"
    );
    ensure!(
        left.tensor_type() == TensorType::Q8_0 && right.tensor_type() == TensorType::Q8_0,
        "paired matrix projection requires Q8_0 tensors"
    );
    let [input_size, output_size] = dimensions;
    ensure!(
        vector.len() == input_size,
        "paired matrix input length differs"
    );
    let row_bytes = q8_matrix_row_bytes(left, input_size, output_size)?;
    q8_matrix_row_bytes(right, input_size, output_size)?;
    let activation = Q8Activation::new(vector)?;
    let kernel = Q8DotKernel::detect();
    let mut left_output = vec![0.0; output_size];
    let mut right_output = vec![0.0; output_size];
    left_output
        .par_iter_mut()
        .zip(right_output.par_iter_mut())
        .enumerate()
        .for_each(|(row, (left_value, right_value))| {
            let start = row * row_bytes;
            let end = start + row_bytes;
            *left_value = kernel.dot(&left.data()[start..end], &activation);
            *right_value = kernel.dot(&right.data()[start..end], &activation);
        });
    Ok((left_output, right_output))
}

/// Multiply three `Q8_0` matrices by one shared activation vector.
pub(crate) fn matrix_vector_triple(
    first: &Tensor,
    second: &Tensor,
    third: &Tensor,
    vector: &[f32],
) -> Result<(Vec<f32>, Vec<f32>, Vec<f32>)> {
    let [input_size, first_size] = matrix_dimensions(first)?;
    let [second_input, second_size] = matrix_dimensions(second)?;
    let [third_input, third_size] = matrix_dimensions(third)?;
    ensure!(
        second_input == input_size && third_input == input_size,
        "triple matrix input dimensions differ"
    );
    ensure!(
        first.tensor_type() == TensorType::Q8_0
            && second.tensor_type() == TensorType::Q8_0
            && third.tensor_type() == TensorType::Q8_0,
        "triple matrix projection requires Q8_0 tensors"
    );
    ensure!(
        vector.len() == input_size,
        "triple matrix input length differs"
    );
    let first_row_bytes = q8_matrix_row_bytes(first, input_size, first_size)?;
    let second_row_bytes = q8_matrix_row_bytes(second, input_size, second_size)?;
    let third_row_bytes = q8_matrix_row_bytes(third, input_size, third_size)?;
    let activation = Q8Activation::new(vector)?;
    let kernel = Q8DotKernel::detect();
    let mut first_output = vec![0.0; first_size];
    let mut second_output = vec![0.0; second_size];
    let mut third_output = vec![0.0; third_size];
    first_output
        .par_iter_mut()
        .chain(second_output.par_iter_mut())
        .chain(third_output.par_iter_mut())
        .enumerate()
        .for_each(|(row, value)| {
            let (matrix, matrix_row, row_bytes) = if row < first_size {
                (first, row, first_row_bytes)
            } else if row < first_size + second_size {
                (second, row - first_size, second_row_bytes)
            } else {
                (third, row - first_size - second_size, third_row_bytes)
            };
            let start = matrix_row * row_bytes;
            *value = kernel.dot(&matrix.data()[start..start + row_bytes], &activation);
        });
    Ok((first_output, second_output, third_output))
}

/// Return the index of the largest output from a `Q8_0` matrix-vector product.
pub(crate) fn matrix_vector_argmax(matrix: &Tensor, vector: &[f32]) -> Result<usize> {
    #[derive(Clone, Copy)]
    struct Candidate {
        invalid: Option<usize>,
        index: usize,
        value: f32,
    }

    let [input_size, output_size] = matrix_dimensions(matrix)?;
    ensure!(
        output_size != 0,
        "cannot select from an empty matrix output"
    );
    ensure!(
        vector.len() == input_size,
        "matrix input is {input_size}, but vector length is {}",
        vector.len()
    );
    ensure!(
        matrix.tensor_type() == TensorType::Q8_0,
        "fused matrix argmax requires a Q8_0 tensor"
    );
    let row_bytes = q8_matrix_row_bytes(matrix, input_size, output_size)?;
    let activation = Q8Activation::new(vector)?;
    let kernel = Q8DotKernel::detect();
    let best = (0..output_size)
        .into_par_iter()
        .map(|index| {
            let start = index * row_bytes;
            let row = &matrix.data()[start..start + row_bytes];
            let value = kernel.dot(row, &activation);
            Candidate {
                invalid: (!value.is_finite()).then_some(index),
                index,
                value,
            }
        })
        .reduce(
            || Candidate {
                invalid: None,
                index: 0,
                value: f32::NEG_INFINITY,
            },
            |left, right| {
                let take_right = right.value.partial_cmp(&left.value).is_some_and(|order| {
                    order.is_gt() || (order.is_eq() && right.index < left.index)
                });
                let (index, value) = if take_right {
                    (right.index, right.value)
                } else {
                    (left.index, left.value)
                };
                Candidate {
                    invalid: match (left.invalid, right.invalid) {
                        (Some(left), Some(right)) => Some(left.min(right)),
                        (left, right) => left.or(right),
                    },
                    index,
                    value,
                }
            },
        );
    if let Some(index) = best.invalid {
        bail!("matrix output {index} is not finite");
    }
    Ok(best.index)
}

/// Multiply a GGUF matrix with logical dimensions `[input, output]` by a vector.
pub(crate) fn matrix_vector(matrix: &Tensor, vector: &[f32]) -> Result<Vec<f32>> {
    let [input_size, output_size] = matrix_dimensions(matrix)?;
    ensure!(
        vector.len() == input_size,
        "matrix input is {input_size}, but vector length is {}",
        vector.len()
    );

    let mut output = vec![0.0; output_size];
    match matrix.tensor_type() {
        TensorType::F32 => {
            let data = f32_matrix_data(matrix, input_size, output_size)?;
            output
                .par_iter_mut()
                .enumerate()
                .for_each(|(row_index, value)| {
                    let row_start = row_index * input_size;
                    *value = dot_f32(&data[row_start..row_start + input_size], vector);
                });
        }
        TensorType::Q8_0 => {
            let row_bytes = q8_matrix_row_bytes(matrix, input_size, output_size)?;
            let activation = Q8Activation::new(vector)?;
            let kernel = Q8DotKernel::detect();
            output
                .par_iter_mut()
                .enumerate()
                .for_each(|(row_index, value)| {
                    let row_start = row_index * row_bytes;
                    *value = kernel.dot(
                        &matrix.data()[row_start..row_start + row_bytes],
                        &activation,
                    );
                });
        }
    }

    Ok(output)
}

/// Multiply row-major `F32` vectors by a GGUF matrix with dimensions `[input, output]`.
///
/// `vectors` contains `row_count` consecutive vectors. The result uses the same
/// row-major layout with `output` values per row.
pub(crate) fn matrix_matrix(
    matrix: &Tensor,
    vectors: &[f32],
    row_count: usize,
) -> Result<Vec<f32>> {
    let [input_size, output_size] = matrix_dimensions(matrix)?;
    let expected = row_count
        .checked_mul(input_size)
        .ok_or_else(|| anyhow::anyhow!("matrix-matrix input size overflow"))?;
    ensure!(
        vectors.len() == expected,
        "matrix-matrix input has {} values, expected {expected} ({row_count} x {input_size})",
        vectors.len()
    );
    let output_len = row_count
        .checked_mul(output_size)
        .ok_or_else(|| anyhow::anyhow!("matrix-matrix output size overflow"))?;
    let mut output = vec![0.0; output_len];

    match matrix.tensor_type() {
        TensorType::F32 => {
            let data = f32_matrix_data(matrix, input_size, output_size)?;
            let kernel = F32DotKernel::detect();
            output
                .par_chunks_mut(MATRIX_ROW_TILE * output_size)
                .zip(vectors.par_chunks(MATRIX_ROW_TILE * input_size))
                .for_each(|(output_rows, input_rows)| {
                    for output_start in (0..output_size).step_by(MATRIX_OUTPUT_TILE) {
                        let output_end = (output_start + MATRIX_OUTPUT_TILE).min(output_size);
                        for output_channel in output_start..output_end {
                            let row_start = output_channel * input_size;
                            let weights = &data[row_start..row_start + input_size];
                            for (output_row, input_row) in output_rows
                                .chunks_exact_mut(output_size)
                                .zip(input_rows.chunks_exact(input_size))
                            {
                                output_row[output_channel] = kernel.dot(weights, input_row);
                            }
                        }
                    }
                });
        }
        TensorType::Q8_0 => {
            let row_bytes = q8_matrix_row_bytes(matrix, input_size, output_size)?;
            let kernel = Q8DotKernel::detect();
            output
                .par_chunks_mut(MATRIX_ROW_TILE * output_size)
                .zip(vectors.par_chunks(MATRIX_ROW_TILE * input_size))
                .try_for_each(|(output_rows, input_rows)| -> Result<()> {
                    let activations = input_rows
                        .chunks_exact(input_size)
                        .map(Q8Activation::new)
                        .collect::<Result<Vec<_>>>()?;
                    for output_start in (0..output_size).step_by(MATRIX_OUTPUT_TILE) {
                        let output_end = (output_start + MATRIX_OUTPUT_TILE).min(output_size);
                        for output_channel in output_start..output_end {
                            let row_start = output_channel * row_bytes;
                            let weights = &matrix.data()[row_start..row_start + row_bytes];
                            for (output_row, activation) in
                                output_rows.chunks_exact_mut(output_size).zip(&activations)
                            {
                                output_row[output_channel] = kernel.dot(weights, activation);
                            }
                        }
                    }
                    Ok(())
                })?;
        }
    }

    Ok(output)
}

fn matrix_dimensions(matrix: &Tensor) -> Result<[usize; 2]> {
    let dimensions = match matrix.dimensions() {
        [input_size, output_size] => [*input_size, *output_size],
        dimensions => {
            bail!("matrix must have two logical dimensions [input, output], got {dimensions:?}")
        }
    };
    ensure!(
        dimensions[0] != 0 && dimensions[1] != 0,
        "matrix dimensions must be nonzero, got {dimensions:?}"
    );
    Ok(dimensions)
}

fn f32_matrix_data<'a>(
    matrix: &'a Tensor<'_>,
    input_size: usize,
    output_size: usize,
) -> Result<&'a [f32]> {
    let data = matrix.f32_slice()?;
    let expected = input_size
        .checked_mul(output_size)
        .ok_or_else(|| anyhow::anyhow!("F32 matrix element count overflow"))?;
    ensure!(
        data.len() == expected,
        "F32 matrix has {} values, expected {expected}",
        data.len()
    );
    Ok(data)
}

fn q8_matrix_row_bytes(matrix: &Tensor, input_size: usize, output_size: usize) -> Result<usize> {
    ensure!(
        input_size.is_multiple_of(Q8_0_BLOCK_VALUES),
        "Q8_0 matrix input dimension {input_size} is not a multiple of {Q8_0_BLOCK_VALUES}"
    );
    let row_bytes = input_size / Q8_0_BLOCK_VALUES * Q8_0_BLOCK_BYTES;
    let expected = row_bytes
        .checked_mul(output_size)
        .ok_or_else(|| anyhow::anyhow!("Q8_0 matrix byte size overflow"))?;
    ensure!(
        matrix.data().len() == expected,
        "Q8_0 matrix has {} data bytes, expected {expected}",
        matrix.data().len()
    );
    Ok(row_bytes)
}

#[derive(Clone, Copy)]
enum F32DotKernel {
    Scalar,
    #[cfg(target_arch = "x86_64")]
    Avx2Fma,
}

impl F32DotKernel {
    fn detect() -> Self {
        static KERNEL: OnceLock<F32DotKernel> = OnceLock::new();

        *KERNEL.get_or_init(|| {
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma") {
                return Self::Avx2Fma;
            }
            Self::Scalar
        })
    }

    fn dot(self, row: &[f32], vector: &[f32]) -> f32 {
        assert_eq!(row.len(), vector.len(), "F32 dot product lengths differ");
        match self {
            Self::Scalar => dot_f32_scalar(row, vector),
            #[cfg(target_arch = "x86_64")]
            Self::Avx2Fma => {
                // The variant is only constructed after runtime AVX2 and FMA detection.
                unsafe { dot_f32_avx2_fma(row, vector) }
            }
        }
    }
}

fn dot_f32(row: &[f32], vector: &[f32]) -> f32 {
    F32DotKernel::detect().dot(row, vector)
}

fn dot_f32_scalar(row: &[f32], vector: &[f32]) -> f32 {
    row.iter()
        .zip(vector)
        .map(|(left, right)| left * right)
        .sum()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn dot_f32_avx2_fma(row: &[f32], vector: &[f32]) -> f32 {
    use std::arch::x86_64::{
        _mm256_fmadd_ps, _mm256_loadu_ps, _mm256_setzero_ps, _mm256_storeu_ps,
    };

    let vectorized_len = row.len() / 8 * 8;
    let mut sums = _mm256_setzero_ps();
    for index in (0..vectorized_len).step_by(8) {
        let left = unsafe { _mm256_loadu_ps(row.as_ptr().add(index)) };
        let right = unsafe { _mm256_loadu_ps(vector.as_ptr().add(index)) };
        sums = _mm256_fmadd_ps(left, right, sums);
    }
    let mut lanes = [0.0_f32; 8];
    unsafe { _mm256_storeu_ps(lanes.as_mut_ptr(), sums) };
    lanes.into_iter().sum::<f32>()
        + dot_f32_scalar(&row[vectorized_len..], &vector[vectorized_len..])
}

/// Add a channel bias to each row in place.
pub(crate) fn add_bias(values: &mut [f32], bias: &[f32]) -> Result<()> {
    ensure!(!bias.is_empty(), "bias must not be empty");
    ensure!(
        values.len().is_multiple_of(bias.len()),
        "value length {} is not divisible by bias length {}",
        values.len(),
        bias.len()
    );
    if values.len() < PARALLEL_MIN_VALUES {
        for row in values.chunks_mut(bias.len()) {
            row.iter_mut()
                .zip(bias)
                .for_each(|(value, bias)| *value += bias);
        }
    } else {
        values.par_chunks_mut(bias.len()).for_each(|row| {
            row.iter_mut()
                .zip(bias)
                .for_each(|(value, bias)| *value += bias);
        });
    }
    Ok(())
}

/// Apply RMS normalization independently to each row.
pub(crate) fn rms_norm(
    values: &[f32],
    width: usize,
    weight: &[f32],
    epsilon: f32,
) -> Result<Vec<f32>> {
    validate_norm_shape(values, width, weight, None, epsilon)?;
    let mut output = vec![0.0; values.len()];
    let normalize = |output_row: &mut [f32], input_row: &[f32]| {
        let mean_square =
            input_row.iter().map(|value| value * value).sum::<f32>() / dim_to_f32(width);
        let scale = (mean_square + epsilon).sqrt().recip();
        for index in 0..width {
            output_row[index] = input_row[index] * scale * weight[index];
        }
    };
    if values.len() < PARALLEL_MIN_VALUES {
        output
            .chunks_mut(width)
            .zip(values.chunks(width))
            .for_each(|(output_row, input_row)| normalize(output_row, input_row));
    } else {
        output
            .par_chunks_mut(width)
            .zip(values.par_chunks(width))
            .for_each(|(output_row, input_row)| normalize(output_row, input_row));
    }
    Ok(output)
}

/// Apply `LayerNorm` independently to each row.
pub(crate) fn layer_norm(
    values: &[f32],
    width: usize,
    weight: &[f32],
    bias: &[f32],
    epsilon: f32,
) -> Result<Vec<f32>> {
    validate_norm_shape(values, width, weight, Some(bias), epsilon)?;
    let mut output = vec![0.0; values.len()];
    let normalize = |output_row: &mut [f32], input_row: &[f32]| {
        let mean = input_row.iter().sum::<f32>() / dim_to_f32(width);
        let variance = input_row
            .iter()
            .map(|value| {
                let centered = value - mean;
                centered * centered
            })
            .sum::<f32>()
            / dim_to_f32(width);
        let scale = (variance + epsilon).sqrt().recip();
        for index in 0..width {
            output_row[index] = (input_row[index] - mean) * scale * weight[index] + bias[index];
        }
    };
    if values.len() < PARALLEL_MIN_VALUES {
        output
            .chunks_mut(width)
            .zip(values.chunks(width))
            .for_each(|(output_row, input_row)| normalize(output_row, input_row));
    } else {
        output
            .par_chunks_mut(width)
            .zip(values.par_chunks(width))
            .for_each(|(output_row, input_row)| normalize(output_row, input_row));
    }
    Ok(output)
}

fn validate_norm_shape(
    values: &[f32],
    width: usize,
    weight: &[f32],
    bias: Option<&[f32]>,
    epsilon: f32,
) -> Result<()> {
    ensure!(width != 0, "normalization width must not be zero");
    ensure!(
        values.len().is_multiple_of(width),
        "value length {} is not divisible by normalization width {width}",
        values.len()
    );
    ensure!(
        weight.len() == width,
        "normalization weight length is {}, expected {width}",
        weight.len()
    );
    if let Some(bias) = bias {
        ensure!(
            bias.len() == width,
            "normalization bias length is {}, expected {width}",
            bias.len()
        );
    }
    ensure!(
        epsilon.is_finite() && epsilon >= 0.0,
        "normalization epsilon must be finite and non-negative"
    );
    Ok(())
}

/// Apply `SiLU` in place.
pub(crate) fn silu(values: &mut [f32]) {
    let apply = |value: &mut f32| *value /= 1.0 + (-*value).exp();
    if values.len() < PARALLEL_MIN_VALUES {
        values.iter_mut().for_each(apply);
    } else {
        values.par_iter_mut().for_each(apply);
    }
}

/// Apply the GELU tanh approximation in place.
pub(crate) fn gelu(values: &mut [f32]) {
    const COEFFICIENT: f32 = 0.044_715;
    const SQRT_TWO_OVER_PI: f32 = 0.797_884_6;

    let apply = |value: &mut f32| {
        let input = *value;
        *value =
            0.5 * input * (1.0 + (SQRT_TWO_OVER_PI * (input + COEFFICIENT * input.powi(3))).tanh());
    };
    if values.len() < PARALLEL_MIN_VALUES {
        values.iter_mut().for_each(apply);
    } else {
        values.par_iter_mut().for_each(apply);
    }
}

/// Apply softmax in place. Empty slices are accepted.
pub(crate) fn softmax(values: &mut [f32]) {
    if values.is_empty() {
        return;
    }
    let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum = values
        .iter_mut()
        .map(|value| {
            *value = (*value - maximum).exp();
            *value
        })
        .sum::<f32>();
    for value in values {
        *value /= sum;
    }
}

/// Add `right` to `left` in place.
pub(crate) fn vector_add(left: &mut [f32], right: &[f32]) -> Result<()> {
    ensure!(
        left.len() == right.len(),
        "vector lengths differ: {} and {}",
        left.len(),
        right.len()
    );
    if left.len() < PARALLEL_MIN_VALUES {
        left.iter_mut()
            .zip(right)
            .for_each(|(left, right)| *left += right);
    } else {
        left.par_iter_mut()
            .zip(right.par_iter())
            .for_each(|(left, right)| *left += right);
    }
    Ok(())
}

/// Multiply `left` by `right` in place.
pub(crate) fn vector_multiply(left: &mut [f32], right: &[f32]) -> Result<()> {
    ensure!(
        left.len() == right.len(),
        "vector lengths differ: {} and {}",
        left.len(),
        right.len()
    );
    if left.len() < PARALLEL_MIN_VALUES {
        left.iter_mut()
            .zip(right)
            .for_each(|(left, right)| *left *= right);
    } else {
        left.par_iter_mut()
            .zip(right.par_iter())
            .for_each(|(left, right)| *left *= right);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_bits(actual: f32, expected: f32) {
        assert!(
            actual.to_bits() == expected.to_bits(),
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn fp16_round_trips_all_non_nan_values() {
        for bits in 0_u16..=u16::MAX {
            let value = fp16_to_f32(bits);
            let encoded = f32_to_fp16(value);
            if value.is_nan() {
                assert!(fp16_to_f32(encoded).is_nan());
            } else {
                assert_eq!(encoded, bits);
            }
        }
    }

    #[test]
    fn fp16_handles_boundaries() {
        assert_eq!(f32_to_fp16(0.0), 0x0000);
        assert_eq!(f32_to_fp16(-0.0), 0x8000);
        assert_eq!(f32_to_fp16(1.0), 0x3c00);
        assert_eq!(f32_to_fp16(f32::INFINITY), 0x7c00);
        assert_eq!(f32_to_fp16(f32::NEG_INFINITY), 0xfc00);
        assert!(fp16_to_f32(f32_to_fp16(f32::NAN)).is_nan());
    }
    #[test]
    fn quantizes_zero_and_signed_extrema() {
        let mut values = vec![0.0; Q8_0_BLOCK_VALUES * 2];
        values[Q8_0_BLOCK_VALUES] = -127.0;
        values[Q8_0_BLOCK_VALUES + 1] = 127.0;
        let activation = Q8Activation::new(&values).expect("quantize activation");
        assert_eq!(activation.scales.len(), 2);
        assert_bits(activation.scales[0], 0.0);
        assert_bits(activation.scales[1], 1.0);
        assert!(
            activation.values[..Q8_0_BLOCK_VALUES]
                .iter()
                .all(|value| *value == 0)
        );
        assert_eq!(activation.values[Q8_0_BLOCK_VALUES], -127);
        assert_eq!(activation.values[Q8_0_BLOCK_VALUES + 1], 127);
    }

    #[test]
    fn quantized_dot_matches_dequantized_reference() {
        let mut vector = (0..Q8_0_BLOCK_VALUES)
            .map(|index| dim_to_f32(index) - 16.0)
            .collect::<Vec<_>>();
        vector[Q8_0_BLOCK_VALUES - 1] = 127.0;
        let mut row = vec![0_u8; Q8_0_BLOCK_BYTES];
        row[..2].copy_from_slice(&0x3c00_u16.to_le_bytes());
        for (index, value) in row[2..].iter_mut().enumerate() {
            *value = (i8::try_from(index).expect("index fits i8") - 16).to_ne_bytes()[0];
        }
        row[2] = (-128_i8).to_ne_bytes()[0];
        let activation = Q8Activation::new(&vector).expect("quantize activation");
        let expected = dot_q8_0_scalar(&row, &vector);
        assert_bits(dot_q8_0_quantized_scalar(&row, &activation), expected);
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            let actual = unsafe { dot_q8_0_quantized_avx2(&row, &activation) };
            assert_bits(actual, expected);
        }
    }
}
