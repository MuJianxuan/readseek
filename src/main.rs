// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::{Context, Result, bail};
use argh::FromArgs;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::{env, process};
use strum_macros::{Display, EnumString};
use syntect::parsing::SyntaxSet;
use tree_sitter::{Node, Parser};

mod cache;
mod symbols;

/// readseek
#[derive(Debug, FromArgs)]
#[argh(help_triggers("-h", "--help"))]
struct Cli {
    /// print version and exit
    #[argh(switch, short = 'V')]
    version: bool,

    /// command to run
    #[argh(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Command {
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
struct FileCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    target: Option<String>,

    /// read document contents from stdin
    #[argh(switch)]
    stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    path: Option<PathBuf>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
}

/// read and hash from a line range
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "read")]
#[argh(help_triggers("-h", "--help"))]
struct ReadCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    target: Option<String>,

    /// read document contents from stdin
    #[argh(switch)]
    stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    path: Option<PathBuf>,

    /// first line to include
    #[argh(option)]
    start: Option<usize>,

    /// last line to include
    #[argh(option)]
    end: Option<usize>,

    /// first line to include (alias for --start)
    #[argh(option)]
    offset: Option<usize>,

    /// maximum number of lines to include
    #[argh(option)]
    limit: Option<usize>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
}

/// map a file to symbols
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "map")]
#[argh(help_triggers("-h", "--help"))]
struct MapCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    target: Option<String>,

    /// read document contents from stdin
    #[argh(switch)]
    stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    path: Option<PathBuf>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
}

/// read the line range for a symbol
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "symbol")]
#[argh(help_triggers("-h", "--help"))]
struct SymbolCommand {
    /// takes [<file>, <file>:<line>, <file>:<hash> or <file>:<symbol>] [qualified-name]
    #[argh(positional)]
    args: Vec<String>,

    /// read document contents from stdin
    #[argh(switch)]
    stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    path: Option<PathBuf>,

    /// one-based target line
    #[argh(option)]
    line: Option<usize>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
}

/// identify the cursor token and enclosing symbol
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "identify")]
#[argh(help_triggers("-h", "--help"))]
struct IdentifyCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    target: Option<String>,

    /// read document contents from stdin
    #[argh(switch)]
    stdin: bool,

    /// document path to use with --stdin
    #[argh(option)]
    path: Option<PathBuf>,

    /// one-based cursor line
    #[argh(option)]
    line: Option<usize>,

    /// one-based cursor byte column
    #[argh(option)]
    column: Option<usize>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
}

/// find structural symbol definitions
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "definition")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
struct DefinitionCommand {
    /// file or directory to search
    #[argh(positional)]
    target: PathBuf,

    /// qualified symbol name or unqualified name
    #[argh(positional)]
    name: Option<String>,

    /// read identify output from stdin to choose the symbol name
    #[argh(switch)]
    stdin: bool,

    /// emit flat quickfix-friendly locations
    #[argh(switch)]
    compact: bool,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,

    /// search tracked/indexed files when searching a Git repository
    #[argh(switch, short = 'c')]
    cached: bool,

    /// search untracked files when searching a Git repository
    #[argh(switch, short = 'o')]
    others: bool,

    /// include ignored untracked files when searching a Git repository
    #[argh(switch, short = 'i')]
    ignored: bool,
}

/// find identifier references
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "references")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
struct ReferencesCommand {
    /// file or directory to search
    #[argh(positional)]
    target: PathBuf,

    /// identifier to search for
    #[argh(positional)]
    name: String,

    /// emit flat quickfix-friendly locations
    #[argh(switch)]
    compact: bool,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,

    /// search tracked/indexed files when searching a Git repository
    #[argh(switch, short = 'c')]
    cached: bool,

    /// search untracked files when searching a Git repository
    #[argh(switch, short = 'o')]
    others: bool,

    /// include ignored untracked files when searching a Git repository
    #[argh(switch, short = 'i')]
    ignored: bool,
}

/// search files with an AST pattern
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "search")]
#[argh(help_triggers("-h", "--help"))]
#[allow(clippy::struct_excessive_bools)]
struct SearchCommand {
    /// file or directory to search
    #[argh(positional)]
    target: PathBuf,

    /// ast-grep-style pattern
    #[argh(positional)]
    pattern: String,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,

    /// search tracked/indexed files when searching a Git repository
    #[argh(switch, short = 'c')]
    cached: bool,

    /// search untracked files when searching a Git repository
    #[argh(switch, short = 'o')]
    others: bool,

    /// include ignored untracked files when searching a Git repository
    #[argh(switch, short = 'i')]
    ignored: bool,
}

#[derive(Clone, Copy, Debug, Display, EnumString, Eq, PartialEq)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
enum Language {
    Assembly,
    C,
    Bash,
    Cpp,
    CSharp,
    Css,
    Dockerfile,
    Go,
    Gdscript,
    Java,
    JavaScript,
    Jsx,
    Html,
    Json,
    Kconfig,
    Latex,
    Lua,
    Markdown,
    Xml,
    Yaml,
    Just,
    Make,
    Meson,
    Nix,
    Perl,
    Python,
    Php,
    Puppet,
    Ruby,
    Riscv,
    Rust,
    Swift,
    Sql,
    TypeScript,
    Typst,
    Toml,
    Tsx,
    Vimscript,
    Zig,
    Unknown,
}

impl Language {
    fn id(self) -> &'static str {
        language_spec(self).map_or("unknown", |spec| spec.id)
    }
}

#[derive(Clone, Copy)]
struct LanguageSpec {
    language: Language,
    id: &'static str,
    engine: AnalysisEngine,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    syntax_names: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum AnalysisEngine {
    TreeSitter,
    #[allow(dead_code)]
    Llvm,
    None,
}

impl AnalysisEngine {
    const fn id(self) -> &'static str {
        match self {
            Self::TreeSitter => "tree-sitter",
            Self::Llvm => "llvm",
            Self::None => "none",
        }
    }
}

