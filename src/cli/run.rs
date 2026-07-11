// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
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
use crate::engine::{def, output, refs, rename, repo, vision_cache};

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
        cli::Command::Detect(command) => command.run()?,
        cli::Command::Read(command) => command.run()?,
        cli::Command::Map(command) => command.run()?,
        cli::Command::Check(command) => command.run()?,
        cli::Command::Symbol(command) => command.run()?,
        cli::Command::Identify(command) => command.run()?,
        cli::Command::Def(command) => command.run()?,
        cli::Command::Refs(command) => command.run()?,
        cli::Command::Rename(command) => command.run()?,
        cli::Command::Search(command) => command.run()?,
        cli::Command::Init(command) => {
            command.run()?;
            return Ok(());
        }
    };

    write_output(&json, output_path.as_deref())
}

impl cli::DetectCommand {
    fn run(&self) -> Result<String> {
        let source = load_path_source(self.target.as_deref(), self.language, BinaryMode::Detect)?;
        let path = source.path.clone();
        let image_bytes = source.image_bytes;
        let mut output = output::DetectOutput::from_detection(source.detection);
        self.apply_vision(&path, image_bytes.as_deref(), &mut output);
        Ok(serde_json::to_string(&output)?)
    }

    fn apply_vision(&self, path: &Path, bytes: Option<&[u8]>, output: &mut output::DetectOutput) {
        let request = crate::engine::vision::Request {
            caption: self.caption,
            objects: self.objects,
            ocr: self.ocr,
        };
        if (!request.caption && !request.objects && !request.ocr) || !output.is_image() {
            return;
        }
        let Some(bytes) = bytes else {
            log::warn!("vision skipped: missing image bytes for {}", path.display());
            return;
        };

        // Resolve the cache root from CWD (or an ancestor); None disables caching.
        let readseek_dir = std::env::current_dir()
            .ok()
            .and_then(|cwd| repo::find_readseek_dir(&cwd));
        let hash = crate::engine::hash::hash_bytes(bytes);

        let mut entry = readseek_dir
            .as_deref()
            .and_then(|dir| vision_cache::load(dir, &hash))
            .unwrap_or_else(vision_cache::CacheEntry::new_empty);

        // Run only the requested tasks that are not already cached.
        let missing = crate::engine::vision::Request {
            caption: request.caption && entry.caption.is_none(),
            objects: request.objects && entry.objects.is_none(),
            ocr: request.ocr && entry.ocr.is_none(),
        };
        if missing.caption || missing.objects || missing.ocr {
            match crate::engine::vision::analyze(bytes, missing) {
                Ok(analysis) => {
                    if missing.caption {
                        entry.caption = analysis.caption;
                    }
                    if missing.objects {
                        entry.objects = analysis.objects;
                    }
                    if missing.ocr {
                        entry.ocr = analysis.ocr;
                    }
                    if let Some(dir) = readseek_dir.as_deref() {
                        vision_cache::store(dir, &hash, &entry);
                    }
                }
                Err(error) => log::warn!("vision skipped: {error:#}"),
            }
        }

        output.set_analysis(crate::engine::vision::Analysis {
            caption: if request.caption { entry.caption } else { None },
            objects: if request.objects { entry.objects } else { None },
            ocr: if request.ocr { entry.ocr } else { None },
        });
    }
}

impl cli::ReadCommand {
    fn run(&self) -> Result<String> {
        let (target, source) = load_source(
            self.target.as_deref(),
            false,
            self.language,
            BinaryMode::Lossy,
        )?;
        let start = output::resolve_target(&source, &target)?;

        if self.end.is_some() && self.limit.is_some() {
            bail!("cannot combine --end with --limit");
        }

        let end = if let Some(limit) = self.limit {
            if limit == 0 {
                bail!("limit must be greater than zero");
            }
            let start_line = start.unwrap_or(1);
            Some(
                start_line
                    .checked_add(limit - 1)
                    .context("read range exceeds supported line numbers")?,
            )
        } else {
            self.end
        };
        let output = output::read_output(&source, start, end)?;
        Ok(serde_json::to_string(&output)?)
    }
}

impl cli::MapCommand {
    fn run(&self) -> Result<String> {
        let source = load_path_source(self.target.as_deref(), self.language, BinaryMode::Reject)?;
        Ok(serde_json::to_string(&output::map_output(&source)?)?)
    }
}

