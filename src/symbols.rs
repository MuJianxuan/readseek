// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::{Language, SourceFile, SourceLine, SourceMap, Symbol};
use anyhow::{Result, anyhow};
use tree_sitter::{Node, Parser};

pub(crate) fn has_parser(language: Language) -> bool {
    tree_sitter_language(language).is_some()
}

pub(crate) fn parse_source_map(source: &SourceFile) -> Result<SourceMap> {
    let mut symbols = Vec::new();

    if !source.kind.supports_symbols() {
        return Ok(SourceMap { symbols });
    }

    let Some(language) = tree_sitter_language(source.detection.language) else {
        return Ok(SourceMap { symbols });
    };

    if !language_has_symbols(source.detection.language) {
        return Ok(SourceMap { symbols });
    }

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| anyhow!("set tree-sitter language: {error}"))?;
    let tree = parser
        .parse(&source.text, None)
        .ok_or_else(|| anyhow!("tree-sitter parse failed"))?;
    collect_symbols(
        tree.root_node(),
        &source.text,
        source.detection.language,
        None,
        &source.lines,
        &mut symbols,
    );
    symbols.sort_by_key(|symbol| (symbol.start_line, symbol.end_line));

    Ok(SourceMap { symbols })
}

pub(crate) fn tree_sitter_language(language: Language) -> Option<tree_sitter::Language> {
    let parser = match language {
        Language::Assembly => tree_sitter_asm_language,
        Language::Bash => tree_sitter_bash_language,
        Language::C => tree_sitter_c_language,
        Language::Cpp => tree_sitter_cpp_language,
        Language::CSharp => tree_sitter_csharp_language,
        Language::Css => tree_sitter_css_language,
        Language::Dockerfile => tree_sitter_dockerfile_language,
        Language::Go => tree_sitter_go_language,
        Language::Gdscript => tree_sitter_gdscript_language,
        Language::Java => tree_sitter_java_language,
        Language::JavaScript | Language::Jsx => tree_sitter_javascript_language,
        Language::Html => tree_sitter_html_language,
        Language::Json => tree_sitter_json_language,
        Language::Xml => tree_sitter_xml_language,
        Language::Yaml => tree_sitter_yaml_language,
        Language::Just => tree_sitter_just_language,
        Language::Kconfig => tree_sitter_kconfig_language,
        Language::Latex => tree_sitter_latex_language,
        Language::Lua => tree_sitter_lua_language,
        Language::Make => tree_sitter_make_language,
        Language::Markdown => tree_sitter_markdown_language,
        Language::Meson => tree_sitter_meson_language,
        Language::Nix => tree_sitter_nix_language,
        Language::Perl => tree_sitter_perl_language,
        Language::Python => tree_sitter_python_language,
        Language::Php => tree_sitter_php_language,
        Language::Puppet => tree_sitter_puppet_language,
        Language::Ruby => tree_sitter_ruby_language,
        Language::Riscv => tree_sitter_riscv_language,
        Language::Rust => tree_sitter_rust_language,
        Language::Swift => tree_sitter_swift_language,
        Language::Sql => tree_sitter_sql_language,
        Language::Typst => tree_sitter_typst_language,
        Language::TypeScript => tree_sitter_typescript_language,
        Language::Tsx => tree_sitter_tsx_language,
        Language::Toml => tree_sitter_toml_language,
        Language::Vimscript => tree_sitter_vim_language,
        Language::Zig => tree_sitter_zig_language,
        Language::Unknown => return None,
    };

    Some(parser())
}

fn tree_sitter_asm_language() -> tree_sitter::Language {
    tree_sitter_asm::LANGUAGE.into()
}
fn tree_sitter_bash_language() -> tree_sitter::Language {
    tree_sitter_bash::LANGUAGE.into()
}

fn tree_sitter_c_language() -> tree_sitter::Language {
    tree_sitter_c::LANGUAGE.into()
}

fn tree_sitter_cpp_language() -> tree_sitter::Language {
    tree_sitter_cpp::LANGUAGE.into()
}

fn tree_sitter_csharp_language() -> tree_sitter::Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}

fn tree_sitter_css_language() -> tree_sitter::Language {
    tree_sitter_css::LANGUAGE.into()
}

fn tree_sitter_dockerfile_language() -> tree_sitter::Language {
    tree_sitter_containerfile::LANGUAGE.into()
}

fn tree_sitter_go_language() -> tree_sitter::Language {
    tree_sitter_go::LANGUAGE.into()
}

fn tree_sitter_gdscript_language() -> tree_sitter::Language {
    tree_sitter_gdscript::LANGUAGE.into()
}

fn tree_sitter_java_language() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
}

