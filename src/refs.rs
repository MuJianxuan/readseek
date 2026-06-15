use crate::cli::RefsCommand;
use crate::flags::GitFlags;
use crate::lang::{AnalysisEngine, Language};
use crate::output::is_identifier_byte;
use crate::output::{CompactLocation, CompactOutput, RefLocation, RefsOutput};
use crate::paths::{bytes_contain_identifier, command_paths};
use crate::source::{SourceFile, Symbol, find_symbol, source_from_text, source_map_with_dir};
use crate::symbols;
use anyhow::{Result, bail};
use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tree_sitter::{Node, Parser};

pub(crate) fn output(command: &RefsCommand) -> Result<RefsOutput> {
    let name = &command.name;
    if name.is_empty() {
        bail!("reference name must not be empty");
    }
    if !name.bytes().all(is_identifier_byte) {
        bail!("reference name must be an ASCII identifier");
    }
    let readseek_dir = crate::repo::find_readseek_dir(&command.target);
    let paths = command_paths(
        &command.target,
        GitFlags {
            cached: command.cached,
            others: command.others,
            ignored: command.ignored,
        },
    )?;

    let references: Vec<RefLocation> = paths
        .par_iter()
        .flat_map(|path| {
            let Ok(bytes) = fs::read(path) else {
                return vec![];
            };
            if !bytes_contain_identifier(&bytes, name.as_bytes()) {
                return vec![];
            }
            let Ok(text) = String::from_utf8(bytes) else {
                return vec![];
            };
            let Ok(source) = source_from_text(path, &text, command.language, false, None) else {
                return vec![];
            };
            let needs_parser = matches!(
                source.detection.language,
                crate::lang::Language::C | crate::lang::Language::Cpp
            );
            let mut parser = if needs_parser {
                Some(tree_sitter::Parser::new())
            } else {
                None
            };
            scan_source(&source, name, parser.as_mut(), readseek_dir.as_deref())
        })
        .collect();

    Ok(RefsOutput { references })
}

pub(crate) fn compact(output: &RefsOutput) -> CompactOutput {
    CompactOutput {
        locations: output
            .references
            .iter()
            .map(|reference| {
                let symbol = reference.symbol.as_ref();
                CompactLocation {
                    file: reference.file.clone(),
                    line: reference.line,
                    column: reference.column,
                    line_hash: reference.line_hash.clone(),
                    text: reference.text.clone(),
                    kind: symbol.map(|symbol| symbol.kind.clone()),
                    name: symbol.map(|symbol| symbol.name.clone()),
                    qualified_name: symbol.map(|symbol| symbol.qualified_name.clone()),
                }
            })
            .collect(),
    }
}

fn scan_source(
    source: &SourceFile,
    name: &str,
    parser: Option<&mut Parser>,
    readseek_dir: Option<&Path>,
) -> Vec<RefLocation> {
    let source_map = source_map_with_dir(source, readseek_dir).ok();
    let ignored_ranges = parser
        .map(|p| scan_ignored_ranges(source, p))
        .unwrap_or_default();
    let line_starts = &source.line_starts;

    let text_bytes = source.text.as_bytes();
    let name_bytes = name.as_bytes();

    let mut compact: Vec<(usize, usize)> = Vec::new();
    for byte_index in memchr::memmem::find_iter(text_bytes, name_bytes) {
        let before = byte_index.checked_sub(1).map(|i| text_bytes[i]);
        let after = text_bytes.get(byte_index + name.len()).copied();
        if before.is_some_and(is_identifier_byte) || after.is_some_and(is_identifier_byte) {
            continue;
        }
        let line_idx = line_starts
            .partition_point(|&start| start <= byte_index)
            .saturating_sub(1);
        let Some(line) = source.lines.get(line_idx) else {
            continue;
        };
        let index = ignored_ranges.partition_point(|&(start, _)| start <= byte_index);
        if index > 0 && byte_index < ignored_ranges[index - 1].1 {
            continue;
        }
        compact.push((line.number, byte_index - line_starts[line_idx] + 1));
    }

    if compact.is_empty() {
        return Vec::new();
    }

    let file = Arc::new(source.path.clone());
    let file_hash: Arc<str> = Arc::from(source.file_hash.as_str());
    let language = source.detection.language;
    let engine = source.detection.engine;

    let mut references = Vec::with_capacity(compact.len());
    let mut last_line = 0;
    let mut cached_line_hash = String::new();
    let mut cached_text = String::new();
    let mut cached_symbol: Option<Symbol> = None;

    for (line_number, column) in compact {
        if line_number != last_line {
            let line = &source.lines[line_number - 1];
            last_line = line_number;
            cached_line_hash.clone_from(&line.hash);
            cached_text.clone_from(&line.text);
            cached_symbol = source_map
                .as_ref()
                .and_then(|sm| find_symbol(sm, line_number));
        }

        references.push(RefLocation {
            file: Arc::clone(&file),
            language,
            engine,
            file_hash: Arc::clone(&file_hash),
            line: line_number,
            column,
            line_hash: cached_line_hash.clone(),
            text: cached_text.clone(),
            symbol: cached_symbol.clone(),
        });
    }

    references
}

fn scan_ignored_ranges(source: &SourceFile, parser: &mut Parser) -> Vec<(usize, usize)> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }
    if source.detection.engine.0 != Some(AnalysisEngine::TreeSitter) {
        return Vec::new();
    }
    let Some(language) = symbols::tree_sitter_language(source.detection.language) else {
        return Vec::new();
    };

    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(&source.text, None) else {
        return Vec::new();
    };

    let mut ranges = Vec::new();
    collect_ignored_ranges(tree.root_node(), &mut ranges);
    ranges
}

fn collect_ignored_ranges(node: Node<'_>, ranges: &mut Vec<(usize, usize)>) {
    if node.kind() == "comment"
        || node.kind().ends_with("string_literal")
        || node.kind() == "char_literal"
    {
        ranges.push((node.start_byte(), node.end_byte()));
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ignored_ranges(child, ranges);
    }
}
