// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::{Language, LANGUAGE_SPECS, Target, TargetAddress};
use anyhow::{Context, Result, bail};
use argh::FromArgs;
use std::path::{Path, PathBuf};

/// readseek
#[derive(Debug, FromArgs)]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct Cli {
    /// print version and exit
    #[argh(switch, short = 'V')]
    pub(crate) version: bool,

    /// command to run
    #[argh(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
pub(crate) enum Command {
    Detect(FileCommand),
    Read(ReadCommand),
    Map(MapCommand),
    Symbol(SymbolCommand),
    Identify(IdentifyCommand),
    Definition(DefinitionCommand),
    References(ReferencesCommand),
    Search(SearchCommand),
}

/// detect the file type
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "file")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct FileCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    pub(crate) target: Option<String>,

    /// read document contents from stdin
    #[argh(switch)]
    pub(crate) stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    pub(crate) path: Option<PathBuf>,

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

    /// read document contents from stdin
    #[argh(switch)]
    pub(crate) stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    pub(crate) path: Option<PathBuf>,

    /// first line to include
    #[argh(option)]
    pub(crate) start: Option<usize>,

    /// last line to include
    #[argh(option)]
    pub(crate) end: Option<usize>,

    /// first line to include (alias for --start)
    #[argh(option)]
    pub(crate) offset: Option<usize>,

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

    /// read document contents from stdin
    #[argh(switch)]
    pub(crate) stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    pub(crate) path: Option<PathBuf>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    pub(crate) language: Option<Language>,
}

/// read the line range for a symbol
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "symbol")]
#[argh(help_triggers("-h", "--help"))]
pub(crate) struct SymbolCommand {
    /// takes [<file>, <file>:<line>, <file>:<hash> or <file>:<symbol>] [qualified-name]
    #[argh(positional)]
    pub(crate) args: Vec<String>,

    /// read document contents from stdin
    #[argh(switch)]
    pub(crate) stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    pub(crate) path: Option<PathBuf>,

    /// one-based target line
    #[argh(option)]
    pub(crate) line: Option<usize>,

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

    /// read document contents from stdin
    #[argh(switch)]
    pub(crate) stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    pub(crate) path: Option<PathBuf>,

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
#[argh(subcommand, name = "definition")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct DefinitionCommand {
    /// file or directory to search
    #[argh(positional)]
    pub(crate) target: PathBuf,

    /// qualified symbol name or unqualified name
    #[argh(positional)]
    pub(crate) name: Option<String>,

    /// read identify output from stdin to choose the symbol name
    #[argh(switch)]
    pub(crate) stdin: bool,

    /// emit flat quickfix-friendly locations
    #[argh(switch)]
    pub(crate) compact: bool,

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
#[argh(subcommand, name = "references")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct ReferencesCommand {
    /// file or directory to search
    #[argh(positional)]
    pub(crate) target: PathBuf,

    /// identifier to search for
    #[argh(positional)]
    pub(crate) name: String,

    /// emit flat quickfix-friendly locations
    #[argh(switch)]
    pub(crate) compact: bool,

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

pub(crate) fn parse_language(value: &str) -> std::result::Result<Language, String> {
    let alias = value.to_ascii_lowercase().replace(['-', '_'], "");
    if alias == "unknown" {
        return Ok(Language::Unknown);
    }

    LANGUAGE_SPECS
        .iter()
        .find_map(|spec| {
            spec.aliases
                .contains(&alias.as_str())
                .then_some(spec.language)
        })
        .ok_or_else(|| format!("unknown language: {value}"))
}

pub(crate) fn parse_input_target(
    target: Option<&str>,
    stdin: bool,
    path: Option<&Path>,
) -> Result<Target> {
    parse_input_target_with(target, stdin, path, parse_target)
}

pub(crate) fn parse_symbol_input_target(
    target: Option<&str>,
    stdin: bool,
    path: Option<&Path>,
) -> Result<Target> {
    parse_input_target_with(target, stdin, path, parse_symbol_target)
}

fn parse_input_target_with(
    target: Option<&str>,
    stdin: bool,
    path: Option<&Path>,
    parse: fn(&str) -> Result<Target>,
) -> Result<Target> {
    if stdin {
        if target.is_some() {
            bail!("target cannot be combined with --stdin");
        }
        let path = path.context("--stdin requires --path")?;
        return Ok(Target {
            path: path.to_path_buf(),
            address: None,
        });
    }
    if path.is_some() {
        bail!("--path requires --stdin");
    }
    parse(target.context("target required")?)
}

pub(crate) fn symbol_args(
    args: &[String],
    stdin: bool,
) -> Result<(Option<&str>, Option<&str>)> {
    match (stdin, args) {
        (true, []) => Ok((None, None)),
        (true, [address]) => Ok((None, Some(address.as_str()))),
        (true, _) => bail!("symbol with --stdin accepts at most one qualified name argument"),
        (false, [target]) => Ok((Some(target.as_str()), None)),
        (false, [target, address]) => Ok((Some(target.as_str()), Some(address.as_str()))),
        (false, []) => bail!("target required"),
        (false, _) => bail!("symbol accepts at most target and qualified name arguments"),
    }
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

fn parse_symbol_target(value: &str) -> Result<Target> {
    let target = parse_target(value)?;
    if target.address.is_some() || Path::new(value).exists() {
        return Ok(target);
    }

    let Some((path, symbol)) = value.rsplit_once(':') else {
        return Ok(target);
    };
    if path.is_empty() || symbol.is_empty() {
        return Ok(target);
    }

    Ok(Target {
        path: PathBuf::from(path),
        address: Some(TargetAddress::Symbol(symbol.to_owned())),
    })
}

fn is_line_hash(value: &str) -> bool {
    value.len() == 3 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}
