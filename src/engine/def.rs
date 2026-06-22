// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::flags::GitFlags;
use crate::engine::lang::Language;
use crate::engine::output::{CompactLocation, CompactOutput, DefLocation, DefOutput};
use crate::engine::paths::def_candidate_paths;
use crate::engine::source::{SourceFile, Symbol, read_source_containing, source_map_with_dir};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Inputs for [`output`]: the symbol to resolve and where to search for it.
pub(crate) struct Request {
    pub(crate) target: PathBuf,
    pub(crate) name: Option<String>,
    pub(crate) from_identify: bool,
    pub(crate) language: Option<Language>,
    pub(crate) flags: GitFlags,
}

pub(crate) fn output(request: &Request) -> Result<DefOutput> {
    let name = match (request.name.as_ref(), request.from_identify) {
        (Some(name), _) => Ok::<String, anyhow::Error>(name.clone()),
        (None, false) => bail!("definition requires a name or --from-identify context"),
        (None, true) => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .context("read identify context from stdin")?;
            let input: IdentifyInput =
                serde_json::from_str(&text).context("parse identify context")?;
            if let Some(identifier) = input.identifier {
                Ok(identifier.text)
            } else if let Some(symbol) = input.symbol {
                Ok(symbol.qualified_name)
            } else {
                bail!("identify context has no symbol or identifier")
            }
        }
    }?;
    let search_name = name
        .rsplit('.')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(&name);
    let readseek_dir = crate::engine::repo::find_readseek_dir(&request.target);
    let readseek_dir = readseek_dir.as_deref();
    let paths = match readseek_dir {
        Some(dir) => match crate::engine::repo::load_index(dir, &name)? {
            Some(entries) if !entries.is_empty() => {
                entries.into_iter().map(|entry| entry.path).collect()
            }
            _ => def_candidate_paths(&request.target, request.flags, search_name)?,
        },
        None => def_candidate_paths(&request.target, request.flags, search_name)?,
    };

    let results = paths
        .par_iter()
        .map(|path| locations_in_path(path, &name, search_name, request.language, readseek_dir))
        .collect::<Result<Vec<_>>>()?;

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

fn locations_in_path(
    path: &Path,
    name: &str,
    search_name: &str,
    language: Option<Language>,
    readseek_dir: Option<&Path>,
) -> Result<Vec<DefLocation>> {
    let Some(source) = read_source_containing(path, search_name, language) else {
        return Ok(Vec::new());
    };
    let mut definitions = macro_locations(&source, search_name);

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
            line_hash: line.hash(),
            text: line.text.clone(),
            symbol,
        });
    }

    Ok(definitions)
}

pub(crate) fn compact(output: &DefOutput) -> CompactOutput {
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

fn macro_locations(source: &SourceFile, name: &str) -> Vec<DefLocation> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }

    source
        .lines
        .iter()
        .filter(|line| {
            let Some(rest) = line.text.trim_start().strip_prefix("#define") else {
                return false;
            };
            if !rest.starts_with(char::is_whitespace) {
                return false;
            }
            let rest = rest.trim_start();
            let name_len = rest
                .find(|ch: char| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_'))
                .unwrap_or(rest.len());
            name_len > 0 && &rest[..name_len] == name
        })
        .map(|line| {
            let line_start = source.line_starts[line.number - 1];
            let name_byte = line
                .text
                .find(name)
                .map_or(line_start, |offset| line_start + offset);
            DefLocation {
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
                    start_hash: line.hash(),
                    end_hash: line.hash(),
                    start_byte: line_start,
                    end_byte: line_start + line.text.len(),
                    name_byte,
                },
                line_hash: line.hash(),
                text: line.text.clone(),
            }
        })
        .collect()
}
