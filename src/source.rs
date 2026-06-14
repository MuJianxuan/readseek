use crate::hash::{hash_line, hash_text};
use crate::lang::{
    AnalysisEngine, BinaryMode, DocumentKind, Language, analysis_engine, detect_by_path,
    detect_language, document_extractor, document_kind, is_binary_mime, normalize_source_text,
};
use crate::{cache, symbols};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct Detection {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    pub(crate) engine: AnalysisEngine,
    pub(crate) supported: bool,
    pub(crate) binary: bool,
    pub(crate) mime: Option<String>,
    pub(crate) syntax: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HashLine {
    pub(crate) line: usize,
    pub(crate) hash: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Symbol {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) start_hash: String,
    pub(crate) end_hash: String,
}

#[derive(Debug)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    pub(crate) text: String,
    pub(crate) kind: DocumentKind,
    pub(crate) detection: Detection,
    pub(crate) lines: Vec<SourceLine>,
    pub(crate) file_hash: String,
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
    pub(crate) hash: String,
}

#[derive(Debug)]
pub(crate) struct SourceMap {
    pub(crate) symbols: Vec<Symbol>,
}

#[derive(Debug)]
pub(crate) enum SymbolLookup {
    Found(Symbol),
    NotFound,
    Ambiguous,
}

pub(crate) fn load_source(
    path: &Path,
    language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    let document = load_document(path, binary_mode)?;
    source_from_text(
        path,
        &document.text,
        language,
        document.binary,
        document.mime,
    )
}

pub(crate) fn source_from_text(
    path: &Path,
    text: &str,
    language: Option<Language>,
    binary: bool,
    mime: Option<String>,
) -> Result<SourceFile> {
    let text = normalize_source_text(text);
    let path_language = detect_by_path(path);
    let (detected_language, syntax) = if binary && language.is_none() && path_language.is_none() {
        (Language::Unknown, None)
    } else {
        detect_language(path, &text)?
    };
    let language = language.unwrap_or(detected_language);
    let engine = analysis_engine(language);
    let kind = document_kind(language);
    let file_hash = hash_text(&text);
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, text)| SourceLine {
            number: index + 1,
            text: text.to_owned(),
            hash: hash_line(index + 1, text),
        })
        .collect();

    Ok(SourceFile {
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

pub(crate) fn source_map(source: &SourceFile) -> Result<SourceMap> {
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

pub(crate) fn symbol_at_line_uncached(source: &SourceFile, line: usize) -> Result<Option<Symbol>> {
    let source_map = source_map(source)?;
    Ok(symbol_at_line_in_map(&source_map, line))
}

pub(crate) fn symbol_at_line_in_map(source_map: &SourceMap, line: usize) -> Option<Symbol> {
    source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .min_by_key(|symbol| symbol.end_line - symbol.start_line)
        .cloned()
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
