// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use anyhow::{Context, Result};
use serde::{Serialize, Serializer};
use std::path::Path;
use std::sync::OnceLock;
use strum_macros::{Display, EnumString};
use syntect::parsing::SyntaxSet;

fn cached_syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

#[derive(Clone, Copy, Debug, Display, EnumString, Eq, PartialEq)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub(crate) enum Language {
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
    pub(crate) fn id(self) -> &'static str {
        language_spec(self).map_or("unknown", |spec| spec.id)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LanguageSpec {
    pub(crate) language: Language,
    pub(crate) id: &'static str,
    pub(crate) engine: AnalysisEngine,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) extensions: &'static [&'static str],
    pub(crate) file_names: &'static [&'static str],
    pub(crate) syntax_names: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnalysisEngine {
    TreeSitter,
    #[allow(dead_code)]
    Llvm,
    None,
}

impl AnalysisEngine {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::TreeSitter => "tree-sitter",
            Self::Llvm => "llvm",
            Self::None => "none",
        }
    }
}

pub(crate) const LANGUAGE_SPECS: &[LanguageSpec] = &[
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
pub(crate) enum BinaryMode {
    Reject,
    Lossy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentKind {
    Source,
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentFormat {
    PlainText,
}

impl DocumentFormat {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::PlainText => "plain-text",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DocumentExtractor {
    pub(crate) format: DocumentFormat,
    pub(crate) extensions: &'static [&'static str],
    pub(crate) mime_prefixes: &'static [&'static str],
    pub(crate) extract: fn(&Path, &[u8], BinaryMode) -> Result<String>,
}

pub(crate) const DOCUMENT_EXTRACTORS: &[DocumentExtractor] = &[DocumentExtractor {
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

pub(crate) fn detect_language(path: &Path, text: &str) -> Result<(Language, Option<String>)> {
    if let Some(language) = detect_by_path(path) {
        return Ok((language, None));
    }

    if let Some(language) = detect_by_shebang(text) {
        return Ok((language, None));
    }

    let syntax_set = cached_syntax_set();
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

pub(crate) fn detect_by_path(path: &Path) -> Option<Language> {
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

pub(crate) fn language_spec(language: Language) -> Option<&'static LanguageSpec> {
    LANGUAGE_SPECS.iter().find(|spec| spec.language == language)
}

pub(crate) fn analysis_engine(language: Language) -> AnalysisEngine {
    language_spec(language).map_or(AnalysisEngine::None, |spec| spec.engine)
}

pub(crate) fn document_kind(language: Language) -> DocumentKind {
    if language == Language::Unknown {
        DocumentKind::Text
    } else {
        DocumentKind::Source
    }
}

pub(crate) fn document_extractor(path: &Path, mime: Option<&str>) -> &'static DocumentExtractor {
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

pub(crate) fn extract_plain_text(
    path: &Path,
    bytes: &[u8],
    binary_mode: BinaryMode,
) -> Result<String> {
    let text = if binary_mode == BinaryMode::Lossy {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        String::from_utf8(bytes.to_vec())
            .with_context(|| format!("{} is not UTF-8 text", path.display()))?
    };

    Ok(normalize_source_text(&text))
}

pub(crate) fn normalize_source_text(text: &str) -> String {
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    without_bom.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn is_binary_mime(mime: Option<&str>) -> bool {
    let Some(mime) = mime else {
        return false;
    };

    mime.starts_with("application/")
        || mime.starts_with("audio/")
        || mime.starts_with("font/")
        || mime.starts_with("image/")
        || mime.starts_with("video/")
}