const LANGUAGE_SPECS: &[LanguageSpec] = &[
    LanguageSpec {
        language: Language::Assembly,
        id: "assembly",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["assembly", "asm", "x86", "arm"],
        extensions: &["asm", "s", "S"],
        file_names: &[],
        syntax_names: &["Assembly", "ARM Assembly", "x86 Assembly"],
    },
    LanguageSpec {
        language: Language::C,
        id: "c",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["c"],
        extensions: &["c", "h"],
        file_names: &[],
        syntax_names: &["C"],
    },
    LanguageSpec {
        language: Language::Bash,
        id: "bash",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["bash", "sh", "shell"],
        extensions: &["bash", "sh"],
        file_names: &[".bashrc", ".bash_profile", ".profile"],
        syntax_names: &[
            "Bourne Again Shell (bash)",
            "Shell-Unix-Generic",
            "ShellScript",
        ],
    },
    LanguageSpec {
        language: Language::Cpp,
        id: "cpp",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["cpp", "cxx", "cplusplus"],
        extensions: &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
        file_names: &[],
        syntax_names: &["C++"],
    },
    LanguageSpec {
        language: Language::CSharp,
        id: "csharp",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["csharp", "cs", "c#"],
        extensions: &["cs"],
        file_names: &[],
        syntax_names: &["C#"],
    },
    LanguageSpec {
        language: Language::Css,
        id: "css",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["css"],
        extensions: &["css"],
        file_names: &[],
        syntax_names: &["CSS"],
    },
    LanguageSpec {
        language: Language::Dockerfile,
        id: "dockerfile",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["dockerfile", "containerfile"],
        extensions: &[],
        file_names: &["Dockerfile", "Containerfile"],
        syntax_names: &["Dockerfile"],
    },
    LanguageSpec {
        language: Language::Go,
        id: "go",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["go", "golang"],
        extensions: &["go"],
        file_names: &["go.mod"],
        syntax_names: &["Go"],
    },
    LanguageSpec {
        language: Language::Gdscript,
        id: "gdscript",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["gdscript", "gd"],
        extensions: &["gd"],
        file_names: &[],
        syntax_names: &["GDScript"],
    },
    LanguageSpec {
        language: Language::Java,
        id: "java",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["java"],
        extensions: &["java"],
        file_names: &[],
        syntax_names: &["Java"],
    },
    LanguageSpec {
        language: Language::JavaScript,
        id: "javascript",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["javascript", "js"],
        extensions: &["js", "mjs", "cjs"],
        file_names: &[],
        syntax_names: &["JavaScript"],
    },
    LanguageSpec {
        language: Language::Jsx,
        id: "jsx",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["jsx"],
        extensions: &["jsx"],
        file_names: &[],
        syntax_names: &[],
    },
    LanguageSpec {
        language: Language::Html,
        id: "html",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["html", "htm"],
        extensions: &["html", "htm"],
        file_names: &[],
        syntax_names: &["HTML"],
    },
    LanguageSpec {
        language: Language::Json,
        id: "json",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["json"],
        extensions: &["json"],
        file_names: &["package-lock.json", "composer.lock"],
        syntax_names: &["JSON"],
    },
    LanguageSpec {
        language: Language::Xml,
        id: "xml",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["xml"],
        extensions: &["xml", "xsd", "xsl", "xslt"],
        file_names: &[],
        syntax_names: &["XML"],
    },
    LanguageSpec {
        language: Language::Yaml,
        id: "yaml",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["yaml", "yml"],
        extensions: &["yaml", "yml"],
        file_names: &[],
        syntax_names: &["YAML"],
    },
    LanguageSpec {
        language: Language::Just,
        id: "just",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["just", "justfile"],
        extensions: &["just"],
        file_names: &["justfile", "Justfile", ".justfile"],
        syntax_names: &["Just"],
    },
    LanguageSpec {
        language: Language::Kconfig,
        id: "kconfig",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["kconfig"],
        extensions: &[],
        file_names: &["Kconfig"],
        syntax_names: &["Kconfig"],
    },
    LanguageSpec {
        language: Language::Make,
        id: "make",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["make", "makefile"],
        extensions: &["mk", "mak", "make"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        syntax_names: &["Makefile"],
    },
    LanguageSpec {
        language: Language::Latex,
        id: "latex",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["latex", "tex"],
        extensions: &["tex", "ltx", "latex"],
        file_names: &[],
        syntax_names: &["LaTeX", "TeX"],
    },
    LanguageSpec {
        language: Language::Lua,
        id: "lua",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["lua"],
        extensions: &["lua"],
        file_names: &[],
        syntax_names: &["Lua"],
    },
    LanguageSpec {
        language: Language::Markdown,
        id: "markdown",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["markdown", "md"],
        extensions: &["md", "markdown", "mdown", "mkd"],
        file_names: &[],
        syntax_names: &["Markdown"],
    },
    LanguageSpec {
        language: Language::Meson,
        id: "meson",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["meson"],
        extensions: &[],
        file_names: &["meson.build", "meson_options.txt"],
        syntax_names: &["Meson"],
    },
    LanguageSpec {
        language: Language::Nix,
        id: "nix",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["nix"],
        extensions: &["nix"],
        file_names: &["flake.lock"],
        syntax_names: &["Nix"],
    },
    LanguageSpec {
        language: Language::Perl,
        id: "perl",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["perl", "pl", "pm"],
        extensions: &["pl", "pm", "t"],
        file_names: &[],
        syntax_names: &["Perl"],
    },
    LanguageSpec {
        language: Language::Python,
        id: "python",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["python", "py"],
        extensions: &["py", "pyw"],
        file_names: &[],
        syntax_names: &["Python"],
    },
    LanguageSpec {
        language: Language::Php,
        id: "php",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["php"],
        extensions: &["php", "php3", "php4", "php5", "phtml"],
        file_names: &[],
        syntax_names: &["PHP"],
    },
    LanguageSpec {
        language: Language::Puppet,
        id: "puppet",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["puppet", "pp"],
        extensions: &["pp"],
        file_names: &["Puppetfile"],
        syntax_names: &["Puppet"],
    },
    LanguageSpec {
        language: Language::Ruby,
        id: "ruby",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["ruby", "rb"],
        extensions: &["rb", "rake", "gemspec"],
        file_names: &["Gemfile", "Rakefile"],
        syntax_names: &["Ruby"],
    },
    LanguageSpec {
        language: Language::Riscv,
        id: "riscv",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["riscv", "risc-v", "riscv64"],
        extensions: &["riscv"],
        file_names: &[],
        syntax_names: &["RISC-V"],
    },
    LanguageSpec {
        language: Language::Rust,
        id: "rust",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["rust", "rs"],
        extensions: &["rs"],
        file_names: &[],
        syntax_names: &["Rust"],
    },
    LanguageSpec {
        language: Language::Swift,
        id: "swift",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["swift"],
        extensions: &["swift"],
        file_names: &[],
        syntax_names: &["Swift"],
    },
    LanguageSpec {
        language: Language::Sql,
        id: "sql",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["sql"],
        extensions: &["sql"],
        file_names: &[],
        syntax_names: &["SQL"],
    },
    LanguageSpec {
        language: Language::TypeScript,
        id: "typescript",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["typescript", "ts"],
        extensions: &["ts", "mts", "cts"],
        file_names: &[],
        syntax_names: &["TypeScript"],
    },
    LanguageSpec {
        language: Language::Tsx,
        id: "tsx",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["tsx"],
        extensions: &["tsx"],
        file_names: &[],
        syntax_names: &[],
    },
    LanguageSpec {
        language: Language::Toml,
        id: "toml",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["toml"],
        extensions: &["toml"],
        file_names: &["Cargo.lock"],
        syntax_names: &["TOML"],
    },
    LanguageSpec {
        language: Language::Typst,
        id: "typst",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["typst", "typ"],
        extensions: &["typ"],
        file_names: &[],
        syntax_names: &["Typst"],
    },
    LanguageSpec {
        language: Language::Vimscript,
        id: "vimscript",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["vimscript", "vim", "viml"],
        extensions: &["vim", "vimrc", "gvimrc"],
        file_names: &[".vimrc", ".gvimrc"],
        syntax_names: &["VimL", "Vimscript"],
    },
    LanguageSpec {
        language: Language::Zig,
        id: "zig",
        engine: AnalysisEngine::TreeSitter,
        aliases: &["zig"],
        extensions: &["zig", "zon"],
        file_names: &["build.zig", "build.zig.zon"],
        syntax_names: &["Zig"],
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinaryMode {
    Reject,
    Lossy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentKind {
    Source,
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentFormat {
    PlainText,
}

impl DocumentFormat {
    const fn id(self) -> &'static str {
        match self {
            Self::PlainText => "plain-text",
        }
    }
}

#[derive(Clone, Copy)]
struct DocumentExtractor {
    format: DocumentFormat,
    extensions: &'static [&'static str],
    mime_prefixes: &'static [&'static str],
    extract: fn(&Path, &[u8], BinaryMode) -> Result<String>,
}

const DOCUMENT_EXTRACTORS: &[DocumentExtractor] = &[DocumentExtractor {
    format: DocumentFormat::PlainText,
    extensions: &["txt"],
    mime_prefixes: &["text/"],
    extract: extract_plain_text,
}];

impl Serialize for Language {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.id())
    }
}

