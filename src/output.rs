use crate::cli::ReadCommand;
use crate::lang::{AnalysisEngine, BinaryMode, Language};
use crate::source::{
    HashLine, SourceFile, Symbol, load_source, range_hashlines, source_from_text, source_map,
    symbol_at_line_in_map, symbol_at_line_uncached,
};
use crate::{Target, TargetAddress};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct ReadOutput {
    file: PathBuf,
    language: Language,
    #[serde(serialize_with = "crate::lang::serialize_engine")]
    engine: Option<AnalysisEngine>,
    line_count: usize,
    file_hash: String,
    start_line: usize,
    end_line: usize,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MapOutput {
    file: PathBuf,
    language: Language,
    #[serde(serialize_with = "crate::lang::serialize_engine")]
    engine: Option<AnalysisEngine>,
    line_count: usize,
    file_hash: String,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SymbolOutput {
    file: PathBuf,
    language: Language,
    #[serde(serialize_with = "crate::lang::serialize_engine")]
    engine: Option<AnalysisEngine>,
    line_count: usize,
    file_hash: String,
    symbol: Symbol,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
pub(crate) struct IdentifyOutput {
    file: PathBuf,
    language: Language,
    #[serde(serialize_with = "crate::lang::serialize_engine")]
    engine: Option<AnalysisEngine>,
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
pub(crate) struct IdentifierOutput {
    text: String,
    start_column: usize,
    end_column: usize,
}

pub(crate) fn resolve_target_line(source: &SourceFile, target: &Target) -> Result<Option<usize>> {
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

pub(crate) fn resolve_explicit_target_line(
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

pub(crate) fn load_source_for_input(
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
        return source_from_text(path, &text, override_language, false, None);
    }
    load_source(path, override_language, binary_mode)
}

pub(crate) fn resolve_read_range(
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

pub(crate) fn read_output(
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

pub(crate) fn map_output(source: &SourceFile) -> Result<MapOutput> {
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

pub(crate) fn symbol_address<'a>(
    target: &'a Target,
    address: Option<&'a str>,
) -> Result<Option<&'a str>> {
    match (target.address.as_ref(), address) {
        (Some(TargetAddress::Symbol(_)), Some(_)) => {
            bail!("qualified symbol name specified both in target and as argument")
        }
        (Some(TargetAddress::Symbol(symbol)), None) => Ok(Some(symbol.as_str())),
        (_, address) => Ok(address),
    }
}

fn symbol_output(source: &SourceFile, address: &str) -> Result<SymbolOutput> {
    let source_map = source_map(source)?;
    let matches = source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.qualified_name == address || symbol.name == address)
        .collect::<Vec<_>>();

    let symbol = match matches.as_slice() {
        [] => bail!("symbol not found: {address}"),
        [symbol] => (*symbol).clone(),
        _ => bail!("qualified symbol name is ambiguous: {address}"),
    };

    Ok(SymbolOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        hashlines: range_hashlines(source, symbol.start_line, symbol.end_line),
        symbol,
    })
}

pub(crate) fn symbol_command_output(
    source: &SourceFile,
    address: Option<&str>,
    target_line: Option<usize>,
) -> Result<SymbolOutput> {
    if let Some(address) = address {
        return symbol_output(source, address);
    }

    let line = target_line.context("symbol requires qualified name or target line/hash")?;
    let source_map = source_map(source)?;
    let symbol = symbol_at_line_in_map(&source_map, line)
        .with_context(|| format!("symbol not found at line {line}"))?;
    Ok(SymbolOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        hashlines: range_hashlines(source, symbol.start_line, symbol.end_line),
        symbol,
    })
}

pub(crate) fn identify_output(
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

pub(crate) fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
