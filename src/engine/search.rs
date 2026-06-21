// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::lang::{AnalysisEngine, BinaryMode, Language};
use crate::engine::output::{SearchCapture, SearchFileOutput, SearchMatch};
use crate::engine::source::{SourceFile, line_hash, load_source, range_hashlines};
use crate::engine::symbols;
use anyhow::{Context, Result, bail};
use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PatternMetaKind {
    Single,
    Variadic,
}

#[derive(Clone, Debug)]
pub(crate) struct PatternMeta {
    pub(crate) placeholder: String,
    pub(crate) name: String,
    pub(crate) kind: PatternMetaKind,
}

#[derive(Debug)]
pub(crate) struct SearchPattern {
    pub(crate) text: String,
    pub(crate) metas: Vec<PatternMeta>,
    pub(crate) tree: Option<Tree>,
}

struct SearchCaptureRange<'a> {
    name: &'a str,
    text: &'a str,
    start_line: usize,
    end_line: usize,
}

/// Search a single file for AST pattern matches.
pub(crate) fn search_file(
    path: &Path,
    override_language: Option<Language>,
    pattern: &SearchPattern,
    parser: &mut Parser,
) -> Result<Option<SearchFileOutput>> {
    let Ok(source) = load_source(path, override_language, BinaryMode::Reject) else {
        return Ok(None);
    };
    let detected_language = source.detection.language;
    if source.detection.engine.0 != Some(AnalysisEngine::TreeSitter) {
        return Ok(None);
    }
    let Some(language) = symbols::tree_sitter_language(detected_language) else {
        return Ok(None);
    };
    parser
        .set_language(&language)
        .map_err(|error| anyhow::anyhow!("set tree-sitter language: {error}"))?;
    let tree = parser
        .parse(&source.text, None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter parse failed"))?;

    let owned_pattern_tree;
    let pattern_tree_ref = if let Some(pre_parsed) = &pattern.tree {
        if pattern_tree_has_error(pre_parsed) {
            bail!("pattern is not valid {} syntax", detected_language.id());
        }
        pre_parsed
    } else {
        owned_pattern_tree = parser
            .parse(&pattern.text, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter pattern parse failed"))?;
        if pattern_tree_has_error(&owned_pattern_tree) {
            bail!("pattern is not valid {} syntax", detected_language.id());
        }
        &owned_pattern_tree
    };
    if pattern_tree_ref.root_node().named_child_count() == 0 {
        bail!("pattern is not valid {} syntax", detected_language.id());
    }
    let pattern_root = if pattern_tree_ref.root_node().named_child_count() == 1 {
        pattern_tree_ref.root_node().named_child(0)
    } else {
        Some(pattern_tree_ref.root_node())
    }
    .context("empty search pattern")?;

    let mut matches = Vec::new();
    collect_search_matches(
        &source,
        pattern,
        pattern_root,
        tree.root_node(),
        &mut matches,
    )?;

    Ok(Some(SearchFileOutput {
        file: source.path,
        language: detected_language,
        engine: source.detection.engine,
        file_hash: source.file_hash,
        matches,
    }))
}

/// Compile an ast-grep-style pattern string into a `SearchPattern`.
pub(crate) fn compile_search(pattern: &str) -> SearchPattern {
    let mut text = String::with_capacity(pattern.len());
    let mut metas = Vec::new();
    let bytes = pattern.as_bytes();
    let mut index = 0;

    while index < pattern.len() {
        let rest = &pattern[index..];
        if !rest.starts_with('$') {
            let Some(ch) = rest.chars().next() else {
                break;
            };
            text.push(ch);
            index += ch.len_utf8();
            continue;
        }

        let (kind, name_start) = if rest.starts_with("$$$") {
            (PatternMetaKind::Variadic, index + 3)
        } else {
            (PatternMetaKind::Single, index + 1)
        };
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
        {
            name_end += 1;
        }
        if name_end == name_start {
            text.push('$');
            index += 1;
            continue;
        }

        let name = &pattern[name_start..name_end];
        let placeholder = match kind {
            PatternMetaKind::Single => format!("__readseek_meta_{name}"),
            PatternMetaKind::Variadic => format!("__readseek_variadic_{name}"),
        };
        text.push_str(&placeholder);
        metas.push(PatternMeta {
            placeholder,
            name: name.to_owned(),
            kind,
        });
        index = name_end;
    }

    SearchPattern {
        text,
        metas,
        tree: None,
    }
}

/// Pre-compile the pattern text into a tree-sitter tree for the given language.
///
/// This allows the pattern tree to be reused across multiple files of the same
/// language, avoiding redundant parsing.
pub(crate) fn prepare_tree(pattern: &mut SearchPattern, language: &tree_sitter::Language) {
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return;
    }
    pattern.tree = parser.parse(&pattern.text, None);
}

fn collect_search_matches<'a>(
    source: &'a SourceFile,
    pattern: &'a SearchPattern,
    pattern_node: Node<'_>,
    source_node: Node<'_>,
    matches: &mut Vec<SearchMatch>,
) -> Result<()> {
    let mut captures = Vec::new();
    if nodes_match(source, pattern, pattern_node, source_node, &mut captures) {
        matches.push(search_match(source, source_node, captures)?);
    }

    let mut cursor = source_node.walk();
    for child in source_node.named_children(&mut cursor) {
        collect_search_matches(source, pattern, pattern_node, child, matches)?;
    }

    Ok(())
}

