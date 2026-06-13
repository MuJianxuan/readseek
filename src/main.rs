// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::{Context, Result, bail};
use argh::FromArgs;
use serde::{Serialize, Serializer};
use std::collections::BTreeSet;
use std::fs;
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
    Search(SearchCommand),
}

/// detect the file type
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "file")]
#[argh(help_triggers("-h", "--help"))]
struct FileCommand {
    /// takes <file>, <file>:<line> or <file>:<hash>
    #[argh(positional)]
    target: String,

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
    target: String,

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
    target: String,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
}

/// read the line range for a symbol
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "symbol")]
#[argh(help_triggers("-h", "--help"))]
struct SymbolCommand {
    /// takes <file>, <file>:<line>, <file>:<hash> or <file>:<symbol>
    #[argh(positional)]
    target: String,

    /// symbol address or unqualified name
    #[argh(positional)]
    address: Option<String>,

    /// language override
    #[argh(option, from_str_fn(parse_language))]
    language: Option<Language>,
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
    Go,
    Gdscript,
    Java,
    JavaScript,
    Jsx,
    Html,
    Json,
    Kconfig,
    Latex,
    Markdown,
    Xml,
    Yaml,
    Just,
    Make,
    Meson,
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
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    syntax_names: &'static [&'static str],
}

