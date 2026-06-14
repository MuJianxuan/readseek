// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::{Context, Result, bail};
use argh::FromArgs;
use serde::Serialize;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::{env, process};

use crate::cli::{Cli, DefinitionCommand, ReadCommand, ReferencesCommand, SearchCommand};
use crate::lang::{AnalysisEngine, Language};
use crate::lang::{
    BinaryMode, DocumentKind, analysis_engine, detect_by_path, detect_language, document_extractor,
    document_kind, is_binary_mime, normalize_source_text,
};
use crate::paths::command_paths;

mod cache;
mod cli;
mod hash;
mod lang;
mod navigation;
mod paths;
mod search;
mod symbols;

#[derive(Debug, Serialize)]
struct Detection {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    supported: bool,
    binary: bool,
    mime: Option<String>,
    syntax: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    start_line: usize,
    end_line: usize,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
struct MapOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize)]
struct SymbolOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    symbol: Symbol,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
struct IdentifyOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    line: usize,
    column: usize,
    line_hash: String,
    hashlines: Vec<HashLine>,
    identifier: Option<IdentifierOutput>,
    symbol: Option<Symbol>,
}

#[derive(Debug, Serialize)]
struct IdentifierOutput {
    text: String,
    start_column: usize,
    end_column: usize,
}
#[derive(Debug, Serialize)]
struct DefinitionOutput {
    definitions: Vec<DefinitionLocation>,
}

#[derive(Debug, Serialize)]
struct DefinitionLocation {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    symbol: Symbol,
    #[serde(skip_serializing)]
    line_hash: String,
    #[serde(skip_serializing)]
    text: String,
}

#[derive(Debug, Serialize)]
struct ReferencesOutput {
    references: Vec<ReferenceLocation>,
}

#[derive(Debug, Serialize)]
struct ReferenceLocation {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    line: usize,
    column: usize,
    line_hash: String,
    text: String,
    symbol: Option<Symbol>,
}

#[derive(Debug, Serialize)]
struct CompactOutput {
    locations: Vec<CompactLocation>,
}

