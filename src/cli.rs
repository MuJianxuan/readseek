// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::lang::{LANGUAGE_SPECS, Language};
use crate::engine::target::{Target, TargetAddress};
use anyhow::{Context, Result, bail};
use argh::FromArgs;
use std::path::{Path, PathBuf};

pub(crate) mod run;

/// readseek
#[derive(Debug, FromArgs)]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct Cli {
    /// print version and exit
    #[argh(switch, short = 'V')]
    pub(crate) version: bool,

    /// write output to file instead of stdout
    #[argh(option, long = "output")]
    pub(crate) output: Option<PathBuf>,

    /// use the given .readseek directory instead of discovering one
    #[argh(option, long = "readseek-dir")]
    pub(crate) readseek_dir: Option<PathBuf>,

    /// command to run
    #[argh(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
pub(crate) enum Command {
    Detect(DetectCommand),
    Read(ReadCommand),
    Map(MapCommand),
    Check(CheckCommand),
    Symbol(SymbolCommand),
    Identify(IdentifyCommand),
    Def(DefCommand),
    Refs(RefsCommand),
    Rename(RenameCommand),
    Search(SearchCommand),
    Init(InitCommand),
}

/// detect the file type
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "detect")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct DetectCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin as the given path
    #[argh(option)]
    pub(crate) stdin: Option<PathBuf>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// read and hash from a line range
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "read")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct ReadCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin as the given path
    #[argh(option)]
    pub(crate) stdin: Option<PathBuf>,

    /// first line to include
    #[argh(option)]
    pub(crate) start: Option<usize>,

    /// last line to include
    #[argh(option)]
    pub(crate) end: Option<usize>,

    /// maximum number of lines to include
    #[argh(option)]
    pub(crate) limit: Option<usize>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// map a file to symbols
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "map")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct MapCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin as the given path
    #[argh(option)]
    pub(crate) stdin: Option<PathBuf>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// report parse diagnostics for a file
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "check")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct CheckCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin as the given path
    #[argh(option)]
    pub(crate) stdin: Option<PathBuf>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// read the line range for a symbol
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "symbol")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct SymbolCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin as the given path
    #[argh(option)]
    pub(crate) stdin: Option<PathBuf>,

    /// one-based target line
    #[argh(option)]
    pub(crate) line: Option<usize>,

    /// qualified symbol name
    #[argh(option)]
    pub(crate) name: Option<String>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// identify the cursor token and enclosing symbol
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "identify")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct IdentifyCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin as the given path
    #[argh(option)]
    pub(crate) stdin: Option<PathBuf>,

    /// one-based cursor line
    #[argh(option)]
    pub(crate) line: Option<usize>,

    /// one-based cursor byte column
    #[argh(option)]
    pub(crate) column: Option<usize>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// find structural symbol definitions
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "def")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct DefCommand {
    /// file or directory to search
    #[argh(positional)]
    pub(crate) target: PathBuf,

    /// qualified symbol name or unqualified name
    #[argh(positional)]
    pub(crate) name: Option<String>,

    /// read identify output from stdin to choose the symbol name
    #[argh(switch)]
    pub(crate) from_identify: bool,

    /// output format
    #[argh(
        option,
        long = "format",
        default = "crate::engine::output::Format::Json"
    )]
    pub(crate) format: crate::engine::output::Format,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,

    /// search tracked/indexed files when searching a Git repository
    #[argh(switch, short = 'c')]
    pub(crate) cached: bool,

    /// search untracked files when searching a Git repository
    #[argh(switch, short = 'o')]
    pub(crate) others: bool,

    /// include ignored untracked files when searching a Git repository
    #[argh(switch, short = 'i')]
    pub(crate) ignored: bool,
}

/// find identifier references
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "refs")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RefsCommand {
    /// file or directory to search
    #[argh(positional)]
    pub(crate) target: PathBuf,

    /// identifier to search for
    #[argh(positional)]
    pub(crate) name: String,

    /// output format
    #[argh(
        option,
        long = "format",
        default = "crate::engine::output::Format::Json"
    )]
    pub(crate) format: crate::engine::output::Format,

    /// restrict results to the binding under --line/--column (single file)
    #[argh(switch)]
    pub(crate) scope: bool,

    /// one-based cursor line, used with --scope
    #[argh(option)]
    pub(crate) line: Option<usize>,

    /// one-based cursor byte column, used with --scope
    #[argh(option)]
    pub(crate) column: Option<usize>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,

    /// search tracked/indexed files when searching a Git repository
    #[argh(switch, short = 'c')]
    pub(crate) cached: bool,

    /// search untracked files when searching a Git repository
    #[argh(switch, short = 'o')]
    pub(crate) others: bool,

    /// include ignored untracked files when searching a Git repository
    #[argh(switch, short = 'i')]
    pub(crate) ignored: bool,
}