impl cli::CheckCommand {
    fn run(&self) -> Result<String> {
        let source = load_path_source(self.target.as_deref(), self.language, BinaryMode::Reject)?;
        Ok(serde_json::to_string(&output::check_output(&source)?)?)
    }
}

impl cli::SymbolCommand {
    fn run(&self) -> Result<String> {
        let (target, source) = load_source(
            self.target.as_deref(),
            self.name,
            self.language,
            BinaryMode::Reject,
        )?;
        let address = if let Some(crate::engine::target::TargetAddress::Name(name)) =
            target.address.as_ref()
        {
            output::SymbolAddress::Name(name)
        } else {
            if self.name {
                bail!("--name requires a target name suffix; use <target>:<name> --name");
            }
            let target_line = output::resolve_target(&source, &target)?;
            match target_line {
                Some(line) => output::SymbolAddress::Line(line),
                None => bail!("symbol requires a name or target line/hash"),
            }
        };
        let output = output::symbol_output(&source, address)?;
        Ok(serde_json::to_string(&output)?)
    }
}

impl cli::IdentifyCommand {
    fn run(&self) -> Result<String> {
        let (target, source) = load_source(
            self.target.as_deref(),
            false,
            self.language,
            BinaryMode::Reject,
        )?;
        let target_line = output::resolve_target(&source, &target)?;
        let output = output::identify_output(&source, target_line, self.column)?;
        Ok(serde_json::to_string(&output)?)
    }
}

impl cli::DefCommand {
    fn run(self) -> Result<String> {
        let flags = self.git_flags();
        let request = def::Request {
            target: self.target,
            name: self.name,
            language: self.language,
            flags,
        };
        let output = def::output(&request)?;
        match self.format {
            output::Format::Plain => Ok(serde_json::to_string(&def::compact(&output))?),
            output::Format::Json => Ok(serde_json::to_string(&output)?),
        }
    }
}

impl cli::RefsCommand {
    fn run(self) -> Result<String> {
        let flags = self.git_flags();
        let request = refs::Request {
            target: self.target,
            name: self.name,
            scope: self.scope,
            line: self.line,
            column: self.column,
            language: self.language,
            flags,
        };
        let output = refs::output(&request)?;
        match self.format {
            output::Format::Plain => Ok(serde_json::to_string(&refs::compact(&output))?),
            output::Format::Json => Ok(serde_json::to_string(&output)?),
        }
    }
}

impl cli::RenameCommand {
    fn run(self) -> Result<String> {
        let flags = self.git_flags();
        let request = rename::Request {
            target: self.target,
            line: self.line,
            column: self.column,
            to: self.to,
            workspace: self.workspace,
            apply: self.apply,
            language: self.language,
            flags,
        };
        Ok(serde_json::to_string(&rename::output(&request)?)?)
    }
}

impl cli::SearchCommand {
    fn run(&self) -> Result<String> {
        let paths = command_paths(&self.target, self.git_flags())?;
        let mut pattern = crate::engine::search::compile_search(&self.pattern);
        if let Some(language) = self
            .language
            .and_then(crate::engine::symbols::tree_sitter_language)
        {
            crate::engine::search::prepare_tree(&mut pattern, &language);
        }

        let results: Vec<_> = paths
            .par_iter()
            .map_init(Parser::new, |parser, path| {
                crate::engine::search::search_file(path, self.language, &pattern, parser)
                    .map(|result| result.filter(|result| !result.matches.is_empty()))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(serde_json::to_string(&SearchOutput { results })?)
    }
}

impl cli::InitCommand {
    fn run(&self) -> Result<()> {
        let path = self.path.as_deref().unwrap_or(Path::new("."));
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
}

fn load_source(
    target_str: Option<&str>,
    name_mode: bool,
    language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<(Target, SourceFile)> {
    let target = crate::cli::parse_target(target_str.context("target required")?, name_mode)?;
    let source = output::load_source_for_input(&target, language, binary_mode)?;
    Ok((target, source))
}

fn load_path_source(
    target_str: Option<&str>,
    language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    let target = crate::cli::parse_target(target_str.context("target required")?, false)?;
    if target.address.is_some() {
        bail!("this command takes a file path, not a line or hash suffix");
    }
    output::load_source_for_input(&target, language, binary_mode)
}

fn write_output(json: &str, path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        std::fs::write(path, json).with_context(|| format!("write {}", path.display()))
    } else {
        println!("{json}");
        Ok(())
    }
}