fn tree_sitter_html_language() -> tree_sitter::Language {
    tree_sitter_html::LANGUAGE.into()
}

fn tree_sitter_json_language() -> tree_sitter::Language {
    tree_sitter_json::LANGUAGE.into()
}

fn tree_sitter_xml_language() -> tree_sitter::Language {
    tree_sitter_xml::LANGUAGE_XML.into()
}

fn tree_sitter_yaml_language() -> tree_sitter::Language {
    tree_sitter_yaml::LANGUAGE.into()
}

fn tree_sitter_javascript_language() -> tree_sitter::Language {
    tree_sitter_javascript::LANGUAGE.into()
}

fn tree_sitter_just_language() -> tree_sitter::Language {
    tree_sitter_just::LANGUAGE.into()
}

fn tree_sitter_kconfig_language() -> tree_sitter::Language {
    tree_sitter_kconfig::LANGUAGE.into()
}

fn tree_sitter_latex_language() -> tree_sitter::Language {
    codebook_tree_sitter_latex::LANGUAGE.into()
}

fn tree_sitter_lua_language() -> tree_sitter::Language {
    tree_sitter_lua::LANGUAGE.into()
}

fn tree_sitter_make_language() -> tree_sitter::Language {
    tree_sitter_make::LANGUAGE.into()
}

fn tree_sitter_markdown_language() -> tree_sitter::Language {
    tree_sitter_md_025::LANGUAGE.into()
}

fn tree_sitter_meson_language() -> tree_sitter::Language {
    arborium_meson::language().into()
}

fn tree_sitter_nix_language() -> tree_sitter::Language {
    tree_sitter_nix::LANGUAGE.into()
}

fn tree_sitter_perl_language() -> tree_sitter::Language {
    ts_parser_perl::LANGUAGE.into()
}

fn tree_sitter_php_language() -> tree_sitter::Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}

fn tree_sitter_python_language() -> tree_sitter::Language {
    tree_sitter_python::LANGUAGE.into()
}

fn tree_sitter_puppet_language() -> tree_sitter::Language {
    tree_sitter_puppet::LANGUAGE.into()
}

fn tree_sitter_ruby_language() -> tree_sitter::Language {
    tree_sitter_ruby::LANGUAGE.into()
}

fn tree_sitter_riscv_language() -> tree_sitter::Language {
    tree_sitter_riscv::LANGUAGE.into()
}

fn tree_sitter_rust_language() -> tree_sitter::Language {
    tree_sitter_rust::LANGUAGE.into()
}

fn tree_sitter_swift_language() -> tree_sitter::Language {
    tree_sitter_swift::LANGUAGE.into()
}

fn tree_sitter_sql_language() -> tree_sitter::Language {
    tree_sitter_sequel::LANGUAGE.into()
}

fn tree_sitter_typst_language() -> tree_sitter::Language {
    codebook_tree_sitter_typst::LANGUAGE.into()
}

fn tree_sitter_typescript_language() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}

fn tree_sitter_tsx_language() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}

fn tree_sitter_toml_language() -> tree_sitter::Language {
    tree_sitter_toml_ng::LANGUAGE.into()
}

fn tree_sitter_vim_language() -> tree_sitter::Language {
    tree_sitter_vim::language()
}

fn tree_sitter_zig_language() -> tree_sitter::Language {
    tree_sitter_zig::LANGUAGE.into()
}

fn collect_symbols(
    node: Node<'_>,
    source: &str,
    language: Language,
    parent: Option<&str>,
    lines: &[SourceLine],
    symbols: &mut Vec<Symbol>,
) {
    let symbol = symbol_for_node(node, source, language, parent, lines);
    let next_parent = symbol
        .as_ref()
        .map(|symbol| symbol.address.clone())
        .or_else(|| parent.map(ToOwned::to_owned));
    if let Some(symbol) = symbol {
        symbols.push(symbol);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(
            child,
            source,
            language,
            next_parent.as_deref(),
            lines,
            symbols,
        );
    }
}

fn language_has_symbols(language: Language) -> bool {
    !matches!(
        language,
        Language::Assembly
            | Language::Css
            | Language::Dockerfile
            | Language::Html
            | Language::Gdscript
            | Language::Json
            | Language::Latex
            | Language::Lua
            | Language::Meson
            | Language::Nix
            | Language::Perl
            | Language::Puppet
            | Language::Riscv
            | Language::Sql
            | Language::Xml
            | Language::Typst
            | Language::Toml
            | Language::Yaml
            | Language::Zig
            | Language::Unknown
    )
}