/// plan a binding-accurate rename within a single file
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "rename")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RenameCommand {
    /// file holding the binding under the cursor (a single regular file)
    #[argh(positional)]
    pub(crate) target: PathBuf,

    /// one-based cursor line of the binding to rename
    #[argh(option)]
    pub(crate) line: usize,

    /// one-based cursor byte column of the binding to rename
    #[argh(option)]
    pub(crate) column: Option<usize>,

    /// new name for the binding
    #[argh(option, long = "to")]
    pub(crate) to: String,

    /// expand the rename across this directory or repository (name-based
    /// outside the cursor file); omit for a single-file rename
    #[argh(option)]
    pub(crate) workspace: Option<PathBuf>,

    /// write the planned edits to the file after verifying line hashes
    #[argh(switch)]
    pub(crate) apply: bool,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,

    /// search tracked/indexed files when expanding across a Git repository
    #[argh(switch, short = 'c')]
    pub(crate) cached: bool,

    /// search untracked files when expanding across a Git repository
    #[argh(switch, short = 'o')]
    pub(crate) others: bool,

    /// include ignored untracked files when expanding across a Git repository
    #[argh(switch, short = 'i')]
    pub(crate) ignored: bool,
}

/// search files with an AST pattern
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "search")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct SearchCommand {
    /// file or directory to search
    #[argh(positional)]
    pub(crate) target: PathBuf,

    /// ast-grep-style pattern
    #[argh(positional)]
    pub(crate) pattern: String,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,

    /// search tracked/indexed files when searching a Git repository
    #[argh(switch, short = 'c')]
    pub(crate) cached: bool,

    /// search untracked files when searching a Git repository
    #[argh(switch, short = 'o')]
    pub(crate) others: bool,

    /// include ignored untracked files when searching a Git repository
    #[argh(switch, short = 'i')]
    pub(crate) ignored: bool,
}

/// initialize .readseek/ directory
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "init")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct InitCommand {
    /// path to a directory (defaults to current directory)
    #[argh(positional)]
    pub(crate) path: Option<PathBuf>,
}

pub(crate) fn parse_language(value: &str) -> std::result::Result<Language, String> {
    if let Ok(language) = value.parse::<Language>() {
        return Ok(language);
    }

    let alias = value.to_ascii_lowercase().replace(['-', '_'], "");
    LANGUAGE_SPECS
        .iter()
        .find_map(|spec| {
            spec.aliases
                .contains(&alias.as_str())
                .then_some(spec.language)
        })
        .ok_or_else(|| format!("unknown language `{value}`"))
}

fn parse_input_target_with(
    target: Option<&str>,
    stdin: Option<&Path>,
    parse: fn(&str) -> Result<Target>,
) -> Result<Target> {
    if let Some(path) = stdin {
        if target.is_some() {
            bail!("target cannot be combined with --stdin");
        }
        return Ok(Target {
            path: path.to_path_buf(),
            address: None,
        });
    }
    parse(target.context("target required")?)
}

fn parse_target(value: &str) -> Result<Target> {
    if value.is_empty() {
        bail!("target must not be empty");
    }

    if let Some((path, suffix)) = value.rsplit_once(':') {
        if path.is_empty() {
            bail!("target path must not be empty");
        }
        if suffix.chars().all(|ch| ch.is_ascii_digit()) {
            let line = suffix
                .parse::<usize>()
                .with_context(|| format!("invalid target line: {suffix}"))?;
            if line == 0 {
                bail!("target line must be greater than zero");
            }
            return Ok(Target {
                path: PathBuf::from(path),
                address: Some(TargetAddress::Line(line)),
            });
        }
        if is_line_hash(suffix) {
            return Ok(Target {
                path: PathBuf::from(path),
                address: Some(TargetAddress::Hash(suffix.to_ascii_lowercase())),
            });
        }
    }

    Ok(Target {
        path: PathBuf::from(value),
        address: None,
    })
}

fn is_line_hash(value: &str) -> bool {
    value.len() == 3 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub(crate) struct InputArgs<'a> {
    pub(crate) target: Option<&'a str>,
    pub(crate) stdin: Option<&'a Path>,
    pub(crate) language: Option<Language>,
}

impl InputArgs<'_> {
    pub(crate) fn to_target(&self) -> Result<Target> {
        parse_input_target_with(self.target, self.stdin, parse_target)
    }
}

pub(crate) trait Input {
    fn input(&self) -> InputArgs<'_>;
}

macro_rules! impl_input {
    ($($command:ty),+ $(,)?) => {
        $(
            impl Input for $command {
                fn input(&self) -> InputArgs<'_> {
                    InputArgs {
                        target: self.target.as_deref(),
                        stdin: self.stdin.as_deref(),
                        language: self.language,
                    }
                }
            }
        )+
    };
}

impl_input!(
    DetectCommand,
    ReadCommand,
    MapCommand,
    CheckCommand,
    SymbolCommand,
    IdentifyCommand,
);
