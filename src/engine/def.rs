// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::flags::GitFlags;
use crate::engine::lang::Language;
use crate::engine::output::{CompactLocation, CompactOutput, DefLocation, DefOutput};
use crate::engine::paths::def_candidate_paths;
use crate::engine::source::{read_source_containing, source_map_with_dir};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Inputs for [`output`]: the symbol to resolve and where to search for it.
pub(crate) struct Request {
    pub(crate) target: PathBuf,
    pub(crate) name: String,
    pub(crate) language: Option<Language>,
    pub(crate) flags: GitFlags,
}

pub(crate) fn output(request: &Request) -> Result<DefOutput> {
    let name = &request.name;
    if name.is_empty() {
        bail!("definition name must not be empty");
    }
    if !is_qualified_name(name) {
        bail!("definition name must be a qualified identifier");
    }
    let search_name = name.rsplit('.').next().unwrap();
    let readseek_dir = crate::engine::repo::find_readseek_dir(&request.target);
    let readseek_dir = readseek_dir.as_deref();
    let paths = match readseek_dir {
        Some(dir) => match crate::engine::repo::load_index(dir, name)? {
            Some(indexed) if !indexed.is_empty() => {
                let scoped = paths_in_scope(indexed, &request.target);
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
        .map(|path| locations_in_path(path, name, search_name, request.language, readseek_dir))
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

/// Keep the indexed paths whose file lies within the requested `target` scope.
///
/// The target is canonicalized once; a path is retained when its own
/// canonical path equals the target (a file) or is nested under it (a
/// directory). Paths that no longer canonicalize are dropped, so a stale
/// index cannot return a deleted file.
fn paths_in_scope(paths: Vec<PathBuf>, target: &Path) -> Vec<PathBuf> {
    let Ok(target) = target.canonicalize() else {
        return Vec::new();
    };
    paths
        .into_iter()
        .filter_map(|path| {
            let canonical = path.canonicalize().ok()?;
            canonical.starts_with(&target).then_some(path)
        })
        .collect()
}

/// Whether `name` is a qualified identifier: dot-separated plain identifiers
/// with no empty segments. Matches the shape of `Symbol::qualified_name`.
fn is_qualified_name(name: &str) -> bool {
    name.split('.').all(|segment| {
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        (first.is_ascii_alphabetic() || first == '_')
            && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
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
