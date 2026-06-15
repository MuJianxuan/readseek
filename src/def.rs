use crate::cli::DefCommand;
use crate::lang::Language;
use crate::output::{CompactLocation, CompactOutput, DefLocation, DefOutput};
use crate::paths::{bytes_contain_identifier, def_candidate_paths};
use crate::source::{SourceFile, Symbol, source_from_text, source_map_with_dir};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read as _};
use std::path::Path;
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

pub(crate) fn output(command: &DefCommand) -> Result<DefOutput> {
    let name = match (command.name.as_ref(), command.stdin) {
        (Some(name), _) => Ok::<String, anyhow::Error>(name.clone()),
        (None, false) => bail!("definition requires a name or --stdin identify context"),
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
    let readseek_dir = crate::repo::find_readseek_dir(&command.target);
    let results = if let Some(readseek_dir) = readseek_dir.as_deref() {
        if let Some(index_entries) = crate::repo::load_index(readseek_dir, &name)? {
            if index_entries.is_empty() {
                def_candidate_paths(command, search_name)?
                    .par_iter()
                    .map(|path| {
                        locations_in_path(
                            path,
                            &name,
                            search_name,
                            command.language,
                            Some(readseek_dir),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                index_entries
                    .par_iter()
                    .map(|entry| {
                        locations_in_path(
                            &entry.path,
                            &name,
                            search_name,
                            command.language,
                            Some(readseek_dir),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        } else {
            def_candidate_paths(command, search_name)?
                .par_iter()
                .map(|path| {
                    locations_in_path(
                        path,
                        &name,
                        search_name,
                        command.language,
                        Some(readseek_dir),
                    )
                })
                .collect::<Result<Vec<_>>>()?
        }
    } else {
        def_candidate_paths(command, search_name)?
            .par_iter()
            .map(|path| locations_in_path(path, &name, search_name, command.language, None))
            .collect::<Result<Vec<_>>>()?
    };
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
    let Ok(bytes) = fs::read(path) else {
        return Ok(Vec::new());
    };
    if !bytes_contain_identifier(&bytes, search_name.as_bytes()) {
        return Ok(Vec::new());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(Vec::new());
    };
    let Ok(source) = source_from_text(path, text, language, false, None) else {
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
        .map(|line| DefLocation {
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
            },
            line_hash: line.hash(),
            text: line.text.clone(),
        })
        .collect()
}
