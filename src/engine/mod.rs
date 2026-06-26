// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Source-analysis core: detection, parsing, indexing, and query operations.

pub(crate) mod binding;
pub(crate) mod def;
pub(crate) mod flags;
pub(crate) mod hash;
pub(crate) mod image;
pub(crate) mod lang;
pub(crate) mod output;
pub(crate) mod paths;
pub(crate) mod refs;
pub(crate) mod rename;
pub(crate) mod repo;
pub(crate) mod search;
pub(crate) mod source;
pub(crate) mod symbols;
pub(crate) mod target;
