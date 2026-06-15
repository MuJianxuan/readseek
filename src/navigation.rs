use crate::cli::{DefCommand, RefsCommand};
use crate::flags::GitFlags;
use crate::lang::{AnalysisEngine, Language};
use crate::output::is_identifier_byte;
use crate::output::{
    CompactLocation, CompactOutput, DefLocation, DefOutput, RefLocation, RefsOutput,
};
use crate::paths::{command_paths, contains_identifier, def_candidate_paths};
use crate::source::{SourceFile, Symbol, find_symbol, source_from_text, source_map_with_dir};
use crate::symbols;
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read as _};
use std::path::Path;
use std::sync::Arc;
use tree_sitter::{Node, Parser};

#[derive(Debug, Deserialize)]
struct IdentifyInput {
    identifier: Option<IdentifierInput>,
    symbol: Option<SymbolInput>,
}

#[derive(Debug, Deserialize)]
struct IdentifierInput {
    text: String,
}

#[derive(Debug, Deserialize)]
struct SymbolInput {
    qualified_name: String,
}

pub(crate) fn def_output(command: &DefCommand) -> Result<DefOutput> {
    let name = def_name(command)?;
    let search_name = def_search_name(&name);
    let readseek_dir = crate::repo::find_readseek_dir(&command.target);
    let results = if let Some(readseek_dir) = readseek_dir.as_deref() {
        if let Some(index_entries) = crate::repo::load_def_index(readseek_dir, &name)? {
            if index_entries.is_empty() {
                def_candidate_paths(command, search_name)?
                    .par_iter()
                    .map(|path| {
                        def_locations_in_path(
                            path,
                            &name,
                            search_name,
                            command.language,
                            Some(readseek_dir),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                index_entries
                    .par_iter()
                    .map(|entry| {
                        def_locations_in_path(
                            &entry.path,
                            &name,
                            search_name,
                            command.language,
                            Some(readseek_dir),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        } else {
            def_candidate_paths(command, search_name)?
                .par_iter()
                .map(|path| {
                    def_locations_in_path(
                        path,
                        &name,
                        search_name,
                        command.language,
                        Some(readseek_dir),
                    )
                })
                .collect::<Result<Vec<_>>>()?
        }
    } else {
        def_candidate_paths(command, search_name)?
            .par_iter()
            .map(|path| def_locations_in_path(path, &name, search_name, command.language, None))
            .collect::<Result<Vec<_>>>()?
    };
    let mut seen = BTreeSet::new();
    let mut definitions = Vec::new();

    for definition in results.into_iter().flatten() {
        let key = (
            definition.file.clone(),
            definition.symbol.kind.clone(),
            definition.symbol.name.clone(),
            definition.symbol.start_line,
            definition.symbol.end_line,
        );
        if seen.insert(key) {
            definitions.push(definition);
        }
    }

    Ok(DefOutput { definitions })
}

fn def_locations_in_path(
    path: &Path,
    name: &str,
    search_name: &str,
    language: Option<Language>,
    readseek_dir: Option<&Path>,
) -> Result<Vec<DefLocation>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    if !contains_identifier(&text, search_name) {
        return Ok(Vec::new());
    }
    let Ok(source) = source_from_text(path, &text, language, false, None) else {
        return Ok(Vec::new());
    };
    let mut definitions = macro_def_locations(&source, search_name);

    let Ok(source_map) = source_map_with_dir(&source, readseek_dir) else {
        return Ok(definitions);
    };
    for symbol in source_map.symbols {
        if symbol.qualified_name != name && symbol.name != search_name && symbol.name != name {
            continue;
        }
        let line = source
            .line(symbol.start_line)
            .context("definition symbol line is out of range")?;
        definitions.push(DefLocation {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            file_hash: source.file_hash.clone(),
            line_hash: line.hash.clone(),
            text: line.text.clone(),
            symbol,
        });
    }

    Ok(definitions)
}

pub(crate) fn compact_defs(output: &DefOutput) -> CompactOutput {
    CompactOutput {
        locations: output
            .definitions
            .iter()
            .map(|definition| CompactLocation {
                file: Arc::new(definition.file.clone()),
                line: definition.symbol.start_line,
                column: 1,
                line_hash: definition.line_hash.clone(),
                text: definition.text.clone(),
                kind: Some(definition.symbol.kind.clone()),
                name: Some(definition.symbol.name.clone()),
                qualified_name: Some(definition.symbol.qualified_name.clone()),
            })
            .collect(),
    }
}

fn def_name(command: &DefCommand) -> Result<String> {
    match (command.name.as_ref(), command.stdin) {
        (Some(name), _) => Ok(name.clone()),
        (None, false) => bail!("definition requires a name or --stdin identify context"),
        (None, true) => def_name_from_stdin(),
    }
}

fn def_search_name(name: &str) -> &str {
    name.rsplit('.')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(name)
}

fn macro_def_locations(source: &SourceFile, name: &str) -> Vec<DefLocation> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }

    source
        .lines
        .iter()
        .filter(|line| macro_def_name(&line.text) == Some(name))
        .map(|line| DefLocation {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            file_hash: source.file_hash.clone(),
            symbol: Symbol {
                kind: "macro".to_owned(),
                name: name.to_owned(),
                qualified_name: name.to_owned(),
                start_line: line.number,
                end_line: line.number,
                start_hash: line.hash.clone(),
                end_hash: line.hash.clone(),
            },
            line_hash: line.hash.clone(),
            text: line.text.clone(),
        })
        .collect()
}

fn macro_def_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("#define")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }

    let rest = rest.trim_start();
    let name_len = rest
        .find(|ch: char| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_'))
        .unwrap_or(rest.len());
    if name_len == 0 {
        return None;
    }

    Some(&rest[..name_len])
}

fn def_name_from_stdin() -> Result<String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .context("read identify context from stdin")?;
    let input: IdentifyInput = serde_json::from_str(&text).context("parse identify context")?;
    if let Some(identifier) = input.identifier {
        return Ok(identifier.text);
    }
    if let Some(symbol) = input.symbol {
        return Ok(symbol.qualified_name);
    }
    bail!("identify context has no symbol or identifier")
}

pub(crate) fn refs_output(command: &RefsCommand) -> Result<RefsOutput> {
    validate_ref_name(&command.name)?;
    let name = &command.name;
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
            let Ok(text) = fs::read_to_string(path) else {
                return vec![];
            };
            if !contains_identifier(&text, name) {
                return vec![];
            }
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
            refs_in_source(&source, name, parser.as_mut(), readseek_dir.as_deref())
        })
        .collect();

    Ok(RefsOutput { references })
}

