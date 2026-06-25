// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::hash::{LineHash, hash_line, hash_text};
use crate::engine::lang::{
    AnalysisEngine, BinaryMode, DocumentKind, Language, detect_by_path, detect_language,
    extract_plain_text, language_spec, normalize_source_text, serialize_engine,
};
use crate::engine::paths::bytes_contain_identifier;
use crate::engine::symbols;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct Detection {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    #[serde(serialize_with = "serialize_engine")]
    pub(crate) engine: Option<AnalysisEngine>,
    pub(crate) supported: bool,
    pub(crate) binary: bool,
    pub(crate) mime: Option<String>,
    pub(crate) syntax: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HashLine {
    pub(crate) line: usize,
    pub(crate) hash: LineHash,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Symbol {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) start_hash: LineHash,
    pub(crate) end_hash: LineHash,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) name_byte: usize,
}

#[derive(Debug)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    pub(crate) text: String,
    pub(crate) kind: DocumentKind,
    pub(crate) detection: Detection,
    pub(crate) lines: Vec<SourceLine>,
    pub(crate) line_starts: Vec<usize>,
    pub(crate) file_hash: String,
}

impl SourceFile {
    pub(crate) fn line(&self, n: usize) -> Option<&SourceLine> {
        self.lines.get(n.checked_sub(1)?)
    }

    /// Zero-based index into `lines`/`line_starts` of the line holding `byte`.
    pub(crate) fn line_index(&self, byte: usize) -> usize {
        self.line_starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }

    /// One-based `(line, column)` of `byte`, with `column` measured in bytes.
    pub(crate) fn line_column(&self, byte: usize) -> (usize, usize) {
        if self.line_starts.is_empty() {
            return (1, 1);
        }
        let index = self.line_index(byte);
        let number = self.lines.get(index).map_or(1, |line| line.number);
        (number, byte - self.line_starts[index] + 1)
    }

    pub(crate) fn cursor_byte(&self, line: usize, column: usize) -> Result<usize> {
        if line == 0 || column == 0 {
            bail!("line and column must be greater than zero");
        }
        let source_line = self
            .line(line)
            .with_context(|| format!("line {line} not found in {}", self.path.display()))?;
        let max_column = source_line.text.len() + 1;
        if column > max_column {
            bail!("column {column} exceeds maximum column {max_column} for line {line}");
        }
        Ok(self.line_starts[line - 1] + column - 1)
    }
}

#[derive(Debug)]
struct LoadedDocument {
    text: String,
    binary: bool,
    mime: Option<String>,
}

#[derive(Debug)]
pub(crate) struct SourceLine {
    pub(crate) number: usize,
    pub(crate) text: String,
}

impl SourceLine {
    pub(crate) fn hash(&self) -> LineHash {
        hash_line(&self.text)
    }
}

