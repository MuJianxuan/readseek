// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Compact projections of indexed documents.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::engine::document::Document;

pub(crate) fn render_outline(document: &Document) -> String {
    let mut output = format!(
        "{} ({}, {} pages)\n",
        document.title,
        document.format.as_str().to_uppercase(),
        document.pages
    );
    if document.nodes.is_empty() {
        output.push_str("(no outline)");
        return output;
    }

    let depths = node_depths(document);
    for node in &document.nodes {
        let depth = depths.get(&node.id).copied().unwrap_or_default();
        let title = node.title.as_deref().unwrap_or("(untitled)");
        let page = node
            .source_anchor
            .as_ref()
            .and_then(|anchor| anchor.page)
            .map(|page| format!(" [page {page}]"))
            .unwrap_or_default();
        writeln!(output, "{}[{}] {title}{page}", "  ".repeat(depth), node.id).unwrap();
    }
    output.pop();
    output
}

fn node_depths(document: &Document) -> HashMap<String, usize> {
    let mut depths = HashMap::with_capacity(document.nodes.len());
    for node in &document.nodes {
        let depth = node
            .parent_id
            .as_ref()
            .and_then(|parent| depths.get(parent))
            .map_or(0, |depth| depth + 1);
        depths.insert(node.id.clone(), depth);
    }
    depths
}
