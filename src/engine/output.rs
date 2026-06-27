// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::hash::LineHash;
use crate::engine::image::{ImageInfo, OcrText};
use crate::engine::lang::{AnalysisEngine, BinaryMode, Language, serialize_engine};
use crate::engine::source::{
    Detection, HashLine, SourceFile, Symbol, find_symbol, load_source, range_hashlines,
    source_from_text, source_map,
};
use crate::engine::target::{Target, TargetAddress};

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
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum DetectOutput {
    Source(DetectSourceOutput),
    Image(DetectImageOutput),
    Binary(DetectBinaryOutput),
    Text(DetectTextOutput),
}

impl DetectOutput {
    pub(crate) fn from_detection(detection: Detection) -> Self {
        if let Some(image) = detection.image {
            return Self::Image(DetectImageOutput::new(
                detection.file,
                detection.mime,
                image,
            ));
        }

        if detection.binary {
            return Self::Binary(DetectBinaryOutput {
                file: detection.file,
                mime: detection.mime,
            });
        }

        if detection.language == Language::Unknown {
            return Self::Text(DetectTextOutput {
                file: detection.file,
                mime: detection.mime,
            });
        }

        Self::Source(DetectSourceOutput {
            file: detection.file,
            language: detection.language,
            engine: detection.engine,
            supported: detection.supported,
            mime: detection.mime,
            syntax: detection.syntax,
        })
    }

    pub(crate) fn is_image(&self) -> bool {
        matches!(self, Self::Image(_))
    }