#[derive(Debug, Serialize)]
struct Detection {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    supported: bool,
    binary: bool,
    mime: Option<String>,
    syntax: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    start_line: usize,
    end_line: usize,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
struct MapOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize)]
struct SymbolOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    symbol: Symbol,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
struct IdentifyOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    line_count: usize,
    file_hash: String,
    line: usize,
    column: usize,
    line_hash: String,
    hashlines: Vec<HashLine>,
    identifier: Option<IdentifierOutput>,
    symbol: Option<Symbol>,
}

#[derive(Debug, Serialize)]
struct IdentifierOutput {
    text: String,
    start_column: usize,
    end_column: usize,
}

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
struct SearchFileOutput {
    file: PathBuf,
    language: Language,
    engine: AnalysisEngine,
    file_hash: String,
    matches: Vec<SearchMatch>,
}

#[derive(Debug, Serialize)]
struct SearchMatch {
    pattern_index: usize,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
    hashlines: Vec<HashLine>,
    captures: Vec<SearchCapture>,
}

#[derive(Debug, Serialize)]
struct SearchCapture {
    name: String,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
    hashlines: Vec<HashLine>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatternMetaKind {
    Single,
    Variadic,
}

#[derive(Clone, Debug)]
struct PatternMeta {
    placeholder: String,
    name: String,
    kind: PatternMetaKind,
}

#[derive(Debug)]
struct SearchPattern {
    text: String,
    metas: Vec<PatternMeta>,
}

#[derive(Clone, Debug)]
struct SearchCaptureRange {
    name: String,
    text: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Debug, Serialize)]
struct HashLine {
    line: usize,
    hash: String,
    text: String,
}

#[derive(Clone, Debug, Serialize)]
struct Symbol {
    kind: String,
    name: String,
    #[serde(rename = "qualified_name")]
    address: String,
    start_line: usize,
    end_line: usize,
    start_hash: String,
    end_hash: String,
}

#[derive(Debug)]
struct SourceFile {
    path: PathBuf,
    text: String,
    kind: DocumentKind,
    detection: Detection,
    lines: Vec<SourceLine>,
    file_hash: String,
}

#[derive(Debug)]
struct LoadedDocument {
    text: String,
    binary: bool,
    mime: Option<String>,
}

#[derive(Debug)]
struct SourceLine {
    number: usize,
    text: String,
    hash: String,
}

#[derive(Debug)]
struct SourceMap {
    symbols: Vec<Symbol>,
}

#[derive(Debug)]
enum SymbolLookup {
    Found(Symbol),
    NotFound,
    Ambiguous,
}

#[derive(Clone, Debug)]
struct Target {
    path: PathBuf,
    address: Option<TargetAddress>,
}

#[derive(Clone, Debug)]
enum TargetAddress {
    Line(usize),
    Hash(String),
    Symbol(String),
}

const XXHASH32_PRIME_1: u32 = 2_654_435_761;
const XXHASH32_PRIME_2: u32 = 2_246_822_519;
const XXHASH32_PRIME_3: u32 = 3_266_489_917;
const XXHASH32_PRIME_4: u32 = 668_265_263;
const XXHASH32_PRIME_5: u32 = 374_761_393;
const HASHLINE_MODULUS: u32 = 0x1000;

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
    let cli: Cli = argh::from_env();
    if cli.version {
        println!("readseek {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    match cli.command.context("command required")? {
        Command::Detect(command) => {
            let target = parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            print_json(&source.detection)?;
        }
        Command::Read(command) => {
            let target = parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Lossy,
            )?;
            let target_line = resolve_target_line(&source, &target)?;
            let (start, end) = resolve_read_range(&command, target_line)?;
            let output = read_output(&source, start, end)?;
            print_json(&output)?;
        }
        Command::Map(command) => {
            let target = parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            print_json(&map_output(&source)?)?;
        }
        Command::Symbol(command) => {
            let (target_arg, address_arg) = symbol_args(&command.args, command.stdin)?;
            let target =
                parse_symbol_input_target(target_arg, command.stdin, command.path.as_deref())?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            let target_line = resolve_explicit_target_line(&source, &target, command.line)?;
            let target_address = symbol_address(&target, address_arg)?;
            let output = symbol_command_output(&source, target_address, target_line)?;
            print_json(&output)?;
        }
        Command::Identify(command) => {
            let target = parse_input_target(
                command.target.as_deref(),
                command.stdin,
                command.path.as_deref(),
            )?;
            let source = load_source_for_input(
                &target.path,
                command.stdin,
                command.language,
                BinaryMode::Reject,
            )?;
            let target_line = resolve_explicit_target_line(&source, &target, command.line)?;
            let output = identify_output(&source, target_line, command.column)?;
            print_json(&output)?;
        }
        Command::Definition(command) => {
            print_definition_output(&command)?;
        }
        Command::References(command) => {
            print_references_output(&command)?;
        }
        Command::Search(command) => {
            print_json(&search_output(&command)?)?;
        }
    }

    Ok(())
}

fn print_definition_output(command: &DefinitionCommand) -> Result<()> {
    let output = definition_output(command)?;
    if command.compact {
        print_json(&compact_definitions(&output))
    } else {
        print_json(&output)
    }
}

fn print_references_output(command: &ReferencesCommand) -> Result<()> {
    let output = references_output(command)?;
    if command.compact {
        print_json(&compact_references(&output))
    } else {
        print_json(&output)
    }
}

fn parse_language(value: &str) -> std::result::Result<Language, String> {
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

fn parse_input_target(target: Option<&str>, stdin: bool, path: Option<&Path>) -> Result<Target> {
    parse_input_target_with(target, stdin, path, parse_target)
}

fn parse_symbol_input_target(
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

fn symbol_args(args: &[String], stdin: bool) -> Result<(Option<&str>, Option<&str>)> {
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

fn resolve_target_line(source: &SourceFile, target: &Target) -> Result<Option<usize>> {
    match target.address.as_ref() {
        Some(TargetAddress::Line(line)) => Ok(Some(*line)),
        Some(TargetAddress::Hash(hash)) => source
            .lines
            .iter()
            .find_map(|line| (line.hash == *hash).then_some(line.number))
            .with_context(|| format!("hash {hash} not found in {}", source.path.display()))
            .map(Some),
        None | Some(TargetAddress::Symbol(_)) => Ok(None),
    }
}

fn resolve_explicit_target_line(
    source: &SourceFile,
    target: &Target,
    line: Option<usize>,
) -> Result<Option<usize>> {
    if matches!(target.address, Some(TargetAddress::Symbol(_))) {
        return resolve_target_line(source, target);
    }
    let target_line = resolve_target_line(source, target)?;
    match (target_line, line) {
        (Some(target_line), Some(line)) if target_line != line => {
            bail!("target line conflicts with --line")
        }
        (Some(line), _) | (_, Some(line)) => Ok(Some(line)),
        (None, None) => Ok(None),
    }
}

fn load_source_for_input(
    path: &Path,
    stdin: bool,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    if stdin {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("read stdin")?;
        return source_from_text(
            path,
            normalize_source_text(&text),
            override_language,
            false,
            None,
        );
    }
    load_source(path, override_language, binary_mode)
}

fn load_source(
    path: &Path,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    let document = load_document(path, binary_mode)?;
    source_from_text(
        path,
        document.text,
        override_language,
        document.binary,
        document.mime,
    )
}

fn source_from_text(
    path: &Path,
    text: String,
    override_language: Option<Language>,
    binary: bool,
    mime: Option<String>,
) -> Result<SourceFile> {
    let path_language = detect_by_path(path);
    let (detected_language, syntax) =
        if binary && override_language.is_none() && path_language.is_none() {
            (Language::Unknown, None)
        } else {
            detect_language(path, &text)?
        };
    let language = override_language.unwrap_or(detected_language);
    let engine = analysis_engine(language);
    let kind = document_kind(language);
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, text)| {
            let number = index + 1;
            SourceLine {
                number,
                text: text.to_owned(),
                hash: hash_line(number, text),
            }
        })
        .collect();
    let file_hash = hash_text(&text);
    let detection = Detection {
        file: path.to_path_buf(),
        language,
        engine,
        supported: language != Language::Unknown,
        binary,
        mime,
        syntax,
    };

    Ok(SourceFile {
        path: path.to_path_buf(),
        text,
        kind,
        detection,
        lines,
        file_hash,
    })
}

fn load_document(path: &Path, binary_mode: BinaryMode) -> Result<LoadedDocument> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mime = infer::get(&bytes).map(|kind| kind.mime_type().to_owned());
    let binary = is_binary_mime(mime.as_deref()) || bytes.contains(&0);
    let extractor = document_extractor(path, mime.as_deref());

    if binary && binary_mode == BinaryMode::Reject {
        bail!(
            "unsupported binary file: {} ({})",
            path.display(),
            mime.as_deref().unwrap_or("unknown mime")
        );
    }

    let text = (extractor.extract)(path, &bytes, binary_mode)
        .with_context(|| format!("extract {} from {}", extractor.format.id(), path.display()))?;

    Ok(LoadedDocument { text, binary, mime })
}

