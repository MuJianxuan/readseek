// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Fixed CPU inference for the pinned Qwen3-VL-2B `Q8_0` model pair.

mod gguf;
mod kernels;
mod text;
mod tokenizer;
mod vision;

pub(crate) use text::TextModel;
pub(crate) use vision::{VisionEmbedding, VisionInput, VisionModel};
