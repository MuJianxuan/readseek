// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::{Context, Result};
use argh::FromArgs;
use serde::Serialize;
use std::path::PathBuf;
use std::{env, process};

use crate::cli::{Cli, DefinitionCommand, ReferencesCommand, SearchCommand};
use crate::lang::{AnalysisEngine, BinaryMode, Language};
use crate::paths::command_paths;
use crate::source::{HashLine, Symbol};

mod cache;
mod cli;
mod hash;
mod lang;
mod navigation;
mod output;
mod paths;
mod search;
mod source;
mod symbols;

#[derive(Debug, Serialize)]
struct DefinitionOutput {
    definitions: Vec<DefinitionLocation>,
}

#[derive(Debug, Serialize)]
struct DefinitionLocation {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    symbol: Symbol,
    #[serde(skip_serializing)]
    line_hash: String,
    #[serde(skip_serializing)]
    text: String,
}

#[derive(Debug, Serialize)]
struct ReferencesOutput {
    references: Vec<ReferenceLocation>,
}

#[derive(Debug, Serialize)]
struct ReferenceLocation {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    line: usize,
    column: usize,
    line_hash: String,
    text: String,
    symbol: Option<Symbol>,
}

#[derive(Debug, Serialize)]
struct CompactOutput {
    locations: Vec<CompactLocation>,
}

#[derive(Debug, Serialize)]
struct CompactLocation {
    file: PathBuf,
    line: usize,
    column: usize,
    line_hash: String,
    text: String,
    kind: Option<String>,
    name: Option<String>,
    qualified_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    results: Vec<SearchFileOutput>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchFileOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    matches: Vec<SearchMatch>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchMatch {
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
    hashlines: Vec<HashLine>,
    captures: Vec<SearchCapture>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchCapture {
    name: String,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
    hashlines: Vec<HashLine>,
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    path: PathBuf,
    address: Option<TargetAddress>,
}

#[derive(Clone, Debug)]
pub(crate) enum TargetAddress {
    Line(usize),
    Hash(String),
    Symbol(String),
}

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
        crate::cli::Command::Detect(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = output::load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            print_json(&source.detection)?;
        }
        crate::cli::Command::Read(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = output::load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Lossy,
            )?;
            let target_line = output::resolve_target_line(&source, &target)?;
            let (start, end) = output::resolve_read_range(&command, target_line)?;
            let output = output::read_output(&source, start, end)?;
            print_json(&output)?;
        }
        crate::cli::Command::Map(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = output::load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            print_json(&output::map_output(&source)?)?;
        }
        crate::cli::Command::Symbol(command) => {
            let (target_arg, address_arg) = crate::cli::symbol_args(&command.args, command.stdin)?;
            let target = crate::cli::parse_symbol_input_target(
                target_arg,
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = output::load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            let target_line = output::resolve_explicit_target_line(&source, &target, command.line)?;
            let target_address = output::symbol_address(&target, address_arg)?;
            let output = output::symbol_command_output(&source, target_address, target_line)?;
            print_json(&output)?;
        }
        crate::cli::Command::Identify(command) => {
            let target = crate::cli::parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = output::load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            let target_line = output::resolve_explicit_target_line(&source, &target, command.line)?;
            let output = output::identify_output(&source, target_line, command.column)?;
            print_json(&output)?;
        }
        crate::cli::Command::Definition(command) => {
            print_definition_output(&command)?;
        }
        crate::cli::Command::References(command) => {
            print_references_output(&command)?;
        }
        crate::cli::Command::Search(command) => {
            print_json(&search_output(&command)?)?;
        }
    }

    Ok(())
}

fn print_definition_output(command: &DefinitionCommand) -> Result<()> {
    let output = navigation::definition_output(command)?;
    if command.compact {
        print_json(&navigation::compact_definitions(&output))
    } else {
        print_json(&output)
    }
}

fn print_references_output(command: &ReferencesCommand) -> Result<()> {
    let output = navigation::references_output(command)?;
    if command.compact {
        print_json(&navigation::compact_references(&output))
    } else {
        print_json(&output)
    }
}

fn search_output(command: &SearchCommand) -> Result<SearchOutput> {
    let paths = command_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
    )?;
    let mut pattern = crate::search::compile_search(&command.pattern);
    if let Some(language) = command
        .language
        .and_then(crate::symbols::tree_sitter_language)
    {
        crate::search::prepare_pattern_tree(&mut pattern, &language);
    }
    let mut parser = tree_sitter::Parser::new();
    let mut results = Vec::new();

    for path in paths {
        let Some(result) =
            crate::search::search_file(&path, command.language, &pattern, &mut parser)?
        else {
            continue;
        };
        if !result.matches.is_empty() {
            results.push(result);
        }
    }

    Ok(SearchOutput { results })
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
