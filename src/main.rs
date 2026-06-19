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

use crate::cli::Cli;
use crate::flags::GitFlags;
use crate::lang::BinaryMode;
use crate::output::SearchOutput;
use crate::paths::command_paths;
use crate::source::SourceFile;
use crate::target::Target;

fn load_source(command: &impl cli::Input, binary_mode: BinaryMode) -> Result<(Target, SourceFile)> {
    let input = command.input();
    let target = input.to_target()?;
    let source =
        output::load_source_for_input(&target.path, input.stdin, input.language, binary_mode)?;
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

    if let Some(dir) = cli.readseek_dir {
        crate::repo::set_dir_override(dir);
    }
    let output_path = cli.output;
    let command = cli.command.context("command required")?;

    let json = match command {
        crate::cli::Command::Detect(command) => run_detect(&command)?,
        crate::cli::Command::Read(command) => run_read(&command)?,
        crate::cli::Command::Map(command) => run_map(&command)?,
        crate::cli::Command::Check(command) => run_check(&command)?,
        crate::cli::Command::Symbol(command) => run_symbol(&command)?,
        crate::cli::Command::Identify(command) => run_identify(&command)?,
        crate::cli::Command::Def(command) => {
            let output = def::output(&command)?;
            match command.format {
                crate::output::Format::Plain => to_json(&def::compact(&output))?,
                crate::output::Format::Json => to_json(&output)?,
            }
        }
        crate::cli::Command::Refs(command) => {
            let output = refs::output(&command)?;
            match command.format {
                crate::output::Format::Plain => to_json(&refs::compact(&output))?,
                crate::output::Format::Json => to_json(&output)?,
            }
        }
        crate::cli::Command::Rename(command) => to_json(&rename::output(&command)?)?,
        crate::cli::Command::Search(command) => run_search(&command)?,
        crate::cli::Command::Init(command) => {
            run_init(&command)?;
            return Ok(());
        }
    };

    write_output(&json, output_path.as_deref())
}

fn run_detect(command: &cli::DetectCommand) -> Result<String> {
    let (_, source) = load_source(command, BinaryMode::Reject)?;
    to_json(&source.detection)
}

fn run_read(command: &cli::ReadCommand) -> Result<String> {
    let (target, source) = load_source(command, BinaryMode::Lossy)?;
    let target_line = output::resolve_target(&source, &target)?;
    let start = match (command.start, target_line) {
        (Some(start), Some(line)) if start != line => {
            anyhow::bail!("target line conflicts with --start")
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
    to_json(&output)
}

fn run_map(command: &cli::MapCommand) -> Result<String> {
    let (_, source) = load_source(command, BinaryMode::Reject)?;
    to_json(&output::map_output(&source)?)
}

fn run_check(command: &cli::CheckCommand) -> Result<String> {
    let (_, source) = load_source(command, BinaryMode::Reject)?;
    to_json(&output::check_output(&source)?)
}

fn run_symbol(command: &cli::SymbolCommand) -> Result<String> {
    let (target, source) = load_source(command, BinaryMode::Reject)?;
    let target_line = output::resolve_explicit_target(&source, &target, command.line)?;
    let address = command.name.as_deref();
    let output = output::symbol_output(&source, address, target_line)?;
    to_json(&output)
}

fn run_identify(command: &cli::IdentifyCommand) -> Result<String> {
    let (target, source) = load_source(command, BinaryMode::Reject)?;
    let target_line = output::resolve_explicit_target(&source, &target, command.line)?;
    let output = output::identify_output(&source, target_line, command.column)?;
    to_json(&output)
}

fn run_search(command: &cli::SearchCommand) -> Result<String> {
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
        .map(|path| {
            let mut parser = tree_sitter::Parser::new();
            crate::search::search_file(path, command.language, &pattern, &mut parser)
                .map(|result| result.filter(|result| !result.matches.is_empty()))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    to_json(&SearchOutput { results })
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

fn to_json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

fn write_output(json: &str, path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        std::fs::write(path, json).with_context(|| format!("write {}", path.display()))
    } else {
        println!("{json}");
        Ok(())
    }
}
