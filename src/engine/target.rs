// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub(crate) path: PathBuf,
    pub(crate) address: Option<TargetAddress>,
    pub(crate) read_stdin: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum TargetAddress {
    Line(usize),
    Hash(String),
    Name(String),
}
