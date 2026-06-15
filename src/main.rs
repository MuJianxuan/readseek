// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::{Context, Result};
use argh::FromArgs;
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{env, process};

use crate::cli::Cli;
use crate::flags::GitFlags;
use crate::lang::BinaryMode;
use crate::output::SearchOutput;
use crate::paths::command_paths;
use crate::source::SourceFile;
use crate::target::Target;

fn load_source(input: &cli::InputArgs, binary_mode: BinaryMode) -> Result<(Target, SourceFile)> {
    let target = input.to_target()?;
    let source = output::load_source_for_input(
        &target.path,
        input.stdin.as_ref(),
        input.language,
        binary_mode,
    )?;
    Ok((target, source))
}

mod cli;
mod flags;
mod hash;
mod ignore;
mod lang;
mod navigation;
mod output;
mod paths;
mod repo;
mod search;
mod source;
mod symbols;
mod target;

static OUTPUT_FILE: OnceLock<PathBuf> = OnceLock::new();

fn main() {
    env_logger::init();
    if env::args_os().len() == 1 {
        match Cli::from_args(&["readseek"], &["--help"]) {
            Err(early_exit) => eprintln!("{}", early_exit.output),
            Ok(_) => eprintln!("readseek: help output unavailable"),
        }
        process::exit(2);
    }
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        process::exit(1);
    }
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<()> {
    let cli: crate::cli::Cli = argh::from_env();
    if cli.version {
        println!("readseek {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(path) = cli.output {
        OUTPUT_FILE.set(path).ok();
    }
    let command = cli.command.context("command required")?;

    match command {
        crate::cli::Command::Detect(command) => {
            let input = cli::InputArgs {
                target: command.target.clone(),
                stdin: command.stdin.clone(),
                language: command.language,
            };
            let (_, source) = load_source(&input, BinaryMode::Reject)?;
            print_json(&source.detection)?;
        }
        crate::cli::Command::Read(command) => {
            let input = cli::InputArgs {
                target: command.target.clone(),
                stdin: command.stdin.clone(),
                language: command.language,
            };
            let (target, source) = load_source(&input, BinaryMode::Lossy)?;
            let target_line = output::resolve_target_line(&source, &target)?;
            let start = match (command.offset, target_line) {
                (Some(start), Some(line)) if start != line => {
                    anyhow::bail!("target line conflicts with --offset")
                }
                (Some(start), _) | (_, Some(start)) => Some(start),
                (None, None) => None,
            };

            if command.end.is_some() && command.limit.is_some() {
                anyhow::bail!("cannot combine --end with --limit");
            }

            let end = if let Some(limit) = command.limit {
                if limit == 0 {
                    anyhow::bail!("limit must be greater than zero");
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
            let output = output::read_output(&source, start, end)?;
            print_json(&output)?;
        }
        crate::cli::Command::Map(command) => {
            let input = cli::InputArgs {
                target: command.target.clone(),
                stdin: command.stdin.clone(),
                language: command.language,
            };
            let (_, source) = load_source(&input, BinaryMode::Reject)?;
            print_json(&output::map_output(&source)?)?;
        }
        crate::cli::Command::Symbol(command) => {
            let input = cli::InputArgs {
                target: command.target.clone(),
                stdin: command.stdin.clone(),
                language: command.language,
            };
            let target = input.to_target()?;
            let source = output::load_source_for_input(
                &target.path,
                input.stdin.as_ref(),
                input.language,
                BinaryMode::Reject,
            )?;
            let target_line = output::resolve_explicit_target_line(&source, &target, command.line)?;
            let address = command.name.as_deref();
            let output = output::symbol_command_output(&source, address, target_line)?;
            print_json(&output)?;
        }
        crate::cli::Command::Identify(command) => {
            let input = cli::InputArgs {
                target: command.target.clone(),
                stdin: command.stdin.clone(),
                language: command.language,
            };
            let (target, source) = load_source(&input, BinaryMode::Reject)?;
            let target_line = output::resolve_explicit_target_line(&source, &target, command.line)?;
            let output = output::identify_output(&source, target_line, command.column)?;
            print_json(&output)?;
        }
        crate::cli::Command::Def(command) => {
            let output = navigation::def_output(&command)?;
            match command.format {
                crate::output::Format::Plain => print_json(&navigation::compact_defs(&output))?,
                crate::output::Format::Json => print_json(&output)?,
            }
        }
        crate::cli::Command::Refs(command) => {
            let output = navigation::refs_output(&command)?;
            match command.format {
                crate::output::Format::Plain => print_json(&navigation::compact_refs(&output))?,
                crate::output::Format::Json => print_json(&output)?,
            }
        }
        crate::cli::Command::Search(command) => {
            let paths = command_paths(
                &command.target,
                GitFlags {
                    cached: command.cached,
                    others: command.others,
                    ignored: command.ignored,
                },
            )?;
            let mut pattern = crate::search::compile_search(&command.pattern);
            if let Some(language) = command
                .language
                .and_then(crate::symbols::tree_sitter_language)
            {
                crate::search::prepare_pattern_tree(&mut pattern, &language);
            }

            let results: Vec<_> = paths
                .par_iter()
                .filter_map(|path| {
                    let mut parser = tree_sitter::Parser::new();
                    crate::search::search_file(path, command.language, &pattern, &mut parser)
                        .ok()
                        .flatten()
                        .filter(|result| !result.matches.is_empty())
                })
                .collect();

            print_json(&SearchOutput { results })?;
        }
        crate::cli::Command::Init(command) => {
            let path = command.path.as_deref().unwrap_or(Path::new("."));
            repo::init(path)?;
            repo::update(
                path,
                GitFlags {
                    cached: true,
                    others: true,
                    ignored: false,
                },
            )?;
        }
    }

    Ok(())
}

fn print_json(value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(path) = OUTPUT_FILE.get() {
        std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    } else {
        println!("{json}");
    }
    Ok(())
}