#[derive(Debug, Serialize)]
struct CompactLocation {
    file: PathBuf,
    line: usize,
    column: usize,
    line_hash: String,
    text: String,
    kind: Option<String>,
    name: Option<String>,
    qualified_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    results: Vec<SearchFileOutput>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchFileOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    matches: Vec<SearchMatch>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchMatch {
    pattern_index: usize,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
    hashlines: Vec<HashLine>,
    captures: Vec<SearchCapture>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchCapture {
    name: String,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HashLine {
    line: usize,
    hash: String,
    text: String,
}

#[derive(Clone, Debug, Serialize)]
struct Symbol {
    kind: String,
    name: String,
    #[serde(rename = "qualified_name")]
    address: String,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
}

#[derive(Debug)]
pub(crate) struct SourceFile {
    path: PathBuf,
    text: String,
    kind: DocumentKind,
    detection: Detection,
    lines: Vec<SourceLine>,
    file_hash: String,
}

#[derive(Debug)]
struct LoadedDocument {
    text: String,
    binary: bool,
    mime: Option<String>,
}

#[derive(Debug)]
pub(crate) struct SourceLine {
    number: usize,
    text: String,
    hash: String,
}

#[derive(Debug)]
struct SourceMap {
    symbols: Vec<Symbol>,
}

#[derive(Debug)]
enum SymbolLookup {
    Found(Symbol),
    NotFound,
    Ambiguous,
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    path: PathBuf,
    address: Option<TargetAddress>,
}

#[derive(Clone, Debug)]
pub(crate) enum TargetAddress {
    Line(usize),
    Hash(String),
    Symbol(String),
}

fn main() {
    env_logger::init();
    if env::args_os().len() == 1 {
        match Cli::from_args(&["readseek"], &["--help"]) {
            Err(early_exit) => eprintln!("{}", early_exit.output),
            Ok(_) => eprintln!("readseek: help output unavailable"),
        }
        process::exit(2);
    }
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli: crate::cli::Cli = argh::from_env();
    if cli.version {
        println!("readseek {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    match cli.command.context("command required")? {
        crate::cli::Command::Detect(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            print_json(&source.detection)?;
        }
        crate::cli::Command::Read(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Lossy,
            )?;
            let target_line = resolve_target_line(&source, &target)?;
            let (start, end) = resolve_read_range(&command, target_line)?;
            let output = read_output(&source, start, end)?;
            print_json(&output)?;
        }
        crate::cli::Command::Map(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            print_json(&map_output(&source)?)?;
        }
        crate::cli::Command::Symbol(command) => {
            let (target_arg, address_arg) = crate::cli::symbol_args(&command.args, command.stdin)?;
            let target = crate::cli::parse_symbol_input_target(
                target_arg,
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            let target_line = resolve_explicit_target_line(&source, &target, command.line)?;
            let target_address = symbol_address(&target, address_arg)?;
            let output = symbol_command_output(&source, target_address, target_line)?;
            print_json(&output)?;
        }
        crate::cli::Command::Identify(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            let target_line = resolve_explicit_target_line(&source, &target, command.line)?;
            let output = identify_output(&source, target_line, command.column)?;
            print_json(&output)?;
        }
        crate::cli::Command::Definition(command) => {
            print_definition_output(&command)?;
        }
        crate::cli::Command::References(command) => {
            print_references_output(&command)?;
        }
        crate::cli::Command::Search(command) => {
            print_json(&search_output(&command)?)?;
        }
    }

    Ok(())
}

fn print_definition_output(command: &DefinitionCommand) -> Result<()> {
    let output = navigation::definition_output(command)?;
    if command.compact {
        print_json(&navigation::compact_definitions(&output))
    } else {
        print_json(&output)
    }
}

fn print_references_output(command: &ReferencesCommand) -> Result<()> {
    let output = navigation::references_output(command)?;
    if command.compact {
        print_json(&navigation::compact_references(&output))
    } else {
        print_json(&output)
    }
}

fn resolve_target_line(source: &SourceFile, target: &Target) -> Result<Option<usize>> {
    match target.address.as_ref() {
        Some(TargetAddress::Line(line)) => Ok(Some(*line)),
        Some(TargetAddress::Hash(hash)) => source
            .lines
            .iter()
            .find_map(|line| (line.hash == *hash).then_some(line.number))
            .with_context(|| format!("hash {hash} not found in {}", source.path.display()))
            .map(Some),
        None | Some(TargetAddress::Symbol(_)) => Ok(None),
    }
}

fn resolve_explicit_target_line(
    source: &SourceFile,
    target: &Target,
    line: Option<usize>,
) -> Result<Option<usize>> {
    if matches!(target.address, Some(TargetAddress::Symbol(_))) {
        return resolve_target_line(source, target);
    }
    let target_line = resolve_target_line(source, target)?;
    match (target_line, line) {
        (Some(target_line), Some(line)) if target_line != line => {
            bail!("target line conflicts with --line")
        }
        (Some(line), _) | (_, Some(line)) => Ok(Some(line)),
        (None, None) => Ok(None),
    }
}

fn load_source_for_input(
    path: &Path,
    stdin: bool,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    if stdin {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("read stdin")?;
        return source_from_text(
            path,
            normalize_source_text(&text),
            override_language,
            false,
            None,
        );
    }
    load_source(path, override_language, binary_mode)
}

pub(crate) fn load_source(
    path: &Path,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    let document = load_document(path, binary_mode)?;
    source_from_text(
        path,
        document.text,
        override_language,
        document.binary,
        document.mime,
    )
}

fn source_from_text(
    path: &Path,
    text: String,
    override_language: Option<Language>,
    binary: bool,
    mime: Option<String>,
) -> Result<SourceFile> {
    let path_language = detect_by_path(path);
    let (detected_language, syntax) =
        if binary && override_language.is_none() && path_language.is_none() {
            (Language::Unknown, None)
        } else {
            detect_language(path, &text)?
        };
    let language = override_language.unwrap_or(detected_language);
    let engine = analysis_engine(language);
    let kind = document_kind(language);
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, text)| {
            let number = index + 1;
            SourceLine {
                number,
                text: text.to_owned(),
                hash: crate::hash::hash_line(number, text),
            }
        })
        .collect();
    let file_hash = crate::hash::hash_text(&text);
    let detection = Detection {
        file: path.to_path_buf(),
        language,
        engine,
        supported: language != Language::Unknown,
        binary,
        mime,
        syntax,
    };

    Ok(SourceFile {
        path: path.to_path_buf(),
        text,
        kind,
        detection,
        lines,
        file_hash,
    })
}

fn load_document(path: &Path, binary_mode: BinaryMode) -> Result<LoadedDocument> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mime = infer::get(&bytes).map(|kind| kind.mime_type().to_owned());
    let binary = is_binary_mime(mime.as_deref()) || bytes.contains(&0);
    let extractor = document_extractor(path, mime.as_deref());

    if binary && binary_mode == BinaryMode::Reject {
        bail!(
            "unsupported binary file: {} ({})",
            path.display(),
            mime.as_deref().unwrap_or("unknown mime")
        );
    }

    let text = (extractor.extract)(path, &bytes, binary_mode)
        .with_context(|| format!("extract {} from {}", extractor.format.id(), path.display()))?;

    Ok(LoadedDocument { text, binary, mime })
}

fn resolve_read_range(
    command: &ReadCommand,
    target_line: Option<usize>,
) -> Result<(Option<usize>, Option<usize>)> {
    let explicit_start = match (command.start, command.offset) {
        (Some(start), Some(offset)) if start != offset => {
            bail!("--start and --offset specify different start lines")
        }
        (Some(start), _) | (_, Some(start)) => Some(start),
        (None, None) => None,
    };

    let start = match (explicit_start, target_line) {
        (Some(start), Some(line)) if start != line => {
            bail!("target line conflicts with --start/--offset")
        }
        (Some(start), _) | (_, Some(start)) => Some(start),
        (None, None) => None,
    };

    if command.end.is_some() && command.limit.is_some() {
        bail!("cannot combine --end with --limit");
    }

    let end = if let Some(limit) = command.limit {
        if limit == 0 {
            bail!("limit must be greater than zero");
        }
        let start_line = start.unwrap_or(1);
        Some(
            start_line
                .checked_add(limit - 1)
                .context("read range exceeds supported line numbers")?,
        )
    } else {
        command.end
    };

    Ok((start, end))
}

fn read_output(
    source: &SourceFile,
    start: Option<usize>,
    end: Option<usize>,
) -> Result<ReadOutput> {
    let line_count = source.lines.len();
    let start_line = start.unwrap_or(1);
    let requested_end_line = end.unwrap_or(line_count);
    let end_line = requested_end_line.min(line_count);

    if start_line == 0 {
        bail!("start line must be greater than zero");
    }
    if line_count == 0 && start.is_none() && end.is_none() {
        return Ok(ReadOutput {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            line_count,
            file_hash: source.file_hash.clone(),
            start_line,
            end_line,
            hashlines: Vec::new(),
        });
    }
    if requested_end_line < start_line {
        bail!("end line must be greater than or equal to start line");
    }
    if start_line > line_count {
        bail!("start line {start_line} exceeds line count {line_count}");
    }
    let slice_start = start_line - 1;

    let hashlines = source.lines[slice_start..end_line]
        .iter()
        .map(|line| HashLine {
            line: line.number,
            hash: line.hash.clone(),
            text: line.text.clone(),
        })
        .collect();

    Ok(ReadOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count,
        file_hash: source.file_hash.clone(),
        start_line,
        end_line,
        hashlines,
    })
}

