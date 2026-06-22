// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use tree_sitter::Parser;


use crate::cli;
use crate::cli::GitSelection;
use crate::engine::flags::GitFlags;
use crate::engine::lang::{BinaryMode, Language};
use crate::engine::output::SearchOutput;
use crate::engine::paths::command_paths;
use crate::engine::source::SourceFile;
use crate::engine::target::Target;
use crate::engine::{def, output, refs, rename, repo};

/// Parses arguments and runs the requested command, writing its output.
pub(crate) fn run() -> Result<()> {
    let cli: crate::cli::Cli = argh::from_env();
    if cli.version {
        println!("readseek {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(dir) = cli.readseek_dir {
        crate::engine::repo::set_dir_override(dir);
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
            let flags = command.git_flags();
            let name = match (command.name, command.from_identify) {
                (Some(name), _) => def::NameSource::Literal(name),
                (None, true) => def::NameSource::FromIdentify,
                (None, false) => {
                    anyhow::bail!("definition requires a name or --from-identify context")
                }
            };
            let request = def::Request {
                target: command.target,
                name,
                language: command.language,
                flags,
            };
            let output = def::output(&request)?;
            match command.format {
                crate::engine::output::Format::Plain => to_json(&def::compact(&output))?,
                crate::engine::output::Format::Json => to_json(&output)?,
            }
        }
        crate::cli::Command::Refs(command) => {
            let flags = command.git_flags();
            let request = refs::Request {
                target: command.target,
                name: command.name,
                scope: command.scope,
                line: command.line,
                column: command.column,
                language: command.language,
                flags,
            };
            let output = refs::output(&request)?;
            match command.format {
                crate::engine::output::Format::Plain => to_json(&refs::compact(&output))?,
                crate::engine::output::Format::Json => to_json(&output)?,
            }
        }
        crate::cli::Command::Rename(command) => {
            let flags = command.git_flags();
            let request = rename::Request {
                target: command.target,
                line: command.line,
                column: command.column,
                to: command.to,
                workspace: command.workspace,
                apply: command.apply,
                language: command.language,
                flags,
            };
            to_json(&rename::output(&request)?)?
        }
        crate::cli::Command::Search(command) => run_search(&command)?,
        crate::cli::Command::Init(command) => {
            run_init(&command)?;
            return Ok(());
        }
    };

    write_output(&json, output_path.as_deref())
}

fn load_source(
    target_str: Option<&str>,
    stdin: Option<&Path>,
    language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<(Target, SourceFile)> {
    let target = if let Some(path) = stdin {
        if target_str.is_some() {
            bail!("target cannot be combined with --stdin");
        }
        Target {
            path: path.to_path_buf(),
            address: None,
        }
    } else {
        crate::cli::parse_target(target_str.context("target required")?)?
    };
    let source = output::load_source_for_input(&target.path, stdin, language, binary_mode)?;
    Ok((target, source))
}

fn run_detect(command: &cli::DetectCommand) -> Result<String> {
    let (_, source) = load_source(command.target.as_deref(), command.stdin.as_deref(), command.language, BinaryMode::Reject)?;
    to_json(&source.detection)
}

fn run_read(command: &cli::ReadCommand) -> Result<String> {
    let (target, source) = load_source(command.target.as_deref(), command.stdin.as_deref(), command.language, BinaryMode::Lossy)?;
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
    let (_, source) = load_source(command.target.as_deref(), command.stdin.as_deref(), command.language, BinaryMode::Reject)?;
    to_json(&output::map_output(&source)?)
}

fn run_check(command: &cli::CheckCommand) -> Result<String> {
    let (_, source) = load_source(command.target.as_deref(), command.stdin.as_deref(), command.language, BinaryMode::Reject)?;
    to_json(&output::check_output(&source)?)
}

fn run_symbol(command: &cli::SymbolCommand) -> Result<String> {
    let (target, source) = load_source(command.target.as_deref(), command.stdin.as_deref(), command.language, BinaryMode::Reject)?;
    let target_line = output::resolve_explicit_target(&source, &target, command.line)?;
    let address = match (command.name.as_deref(), target_line) {
        (Some(name), _) => output::SymbolAddress::Name(name),
        (None, Some(line)) => output::SymbolAddress::Line(line),
        (None, None) => anyhow::bail!("symbol requires qualified name or target line/hash"),
    };
    let output = output::symbol_output(&source, address)?;
    to_json(&output)
}

fn run_identify(command: &cli::IdentifyCommand) -> Result<String> {
    let (target, source) = load_source(command.target.as_deref(), command.stdin.as_deref(), command.language, BinaryMode::Reject)?;
    let target_line = output::resolve_explicit_target(&source, &target, command.line)?;
    let output = output::identify_output(&source, target_line, command.column)?;
    to_json(&output)
}

fn run_search(command: &cli::SearchCommand) -> Result<String> {
    let paths = command_paths(&command.target, command.git_flags())?;
    let mut pattern = crate::engine::search::compile_search(&command.pattern);
    if let Some(language) = command
        .language
        .and_then(crate::engine::symbols::tree_sitter_language)
    {
        crate::engine::search::prepare_tree(&mut pattern, &language);
    }

    let results: Vec<_> = paths
        .par_iter()
        .map_init(Parser::new, |parser, path| {
            crate::engine::search::search_file(path, command.language, &pattern, parser)
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
