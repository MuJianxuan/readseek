// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::lang::{AnalysisEngine, DocumentKind, Language, language_spec};
use crate::source::{SourceFile, SourceLine, SourceMap, Symbol};
use anyhow::{Result, anyhow};
use tree_sitter::{Node, Parser};

pub(crate) fn parse_source_map(source: &SourceFile) -> Result<SourceMap> {
    let symbols = Vec::new();

    if source.kind != DocumentKind::Source {
        return Ok(SourceMap { symbols });
    }

    match source.detection.engine.0 {
        Some(AnalysisEngine::TreeSitter) => parse_tree_sitter_source_map(source),
        _ => Ok(SourceMap { symbols }),
    }
}

fn parse_tree_sitter_source_map(source: &SourceFile) -> Result<SourceMap> {
    let mut symbols = Vec::new();

    let Some(language) = tree_sitter_language(source.detection.language) else {
        return Ok(SourceMap { symbols });
    };

    if !language_spec(source.detection.language).is_some_and(|s| s.has_symbols) {
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
    let language = match language {
        Language::Assembly => tree_sitter_asm::LANGUAGE.into(),
        Language::Bash => tree_sitter_bash::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Language::Css => tree_sitter_css::LANGUAGE.into(),
        Language::Dockerfile => tree_sitter_containerfile::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Gdscript => tree_sitter_gdscript::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Language::Html => tree_sitter_html::LANGUAGE.into(),
        Language::Json => tree_sitter_json::LANGUAGE.into(),
        Language::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
        Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        Language::Just => tree_sitter_just::LANGUAGE.into(),
        Language::Kconfig => tree_sitter_kconfig::LANGUAGE.into(),
        Language::Latex => codebook_tree_sitter_latex::LANGUAGE.into(),
        Language::Lua => tree_sitter_lua::LANGUAGE.into(),
        Language::Make => tree_sitter_make::LANGUAGE.into(),
        Language::Markdown => tree_sitter_md_025::LANGUAGE.into(),
        Language::Meson => arborium_meson::language().into(),
        Language::Nix => tree_sitter_nix::LANGUAGE.into(),
        Language::Perl => ts_parser_perl::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Language::Puppet => tree_sitter_puppet::LANGUAGE.into(),
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Language::Riscv => tree_sitter_riscv::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Swift => tree_sitter_swift::LANGUAGE.into(),
        Language::Sql => tree_sitter_sequel::LANGUAGE.into(),
        Language::Typst => codebook_tree_sitter_typst::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Language::Vimscript => tree_sitter_vim::language(),
        Language::Zig => tree_sitter_zig::LANGUAGE.into(),
        Language::Unknown => return None,
    };

    Some(language)
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
        .map(|symbol| symbol.qualified_name.clone())
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

fn symbol_for_node(
    node: Node<'_>,
    source: &str,
    language: Language,
    parent: Option<&str>,
    lines: &[SourceLine],
) -> Option<Symbol> {
    if !language_spec(language).is_some_and(|s| s.has_symbols) {
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
        _ => return None,
    }?;

    let (start_line, end_line) = symbol_line_range(language, node);
    let start_hash = line_hash(lines, start_line)?;
    let end_hash = line_hash(lines, end_line)?;
    let qualified_name = parent.map_or_else(|| name.clone(), |parent| format!("{parent}.{name}"));

    Some(Symbol {
        kind,
        name,
        qualified_name,
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
        "declaration" | "type_definition" => c_declaration_symbol(node, source),
        "preproc_def" | "preproc_function_def" => {
            descendant_identifier(node, source).map(|name| ("macro".to_owned(), name))
        }
        _ => None,
    }
}

fn c_declaration_symbol(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let text = node.utf8_text(source.as_bytes()).ok()?;
    if contains_word(text, "typedef") {
        return c_typedef_symbol(node, text, source);
    }
    if node.kind() != "declaration" || !is_c_file_scope_declaration(node) {
        return None;
    }

    if let Some(function) = descendant_of_kind(node, "function_declarator") {
        return function
            .child_by_field_name("declarator")
            .and_then(|declarator| declarator_identifier(declarator, source))
            .map(|name| ("function".to_owned(), name));
    }

    last_identifier(text).map(|name| ("variable".to_owned(), name))
}

fn c_typedef_symbol(node: Node<'_>, text: &str, source: &str) -> Option<(String, String)> {
    if !contains_word(text, "typedef") {
        return None;
    }

    let name = node
        .child_by_field_name("declarator")
        .and_then(|declarator| declarator_identifier(declarator, source))
        .or_else(|| last_identifier(text))?;

    Some(("type".to_owned(), name))
}

fn is_c_file_scope_declaration(node: Node<'_>) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "translation_unit" | "namespace_definition" | "linkage_specification" => return true,
            "function_definition"
            | "compound_statement"
            | "class_specifier"
            | "struct_specifier"
            | "enum_specifier" => return false,
            _ => current = parent,
        }
    }

    false
}

fn descendant_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(node) = descendant_of_kind(child, kind) {
            return Some(node);
        }
    }

    None
}

fn declarator_identifier(node: Node<'_>, source: &str) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "field_identifier"
    ) {
        return node
            .utf8_text(source.as_bytes())
            .ok()
            .map(ToOwned::to_owned);
    }

    let mut cursor = node.walk();
    let children = node.named_children(&mut cursor).collect::<Vec<_>>();
    for child in children.into_iter().rev() {
        if let Some(name) = declarator_identifier(child, source) {
            return Some(name);
        }
    }

    None
}

fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let word = word.as_bytes();
    let Some(last_start) = bytes.len().checked_sub(word.len()) else {
        return false;
    };

    for index in 0..=last_start {
        if &bytes[index..index + word.len()] != word {
            continue;
        }
        let before = index.checked_sub(1).map(|before_index| bytes[before_index]);
        let after = bytes.get(index + word.len()).copied();
        if before.is_some_and(is_c_identifier_byte) || after.is_some_and(is_c_identifier_byte) {
            continue;
        }
        return true;
    }

    false
}

fn last_identifier(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = bytes.len();
    while index > 0 {
        index -= 1;
        if !is_c_identifier_byte(bytes[index]) {
            continue;
        }

        let end = index + 1;
        while index > 0 && is_c_identifier_byte(bytes[index - 1]) {
            index -= 1;
        }
        if bytes[index].is_ascii_digit() {
            continue;
        }
        return text.get(index..end).map(ToOwned::to_owned);
    }

    None
}

fn is_c_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
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