fn document_extractor(path: &Path, mime: Option<&str>) -> &'static DocumentExtractor {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    DOCUMENT_EXTRACTORS
        .iter()
        .find(|extractor| {
            extension
                .as_deref()
                .is_some_and(|extension| extractor.extensions.contains(&extension))
                || mime.is_some_and(|mime| {
                    extractor
                        .mime_prefixes
                        .iter()
                        .any(|prefix| mime.starts_with(prefix))
                })
        })
        .unwrap_or(&DOCUMENT_EXTRACTORS[0])
}

fn extract_plain_text(path: &Path, bytes: &[u8], binary_mode: BinaryMode) -> Result<String> {
    let text = if binary_mode == BinaryMode::Lossy {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        String::from_utf8(bytes.to_vec())
            .with_context(|| format!("{} is not UTF-8 text", path.display()))?
    };

    Ok(normalize_source_text(&text))
}

fn normalize_source_text(text: &str) -> String {
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    without_bom.replace("\r\n", "\n").replace('\r', "\n")
}

fn is_binary_mime(mime: Option<&str>) -> bool {
    let Some(mime) = mime else {
        return false;
    };

    mime.starts_with("application/")
        || mime.starts_with("audio/")
        || mime.starts_with("font/")
        || mime.starts_with("image/")
        || mime.starts_with("video/")
}

fn detect_language(path: &Path, text: &str) -> Result<(Language, Option<String>)> {
    if let Some(language) = detect_by_path(path) {
        return Ok((language, None));
    }

    if let Some(language) = detect_by_shebang(text) {
        return Ok((language, None));
    }

    let syntax_set = SyntaxSet::load_defaults_newlines();
    let syntax = syntax_set
        .find_syntax_for_file(path)
        .with_context(|| format!("detect syntax for {}", path.display()))?;

    let Some(syntax) = syntax else {
        return Ok((Language::Unknown, None));
    };

    Ok((
        language_from_syntax(&syntax.name),
        Some(syntax.name.clone()),
    ))
}

fn detect_by_path(path: &Path) -> Option<Language> {
    let file_name = path.file_name()?.to_str()?;
    if let Some(language) = LANGUAGE_SPECS.iter().find_map(|spec| {
        spec.file_names
            .contains(&file_name)
            .then_some(spec.language)
    }) {
        return Some(language);
    }

    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    LANGUAGE_SPECS.iter().find_map(|spec| {
        spec.extensions
            .contains(&extension.as_str())
            .then_some(spec.language)
    })
}

fn detect_by_shebang(text: &str) -> Option<Language> {
    let line = text.lines().next()?;
    if !line.starts_with("#!") {
        return None;
    }

    if line.contains("python") {
        Some(Language::Python)
    } else if line.contains("node") {
        Some(Language::JavaScript)
    } else {
        None
    }
}

fn language_from_syntax(name: &str) -> Language {
    LANGUAGE_SPECS
        .iter()
        .find_map(|spec| spec.syntax_names.contains(&name).then_some(spec.language))
        .unwrap_or(Language::Unknown)
}

fn language_spec(language: Language) -> Option<&'static LanguageSpec> {
    LANGUAGE_SPECS.iter().find(|spec| spec.language == language)
}

fn analysis_engine(language: Language) -> AnalysisEngine {
    language_spec(language).map_or(AnalysisEngine::None, |spec| spec.engine)
}

fn document_kind(language: Language) -> DocumentKind {
    if language == Language::Unknown {
        DocumentKind::Text
    } else {
        DocumentKind::Source
    }
}

fn resolve_read_range(
    command: &ReadCommand,
    target_line: Option<usize>,
) -> Result<(Option<usize>, Option<usize>)> {
    let explicit_start = match (command.start, command.offset) {
        (Some(start), Some(offset)) if start != offset => {
            bail!("--start and --offset specify different start lines")
        }
        (Some(start), _) | (_, Some(start)) => Some(start),
        (None, None) => None,
    };

    let start = match (explicit_start, target_line) {
        (Some(start), Some(line)) if start != line => {
            bail!("target line conflicts with --start/--offset")
        }
        (Some(start), _) | (_, Some(start)) => Some(start),
        (None, None) => None,
    };

    if command.end.is_some() && command.limit.is_some() {
        bail!("cannot combine --end with --limit");
    }

    let end = if let Some(limit) = command.limit {
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
        command.end
    };

    Ok((start, end))
}

fn read_output(
    source: &SourceFile,
    start: Option<usize>,
    end: Option<usize>,
) -> Result<ReadOutput> {
    let line_count = source.lines.len();
    let start_line = start.unwrap_or(1);
    let requested_end_line = end.unwrap_or(line_count);
    let end_line = requested_end_line.min(line_count);

    if start_line == 0 {
        bail!("start line must be greater than zero");
    }
    if line_count == 0 && start.is_none() && end.is_none() {
        return Ok(ReadOutput {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            line_count,
            file_hash: source.file_hash.clone(),
            start_line,
            end_line,
            hashlines: Vec::new(),
        });
    }
    if requested_end_line < start_line {
        bail!("end line must be greater than or equal to start line");
    }
    if start_line > line_count {
        bail!("start line {start_line} exceeds line count {line_count}");
    }
    let slice_start = start_line - 1;

    let hashlines = source.lines[slice_start..end_line]
        .iter()
        .map(|line| HashLine {
            line: line.number,
            hash: line.hash.clone(),
            text: line.text.clone(),
        })
        .collect();

    Ok(ReadOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count,
        file_hash: source.file_hash.clone(),
        start_line,
        end_line,
        hashlines,
    })
}

fn map_output(source: &SourceFile) -> Result<MapOutput> {
    let source_map = source_map(source)?;

    Ok(MapOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        symbols: source_map.symbols,
    })
}

fn source_map(source: &SourceFile) -> Result<SourceMap> {
    match cache::load_source_map(source) {
        Ok(Some(source_map)) => return Ok(source_map),
        Ok(None) => {}
        Err(error) => log::warn!("cache load error: {error:#}"),
    }

    parse_and_cache_source_map(source)
}

fn parse_and_cache_source_map(source: &SourceFile) -> Result<SourceMap> {
    let source_map = symbols::parse_source_map(source)?;
    if let Err(error) = cache::store_source_map(source, &source_map) {
        log::warn!("cache store error: {error:#}");
    }

    Ok(source_map)
}

