// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::flags::GitFlags;
use crate::engine::lang::Language;
use crate::engine::output::{CompactLocation, CompactOutput, DefLocation, DefOutput};
use crate::engine::paths::def_candidate_paths;
use crate::engine::source::{read_source_containing, source_map_with_dir};
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

/// How [`output`] obtains the symbol name to resolve.
pub(crate) enum NameSource {
    /// A name supplied directly on the command line.
    Literal(String),
    /// The name read from `identify` output on stdin.
    FromIdentify,
}

/// Inputs for [`output`]: the symbol to resolve and where to search for it.
pub(crate) struct Request {
    pub(crate) target: PathBuf,
    pub(crate) name: NameSource,
    pub(crate) language: Option<Language>,
    pub(crate) flags: GitFlags,
}

pub(crate) fn output(request: &Request) -> Result<DefOutput> {
    let name = match &request.name {
        NameSource::Literal(name) => name.clone(),
        NameSource::FromIdentify => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .context("read identify context from stdin")?;
            let input: IdentifyInput =
                serde_json::from_str(&text).context("parse identify context")?;
            if let Some(identifier) = input.identifier {
                identifier.text
            } else if let Some(symbol) = input.symbol {
                symbol.qualified_name
            } else {
                bail!("identify context has no symbol or identifier")
            }
        }
    };
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
                let scoped = paths_in_scope(entries, &request.target);
                if scoped.is_empty() {
                    def_candidate_paths(&request.target, request.flags, search_name)?
                } else {
                    scoped
                }
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

/// Keep the index entries whose file lies within the requested `target` scope.
///
/// The target is canonicalized once; an entry is retained when its own
/// canonical path equals the target (a file) or is nested under it (a
/// directory). Entries that no longer canonicalize are dropped, so a stale
/// index cannot return a deleted file.
fn paths_in_scope(entries: Vec<crate::engine::repo::DefIndexEntry>, target: &Path) -> Vec<PathBuf> {
    let Ok(target) = target.canonicalize() else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let canonical = entry.path.canonicalize().ok()?;
            canonical.starts_with(&target).then_some(entry.path)
        })
        .collect()
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
    let qualified = name != search_name;
    let mut definitions = Vec::new();

    let Ok(source_map) = source_map_with_dir(&source, readseek_dir) else {
        return Ok(definitions);
    };
    for symbol in source_map.symbols {
        let matched = symbol.qualified_name == name || (!qualified && symbol.name == search_name);
        if !matched {
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
                line_hash: definition.line_hash,
                text: definition.text.clone(),
                kind: Some(definition.symbol.kind.clone()),
                name: Some(definition.symbol.name.clone()),
                qualified_name: Some(definition.symbol.qualified_name.clone()),
            })
            .collect(),
    }
}