fn map_output(source: &SourceFile) -> Result<MapOutput> {
    let source_map = source_map(source)?;

    Ok(MapOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        symbols: source_map.symbols,
    })
}

fn source_map(source: &SourceFile) -> Result<SourceMap> {
    match cache::load_source_map(source) {
        Ok(Some(source_map)) => return Ok(source_map),
        Ok(None) => {}
        Err(error) => log::warn!("cache load error: {error:#}"),
    }

    parse_and_cache_source_map(source)
}

fn parse_and_cache_source_map(source: &SourceFile) -> Result<SourceMap> {
    let source_map = symbols::parse_source_map(source)?;
    if let Err(error) = cache::store_source_map(source, &source_map) {
        log::warn!("cache store error: {error:#}");
    }

    Ok(source_map)
}

fn symbol_address<'a>(target: &'a Target, address: Option<&'a str>) -> Result<Option<&'a str>> {
    match (target.address.as_ref(), address) {
        (Some(TargetAddress::Symbol(_)), Some(_)) => {
            bail!("qualified symbol name specified both in target and as argument")
        }
        (Some(TargetAddress::Symbol(symbol)), None) => Ok(Some(symbol.as_str())),
        (_, address) => Ok(address),
    }
}

fn symbol_output(source: &SourceFile, address: &str) -> Result<SymbolOutput> {
    if let Some(lookup) = cache::symbol_by_address(source, address)? {
        return match lookup {
            SymbolLookup::Found(symbol) => symbol_output_for_symbol(source, symbol),
            SymbolLookup::NotFound => bail!("symbol not found: {address}"),
            SymbolLookup::Ambiguous => bail!("qualified symbol name is ambiguous: {address}"),
        };
    }

    let source_map = parse_and_cache_source_map(source)?;
    let matches = source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.address == address || symbol.name == address)
        .collect::<Vec<_>>();

    let symbol = match matches.as_slice() {
        [] => bail!("symbol not found: {address}"),
        [symbol] => (*symbol).clone(),
        _ => bail!("qualified symbol name is ambiguous: {address}"),
    };

    symbol_output_for_symbol(source, symbol)
}