fn symbol_address<'a>(target: &'a Target, address: Option<&'a str>) -> Result<Option<&'a str>> {
    match (target.address.as_ref(), address) {
        (Some(TargetAddress::Symbol(_)), Some(_)) => {
            bail!("qualified symbol name specified both in target and as argument")
        }
        (Some(TargetAddress::Symbol(symbol)), None) => Ok(Some(symbol.as_str())),
        (_, address) => Ok(address),
    }
}

fn symbol_output(source: &SourceFile, address: &str) -> Result<SymbolOutput> {
    if let Some(lookup) = cache::symbol_by_address(source, address)? {
        return match lookup {
            SymbolLookup::Found(symbol) => symbol_output_for_symbol(source, symbol),
            SymbolLookup::NotFound => bail!("symbol not found: {address}"),
            SymbolLookup::Ambiguous => bail!("qualified symbol name is ambiguous: {address}"),
        };
    }

    let source_map = parse_and_cache_source_map(source)?;
    let matches = source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.address == address || symbol.name == address)
        .collect::<Vec<_>>();

    let symbol = match matches.as_slice() {
        [] => bail!("symbol not found: {address}"),
        [symbol] => (*symbol).clone(),
        _ => bail!("qualified symbol name is ambiguous: {address}"),
    };

    symbol_output_for_symbol(source, symbol)
}

fn symbol_command_output(
    source: &SourceFile,
    address: Option<&str>,
    target_line: Option<usize>,
) -> Result<SymbolOutput> {
    if let Some(address) = address {
        return symbol_output(source, address);
    }

    let line = target_line.context("symbol requires qualified name or target line/hash")?;
    if let Some(lookup) = cache::symbol_at_line(source, line)? {
        return match lookup {
            SymbolLookup::Found(symbol) => symbol_output_for_symbol(source, symbol),
            SymbolLookup::NotFound => bail!("symbol not found at line {line}"),
            SymbolLookup::Ambiguous => unreachable!("line lookup returns at most one symbol"),
        };
    }

    let source_map = parse_and_cache_source_map(source)?;
    let symbol = symbol_at_line_in_map(&source_map, line)
        .with_context(|| format!("symbol not found at line {line}"))?;
    symbol_output_for_symbol(source, symbol)
}

fn symbol_output_for_symbol(source: &SourceFile, symbol: Symbol) -> Result<SymbolOutput> {
    let read = read_output(source, Some(symbol.start_line), Some(symbol.end_line))?;

    Ok(SymbolOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        symbol,
        hashlines: read.hashlines,
    })
}

fn identify_output(
    source: &SourceFile,
    target_line: Option<usize>,
    column: Option<usize>,
) -> Result<IdentifyOutput> {
    let line = target_line.context("identify requires --line or target line/hash")?;
    let column = column.unwrap_or(1);
    if line == 0 {
        bail!("line must be greater than zero");
    }
    if column == 0 {
        bail!("column must be greater than zero");
    }

    let source_line = source
        .lines
        .get(line - 1)
        .with_context(|| format!("line {line} not found in {}", source.path.display()))?;
    let identifier = identifier_at_column(&source_line.text, column);
    let symbol = symbol_at_line_uncached(source, line)?;

    Ok(IdentifyOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        line,
        column,
        line_hash: source_line.hash.clone(),
        hashlines: vec![HashLine {
            line: source_line.number,
            hash: source_line.hash.clone(),
            text: source_line.text.clone(),
        }],
        identifier,
        symbol,
    })
}

fn identifier_at_column(text: &str, column: usize) -> Option<IdentifierOutput> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = column.saturating_sub(1).min(bytes.len().saturating_sub(1));
    if !is_identifier_byte(bytes[index]) {
        if index > 0 && is_identifier_byte(bytes[index - 1]) {
            index -= 1;
        } else {
            return None;
        }
    }

    let mut start = index;
    while start > 0 && is_identifier_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < bytes.len() && is_identifier_byte(bytes[end]) {
        end += 1;
    }

    Some(IdentifierOutput {
        text: text[start..end].to_owned(),
        start_column: start + 1,
        end_column: end + 1,
    })
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn symbol_at_line_uncached(source: &SourceFile, line: usize) -> Result<Option<Symbol>> {
    let source_map = source_map(source)?;
    Ok(symbol_at_line_in_map(&source_map, line))
}

fn symbol_at_line_in_map(source_map: &SourceMap, line: usize) -> Option<Symbol> {
    source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .min_by_key(|symbol| symbol.end_line - symbol.start_line)
        .cloned()
}

fn definition_output(command: &DefinitionCommand) -> Result<DefinitionOutput> {
    let name = definition_name(command)?;
    let search_name = definition_search_name(&name);
    let mut candidates = Vec::new();
    let mut macro_definitions = Vec::new();
    for path in definition_candidate_paths(command, search_name)? {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains(search_name) {
            continue;
        }

        candidates.push((path, text));
    }

    for (path, text) in &candidates {
        if !text
            .lines()
            .any(|line| macro_definition_name(line) == Some(search_name))
        {
            continue;
        }
        let Ok(source) = source_from_text(path, text.clone(), command.language, false, None) else {
            continue;
        };
        macro_definitions.extend(macro_definition_locations(&source, search_name));
    }

    if !macro_definitions.is_empty() {
        return Ok(DefinitionOutput {
            definitions: macro_definitions,
        });
    }

    let mut definitions = Vec::new();
    for (path, text) in candidates {
        let Ok(source) = source_from_text(&path, text, command.language, false, None) else {
            continue;
        };
        let Ok(source_map) = source_map(&source) else {
            continue;
        };
        for symbol in source_map.symbols {
            if symbol.address != name && symbol.name != search_name {
                continue;
            }
            let line = source
                .lines
                .get(symbol.start_line.saturating_sub(1))
                .context("definition symbol line is out of range")?;
            definitions.push(DefinitionLocation {
                file: source.path.clone(),
                language: source.detection.language,
                engine: source.detection.engine,
                file_hash: source.file_hash.clone(),
                line_hash: line.hash.clone(),
                text: line.text.clone(),
                symbol,
            });
        }
    }

    Ok(DefinitionOutput { definitions })
}

fn compact_definitions(output: &DefinitionOutput) -> CompactOutput {
    CompactOutput {
        locations: output
            .definitions
            .iter()
            .map(|definition| CompactLocation {
                file: definition.file.clone(),
                line: definition.symbol.start_line,
                column: 1,
                line_hash: definition.line_hash.clone(),
                text: definition.text.clone(),
                kind: Some(definition.symbol.kind.clone()),
                name: Some(definition.symbol.name.clone()),
                qualified_name: Some(definition.symbol.address.clone()),
            })
            .collect(),
    }
}

fn definition_name(command: &DefinitionCommand) -> Result<String> {
    match (command.name.as_ref(), command.stdin) {
        (Some(name), _) => Ok(name.clone()),
        (None, false) => bail!("definition requires a name or --stdin identify context"),
        (None, true) => definition_name_from_stdin(),
    }
}

fn definition_search_name(name: &str) -> &str {
    name.rsplit('.')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(name)
}

fn macro_definition_locations(source: &SourceFile, name: &str) -> Vec<DefinitionLocation> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }

    source
        .lines
        .iter()
        .filter(|line| macro_definition_name(&line.text) == Some(name))
        .map(|line| DefinitionLocation {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            file_hash: source.file_hash.clone(),
            symbol: Symbol {
                kind: "macro".to_owned(),
                name: name.to_owned(),
                address: name.to_owned(),
                start_line: line.number,
                end_line: line.number,
                start_hash: line.hash.clone(),
                end_hash: line.hash.clone(),
            },
            line_hash: line.hash.clone(),
            text: line.text.clone(),
        })
        .collect()
}

fn macro_definition_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("#define")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }

    let rest = rest.trim_start();
    let name_len = rest
        .find(|ch: char| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_'))
        .unwrap_or(rest.len());
    if name_len == 0 {
        return None;
    }

    Some(&rest[..name_len])
}

