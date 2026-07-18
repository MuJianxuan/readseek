// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Format-neutral indexed document model.

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DocumentFormat {
    Pdf,
}

impl DocumentFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "pdf" => Ok(Self::Pdf),
            _ => bail!("unsupported indexed document format: {value}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Document {
    pub(crate) id: String,
    pub(crate) format: DocumentFormat,
    pub(crate) source: PathBuf,
    pub(crate) title: String,
    pub(crate) pages: usize,
    pub(crate) nodes: Vec<Node>,
    pub(crate) assets: Vec<Asset>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NodeKind {
    Section,
}

impl NodeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Section => "section",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "section" => Ok(Self::Section),
            _ => bail!("unsupported indexed node kind: {value}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Node {
    pub(crate) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent_id: Option<String>,
    pub(crate) kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_anchor: Option<SourceAnchor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SourceAnchor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) destination: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Asset {
    pub(crate) id: String,
    pub(crate) mime: String,
    pub(crate) path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_anchor: Option<SourceAnchor>,
}