impl From<&SourceLine> for HashLine {
    fn from(line: &SourceLine) -> Self {
        Self {
            line: line.number,
            hash: line.hash(),
            text: line.text.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SourceMap {
    pub(crate) symbols: Vec<Symbol>,
}

pub(crate) fn load_source(
    path: &Path,
    language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    let document = load_document(path, binary_mode)?;
    Ok(source_from_text(
        path,
        document.text,
        language,
        document.binary,
        document.mime,
    ))
}

/// Load `path` as a source file, but only when it contains `name` as a whole
/// identifier.
///
/// Returns `None` on a read, UTF-8, or parse error, or when the identifier is
/// absent, so callers scanning many files can skip it without distinguishing the
/// reasons.
pub(crate) fn read_source_containing(
    path: &Path,
    name: &str,
    language: Option<Language>,
) -> Option<SourceFile> {
    let bytes = fs::read(path).ok()?;
    if !bytes_contain_identifier(&bytes, name.as_bytes()) {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    Some(source_from_text(path, text, language, false, None))
}

pub(crate) fn load_indexable_source(
    path: &Path,
    language: Option<Language>,
) -> Result<Option<SourceFile>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mime = infer::get(&bytes).map(|kind| kind.mime_type().to_owned());
    if is_binary_document(&bytes, mime.as_deref()) {
        return Ok(None);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    Ok(Some(source_from_text(path, text, language, false, mime)))
}

pub(crate) fn source_from_text(
    path: &Path,
    text: String,
    language: Option<Language>,
    binary: bool,
    mime: Option<String>,
) -> SourceFile {
    let text = normalize_source_text(text);
    let path_language = detect_by_path(path);
    let (detected_language, syntax) = if binary && language.is_none() && path_language.is_none() {
        (Language::Unknown, None)
    } else {
        detect_language(path, &text)
    };
    let language = language.unwrap_or(detected_language);
    let engine = language_spec(language).and_then(|spec| spec.engine);
    let kind = if language == Language::Unknown {
        DocumentKind::Text
    } else {
        DocumentKind::Source
    };
    let file_hash = hash_text(&text);
    let mut line_starts = Vec::new();
    let mut lines = Vec::new();
    let mut offset = 0;
    for (index, part) in text.split_terminator('\n').enumerate() {
        line_starts.push(offset);
        offset += part.len() + 1;
        lines.push(SourceLine {
            number: index + 1,
            text: part.to_owned(),
        });
    }

    SourceFile {
        path: path.to_path_buf(),
        text,
        kind,
        detection: Detection {
            file: path.to_path_buf(),
            language,
            engine,
            supported: language != Language::Unknown,
            binary,
            mime,
            syntax,
        },
        lines,
        line_starts,
        file_hash,
    }
}

fn load_document(path: &Path, binary_mode: BinaryMode) -> Result<LoadedDocument> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mime = infer::get(&bytes).map(|kind| kind.mime_type().to_owned());
    let binary = is_binary_document(&bytes, mime.as_deref());

    if binary && binary_mode == BinaryMode::Reject {
        bail!(
            "unsupported binary file: {} ({})",
            path.display(),
            mime.as_deref().unwrap_or("unknown mime")
        );
    }

    let text = extract_plain_text(path, bytes, binary_mode)
        .with_context(|| format!("extract from {}", path.display()))?;

    Ok(LoadedDocument { text, binary, mime })
}

fn is_binary_document(bytes: &[u8], mime: Option<&str>) -> bool {
    const BINARY_PREFIXES: &[&str] = &["application/", "audio/", "font/", "image/", "video/"];
    mime.is_some_and(|mime| {
        BINARY_PREFIXES
            .iter()
            .any(|prefix| mime.starts_with(prefix))
    }) || bytes.contains(&0)
}

pub(crate) fn source_map(source: &SourceFile) -> Result<SourceMap> {
    let readseek_dir = crate::engine::repo::find_readseek_dir(&source.path);
    source_map_with_dir(source, readseek_dir.as_deref())
}

pub(crate) fn source_map_with_dir(
    source: &SourceFile,
    readseek_dir: Option<&Path>,
) -> Result<SourceMap> {
    if let Some(readseek_dir) = readseek_dir {
        match crate::engine::repo::load_map(readseek_dir, &source.file_hash) {
            Ok(Some((source_map, language, _engine))) => {
                if language == source.detection.language {
                    return Ok(source_map);
                }
                log::warn!(
                    "cache language mismatch for {}: cached {language}, current {}",
                    source.path.display(),
                    source.detection.language
                );
            }
            Ok(None) => {}
            Err(error) => log::warn!("cache load error: {error:#}"),
        }

        let source_map = symbols::parse_source_map(source)?;
        if let Err(error) =
            crate::engine::repo::store_map(readseek_dir, &source.file_hash, source, &source_map)
        {
            log::warn!("cache store error: {error:#}");
        }

        return Ok(source_map);
    }

    symbols::parse_source_map(source)
}

pub(crate) fn find_symbol(source_map: &SourceMap, line: usize) -> Option<Symbol> {
    let symbols = &source_map.symbols;
    let idx = symbols.partition_point(|s| s.start_line <= line);
    (0..idx)
        .rev()
        .filter(|&i| symbols[i].end_line >= line)
        .min_by_key(|&i| symbols[i].end_line - symbols[i].start_line)
        .map(|i| symbols[i].clone())
}

pub(crate) fn line_hash(source: &SourceFile, line: usize) -> Result<LineHash> {
    source
        .line(line)
        .map(SourceLine::hash)
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
        .map(HashLine::from)
        .collect()
}