fn definition_name_from_stdin() -> Result<String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .context("read identify context from stdin")?;
    let input: IdentifyInput = serde_json::from_str(&text).context("parse identify context")?;
    if let Some(identifier) = input.identifier {
        return Ok(identifier.text);
    }
    if let Some(symbol) = input.symbol {
        return Ok(symbol.qualified_name);
    }
    bail!("identify context has no symbol or identifier")
}

fn references_output(command: &ReferencesCommand) -> Result<ReferencesOutput> {
    validate_reference_name(&command.name)?;
    let mut references = Vec::new();
    for path in command_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
    )? {
        let Ok(source) = load_source(&path, command.language, BinaryMode::Reject) else {
            continue;
        };
        references.extend(references_in_source(&source, &command.name));
    }

    Ok(ReferencesOutput { references })
}

fn compact_references(output: &ReferencesOutput) -> CompactOutput {
    CompactOutput {
        locations: output
            .references
            .iter()
            .map(|reference| {
                let symbol = reference.symbol.as_ref();
                CompactLocation {
                    file: reference.file.clone(),
                    line: reference.line,
                    column: reference.column,
                    line_hash: reference.line_hash.clone(),
                    text: reference.text.clone(),
                    kind: symbol.map(|symbol| symbol.kind.clone()),
                    name: symbol.map(|symbol| symbol.name.clone()),
                    qualified_name: symbol.map(|symbol| symbol.address.clone()),
                }
            })
            .collect(),
    }
}

fn validate_reference_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("reference name must not be empty");
    }
    if !name.bytes().all(is_identifier_byte) {
        bail!("reference name must be an ASCII identifier");
    }
    Ok(())
}

fn references_in_source(source: &SourceFile, name: &str) -> Vec<ReferenceLocation> {
    let source_map = source_map(source).ok();
    let ignored_ranges = reference_ignored_ranges(source);
    let line_starts = line_start_offsets(&source.text);
    let mut references = Vec::new();
    for line in &source.lines {
        let columns = reference_columns(&line.text, name);
        if columns.is_empty() {
            continue;
        }
        let symbol = source_map
            .as_ref()
            .and_then(|source_map| symbol_at_line_in_map(source_map, line.number));
        for column in columns {
            let byte_offset = line_starts
                .get(line.number.saturating_sub(1))
                .map_or(column - 1, |line_start| line_start + column - 1);
            if is_ignored_reference(byte_offset, &ignored_ranges) {
                continue;
            }
            references.push(ReferenceLocation {
                file: source.path.clone(),
                language: source.detection.language,
                engine: source.detection.engine,
                file_hash: source.file_hash.clone(),
                line: line.number,
                column,
                line_hash: line.hash.clone(),
                text: line.text.clone(),
                symbol: symbol.clone(),
            });
        }
    }
    references
}

fn reference_ignored_ranges(source: &SourceFile) -> Vec<(usize, usize)> {
    if !matches!(source.detection.language, Language::C | Language::Cpp) {
        return Vec::new();
    }
    if source.detection.engine != AnalysisEngine::TreeSitter {
        return Vec::new();
    }
    let Some(language) = symbols::tree_sitter_language(source.detection.language) else {
        return Vec::new();
    };

    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(&source.text, None) else {
        return Vec::new();
    };

    let mut ranges = Vec::new();
    collect_reference_ignored_ranges(tree.root_node(), &mut ranges);
    ranges
}

fn collect_reference_ignored_ranges(node: Node<'_>, ranges: &mut Vec<(usize, usize)>) {
    if is_reference_noise_node(node.kind()) {
        ranges.push((node.start_byte(), node.end_byte()));
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_reference_ignored_ranges(child, ranges);
    }
}

fn is_reference_noise_node(kind: &str) -> bool {
    kind == "comment" || kind.ends_with("string_literal") || kind == "char_literal"
}

fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' && index + 1 < text.len() {
            offsets.push(index + 1);
        }
    }

    offsets
}

fn is_ignored_reference(byte_offset: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|&(start, end)| start <= byte_offset && byte_offset < end)
}

fn reference_columns(text: &str, name: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let name_bytes = name.as_bytes();
    let mut columns = Vec::new();
    let Some(last_start) = bytes.len().checked_sub(name_bytes.len()) else {
        return columns;
    };

    for index in 0..=last_start {
        if &bytes[index..index + name_bytes.len()] != name_bytes {
            continue;
        }
        let before = index.checked_sub(1).map(|before_index| bytes[before_index]);
        let after = bytes.get(index + name_bytes.len()).copied();
        if before.is_some_and(is_identifier_byte) || after.is_some_and(is_identifier_byte) {
            continue;
        }
        columns.push(index + 1);
    }
    columns
}

fn search_output(command: &SearchCommand) -> Result<SearchOutput> {
    let paths = command_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
    )?;
    let pattern = compile_search(&command.pattern);
    let mut results = Vec::new();

    for path in paths {
        let Some(result) = search_file(&path, command.language, &pattern)? else {
            continue;
        };
        if !result.matches.is_empty() {
            results.push(result);
        }
    }

    Ok(SearchOutput { results })
}

fn command_paths(target: &Path, cached: bool, others: bool, ignored: bool) -> Result<Vec<PathBuf>> {
    let metadata = fs::metadata(target).with_context(|| format!("stat {}", target.display()))?;
    if metadata.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if !metadata.is_dir() {
        bail!(
            "search target is not a file or directory: {}",
            target.display()
        );
    }

    if let Some(paths) = git_search_paths(target, cached, others, ignored)? {
        return Ok(paths);
    }

    if has_git_selection_flags(cached, others, ignored) {
        log::debug!(
            "ignoring Git file selection flags outside repository: {}",
            target.display()
        );
    }

    let mut paths = Vec::new();
    collect_search_paths(target, &mut paths)?;
    Ok(paths)
}

fn definition_candidate_paths(
    command: &DefinitionCommand,
    search_name: &str,
) -> Result<Vec<PathBuf>> {
    if let Some(paths) = git_definition_candidate_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
        search_name,
    )? {
        return Ok(paths);
    }

    command_paths(
        &command.target,
        command.cached,
        command.others,
        command.ignored,
    )
}

fn git_definition_candidate_paths(
    target: &Path,
    cached: bool,
    others: bool,
    ignored: bool,
    search_name: &str,
) -> Result<Option<Vec<PathBuf>>> {
    let original_target = target;
    let Ok(repository) = git2::Repository::discover(target) else {
        return Ok(None);
    };

    if ignored && !others {
        bail!("--ignored requires --others");
    }

    let workdir = repository
        .workdir()
        .context("Git repository has no work tree")?;
    let target = target
        .canonicalize()
        .with_context(|| format!("canonicalize {}", target.display()))?;
    let workdir = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize {}", workdir.display()))?;
    let scope = target
        .strip_prefix(&workdir)
        .with_context(|| format!("{} is outside Git work tree", target.display()))?;
    let output_root = output_root_for_scope(original_target, scope)?;
    let default_selection = !has_git_selection_flags(cached, others, ignored);
    let cached = cached || default_selection;
    let others = others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        collect_cached_definition_paths(&repository, &output_root, scope, search_name, &mut paths)?;
    }
    if others {
        collect_other_definition_paths(
            &repository,
            &workdir,
            &output_root,
            scope,
            ignored,
            search_name,
            &mut paths,
        )?;
    }

    Ok(Some(paths.into_iter().collect()))
}