const LANGUAGE_SPECS: &[LanguageSpec] = &[
    LanguageSpec {
        language: Language::Assembly,
        id: "assembly",
        aliases: &["assembly", "asm", "x86", "arm"],
        extensions: &["asm", "s", "S"],
        file_names: &[],
        syntax_names: &["Assembly", "ARM Assembly", "x86 Assembly"],
    },
    LanguageSpec {
        language: Language::C,
        id: "c",
        aliases: &["c"],
        extensions: &["c", "h"],
        file_names: &[],
        syntax_names: &["C"],
    },
    LanguageSpec {
        language: Language::Bash,
        id: "bash",
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
        aliases: &["cpp", "cxx", "cplusplus"],
        extensions: &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
        file_names: &[],
        syntax_names: &["C++"],
    },
    LanguageSpec {
        language: Language::CSharp,
        id: "csharp",
        aliases: &["csharp", "cs", "c#"],
        extensions: &["cs"],
        file_names: &[],
        syntax_names: &["C#"],
    },
    LanguageSpec {
        language: Language::Css,
        id: "css",
        aliases: &["css"],
        extensions: &["css"],
        file_names: &[],
        syntax_names: &["CSS"],
    },
    LanguageSpec {
        language: Language::Go,
        id: "go",
        aliases: &["go", "golang"],
        extensions: &["go"],
        file_names: &["go.mod"],
        syntax_names: &["Go"],
    },
    LanguageSpec {
        language: Language::Gdscript,
        id: "gdscript",
        aliases: &["gdscript", "gd"],
        extensions: &["gd"],
        file_names: &[],
        syntax_names: &["GDScript"],
    },
    LanguageSpec {
        language: Language::Java,
        id: "java",
        aliases: &["java"],
        extensions: &["java"],
        file_names: &[],
        syntax_names: &["Java"],
    },
    LanguageSpec {
        language: Language::JavaScript,
        id: "javascript",
        aliases: &["javascript", "js"],
        extensions: &["js", "mjs", "cjs"],
        file_names: &[],
        syntax_names: &["JavaScript"],
    },
    LanguageSpec {
        language: Language::Jsx,
        id: "jsx",
        aliases: &["jsx"],
        extensions: &["jsx"],
        file_names: &[],
        syntax_names: &[],
    },
    LanguageSpec {
        language: Language::Html,
        id: "html",
        aliases: &["html", "htm"],
        extensions: &["html", "htm"],
        file_names: &[],
        syntax_names: &["HTML"],
    },
    LanguageSpec {
        language: Language::Json,
        id: "json",
        aliases: &["json"],
        extensions: &["json"],
        file_names: &["package-lock.json", "composer.lock"],
        syntax_names: &["JSON"],
    },
    LanguageSpec {
        language: Language::Xml,
        id: "xml",
        aliases: &["xml"],
        extensions: &["xml", "xsd", "xsl", "xslt"],
        file_names: &[],
        syntax_names: &["XML"],
    },
    LanguageSpec {
        language: Language::Yaml,
        id: "yaml",
        aliases: &["yaml", "yml"],
        extensions: &["yaml", "yml"],
        file_names: &[],
        syntax_names: &["YAML"],
    },
    LanguageSpec {
        language: Language::Just,
        id: "just",
        aliases: &["just", "justfile"],
        extensions: &["just"],
        file_names: &["justfile", "Justfile", ".justfile"],
        syntax_names: &["Just"],
    },
    LanguageSpec {
        language: Language::Kconfig,
        id: "kconfig",
        aliases: &["kconfig"],
        extensions: &[],
        file_names: &["Kconfig"],
        syntax_names: &["Kconfig"],
    },
    LanguageSpec {
        language: Language::Make,
        id: "make",
        aliases: &["make", "makefile"],
        extensions: &["mk", "mak", "make"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        syntax_names: &["Makefile"],
    },
    LanguageSpec {
        language: Language::Latex,
        id: "latex",
        aliases: &["latex", "tex"],
        extensions: &["tex", "ltx", "latex"],
        file_names: &[],
        syntax_names: &["LaTeX", "TeX"],
    },
    LanguageSpec {
        language: Language::Markdown,
        id: "markdown",
        aliases: &["markdown", "md"],
        extensions: &["md", "markdown", "mdown", "mkd"],
        file_names: &[],
        syntax_names: &["Markdown"],
    },
    LanguageSpec {
        language: Language::Meson,
        id: "meson",
        aliases: &["meson"],
        extensions: &[],
        file_names: &["meson.build", "meson_options.txt"],
        syntax_names: &["Meson"],
    },
    LanguageSpec {
        language: Language::Python,
        id: "python",
        aliases: &["python", "py"],
        extensions: &["py", "pyw"],
        file_names: &[],
        syntax_names: &["Python"],
    },
    LanguageSpec {
        language: Language::Php,
        id: "php",
        aliases: &["php"],
        extensions: &["php", "php3", "php4", "php5", "phtml"],
        file_names: &[],
        syntax_names: &["PHP"],
    },
    LanguageSpec {
        language: Language::Puppet,
        id: "puppet",
        aliases: &["puppet", "pp"],
        extensions: &["pp"],
        file_names: &["Puppetfile"],
        syntax_names: &["Puppet"],
    },
    LanguageSpec {
        language: Language::Ruby,
        id: "ruby",
        aliases: &["ruby", "rb"],
        extensions: &["rb", "rake", "gemspec"],
        file_names: &["Gemfile", "Rakefile"],
        syntax_names: &["Ruby"],
    },
    LanguageSpec {
        language: Language::Riscv,
        id: "riscv",
        aliases: &["riscv", "risc-v", "riscv64"],
        extensions: &["riscv"],
        file_names: &[],
        syntax_names: &["RISC-V"],
    },
    LanguageSpec {
        language: Language::Rust,
        id: "rust",
        aliases: &["rust", "rs"],
        extensions: &["rs"],
        file_names: &[],
        syntax_names: &["Rust"],
    },
    LanguageSpec {
        language: Language::Swift,
        id: "swift",
        aliases: &["swift"],
        extensions: &["swift"],
        file_names: &[],
        syntax_names: &["Swift"],
    },
    LanguageSpec {
        language: Language::Sql,
        id: "sql",
        aliases: &["sql"],
        extensions: &["sql"],
        file_names: &[],
        syntax_names: &["SQL"],
    },
    LanguageSpec {
        language: Language::TypeScript,
        id: "typescript",
        aliases: &["typescript", "ts"],
        extensions: &["ts", "mts", "cts"],
        file_names: &[],
        syntax_names: &["TypeScript"],
    },
    LanguageSpec {
        language: Language::Tsx,
        id: "tsx",
        aliases: &["tsx"],
        extensions: &["tsx"],
        file_names: &[],
        syntax_names: &[],
    },
    LanguageSpec {
        language: Language::Toml,
        id: "toml",
        aliases: &["toml"],
        extensions: &["toml"],
        file_names: &["Cargo.lock"],
        syntax_names: &["TOML"],
    },
    LanguageSpec {
        language: Language::Typst,
        id: "typst",
        aliases: &["typst", "typ"],
        extensions: &["typ"],
        file_names: &[],
        syntax_names: &["Typst"],
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

impl DocumentKind {
    const fn supports_symbols(self) -> bool {
        matches!(self, Self::Source)
    }
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
    supported: bool,
    binary: bool,
    mime: Option<String>,
    syntax: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadOutput {
    file: PathBuf,
    language: Language,
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
    line_count: usize,
    file_hash: String,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Serialize)]
struct SymbolOutput {
    file: PathBuf,
    language: Language,
    line_count: usize,
    file_hash: String,
    symbol: Symbol,
    hashlines: Vec<HashLine>,
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    results: Vec<SearchFileOutput>,
}

#[derive(Debug, Serialize)]
struct SearchFileOutput {
    file: PathBuf,
    language: Language,
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
    path: PathBuf,
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
            let target = parse_target(&command.target)?;
            let source = load_source(&target.path, command.language, BinaryMode::Reject)?;
            print_json(&source.detection)?;
        }
        Command::Read(command) => {
            let target = parse_target(&command.target)?;
            let source = load_source(&target.path, command.language, BinaryMode::Lossy)?;
            let target_line = resolve_target_line(&source, &target)?;
            let (start, end) = resolve_read_range(&command, target_line)?;
            let output = read_output(&source, start, end)?;
            print_json(&output)?;
        }
        Command::Map(command) => {
            let target = parse_target(&command.target)?;
            let source = load_source(&target.path, command.language, BinaryMode::Reject)?;
            print_json(&map_output(&source)?)?;
        }
        Command::Symbol(command) => {
            let target = parse_symbol_target(&command.target)?;
            let source = load_source(&target.path, command.language, BinaryMode::Reject)?;
            let target_line = resolve_target_line(&source, &target)?;
            let target_address = symbol_address(&target, command.address.as_deref())?;
            let output = symbol_command_output(&source, target_address, target_line)?;
            print_json(&output)?;
        }
        Command::Search(command) => {
            print_json(&search_output(&command)?)?;
        }
    }

    Ok(())
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

fn load_source(
    path: &Path,
    override_language: Option<Language>,
    binary_mode: BinaryMode,
) -> Result<SourceFile> {
    let document = load_document(path, binary_mode)?;
    let path_language = detect_by_path(path);
    let (detected_language, syntax) = if binary_mode == BinaryMode::Lossy
        && override_language.is_none()
        && path_language.is_none()
    {
        (Language::Unknown, None)
    } else {
        detect_language(path, &document.text)?
    };
    let language = override_language.unwrap_or(detected_language);
    let kind = document_kind(language);
    let lines = document
        .text
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
    let file_hash = hash_text(&document.text);
    let detection = Detection {
        file: document.path.clone(),
        language,
        supported: language != Language::Unknown,
        binary: document.binary,
        mime: document.mime,
        syntax,
    };

    Ok(SourceFile {
        path: document.path,
        text: document.text,
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

    Ok(LoadedDocument {
        path: path.to_path_buf(),
        text,
        binary,
        mime,
    })
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

fn document_kind(language: Language) -> DocumentKind {
    if symbols::has_parser(language) {
        DocumentKind::Source
    } else {
        DocumentKind::Text
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
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        symbols: source_map.symbols,
    })
}

fn source_map(source: &SourceFile) -> Result<SourceMap> {
    match cache::load_source_map(source) {
        Ok(Some(source_map)) => return Ok(source_map),
        Ok(None) => {}
        Err(error) => drop(error),
    }

    let source_map = symbols::parse_source_map(source)?;
    if let Err(error) = cache::store_source_map(source, &source_map) {
        drop(error);
    }

    Ok(source_map)
}

fn symbol_address<'a>(target: &'a Target, address: Option<&'a str>) -> Result<Option<&'a str>> {
    match (target.address.as_ref(), address) {
        (Some(TargetAddress::Symbol(_)), Some(_)) => {
            bail!("symbol address specified both in target and as argument")
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
            SymbolLookup::Ambiguous => bail!("symbol address is ambiguous: {address}"),
        };
    }

    let source_map = source_map(source)?;
    let matches = source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.address == address || symbol.name == address)
        .collect::<Vec<_>>();

    let symbol = match matches.as_slice() {
        [] => bail!("symbol not found: {address}"),
        [symbol] => (*symbol).clone(),
        _ => bail!("symbol address is ambiguous: {address}"),
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

    let line = target_line.context("symbol requires address or target line/hash")?;
    if let Some(lookup) = cache::symbol_at_line(source, line)? {
        return match lookup {
            SymbolLookup::Found(symbol) => symbol_output_for_symbol(source, symbol),
            SymbolLookup::NotFound => bail!("symbol not found at line {line}"),
            SymbolLookup::Ambiguous => unreachable!("line lookup returns at most one symbol"),
        };
    }

    let source_map = source_map(source)?;
    let symbol = source_map
        .symbols
        .iter()
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .min_by_key(|symbol| symbol.end_line - symbol.start_line)
        .cloned()
        .with_context(|| format!("symbol not found at line {line}"))?;
    symbol_output_for_symbol(source, symbol)
}

fn symbol_output_for_symbol(source: &SourceFile, symbol: Symbol) -> Result<SymbolOutput> {
    let read = read_output(source, Some(symbol.start_line), Some(symbol.end_line))?;

    Ok(SymbolOutput {
        file: source.path.clone(),
        language: source.detection.language,
        line_count: source.lines.len(),
        file_hash: source.file_hash.clone(),
        symbol,
        hashlines: read.hashlines,
    })
}

fn search_output(command: &SearchCommand) -> Result<SearchOutput> {
    let mut results = Vec::new();

    for path in search_paths(command)? {
        let Some(result) = search_file(&path, command.language, &command.pattern)? else {
            continue;
        };
        if !result.matches.is_empty() {
            results.push(result);
        }
    }

    Ok(SearchOutput { results })
}

fn search_paths(command: &SearchCommand) -> Result<Vec<PathBuf>> {
    let metadata = fs::metadata(&command.target)
        .with_context(|| format!("stat {}", command.target.display()))?;
    if metadata.is_file() {
        return Ok(vec![command.target.clone()]);
    }
    if !metadata.is_dir() {
        bail!(
            "search target is not a file or directory: {}",
            command.target.display()
        );
    }

    if let Some(paths) = git_search_paths(command)? {
        return Ok(paths);
    }

    if has_git_selection_flags(command) {
        log::debug!(
            "ignoring Git file selection flags outside repository: {}",
            command.target.display()
        );
    }

    let mut paths = Vec::new();
    collect_search_paths(&command.target, &mut paths)?;
    Ok(paths)
}

fn git_search_paths(command: &SearchCommand) -> Result<Option<Vec<PathBuf>>> {
    let Ok(repository) = git2::Repository::discover(&command.target) else {
        return Ok(None);
    };

    if command.ignored && !command.others {
        bail!("--ignored requires --others");
    }
    let workdir = repository
        .workdir()
        .context("Git repository has no work tree")?;
    let target = command
        .target
        .canonicalize()
        .with_context(|| format!("canonicalize {}", command.target.display()))?;
    let workdir = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize {}", workdir.display()))?;
    let scope = target
        .strip_prefix(&workdir)
        .with_context(|| format!("{} is outside Git work tree", target.display()))?;
    let default_selection = !has_git_selection_flags(command);
    let cached = command.cached || default_selection;
    let others = command.others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        collect_cached_paths(&repository, &workdir, scope, &mut paths)?;
    }
    if others {
        collect_other_paths(&repository, &workdir, scope, command.ignored, &mut paths)?;
    }

    Ok(Some(paths.into_iter().collect()))
}

fn collect_cached_paths(
    repository: &git2::Repository,
    workdir: &Path,
    scope: &Path,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let index = repository.index().context("read Git index")?;
    for entry in index.iter() {
        let relative = git_path(&entry.path)?;
        insert_scoped_file(workdir, scope, &relative, paths);
    }

    Ok(())
}

fn collect_other_paths(
    repository: &git2::Repository,
    workdir: &Path,
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
        insert_scoped_file(workdir, scope, &relative, paths);
    }

    Ok(())
}

fn has_git_selection_flags(command: &SearchCommand) -> bool {
    command.cached || command.others || command.ignored
}

fn insert_scoped_file(
    workdir: &Path,
    scope: &Path,
    relative: &Path,
    paths: &mut BTreeSet<PathBuf>,
) {
    if !path_is_in_scope(relative, scope) {
        return;
    }

    let path = workdir.join(relative);
    if path.is_file() {
        paths.insert(path);
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
    pattern_text: &str,
) -> Result<Option<SearchFileOutput>> {
    let Ok(source) = load_source(path, override_language, BinaryMode::Reject) else {
        return Ok(None);
    };
    let language_id = source.detection.language;
    let Some(language) = symbols::tree_sitter_language(language_id) else {
        return Ok(None);
    };

    let pattern = compile_search_pattern(pattern_text);
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
        &pattern,
        pattern_root,
        tree.root_node(),
        &mut matches,
    )?;

    Ok(Some(SearchFileOutput {
        file: source.path,
        language: language_id,
        file_hash: source.file_hash,
        matches,
    }))
}

fn compile_search_pattern(pattern: &str) -> SearchPattern {
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
            let (start_line, end_line) = node_line_range(source_node);
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
    if let Some(meta) = pattern_meta(pattern, pattern_child)
        && meta.kind == PatternMetaKind::Variadic
    {
        for count in 0..=source_children.len().saturating_sub(source_index) {
            let mut trial_captures = captures.clone();
            if count > 0 {
                let start_node = source_children[source_index];
                let end_node = source_children[source_index + count - 1];
                let (start_line, _) = node_line_range(start_node);
                let (_, end_line) = node_line_range(end_node);
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

    let (start_line, end_line) = node_line_range(node);

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

fn node_line_range(node: tree_sitter::Node<'_>) -> (usize, usize) {
    let start_line = node.start_position().row + 1;
    let end_position = node.end_position();
    let end_line = if end_position.column == 0 && end_position.row + 1 > start_line {
        end_position.row
    } else {
        end_position.row + 1
    };

    (start_line, end_line)
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
