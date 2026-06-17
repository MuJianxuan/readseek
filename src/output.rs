use crate::lang::{BinaryMode, EngineField, Language};
use crate::source::{
    HashLine, SourceFile, Symbol, find_symbol, load_source, range_hashlines, source_from_text,
    source_map,
};
use crate::target::{Target, TargetAddress};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum Format {
    #[default]
    Json,
    Plain,
}

impl argh::FromArgValue for Format {
    fn from_arg_value(value: &str) -> Result<Self, String> {
        match value {
            "json" => Ok(Self::Json),
            "plain" => Ok(Self::Plain),
            _ => Err(format!("invalid format '{value}': expected json or plain")),
        }
    }
}
#[derive(Debug, Serialize)]
pub(crate) struct ReadOutput {
    file: PathBuf,
    language: Language,
    engine: EngineField,
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
    engine: EngineField,
    line_count: usize,
    file_hash: String,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SymbolOutput {
    file: PathBuf,
    language: Language,
    engine: EngineField,
    line_count: usize,
    file_hash: String,
    symbol: Symbol,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
pub(crate) struct IdentifyOutput {
    file: PathBuf,
    language: Language,
    engine: EngineField,
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
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct DefOutput {
    pub(crate) definitions: Vec<DefLocation>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DefLocation {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    pub(crate) engine: EngineField,
    pub(crate) file_hash: String,
    pub(crate) symbol: Symbol,
    #[serde(skip_serializing)]
    pub(crate) line_hash: String,
    #[serde(skip_serializing)]
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefsOutput {
    pub(crate) references: Vec<RefLocation>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefLocation {
    pub(crate) file: Arc<PathBuf>,
    pub(crate) language: Language,
    pub(crate) engine: EngineField,
    pub(crate) file_hash: Arc<str>,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) line_hash: String,
    pub(crate) text: String,
    pub(crate) symbol: Option<Symbol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurrence: Option<crate::binding::OccurrenceKind>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CompactOutput {
    pub(crate) locations: Vec<CompactLocation>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CompactLocation {
    pub(crate) file: Arc<PathBuf>,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) line_hash: String,
    pub(crate) text: String,
    pub(crate) kind: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) qualified_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RenameOutput {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    pub(crate) engine: EngineField,
    pub(crate) file_hash: String,
    pub(crate) old_name: String,
    pub(crate) new_name: String,
    pub(crate) applied: bool,
    pub(crate) unsupported: bool,
    pub(crate) conflicts: Vec<RenameConflict>,
    pub(crate) edits: Vec<RenameEdit>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RenameEdit {
    pub(crate) line: usize,
    pub(crate) start_column: usize,
    pub(crate) end_column: usize,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) occurrence: crate::binding::OccurrenceKind,
    pub(crate) line_hash: String,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RenameConflict {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchOutput {
    pub(crate) results: Vec<SearchFileOutput>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchFileOutput {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    pub(crate) engine: EngineField,
    pub(crate) file_hash: String,
    pub(crate) matches: Vec<SearchMatch>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchMatch {
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) start_hash: String,
    pub(crate) end_hash: String,
    pub(crate) hashlines: Vec<HashLine>,
    pub(crate) captures: Vec<SearchCapture>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchCapture {
    pub(crate) name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) start_hash: String,
    pub(crate) end_hash: String,
    pub(crate) hashlines: Vec<HashLine>,
}

pub(crate) fn resolve_target(source: &SourceFile, target: &Target) -> Result<Option<usize>> {
    match target.address.as_ref() {
        Some(TargetAddress::Line(line)) => Ok(Some(*line)),
        Some(TargetAddress::Hash(hash)) => source
            .lines
            .iter()
            .find_map(|line| (line.hash() == *hash).then_some(line.number))
            .with_context(|| format!("hash {hash} not found in {}", source.path.display()))
            .map(Some),
        None => Ok(None),
    }
}

pub(crate) fn resolve_explicit_target(
    source: &SourceFile,
    target: &Target,
    line: Option<usize>,
) -> Result<Option<usize>> {
    let target_line = resolve_target(source, target)?;
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
    stdin: Option<&PathBuf>,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    if let Some(stdin_path) = stdin {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("read stdin")?;
        return source_from_text(stdin_path, text, override_language, false, None);
    }
    load_source(path, override_language, binary_mode)
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
            hash: line.hash(),
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

pub(crate) fn symbol_output(
    source: &SourceFile,
    address: Option<&str>,
    target_line: Option<usize>,
) -> Result<SymbolOutput> {
    if let Some(address) = address {
        let source_map = source_map(source)?;
        let mut matches = source_map
            .symbols
            .iter()
            .filter(|symbol| symbol.qualified_name == address || symbol.name == address);

        let symbol = matches
            .next()
            .with_context(|| format!("symbol not found: {address}"))?;

        if matches.next().is_some() {
            bail!("qualified symbol name is ambiguous: {address}");
        }

        let symbol = symbol.clone();

        return Ok(SymbolOutput {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            line_count: source.lines.len(),
            file_hash: source.file_hash.clone(),
            hashlines: range_hashlines(source, symbol.start_line, symbol.end_line),
            symbol,
        });
    }

    let line = target_line.context("symbol requires qualified name or target line/hash")?;
    let source_map = source_map(source)?;
    let symbol = find_symbol(&source_map, line)
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
        .line(line)
        .with_context(|| format!("line {line} not found in {}", source.path.display()))?;
    let line_start = source.line_starts[line - 1];
    let cursor_byte = line_start + column.saturating_sub(1).min(source_line.text.len());
    let identifier = crate::symbols::token_at(source, cursor_byte)
        .map(|token| IdentifierOutput {
            text: token.text,
            start_column: token.start_byte - line_start + 1,
            end_column: token.end_byte - line_start + 1,
            start_byte: token.start_byte,
            end_byte: token.end_byte,
        })
        .or_else(|| identify_byte_scan(source_line, line_start, column));
    let source_map = source_map(source)?;
    let symbol = find_symbol(&source_map, line);

    Ok(IdentifyOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        line,
        column,
        line_hash: source_line.hash(),
        hashlines: vec![HashLine {
            line: source_line.number,
            hash: source_line.hash(),
            text: source_line.text.clone(),
        }],
        identifier,
        symbol,
    })
}

pub(crate) fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Fallback identifier extraction for languages without a tree-sitter parser.
///
/// Walks ASCII identifier bytes around the cursor on a single line. `line_start`
/// is the byte offset of the line within the file, used to report absolute bytes.
fn identify_byte_scan(
    source_line: &crate::source::SourceLine,
    line_start: usize,
    column: usize,
) -> Option<IdentifierOutput> {
    let bytes = source_line.text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = column.saturating_sub(1).min(bytes.len().saturating_sub(1));
    if !is_identifier_byte(bytes[index]) && index > 0 && is_identifier_byte(bytes[index - 1]) {
        index -= 1;
    }
    if !is_identifier_byte(bytes[index]) {
        return None;
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
        text: source_line.text[start..end].to_owned(),
        start_column: start + 1,
        end_column: end + 1,
        start_byte: line_start + start,
        end_byte: line_start + end,
    })
}