fn collect_cached_definition_paths(
    repository: &git2::Repository,
    output_root: &Path,
    scope: &Path,
    search_name: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let index = repository.index().context("read Git index")?;
    for entry in index.iter() {
        let relative = git_path(&entry.path)?;
        if !path_is_in_scope(&relative, scope) {
            continue;
        }

        let Ok(blob) = repository.find_blob(entry.id) else {
            continue;
        };
        if bytes_contain(blob.content(), search_name.as_bytes()) {
            paths.insert(output_root.join(relative));
        }
    }

    Ok(())
}

fn collect_other_definition_paths(
    repository: &git2::Repository,
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    ignored: bool,
    search_name: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let mut other_paths = BTreeSet::new();
    collect_other_paths(
        repository,
        workdir,
        output_root,
        scope,
        ignored,
        &mut other_paths,
    )?;

    for path in other_paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if text.contains(search_name) {
            paths.insert(path);
        }
    }

    Ok(())
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn git_search_paths(
    target: &Path,
    cached: bool,
    others: bool,
    ignored: bool,
) -> Result<Option<Vec<PathBuf>>> {
    let original_target = target;
    let Ok(repository) = git2::Repository::discover(target) else {
        return Ok(None);
    };

    if ignored && !others {
        bail!("--ignored requires --others");
    }
    let workdir = repository
        .workdir()
        .context("Git repository has no work tree")?;
    let target = target
        .canonicalize()
        .with_context(|| format!("canonicalize {}", target.display()))?;
    let workdir = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize {}", workdir.display()))?;
    let scope = target
        .strip_prefix(&workdir)
        .with_context(|| format!("{} is outside Git work tree", target.display()))?;
    let output_root = output_root_for_scope(original_target, scope)?;
    let default_selection = !has_git_selection_flags(cached, others, ignored);
    let cached = cached || default_selection;
    let others = others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        collect_cached_paths(&repository, &workdir, &output_root, scope, &mut paths)?;
    }
    if others {
        collect_other_paths(
            &repository,
            &workdir,
            &output_root,
            scope,
            ignored,
            &mut paths,
        )?;
    }

    Ok(Some(paths.into_iter().collect()))
}

fn output_root_for_scope(target: &Path, scope: &Path) -> Result<PathBuf> {
    let mut output_root = target.to_path_buf();
    for _ in scope.components() {
        if !output_root.pop() {
            bail!("{} is outside Git work tree", target.display());
        }
    }
    Ok(output_root)
}

fn collect_cached_paths(
    repository: &git2::Repository,
    _workdir: &Path,
    output_root: &Path,
    scope: &Path,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let index = repository.index().context("read Git index")?;
    for entry in index.iter() {
        let relative = git_path(&entry.path)?;
        if path_is_in_scope(&relative, scope) {
            paths.insert(output_root.join(relative));
        }
    }

    Ok(())
}

fn collect_other_paths(
    repository: &git2::Repository,
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    ignored: bool,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    if ignored {
        options.include_ignored(true).recurse_ignored_dirs(true);
    }

    for entry in repository.statuses(Some(&mut options))?.iter() {
        let status = entry.status();
        let include = status.contains(git2::Status::WT_NEW)
            || (ignored && status.contains(git2::Status::IGNORED));
        if !include {
            continue;
        }

        let Some(relative) = entry.path().map(PathBuf::from) else {
            continue;
        };
        insert_scoped_file(workdir, output_root, scope, &relative, paths);
    }

    Ok(())
}

fn has_git_selection_flags(cached: bool, others: bool, ignored: bool) -> bool {
    cached || others || ignored
}

fn insert_scoped_file(
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    relative: &Path,
    paths: &mut BTreeSet<PathBuf>,
) {
    if !path_is_in_scope(relative, scope) {
        return;
    }

    let path = workdir.join(relative);
    if path.is_file() {
        paths.insert(output_root.join(relative));
    }
}

fn git_path(path: &[u8]) -> Result<PathBuf> {
    let path = std::str::from_utf8(path).context("Git index path is not UTF-8")?;
    Ok(PathBuf::from(path))
}

fn path_is_in_scope(path: &Path, scope: &Path) -> bool {
    scope.as_os_str().is_empty() || path.starts_with(scope)
}

fn collect_search_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("read directory entry from {}", directory.display()))?;
    entries.sort_by_key(std::fs::DirEntry::path);

    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read file type for {}", path.display()))?;
        if file_type.is_dir() {
            collect_search_paths(&path, paths)?;
        } else if file_type.is_file() {
            paths.push(path);
        }
    }

    Ok(())
}

