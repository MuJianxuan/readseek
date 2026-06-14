use crate::cli::{DefinitionCommand, ReferencesCommand};
use crate::lang::{AnalysisEngine, BinaryMode, Language};
use crate::output::is_identifier_byte;
use crate::paths::{command_paths, definition_candidate_paths};
use crate::source::{
    SourceFile, Symbol, load_source, source_from_text, source_map, symbol_at_line_in_map,
};
use crate::{
    CompactLocation, CompactOutput, DefinitionLocation, DefinitionOutput, ReferenceLocation,
    ReferencesOutput, symbols,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::io::{self, Read as _};
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

pub(crate) fn definition_output(command: &DefinitionCommand) -> Result<DefinitionOutput> {
    let name = definition_name(command)?;
    let search_name = definition_search_name(&name);
    let mut candidates = Vec::new();
    let mut macro_definitions = Vec::new();
    for path in definition_candidate_paths(command, search_name)? {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains(search_name) {
            continue;
        }

        candidates.push((path, text));
    }

    for (path, text) in &candidates {
        if !text
            .lines()
            .any(|line| macro_definition_name(line) == Some(search_name))
        {
            continue;
        }
        let Ok(source) = source_from_text(path, text, command.language, false, None) else {
            continue;
        };
        macro_definitions.extend(macro_definition_locations(&source, search_name));
    }

    if !macro_definitions.is_empty() {
        return Ok(DefinitionOutput {
            definitions: macro_definitions,
        });
    }

    let mut definitions = Vec::new();
    for (path, text) in candidates {
        let Ok(source) = source_from_text(&path, &text, command.language, false, None) else {
            continue;
        };
        let Ok(source_map) = source_map(&source) else {
            continue;
        };
        for symbol in source_map.symbols {
            if symbol.qualified_name != name && symbol.name != search_name {
                continue;
            }
            let line = source
                .lines
                .get(symbol.start_line.saturating_sub(1))
                .context("definition symbol line is out of range")?;
            definitions.push(DefinitionLocation {
                file: source.path.clone(),
                language: source.detection.language,
                engine: source.detection.engine,
                file_hash: source.file_hash.clone(),
                line_hash: line.hash.clone(),
                text: line.text.clone(),
                symbol,
            });
        }
    }

    Ok(DefinitionOutput { definitions })
}

pub(crate) fn compact_definitions(output: &DefinitionOutput) -> CompactOutput {
    CompactOutput {
        locations: output
            .definitions
            .iter()
            .map(|definition| CompactLocation {
                file: definition.file.clone(),
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

fn definition_name(command: &DefinitionCommand) -> Result<String> {
    match (command.name.as_ref(), command.stdin) {
        (Some(name), _) => Ok(name.clone()),
        (None, false) => bail!("definition requires a name or --stdin identify context"),
        (None, true) => definition_name_from_stdin(),
    }
}

fn definition_search_name(name: &str) -> &str {
    name.rsplit('.')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(name)
}

fn macro_definition_locations(source: &SourceFile, name: &str) -> Vec<DefinitionLocation> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }

    source
        .lines
        .iter()
        .filter(|line| macro_definition_name(&line.text) == Some(name))
        .map(|line| DefinitionLocation {
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

fn macro_definition_name(line: &str) -> Option<&str> {
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

fn definition_name_from_stdin() -> Result<String> {
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

pub(crate) fn references_output(command: &ReferencesCommand) -> Result<ReferencesOutput> {
    validate_reference_name(&command.name)?;
    let mut references = Vec::new();
    for path in command_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
    )? {
        let Ok(source) = load_source(&path, command.language, BinaryMode::Reject) else {
            continue;
        };
        references.extend(references_in_source(&source, &command.name));
    }

    Ok(ReferencesOutput { references })
}

pub(crate) fn compact_references(output: &ReferencesOutput) -> CompactOutput {
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

fn validate_reference_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("reference name must not be empty");
    }
    if !name.bytes().all(is_identifier_byte) {
        bail!("reference name must be an ASCII identifier");
    }
    Ok(())
}

fn references_in_source(source: &SourceFile, name: &str) -> Vec<ReferenceLocation> {
    let source_map = source_map(source).ok();
    let ignored_ranges = reference_ignored_ranges(source);
    let line_starts = line_start_offsets(&source.text);
    let mut references = Vec::new();
    for line in &source.lines {
        let columns = reference_columns(&line.text, name);
        if columns.is_empty() {
            continue;
        }
        let symbol = source_map
            .as_ref()
            .and_then(|source_map| symbol_at_line_in_map(source_map, line.number));
        for column in columns {
            let byte_offset = line_starts
                .get(line.number.saturating_sub(1))
                .map_or(column - 1, |line_start| line_start + column - 1);
            if is_ignored_reference(byte_offset, &ignored_ranges) {
                continue;
            }
            references.push(ReferenceLocation {
                file: source.path.clone(),
                language: source.detection.language,
                engine: source.detection.engine,
                file_hash: source.file_hash.clone(),
                line: line.number,
                column,
                line_hash: line.hash.clone(),
                text: line.text.clone(),
                symbol: symbol.clone(),
            });
        }
    }
    references
}

fn reference_ignored_ranges(source: &SourceFile) -> Vec<(usize, usize)> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }
    if source.detection.engine != AnalysisEngine::TreeSitter {
        return Vec::new();
    }
    let Some(language) = symbols::tree_sitter_language(source.detection.language) else {
        return Vec::new();
    };

    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(&source.text, None) else {
        return Vec::new();
    };

    let mut ranges = Vec::new();
    collect_reference_ignored_ranges(tree.root_node(), &mut ranges);
    ranges
}

fn collect_reference_ignored_ranges(node: Node<'_>, ranges: &mut Vec<(usize, usize)>) {
    if is_reference_noise_node(node.kind()) {
        ranges.push((node.start_byte(), node.end_byte()));
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_reference_ignored_ranges(child, ranges);
    }
}

fn is_reference_noise_node(kind: &str) -> bool {
    kind == "comment" || kind.ends_with("string_literal") || kind == "char_literal"
}

fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' && index + 1 < text.len() {
            offsets.push(index + 1);
        }
    }

    offsets
}

fn is_ignored_reference(byte_offset: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|&(start, end)| start <= byte_offset && byte_offset < end)
}

fn reference_columns(text: &str, name: &str) -> Vec<usize> {
    memchr::memmem::find_iter(text.as_bytes(), name.as_bytes())
        .filter(|&index| {
            let bytes = text.as_bytes();
            let before = index.checked_sub(1).map(|before_index| bytes[before_index]);
            let after = bytes.get(index + name.len()).copied();
            !before.is_some_and(is_identifier_byte) && !after.is_some_and(is_identifier_byte)
        })
        .map(|index| index + 1)
        .collect()
}