fn symbol_command_output(
    source: &SourceFile,
    address: Option<&str>,
    target_line: Option<usize>,
) -> Result<SymbolOutput> {
    if let Some(address) = address {
        return symbol_output(source, address);
    }

    let line = target_line.context("symbol requires qualified name or target line/hash")?;
    if let Some(lookup) = cache::symbol_at_line(source, line)? {
        return match lookup {
            SymbolLookup::Found(symbol) => symbol_output_for_symbol(source, symbol),
            SymbolLookup::NotFound => bail!("symbol not found at line {line}"),
            SymbolLookup::Ambiguous => unreachable!("line lookup returns at most one symbol"),
        };
    }

    let source_map = parse_and_cache_source_map(source)?;
    let symbol = symbol_at_line_in_map(&source_map, line)
        .with_context(|| format!("symbol not found at line {line}"))?;
    symbol_output_for_symbol(source, symbol)
}

fn symbol_output_for_symbol(source: &SourceFile, symbol: Symbol) -> Result<SymbolOutput> {
    let read = read_output(source, Some(symbol.start_line), Some(symbol.end_line))?;

    Ok(SymbolOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        symbol,
        hashlines: read.hashlines,
    })
}

fn identify_output(
    source: &SourceFile,
    target_line: Option<usize>,
    column: Option<usize>,
) -> Result<IdentifyOutput> {
    let line = target_line.context("identify requires --line or target line/hash")?;
    let column = column.unwrap_or(1);
    if line == 0 {
        bail!("line must be greater than zero");
    }
    if column == 0 {
        bail!("column must be greater than zero");
    }

    let source_line = source
        .lines
        .get(line - 1)
        .with_context(|| format!("line {line} not found in {}", source.path.display()))?;
    let identifier = identifier_at_column(&source_line.text, column);
    let symbol = symbol_at_line_uncached(source, line)?;

    Ok(IdentifyOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        line,
        column,
        line_hash: source_line.hash.clone(),
        hashlines: vec![HashLine {
            line: source_line.number,
            hash: source_line.hash.clone(),
            text: source_line.text.clone(),
        }],
        identifier,
        symbol,
    })
}

fn identifier_at_column(text: &str, column: usize) -> Option<IdentifierOutput> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = column.saturating_sub(1).min(bytes.len().saturating_sub(1));
    if !is_identifier_byte(bytes[index]) {
        if index > 0 && is_identifier_byte(bytes[index - 1]) {
            index -= 1;
        } else {
            return None;
        }
    }

    let mut start = index;
    while start > 0 && is_identifier_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < bytes.len() && is_identifier_byte(bytes[end]) {
        end += 1;
    }

    Some(IdentifierOutput {
        text: text[start..end].to_owned(),
        start_column: start + 1,
        end_column: end + 1,
    })
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn symbol_at_line_uncached(source: &SourceFile, line: usize) -> Result<Option<Symbol>> {
    let source_map = source_map(source)?;
    Ok(symbol_at_line_in_map(&source_map, line))
}

fn symbol_at_line_in_map(source_map: &SourceMap, line: usize) -> Option<Symbol> {
    source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .min_by_key(|symbol| symbol.end_line - symbol.start_line)
        .cloned()
}
fn search_output(command: &SearchCommand) -> Result<SearchOutput> {
    let paths = command_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
    )?;
    let pattern = crate::search::compile_search(&command.pattern);
    let mut results = Vec::new();

    for path in paths {
        let Some(result) = crate::search::search_file(&path, command.language, &pattern)? else {
            continue;
        };
        if !result.matches.is_empty() {
            results.push(result);
        }
    }

    Ok(SearchOutput { results })
}

pub(crate) fn line_hash(source: &SourceFile, line: usize) -> Result<String> {
    source
        .lines
        .get(line.saturating_sub(1))
        .map(|line| line.hash.clone())
        .with_context(|| format!("line {line} not found in {}", source.path.display()))
}

pub(crate) fn range_hashlines(
    source: &SourceFile,
    start_line: usize,
    end_line: usize,
) -> Vec<HashLine> {
    let start = start_line.saturating_sub(1);
    let end = end_line.min(source.lines.len());
    source.lines[start..end]
        .iter()
        .map(|line| HashLine {
            line: line.number,
            hash: line.hash.clone(),
            text: line.text.clone(),
        })
        .collect()
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
