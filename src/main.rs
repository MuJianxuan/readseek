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

mod binding;
mod cli;
mod def;
mod flags;
mod hash;
mod lang;
mod output;
mod paths;
mod refs;
mod rename;
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
        crate::cli::Command::Detect(command) => run_detect(&command)?,
        crate::cli::Command::Read(command) => run_read(&command)?,
        crate::cli::Command::Map(command) => run_map(&command)?,
        crate::cli::Command::Symbol(command) => run_symbol(&command)?,
        crate::cli::Command::Identify(command) => run_identify(&command)?,
        crate::cli::Command::Def(command) => {
            let output = def::output(&command)?;
            match command.format {
                crate::output::Format::Plain => print_json(&def::compact(&output))?,
                crate::output::Format::Json => print_json(&output)?,
            }
        }
        crate::cli::Command::Refs(command) => {
            let output = refs::output(&command)?;
            match command.format {
                crate::output::Format::Plain => print_json(&refs::compact(&output))?,
                crate::output::Format::Json => print_json(&output)?,
            }
        }
        crate::cli::Command::Rename(command) => print_json(&rename::output(&command)?)?,
        crate::cli::Command::Search(command) => run_search(&command)?,
        crate::cli::Command::Init(command) => run_init(&command)?,
    }

    Ok(())
}

fn run_detect(command: &cli::DetectCommand) -> Result<()> {
    let input = cli::InputArgs {
        target: command.target.clone(),
        stdin: command.stdin.clone(),
        language: command.language,
    };
    let (_, source) = load_source(&input, BinaryMode::Reject)?;
    print_json(&source.detection)
}

fn run_read(command: &cli::ReadCommand) -> Result<()> {
    let input = cli::InputArgs {
        target: command.target.clone(),
        stdin: command.stdin.clone(),
        language: command.language,
    };
    let (target, source) = load_source(&input, BinaryMode::Lossy)?;
    let target_line = output::resolve_target(&source, &target)?;
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
    print_json(&output)
}

fn run_map(command: &cli::MapCommand) -> Result<()> {
    let input = cli::InputArgs {
        target: command.target.clone(),
        stdin: command.stdin.clone(),
        language: command.language,
    };
    let (_, source) = load_source(&input, BinaryMode::Reject)?;
    print_json(&output::map_output(&source)?)
}

fn run_symbol(command: &cli::SymbolCommand) -> Result<()> {
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
    let target_line = output::resolve_explicit_target(&source, &target, command.line)?;
    let address = command.name.as_deref();
    let output = output::symbol_output(&source, address, target_line)?;
    print_json(&output)
}

fn run_identify(command: &cli::IdentifyCommand) -> Result<()> {
    let input = cli::InputArgs {
        target: command.target.clone(),
        stdin: command.stdin.clone(),
        language: command.language,
    };
    let (target, source) = load_source(&input, BinaryMode::Reject)?;
    let target_line = output::resolve_explicit_target(&source, &target, command.line)?;
    let output = output::identify_output(&source, target_line, command.column)?;
    print_json(&output)
}

fn run_search(command: &cli::SearchCommand) -> Result<()> {
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
        crate::search::prepare_tree(&mut pattern, &language);
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

    print_json(&SearchOutput { results })
}

fn run_init(command: &cli::InitCommand) -> Result<()> {
    let path = command.path.as_deref().unwrap_or(Path::new("."));
    let init = repo::init(path)?;
    repo::update(
        path,
        GitFlags {
            cached: true,
            others: true,
            ignored: false,
        },
    )?;
    if init.reinitialized {
        println!(
            "Reinitialized existing readseek repository in {}/",
            init.path.display()
        );
    } else {
        println!(
            "Initialized empty readseek repository in {}/",
            init.path.display()
        );
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