fn nodes_match<'a>(
    source: &'a SourceFile,
    pattern: &'a SearchPattern,
    pattern_node: Node<'_>,
    source_node: Node<'_>,
    captures: &mut Vec<SearchCaptureRange<'a>>,
) -> bool {
    if let Some(meta) = pattern_meta(pattern, pattern_node) {
        if meta.kind == PatternMetaKind::Single {
            let (start_line, end_line) = symbols::node_line_range(source_node);
            let Some(text) = node_text(source_node, &source.text) else {
                return false;
            };
            return bind_capture(captures, &meta.name, text, start_line, end_line);
        }
        return true;
    }

    if pattern_node.kind() != source_node.kind() {
        return false;
    }

    let pattern_children = named_children(pattern_node);
    let source_children = named_children(source_node);
    if pattern_children.is_empty() {
        return node_text(pattern_node, &pattern.text) == node_text(source_node, &source.text);
    }

    child_nodes_match(
        source,
        pattern,
        &pattern_children,
        &source_children,
        0,
        0,
        captures,
    )
}

fn child_nodes_match<'a>(
    source: &'a SourceFile,
    pattern: &'a SearchPattern,
    pattern_children: &[Node<'_>],
    source_children: &[Node<'_>],
    pattern_index: usize,
    source_index: usize,
    captures: &mut Vec<SearchCaptureRange<'a>>,
) -> bool {
    if pattern_index == pattern_children.len() {
        return source_index == source_children.len();
    }

    let pattern_child = pattern_children[pattern_index];
    if let Some(meta) = pattern_meta(pattern, pattern_child) {
        if meta.kind == PatternMetaKind::Variadic {
            for count in 0..=source_children.len().saturating_sub(source_index) {
                let snapshot = captures.len();
                if count > 0 {
                    let start_node = source_children[source_index];
                    let end_node = source_children[source_index + count - 1];
                    let (start_line, _) = symbols::node_line_range(start_node);
                    let (_, end_line) = symbols::node_line_range(end_node);
                    let Some(text) = source
                        .text
                        .get(start_node.start_byte()..end_node.end_byte())
                    else {
                        continue;
                    };
                    if !bind_capture(captures, &meta.name, text, start_line, end_line) {
                        continue;
                    }
                }
                if child_nodes_match(
                    source,
                    pattern,
                    pattern_children,
                    source_children,
                    pattern_index + 1,
                    source_index + count,
                    captures,
                ) {
                    return true;
                }
                captures.truncate(snapshot);
            }
            return false;
        }
    }

    if source_index >= source_children.len() {
        return false;
    }

    let snapshot = captures.len();
    if !nodes_match(
        source,
        pattern,
        pattern_child,
        source_children[source_index],
        captures,
    ) {
        return false;
    }
    if child_nodes_match(
        source,
        pattern,
        pattern_children,
        source_children,
        pattern_index + 1,
        source_index + 1,
        captures,
    ) {
        return true;
    }
    captures.truncate(snapshot);
    false
}

fn bind_capture<'a>(
    captures: &mut Vec<SearchCaptureRange<'a>>,
    name: &'a str,
    text: &'a str,
    start_line: usize,
    end_line: usize,
) -> bool {
    if captures
        .iter()
        .any(|capture| capture.name == name && capture.text != text)
    {
        return false;
    }

    captures.push(SearchCaptureRange {
        name,
        text,
        start_line,
        end_line,
    });
    true
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn pattern_meta<'a>(pattern: &'a SearchPattern, node: Node<'_>) -> Option<&'a PatternMeta> {
    let text = node_text(node, &pattern.text)?;
    pattern.metas.iter().find(|meta| meta.placeholder == text)
}

fn node_text<'a>(node: Node<'_>, text: &'a str) -> Option<&'a str> {
    node.utf8_text(text.as_bytes()).ok()
}

fn pattern_tree_has_error(tree: &Tree) -> bool {
    node_has_error(tree.root_node())
}

fn node_has_error(node: Node<'_>) -> bool {
    if node.has_error() || node.is_error() || node.is_missing() {
        return true;
    }

    let mut cursor = node.walk();
    node.children(&mut cursor).any(node_has_error)
}

fn search_match(
    source: &SourceFile,
    node: Node<'_>,
    capture_ranges: Vec<SearchCaptureRange<'_>>,
) -> Result<SearchMatch> {
    let captures = capture_ranges
        .into_iter()
        .map(|capture| {
            Ok(SearchCapture {
                name: capture.name.to_owned(),
                start_line: capture.start_line,
                end_line: capture.end_line,
                start_hash: line_hash(source, capture.start_line)?,
                end_hash: line_hash(source, capture.end_line)?,
                hashlines: range_hashlines(source, capture.start_line, capture.end_line),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let (start_line, end_line) = symbols::node_line_range(node);

    Ok(SearchMatch {
        start_line,
        end_line,
        start_hash: line_hash(source, start_line)?,
        end_hash: line_hash(source, end_line)?,
        hashlines: range_hashlines(source, start_line, end_line),
        captures,
    })
}
