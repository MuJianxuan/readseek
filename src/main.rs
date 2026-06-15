// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::{Context, Result};
use argh::FromArgs;
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use std::{env, process};

use crate::cli::{Cli, DefCommand, InitCommand, RefsCommand, SearchCommand, UpdateCommand};
use crate::flags::GitFlags;
use crate::lang::BinaryMode;
use crate::output::SearchOutput;
use crate::paths::command_paths;
use crate::source::SourceFile;
use crate::target::Target;

mod cli;
mod flags;
mod hash;
mod lang;
mod navigation;
mod output;
mod paths;
mod repo;
mod search;
mod source;
mod symbols;
mod target;

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

    match cli.command.context("command required")? {
        crate::cli::Command::Detect(command) => run_detect(&command)?,
        crate::cli::Command::Read(command) => run_read(&command)?,
        crate::cli::Command::Map(command) => run_map(&command)?,
        crate::cli::Command::Symbol(command) => run_symbol(&command)?,
        crate::cli::Command::Identify(command) => run_identify(&command)?,
        crate::cli::Command::Def(command) => {
            print_def_output(&command)?;
        }
        crate::cli::Command::Refs(command) => {
            print_refs_output(&command)?;
        }
        crate::cli::Command::Search(command) => print_json(&search_output(&command)?)?,
        crate::cli::Command::Init(command) => run_init(&command)?,
        crate::cli::Command::Update(command) => run_update(&command)?,
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
    let target_line = output::resolve_target_line(&source, &target)?;
    let (start, end) = output::resolve_read_range(command, target_line)?;
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
    let target_line = output::resolve_explicit_target_line(&source, &target, command.line)?;
    let address = command.name.as_deref();
    let output = output::symbol_command_output(&source, address, target_line)?;
    print_json(&output)
}

fn run_identify(command: &cli::IdentifyCommand) -> Result<()> {
    let input = cli::InputArgs {
        target: command.target.clone(),
        stdin: command.stdin.clone(),
        language: command.language,
    };
    let (target, source) = load_source(&input, BinaryMode::Reject)?;
    let target_line = output::resolve_explicit_target_line(&source, &target, command.line)?;
    let output = output::identify_output(&source, target_line, command.column)?;
    print_json(&output)
}

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

fn print_def_output(command: &DefCommand) -> Result<()> {
    let output = navigation::def_output(command)?;
    match command.format {
        crate::output::Format::Plain => print_json(&navigation::compact_defs(&output)),
        crate::output::Format::Json => print_json(&output),
    }
}

fn print_refs_output(command: &RefsCommand) -> Result<()> {
    let output = navigation::refs_output(command)?;
    match command.format {
        crate::output::Format::Plain => print_json(&navigation::compact_refs(&output)),
        crate::output::Format::Json => print_json(&output),
    }
}

fn search_output(command: &SearchCommand) -> Result<SearchOutput> {
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

    Ok(SearchOutput { results })
}

fn run_init(command: &InitCommand) -> Result<()> {
    let path = command.path.as_deref().unwrap_or(Path::new("."));
    repo::init(path)?;
    Ok(())
}

fn run_update(command: &UpdateCommand) -> Result<()> {
    let path = command.path.as_deref().unwrap_or(Path::new("."));
    let stats = repo::update(
        path,
        GitFlags {
            cached: true,
            others: true,
            ignored: false,
        },
    )?;
    println!("{stats:?}");
    Ok(())
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