fn symbol_for_node(
    node: Node<'_>,
    source: &str,
    language: Language,
    parent: Option<&str>,
    lines: &[SourceLine],
) -> Option<Symbol> {
    if !language_has_symbols(language) {
        return None;
    }
    let (kind, name) = match language {
        Language::Rust => rust_symbol(node, source),
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            js_like_symbol(node, source)
        }
        Language::Python => python_symbol(node, source),
        Language::Bash => bash_symbol(node, source),
        Language::C | Language::Cpp => c_like_symbol(node, source),
        Language::CSharp => csharp_symbol(node, source),
        Language::Go => go_symbol(node, source),
        Language::Java => java_symbol(node, source),
        Language::Just => just_symbol(node, source),
        Language::Kconfig => kconfig_symbol(node, source),
        Language::Markdown => markdown_symbol(node, source),
        Language::Make => make_symbol(node, source),
        Language::Php => php_symbol(node, source),
        Language::Ruby => ruby_symbol(node, source),
        Language::Swift => swift_symbol(node, source),
        Language::Vimscript => vimscript_symbol(node, source),
        Language::Assembly
        | Language::Css
        | Language::Dockerfile
        | Language::Html
        | Language::Gdscript
        | Language::Json
        | Language::Latex
        | Language::Lua
        | Language::Meson
        | Language::Nix
        | Language::Perl
        | Language::Puppet
        | Language::Riscv
        | Language::Sql
        | Language::Xml
        | Language::Typst
        | Language::Toml
        | Language::Yaml
        | Language::Zig
        | Language::Unknown => unreachable!(),
    }?;

    let (start_line, end_line) = symbol_line_range(language, node);
    let start_hash = line_hash(lines, start_line)?;
    let end_hash = line_hash(lines, end_line)?;
    let address = parent.map_or_else(|| name.clone(), |parent| format!("{parent}.{name}"));

    Some(Symbol {
        kind,
        name,
        address,
        start_line,
        end_line,
        start_hash,
        end_hash,
    })
}

pub(crate) fn node_line_range(node: Node<'_>) -> (usize, usize) {
    let start_line = node_start_line(node);
    let end_position = node.end_position();
    let end_line = if end_position.column == 0 && end_position.row + 1 > start_line {
        end_position.row
    } else {
        end_position.row + 1
    };

    (start_line, end_line)
}

fn node_start_line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

fn symbol_line_range(language: Language, node: Node<'_>) -> (usize, usize) {
    if language == Language::Markdown && node.kind() == "atx_heading" {
        let start_line = node_start_line(node);
        return (start_line, start_line);
    }

    node_line_range(node)
}

fn rust_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_item" => named_symbol(node, source, "name", "function"),
        "struct_item" => named_symbol(node, source, "name", "struct"),
        "enum_item" => named_symbol(node, source, "name", "enum"),
        "trait_item" => named_symbol(node, source, "name", "trait"),
        "impl_item" => named_symbol(node, source, "type", "impl"),
        "mod_item" => named_symbol(node, source, "name", "module"),
        _ => None,
    }
}

fn js_like_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            named_symbol(node, source, "name", "function")
        }
        "method_definition" => named_symbol(node, source, "name", "method"),
        "class_declaration" => named_symbol(node, source, "name", "class"),
        "interface_declaration" => named_symbol(node, source, "name", "interface"),
        "type_alias_declaration" => named_symbol(node, source, "name", "type"),
        "lexical_declaration" | "variable_declaration" => variable_function_symbol(node, source),
        _ => None,
    }
}

fn python_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_definition" => named_symbol(node, source, "name", "function"),
        "class_definition" => named_symbol(node, source, "name", "class"),
        _ => None,
    }
}

fn just_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "recipe_header" => named_symbol(node, source, "name", "recipe"),
        _ => None,
    }
}

fn kconfig_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "config" | "menuconfig" => named_symbol(node, source, "name", node.kind()),
        "menu" => child_text(node, source, "name")
            .map(|name| ("menu".to_owned(), name.trim_matches('"').to_owned())),
        _ => None,
    }
}

fn make_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "rule" => descendant_identifier(node, source).map(|name| ("target".to_owned(), name)),
        _ => None,
    }
}

fn markdown_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "atx_heading" | "setext_heading" => {
            markdown_heading_name(node, source).map(|name| ("heading".to_owned(), name))
        }
        _ => None,
    }
}

fn markdown_heading_name(node: Node<'_>, source: &str) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?.trim();
    let name = text
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim();
    name.lines()
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn bash_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_definition" => {
            descendant_identifier(node, source).map(|name| ("function".to_owned(), name))
        }
        _ => None,
    }
}

fn vimscript_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_definition" => named_child_of_kind(node, "function_declaration")
            .and_then(|declaration| named_child(declaration, source, "name"))
            .map(|name| ("function".to_owned(), name)),
        _ => None,
    }
}