fn search_file(
    path: &Path,
    override_language: Option<Language>,
    pattern: &SearchPattern,
) -> Result<Option<SearchFileOutput>> {
    let Ok(source) = load_source(path, override_language, BinaryMode::Reject) else {
        return Ok(None);
    };
    let language_id = source.detection.language;
    if source.detection.engine != AnalysisEngine::TreeSitter {
        return Ok(None);
    }
    let Some(language) = symbols::tree_sitter_language(language_id) else {
        return Ok(None);
    };
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| anyhow::anyhow!("set tree-sitter language: {error}"))?;
    let tree = parser
        .parse(&source.text, None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter parse failed"))?;
    let pattern_tree = parser
        .parse(&pattern.text, None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter pattern parse failed"))?;
    if pattern_tree.root_node().has_error() {
        bail!("pattern is not valid {} syntax", language_id.id());
    }

    let mut matches = Vec::new();
    let pattern_root =
        search_pattern_root(pattern_tree.root_node()).context("empty search pattern")?;
    collect_search_matches(
        &source,
        pattern,
        pattern_root,
        tree.root_node(),
        &mut matches,
    )?;

    Ok(Some(SearchFileOutput {
        file: source.path,
        language: language_id,
        engine: source.detection.engine,
        file_hash: source.file_hash,
        matches,
    }))
}

fn compile_search(pattern: &str) -> SearchPattern {
    let mut text = String::with_capacity(pattern.len());
    let mut metas = Vec::new();
    let bytes = pattern.as_bytes();
    let mut index = 0;

    while index < pattern.len() {
        let rest = &pattern[index..];
        if !rest.starts_with('$') {
            let Some(ch) = rest.chars().next() else {
                break;
            };
            text.push(ch);
            index += ch.len_utf8();
            continue;
        }

        let (kind, name_start) = if rest.starts_with("$$$") {
            (PatternMetaKind::Variadic, index + 3)
        } else {
            (PatternMetaKind::Single, index + 1)
        };
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
        {
            name_end += 1;
        }
        if name_end == name_start {
            text.push('$');
            index += 1;
            continue;
        }

        let name = &pattern[name_start..name_end];
        let placeholder = match kind {
            PatternMetaKind::Single => format!("__readseek_meta_{name}"),
            PatternMetaKind::Variadic => format!("__readseek_variadic_{name}"),
        };
        text.push_str(&placeholder);
        metas.push(PatternMeta {
            placeholder,
            name: name.to_owned(),
            kind,
        });
        index = name_end;
    }

    SearchPattern { text, metas }
}

fn search_pattern_root(root: Node<'_>) -> Option<Node<'_>> {
    if root.named_child_count() == 1 {
        root.named_child(0)
    } else {
        Some(root)
    }
}

fn collect_search_matches(
    source: &SourceFile,
    pattern: &SearchPattern,
    pattern_node: Node<'_>,
    source_node: Node<'_>,
    matches: &mut Vec<SearchMatch>,
) -> Result<()> {
    let mut captures = Vec::new();
    if nodes_match(source, pattern, pattern_node, source_node, &mut captures) {
        matches.push(search_match(source, source_node, captures)?);
    }

    let mut cursor = source_node.walk();
    for child in source_node.named_children(&mut cursor) {
        collect_search_matches(source, pattern, pattern_node, child, matches)?;
    }

    Ok(())
}

fn nodes_match(
    source: &SourceFile,
    pattern: &SearchPattern,
    pattern_node: Node<'_>,
    source_node: Node<'_>,
    captures: &mut Vec<SearchCaptureRange>,
) -> bool {
    if let Some(meta) = pattern_meta(pattern, pattern_node) {
        if meta.kind == PatternMetaKind::Single {
            let (start_line, end_line) = symbols::node_line_range(source_node);
            let Some(text) = node_text(source_node, &source.text) else {
                return false;
            };
            return bind_capture(captures, &meta.name, text, start_line, end_line);
        }
        return true;
    }

    if pattern_node.kind() != source_node.kind() {
        return false;
    }

    let pattern_children = named_children(pattern_node);
    let source_children = named_children(source_node);
    if pattern_children.is_empty() {
        return node_text(pattern_node, &pattern.text) == node_text(source_node, &source.text);
    }

    child_nodes_match(
        source,
        pattern,
        &pattern_children,
        &source_children,
        0,
        0,
        captures,
    )
}

fn child_nodes_match(
    source: &SourceFile,
    pattern: &SearchPattern,
    pattern_children: &[Node<'_>],
    source_children: &[Node<'_>],
    pattern_index: usize,
    source_index: usize,
    captures: &mut Vec<SearchCaptureRange>,
) -> bool {
    if pattern_index == pattern_children.len() {
        return source_index == source_children.len();
    }

    let pattern_child = pattern_children[pattern_index];
    if let Some(meta) = pattern_meta(pattern, pattern_child) {
        if meta.kind == PatternMetaKind::Variadic {
            for count in 0..=source_children.len().saturating_sub(source_index) {
                let mut trial_captures = captures.clone();
                if count > 0 {
                    let start_node = source_children[source_index];
                    let end_node = source_children[source_index + count - 1];
                    let (start_line, _) = symbols::node_line_range(start_node);
                    let (_, end_line) = symbols::node_line_range(end_node);
                    let Some(text) = source
                        .text
                        .get(start_node.start_byte()..end_node.end_byte())
                    else {
                        continue;
                    };
                    if !bind_capture(&mut trial_captures, &meta.name, text, start_line, end_line) {
                        continue;
                    }
                }
                if child_nodes_match(
                    source,
                    pattern,
                    pattern_children,
                    source_children,
                    pattern_index + 1,
                    source_index + count,
                    &mut trial_captures,
                ) {
                    *captures = trial_captures;
                    return true;
                }
            }
            return false;
        }
    }

    if source_index >= source_children.len() {
        return false;
    }

    let mut trial_captures = captures.clone();
    if !nodes_match(
        source,
        pattern,
        pattern_child,
        source_children[source_index],
        &mut trial_captures,
    ) {
        return false;
    }
    if !child_nodes_match(
        source,
        pattern,
        pattern_children,
        source_children,
        pattern_index + 1,
        source_index + 1,
        &mut trial_captures,
    ) {
        return false;
    }

    *captures = trial_captures;
    true
}

fn bind_capture(
    captures: &mut Vec<SearchCaptureRange>,
    name: &str,
    text: &str,
    start_line: usize,
    end_line: usize,
) -> bool {
    if captures
        .iter()
        .any(|capture| capture.name == name && capture.text != text)
    {
        return false;
    }

    captures.push(SearchCaptureRange {
        name: name.to_owned(),
        text: text.to_owned(),
        start_line,
        end_line,
    });
    true
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn pattern_meta<'a>(pattern: &'a SearchPattern, node: Node<'_>) -> Option<&'a PatternMeta> {
    let text = node_text(node, &pattern.text)?;
    pattern.metas.iter().find(|meta| meta.placeholder == text)
}

fn node_text<'a>(node: Node<'_>, text: &'a str) -> Option<&'a str> {
    node.utf8_text(text.as_bytes()).ok()
}

fn search_match(
    source: &SourceFile,
    node: Node<'_>,
    capture_ranges: Vec<SearchCaptureRange>,
) -> Result<SearchMatch> {
    let captures = capture_ranges
        .into_iter()
        .map(|capture| {
            Ok(SearchCapture {
                name: capture.name,
                start_line: capture.start_line,
                end_line: capture.end_line,
                start_hash: line_hash(source, capture.start_line)?,
                end_hash: line_hash(source, capture.end_line)?,
                hashlines: range_hashlines(source, capture.start_line, capture.end_line),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let (start_line, end_line) = symbols::node_line_range(node);

    Ok(SearchMatch {
        pattern_index: 0,
        start_line,
        end_line,
        start_hash: line_hash(source, start_line)?,
        end_hash: line_hash(source, end_line)?,
        hashlines: range_hashlines(source, start_line, end_line),
        captures,
    })
}

fn line_hash(source: &SourceFile, line: usize) -> Result<String> {
    source
        .lines
        .get(line.saturating_sub(1))
        .map(|line| line.hash.clone())
        .with_context(|| format!("line {line} not found in {}", source.path.display()))
}

fn range_hashlines(source: &SourceFile, start_line: usize, end_line: usize) -> Vec<HashLine> {
    let start = start_line.saturating_sub(1);
    let end = end_line.min(source.lines.len());
    source.lines[start..end]
        .iter()
        .map(|line| HashLine {
            line: line.number,
            hash: line.hash.clone(),
            text: line.text.clone(),
        })
        .collect()
}

fn hash_text(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn hash_line(_line: usize, text: &str) -> String {
    let text = text.strip_suffix('\r').unwrap_or(text);
    let normalized = text.split_whitespace().collect::<String>();
    format!(
        "{:03x}",
        xxhash32(normalized.as_bytes(), 0) % HASHLINE_MODULUS
    )
}

fn xxhash32(bytes: &[u8], seed: u32) -> u32 {
    let mut index = 0;
    let mut hash;

    if bytes.len() >= 16 {
        let mut v1 = seed
            .wrapping_add(XXHASH32_PRIME_1)
            .wrapping_add(XXHASH32_PRIME_2);
        let mut v2 = seed.wrapping_add(XXHASH32_PRIME_2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(XXHASH32_PRIME_1);

        while index <= bytes.len() - 16 {
            v1 = xxhash32_round(v1, read_u32_le(bytes, index));
            index += 4;
            v2 = xxhash32_round(v2, read_u32_le(bytes, index));
            index += 4;
            v3 = xxhash32_round(v3, read_u32_le(bytes, index));
            index += 4;
            v4 = xxhash32_round(v4, read_u32_le(bytes, index));
            index += 4;
        }

        hash = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
    } else {
        hash = seed.wrapping_add(XXHASH32_PRIME_5);
    }

    let length_bytes = bytes.len().to_le_bytes();
    let length = u32::from_le_bytes([
        length_bytes[0],
        length_bytes[1],
        length_bytes[2],
        length_bytes[3],
    ]);
    hash = hash.wrapping_add(length);

    while index + 4 <= bytes.len() {
        hash = hash
            .wrapping_add(read_u32_le(bytes, index).wrapping_mul(XXHASH32_PRIME_3))
            .rotate_left(17)
            .wrapping_mul(XXHASH32_PRIME_4);
        index += 4;
    }

    while index < bytes.len() {
        hash = hash
            .wrapping_add(u32::from(bytes[index]).wrapping_mul(XXHASH32_PRIME_5))
            .rotate_left(11)
            .wrapping_mul(XXHASH32_PRIME_1);
        index += 1;
    }

    hash ^= hash >> 15;
    hash = hash.wrapping_mul(XXHASH32_PRIME_2);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(XXHASH32_PRIME_3);
    hash ^ (hash >> 16)
}

fn xxhash32_round(accumulator: u32, input: u32) -> u32 {
    accumulator
        .wrapping_add(input.wrapping_mul(XXHASH32_PRIME_2))
        .rotate_left(13)
        .wrapping_mul(XXHASH32_PRIME_1)
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