pub(crate) fn compact_refs(output: &RefsOutput) -> CompactOutput {
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

fn validate_ref_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("reference name must not be empty");
    }
    if !name.bytes().all(is_identifier_byte) {
        bail!("reference name must be an ASCII identifier");
    }
    Ok(())
}

fn refs_in_source(
    source: &SourceFile,
    name: &str,
    parser: Option<&mut Parser>,
    readseek_dir: Option<&Path>,
) -> Vec<RefLocation> {
    let source_map = source_map_with_dir(source, readseek_dir).ok();
    let ignored_ranges = parser
        .map(|p| ref_ignored_ranges(source, p))
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
        if is_ignored_ref(byte_index, &ignored_ranges) {
            continue;
        }
        compact.push((line.number, byte_index - line_starts[line_idx] + 1));
    }

    if compact.is_empty() {
        return Vec::new();
    }

    compact.sort_unstable_by_key(|(l, _)| *l);

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

fn ref_ignored_ranges(source: &SourceFile, parser: &mut Parser) -> Vec<(usize, usize)> {
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
    collect_ref_ignored_ranges(tree.root_node(), &mut ranges);
    ranges.sort_unstable_by_key(|&(start, _)| start);
    ranges
}

fn collect_ref_ignored_ranges(node: Node<'_>, ranges: &mut Vec<(usize, usize)>) {
    if is_ref_noise_node(node.kind()) {
        ranges.push((node.start_byte(), node.end_byte()));
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ref_ignored_ranges(child, ranges);
    }
}

fn is_ref_noise_node(kind: &str) -> bool {
    kind == "comment" || kind.ends_with("string_literal") || kind == "char_literal"
}

fn is_ignored_ref(byte_offset: usize, ranges: &[(usize, usize)]) -> bool {
    let index = ranges.partition_point(|&(start, _)| start <= byte_offset);
    index > 0 && byte_offset < ranges[index - 1].1
}