fn named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn c_like_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_definition" => {
            descendant_identifier(node, source).map(|name| ("function".to_owned(), name))
        }
        "struct_specifier" => named_symbol(node, source, "name", "struct"),
        "enum_specifier" => named_symbol(node, source, "name", "enum"),
        "class_specifier" => named_symbol(node, source, "name", "class"),
        "namespace_definition" => named_symbol(node, source, "name", "namespace"),
        "preproc_def" | "preproc_function_def" => {
            descendant_identifier(node, source).map(|name| ("macro".to_owned(), name))
        }
        _ => None,
    }
}

fn csharp_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "method_declaration" => named_symbol(node, source, "name", "method"),
        "constructor_declaration" => named_symbol(node, source, "name", "constructor"),
        "class_declaration" => named_symbol(node, source, "name", "class"),
        "interface_declaration" => named_symbol(node, source, "name", "interface"),
        "struct_declaration" => named_symbol(node, source, "name", "struct"),
        "enum_declaration" => named_symbol(node, source, "name", "enum"),
        "namespace_declaration" => named_symbol(node, source, "name", "namespace"),
        _ => None,
    }
}

fn go_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_declaration" => named_symbol(node, source, "name", "function"),
        "method_declaration" => named_symbol(node, source, "name", "method"),
        "type_declaration" => {
            descendant_identifier(node, source).map(|name| ("type".to_owned(), name))
        }
        _ => None,
    }
}

fn java_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "method_declaration" => named_symbol(node, source, "name", "method"),
        "constructor_declaration" => named_symbol(node, source, "name", "constructor"),
        "class_declaration" => named_symbol(node, source, "name", "class"),
        "interface_declaration" => named_symbol(node, source, "name", "interface"),
        "enum_declaration" => named_symbol(node, source, "name", "enum"),
        _ => None,
    }
}

fn php_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "function_definition" => named_symbol(node, source, "name", "function"),
        "method_declaration" => named_symbol(node, source, "name", "method"),
        "class_declaration" => named_symbol(node, source, "name", "class"),
        "interface_declaration" => named_symbol(node, source, "name", "interface"),
        "trait_declaration" => named_symbol(node, source, "name", "trait"),
        _ => None,
    }
}

fn ruby_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "method" | "singleton_method" => named_symbol(node, source, "name", "method"),
        "class" => named_symbol(node, source, "name", "class"),
        "module" => named_symbol(node, source, "name", "module"),
        _ => None,
    }
}

fn swift_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "class_declaration" => {
            let declaration_kind = child_text(node, source, "declaration_kind")?;
            let name = named_child(node, source, "name")?;
            Some(("class".to_owned(), format!("{declaration_kind} {name}")))
        }
        "function_declaration" | "protocol_function_declaration" => {
            swift_function_name(node, source).map(|name| ("function".to_owned(), name))
        }
        "deinit_declaration" => Some(("deinit".to_owned(), "deinit".to_owned())),
        _ => None,
    }
}

fn swift_function_name(node: Node<'_>, source: &str) -> Option<String> {
    let name = child_text(node, source, "name")?;
    let start_byte = node.start_byte();
    let end_byte = node
        .child_by_field_name("body")
        .map_or_else(|| node.end_byte(), |body| body.start_byte());
    let prefix = source.get(start_byte..end_byte)?.trim();
    if prefix.starts_with("func ")
        || prefix.starts_with("static func ")
        || prefix.starts_with("class func ")
    {
        return Some(prefix.trim_end().to_owned());
    }
    Some(name)
}

fn child_text(node: Node<'_>, source: &str, field: &str) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|child| child.utf8_text(source.as_bytes()).ok())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn variable_function_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }

        let value = child.child_by_field_name("value")?;
        if !matches!(value.kind(), "arrow_function" | "function_expression") {
            continue;
        }

        return named_child(child, source, "name").map(|name| ("function".to_owned(), name));
    }

    None
}

fn named_child(node: Node<'_>, source: &str, field: &str) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|child| child.utf8_text(source.as_bytes()).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn named_symbol(node: Node<'_>, source: &str, field: &str, kind: &str) -> Option<(String, String)> {
    named_child(node, source, field).map(|name| (kind.to_owned(), name))
}

fn descendant_identifier(node: Node<'_>, source: &str) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "word" | "variable_name"
    ) {
        return node
            .utf8_text(source.as_bytes())
            .ok()
            .map(ToOwned::to_owned);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = descendant_identifier(child, source) {
            return Some(name);
        }
    }

    None
}

fn line_hash(lines: &[SourceLine], line: usize) -> Option<String> {
    lines
        .get(line.checked_sub(1)?)
        .map(|line| line.hash.clone())
}