    pub(crate) fn set_ocr(&mut self, text: OcrText) {
        if let Self::Image(image) = self {
            image.ocr = Some(text);
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DetectSourceOutput {
    file: PathBuf,
    language: Language,
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    engine: Option<AnalysisEngine>,
    supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    syntax: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DetectImageOutput {
    file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
    #[serde(flatten)]
    image: ImageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    ocr: Option<OcrText>,
}

impl DetectImageOutput {
    fn new(file: PathBuf, mime: Option<String>, image: ImageInfo) -> Self {
        Self {
            file,
            mime,
            image,
            ocr: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DetectBinaryOutput {
    file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DetectTextOutput {
    file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceHeader {
    file: PathBuf,
    language: Language,
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    engine: Option<AnalysisEngine>,
    line_count: usize,
    file_hash: String,
}

impl From<&SourceFile> for SourceHeader {
    fn from(source: &SourceFile) -> Self {
        Self {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            line_count: source.lines.len(),
            file_hash: source.file_hash.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadOutput {
    #[serde(flatten)]
    header: SourceHeader,
    start_line: usize,
    end_line: usize,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MapOutput {
    #[serde(flatten)]
    header: SourceHeader,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckOutput {
    #[serde(flatten)]
    header: SourceHeader,
    error_count: usize,
    missing_count: usize,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiagnosticKind {
    Error,
    Missing,
}

#[derive(Debug, Serialize)]
pub(crate) struct Diagnostic {
    pub(crate) kind: DiagnosticKind,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct SymbolOutput {
    #[serde(flatten)]
    header: SourceHeader,
    symbol: Symbol,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
pub(crate) struct IdentifyOutput {
    #[serde(flatten)]
    header: SourceHeader,
    line: usize,
    column: usize,
    line_hash: LineHash,
    hashlines: Vec<HashLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identifier: Option<IdentifierOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) engine: Option<AnalysisEngine>,
    pub(crate) file_hash: String,
    pub(crate) symbol: Symbol,
    #[serde(skip_serializing)]
    pub(crate) line_hash: LineHash,
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
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) engine: Option<AnalysisEngine>,
    pub(crate) file_hash: Arc<str>,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) line_hash: LineHash,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) symbol: Option<Symbol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurrence: Option<crate::engine::binding::OccurrenceKind>,
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
    pub(crate) line_hash: LineHash,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qualified_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RenameOutput {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) engine: Option<AnalysisEngine>,
    pub(crate) file_hash: String,
    pub(crate) old_name: String,
    pub(crate) new_name: String,
    pub(crate) applied: bool,
    pub(crate) conflicts: Vec<RenameConflict>,
    pub(crate) edits: Vec<RenameEdit>,
    /// Additional files edited when `--workspace` expands the rename. The
    /// top-level fields describe the cursor file (binding-accurate); each entry
    /// here is matched by name (binding-resolved only to drop local shadows).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) others: Vec<RenameFileOutput>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RenameFileOutput {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) engine: Option<AnalysisEngine>,
    pub(crate) file_hash: String,
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
    pub(crate) occurrence: crate::engine::binding::OccurrenceKind,
    pub(crate) line_hash: LineHash,
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
    #[serde(serialize_with = "serialize_engine")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) engine: Option<AnalysisEngine>,
    pub(crate) file_hash: String,
    pub(crate) matches: Vec<SearchMatch>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchMatch {
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) start_hash: LineHash,
    pub(crate) end_hash: LineHash,
    pub(crate) hashlines: Vec<HashLine>,
    pub(crate) captures: Vec<SearchCapture>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchCapture {
    pub(crate) name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) start_hash: LineHash,
    pub(crate) end_hash: LineHash,
    pub(crate) hashlines: Vec<HashLine>,
}

pub(crate) fn resolve_target(source: &SourceFile, target: &Target) -> Result<Option<usize>> {
    match target.address.as_ref() {
        Some(TargetAddress::Line(line)) => Ok(Some(*line)),
        Some(TargetAddress::Hash(hash)) => {
            let target = hash
                .parse::<LineHash>()
                .with_context(|| format!("invalid target hash {hash}"))?;
            source
                .lines
                .iter()
                .find_map(|line| (line.hash() == target).then_some(line.number))
                .with_context(|| format!("hash {hash} not found in {}", source.path.display()))
                .map(Some)
        }
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
    stdin: Option<&Path>,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    if let Some(stdin_path) = stdin {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("read stdin")?;
        return Ok(source_from_text(
            stdin_path,
            text,
            override_language,
            false,
            None,
        ));
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
            header: source.into(),
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
        .map(HashLine::from)
        .collect();

    Ok(ReadOutput {
        header: source.into(),
        start_line,
        end_line,
        hashlines,
    })
}

pub(crate) fn map_output(source: &SourceFile) -> Result<MapOutput> {
    let source_map = source_map(source)?;

    Ok(MapOutput {
        header: source.into(),
        symbols: source_map.symbols,
    })
}

pub(crate) fn check_output(source: &SourceFile) -> Result<CheckOutput> {
    let diagnostics = crate::engine::symbols::parse_diagnostics(source)?;
    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::Error))
        .count();
    let missing_count = diagnostics.len() - error_count;

    Ok(CheckOutput {
        header: source.into(),
        error_count,
        missing_count,
        diagnostics,
    })
}

/// How [`symbol_output`] locates the symbol to report.
#[derive(Clone, Copy)]
pub(crate) enum SymbolAddress<'a> {
    /// A qualified or unqualified symbol name.
    Name(&'a str),
    /// A one-based line the symbol must enclose.
    Line(usize),
}

pub(crate) fn symbol_output(
    source: &SourceFile,
    address: SymbolAddress<'_>,
) -> Result<SymbolOutput> {
    let source_map = source_map(source)?;
    let symbol = match address {
        SymbolAddress::Name(address) => {
            let mut matches = source_map
                .symbols
                .iter()
                .filter(|symbol| symbol.qualified_name == address || symbol.name == address);

            let symbol = matches
                .next()
                .with_context(|| format!("no symbol `{address}`"))?;

            if matches.next().is_some() {
                bail!("ambiguous symbol `{address}`");
            }

            symbol.clone()
        }
        SymbolAddress::Line(line) => {
            find_symbol(&source_map, line).with_context(|| format!("no symbol at line {line}"))?
        }
    };

    Ok(SymbolOutput {
        header: source.into(),
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
    let cursor_byte = source.cursor_byte(line, column)?;
    let source_line = source
        .line(line)
        .with_context(|| format!("line {line} not found in {}", source.path.display()))?;
    let line_start = source.line_starts[line - 1];
    let identifier = crate::engine::symbols::token_at(source, cursor_byte)
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
        header: source.into(),
        line,
        column,
        line_hash: source_line.hash(),
        hashlines: vec![HashLine::from(source_line)],
        identifier,
        symbol,
    })
}

/// Fallback identifier extraction for languages without a tree-sitter parser.
///
/// Walks ASCII identifier bytes around the cursor on a single line. `line_start`
/// is the byte offset of the line within the file, used to report absolute bytes.
fn identify_byte_scan(
    source_line: &crate::engine::source::SourceLine,
    line_start: usize,
    column: usize,
) -> Option<IdentifierOutput> {
    let bytes = source_line.text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = column.saturating_sub(1).min(bytes.len().saturating_sub(1));
    if !(bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        && index > 0
        && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_')
    {
        index -= 1;
    }
    if !(bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
        return None;
    }
    let mut start = index;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    let mut end = index + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
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
