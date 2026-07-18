// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Structural selection and compact projections of indexed documents.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use anyhow::{Result, bail};

use crate::engine::document::{Document, Node, NodeKind};

#[derive(Clone, Copy)]
pub(crate) struct Selection<'a> {
    pub(crate) node: Option<&'a str>,
    pub(crate) page: Option<usize>,
    pub(crate) kind: Option<NodeKind>,
    pub(crate) depth: Option<usize>,
    pub(crate) outline: bool,
    pub(crate) overview: bool,
}

pub(crate) fn select(document: &Document, selection: Selection<'_>) -> Result<Document> {
    let by_id: HashMap<&str, &Node> = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    if let Some(root) = selection.node {
        let Some(root_node) = by_id.get(root) else {
            bail!("node {root} not found");
        };
        if selection.outline && root_node.kind != NodeKind::Section {
            bail!("node {root} is not an outline node");
        }
    }
    let overview_kind = selection.overview.then(|| {
        if document
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Section)
        {
            NodeKind::Section
        } else {
            NodeKind::Page
        }
    });

    let mut nodes: Vec<Node> = document
        .nodes
        .iter()
        .filter(|node| {
            if let Some(root) = selection.node
                && node.id != root
                && !is_descendant(node, root, &by_id)
            {
                return false;
            }
            if let Some(page) = selection.page {
                let node_page = node.source_anchor.as_ref().and_then(|anchor| anchor.page);
                if node_page != Some(page) {
                    return false;
                }
                if !selection.outline && node.kind == NodeKind::Section {
                    return false;
                }
            }
            if selection.outline && node.kind != NodeKind::Section {
                return false;
            }
            let kind = selection.kind.or(overview_kind);
            if kind.is_some_and(|kind| node.kind != kind) {
                return false;
            }
            true
        })
        .cloned()
        .collect();
    detach_missing_parents(&mut nodes);
    if let Some(max_depth) = selection.depth {
        let by_id: HashMap<&str, &Node> =
            nodes.iter().map(|node| (node.id.as_str(), node)).collect();
        let depths: HashMap<String, usize> = nodes
            .iter()
            .map(|node| (node.id.clone(), node_depth(&node.id, &by_id)))
            .collect();
        nodes.retain(|node| {
            depths
                .get(&node.id)
                .is_some_and(|depth| *depth <= max_depth)
        });
        detach_missing_parents(&mut nodes);
    }
    let assets = document
        .assets
        .iter()
        .filter(|asset| {
            !selection.outline
                && selection.page.is_none_or(|page| {
                    asset.source_anchor.as_ref().and_then(|anchor| anchor.page) == Some(page)
                })
        })
        .cloned()
        .collect();

    Ok(Document {
        id: document.id.clone(),
        format: document.format,
        source: document.source.clone(),
        title: document.title.clone(),
        pages: document.pages,
        nodes,
        assets,
    })
}

fn detach_missing_parents(nodes: &mut [Node]) {
    let selected_ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let detached: HashSet<String> = nodes
        .iter()
        .filter_map(|node| {
            node.parent_id
                .as_deref()
                .filter(|parent| !selected_ids.contains(parent))
                .map(|_| node.id.clone())
        })
        .collect();
    for node in nodes {
        if detached.contains(&node.id) {
            node.parent_id = None;
        }
    }
}

pub(crate) fn render(document: &Document) -> String {
    let page_label = if document.pages == 1 { "page" } else { "pages" };
    let mut output = format!(
        "{} ({}, {} {page_label})\n",
        document.title,
        document.format.as_str().to_uppercase(),
        document.pages
    );
    if document.nodes.is_empty() {
        output.push_str("(no matching nodes)");
        return output;
    }

    let depths = node_depths(document);
    let minimum_depth = depths.values().copied().min().unwrap_or_default();
    for node in &document.nodes {
        let depth = depths
            .get(&node.id)
            .copied()
            .unwrap_or_default()
            .saturating_sub(minimum_depth);
        let content = node
            .title
            .as_deref()
            .or(node.text.as_deref())
            .map(compact_text)
            .unwrap_or_default();
        let level = node
            .level
            .map(|level| format!(" {level}"))
            .unwrap_or_default();
        let page = node
            .source_anchor
            .as_ref()
            .and_then(|anchor| anchor.page)
            .map(|page| format!(" [page {page}]"))
            .unwrap_or_default();
        let separator = if content.is_empty() { "" } else { ": " };
        writeln!(
            output,
            "{}[{}] {}{level}{separator}{content}{page}",
            "  ".repeat(depth),
            node.id,
            node.kind.as_str()
        )
        .unwrap();
    }
    output.pop();
    output
}

fn compact_text(text: &str) -> String {
    const LIMIT: usize = 500;

    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= LIMIT {
        return normalized;
    }
    let mut compact: String = normalized.chars().take(LIMIT).collect();
    compact.push_str("...");
    compact
}

fn is_descendant(node: &Node, root: &str, by_id: &HashMap<&str, &Node>) -> bool {
    let mut parent = node.parent_id.as_deref();
    while let Some(parent_id) = parent {
        if parent_id == root {
            return true;
        }
        parent = by_id
            .get(parent_id)
            .and_then(|parent_node| parent_node.parent_id.as_deref());
    }
    false
}

fn node_depth(id: &str, by_id: &HashMap<&str, &Node>) -> usize {
    let mut depth = 0;
    let mut parent = by_id.get(id).and_then(|node| node.parent_id.as_deref());
    while let Some(parent_id) = parent {
        depth += 1;
        parent = by_id
            .get(parent_id)
            .and_then(|parent_node| parent_node.parent_id.as_deref());
    }
    depth
}

fn node_depths(document: &Document) -> HashMap<String, usize> {
    let by_id: HashMap<&str, &Node> = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    document
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node_depth(&node.id, &by_id)))
        .collect()
}
