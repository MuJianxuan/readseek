use crate::hash::{hash_line, hash_text};
use crate::lang::{
    BinaryMode, DocumentKind, EngineField, Language, detect_by_path, detect_language,
    extract_plain_text, language_spec, normalize_source_text,
};
use crate::symbols;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct Detection {
    pub(crate) file: PathBuf,
    pub(crate) language: Language,
    pub(crate) engine: EngineField,
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
    pub(crate) line_starts: Vec<usize>,
    pub(crate) file_hash: String,
}

impl SourceFile {
    pub(crate) fn line(&self, n: usize) -> Option<&SourceLine> {
        self.lines.get(n.checked_sub(1)?)
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
    pub(crate) hash: String,
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
    let engine = EngineField(language_spec(language).and_then(|spec| spec.engine));
    let kind = if language == Language::Unknown {
        DocumentKind::Text
    } else {
        DocumentKind::Source
    };
    let file_hash = hash_text(&text);
    let mut line_starts = Vec::new();
    let mut offset = 0;
    let lines = text
        .split_terminator('\n')
        .enumerate()
        .map(|(index, part)| {
            line_starts.push(offset);
            offset += part.len() + 1;
            SourceLine {
                number: index + 1,
                text: part.to_owned(),
                hash: hash_line(index + 1, part),
            }
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
        line_starts,
        file_hash,
    })
}

fn load_document(path: &Path, binary_mode: BinaryMode) -> Result<LoadedDocument> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mime = infer::get(&bytes).map(|kind| kind.mime_type().to_owned());
    let binary = mime.as_deref().is_some_and(|mime| {
        mime.starts_with("application/")
            || mime.starts_with("audio/")
            || mime.starts_with("font/")
            || mime.starts_with("image/")
            || mime.starts_with("video/")
    }) || bytes.contains(&0);

    if binary && binary_mode == BinaryMode::Reject {
        bail!(
            "unsupported binary file: {} ({})",
            path.display(),
            mime.as_deref().unwrap_or("unknown mime")
        );
    }

    let text = extract_plain_text(path, &bytes, binary_mode)
        .with_context(|| format!("extract from {}", path.display()))?;

    Ok(LoadedDocument { text, binary, mime })
}

pub(crate) fn source_map(source: &SourceFile) -> Result<SourceMap> {
    let readseek_dir = crate::repo::find_readseek_dir(&source.path);
    source_map_with_dir(source, readseek_dir.as_deref())
}

pub(crate) fn source_map_with_dir(
    source: &SourceFile,
    readseek_dir: Option<&Path>,
) -> Result<SourceMap> {
    if let Some(readseek_dir) = readseek_dir {
        match crate::repo::load_map(readseek_dir, &source.file_hash) {
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
            crate::repo::store_map(readseek_dir, &source.file_hash, source, &source_map)
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
        .take_while(|&i| symbols[i].end_line >= line)
        .min_by_key(|&i| symbols[i].end_line - symbols[i].start_line)
        .map(|i| symbols[i].clone())
}

pub(crate) fn line_hash(source: &SourceFile, line: usize) -> Result<String> {
    source
        .line(line)
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
