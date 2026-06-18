// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Per-file scope and binding resolution.
//!
//! Given a cursor position, this resolves which other occurrences in the same
//! file bind to the *same* declaration, so callers can distinguish a local from
//! a same-named binding in another scope. Resolution is conservative: it is only
//! attempted for languages with a binding table below, and only for names that
//! resolve to a lexical declaration. Everything else is reported as unresolved
//! so callers can fall back to name matching without silently over-matching.

use crate::lang::Language;
use crate::source::SourceFile;
use crate::symbols::tree_sitter_language;
use serde::Serialize;
use tree_sitter::{Node, Parser};

/// How an occurrence relates to the resolved binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OccurrenceKind {
    /// The identifier that introduces the binding.
    Definition,
    /// A use that resolves to the binding.
    Reference,
    /// Same name, but resolves to a different binding (shadowed or unrelated).
    Shadowed,
}

/// One resolved occurrence of the target name within a file.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Occurrence {
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) kind: OccurrenceKind,
}

/// The binding the cursor token resolves to, with every same-file occurrence
/// classified relative to it.
#[derive(Debug)]
pub(crate) struct Binding {
    pub(crate) name: String,
    pub(crate) occurrences: Vec<Occurrence>,
}

/// A name collision a rename to `new_name` would introduce.
#[derive(Debug)]
pub(crate) struct Conflict {
    pub(crate) byte: usize,
    pub(crate) reason: String,
}

/// Whether binding resolution is implemented for `language`. Lets callers
/// distinguish an unsupported language from a genuine resolution failure
/// without exposing the private binding-table type.
pub(crate) fn supported(language: Language) -> bool {
    binding_table(language).is_some()
}

/// Resolve the lexical binding for the identifier covering `byte`.
///
/// Returns `None` when the language is unsupported, the parse fails, the cursor
/// is not on an identifier, or the name has no resolvable lexical declaration.
pub(crate) fn resolve(source: &SourceFile, byte: usize) -> Option<Binding> {
    resolve_with_conflicts(source, byte, None).map(|(binding, _)| binding)
}

/// Resolve the binding and, when `new_name` is given, report rename conflicts.
///
/// A conflict is reported when `new_name` already resolves to a declaration that
/// is visible from a renamed occurrence; renaming would then change which
/// declaration that occurrence binds to. This is conservative: it flags possible
/// capture rather than proving it.
pub(crate) fn resolve_with_conflicts(
    source: &SourceFile,
    byte: usize,
    new_name: Option<&str>,
) -> Option<(Binding, Vec<Conflict>)> {
    let table = binding_table(source.detection.language)?;
    let language = tree_sitter_language(source.detection.language)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source.text, None)?;
    let root = tree.root_node();
    let src = source.text.as_bytes();

    let lookup = byte.min(source.text.len().saturating_sub(1));
    let cursor = identifier_leaf(root.descendant_for_byte_range(lookup, lookup)?, table)?;
    let name = cursor.utf8_text(src).ok()?.to_owned();

    let mut declarations = Vec::new();
    collect_declarations(root, src, table, &mut declarations);

    let target_def = resolve_node(cursor, &name, &declarations)?;

    let mut occurrences = Vec::new();
    collect_occurrences(
        root,
        src,
        table,
        &name,
        target_def,
        &declarations,
        &mut occurrences,
    );
    occurrences.sort_by_key(|occurrence| occurrence.start_byte);

    let conflicts = new_name
        .map(|new_name| find_conflicts(root, new_name, &occurrences, &declarations))
        .unwrap_or_default();

    Some((Binding { name, occurrences }, conflicts))
}

/// The identifier text under `byte`, if the cursor sits on one.
///
/// Unlike [`resolve`], this does not require the name to bind to a local
/// declaration, so it also names top-level symbols (functions, types) the
/// resolver does not track but a cross-file rename still targets.
pub(crate) fn identifier_at(source: &SourceFile, byte: usize) -> Option<String> {
    let table = binding_table(source.detection.language)?;
    let language = tree_sitter_language(source.detection.language)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source.text, None)?;
    let root = tree.root_node();
    let lookup = byte.min(source.text.len().saturating_sub(1));
    let leaf = identifier_leaf(root.descendant_for_byte_range(lookup, lookup)?, table)?;
    leaf.utf8_text(source.text.as_bytes())
        .ok()
        .map(str::to_owned)
}

/// Name occurrences a cross-file rename should touch within one file.
pub(crate) struct CrossFileMatches {
    /// Byte spans of free uses of the old name to rename.
    pub(crate) occurrences: Vec<(usize, usize)>,
    /// Byte offsets where renaming to the new name would capture a local binding.
    pub(crate) conflicts: Vec<usize>,
}

/// Find the occurrences of `name` in a file that take part in a cross-file
/// rename to `new_name`.
///
/// readseek has no cross-file symbol resolver, so a use in another file is taken
/// to reference the cross-file target only when it does *not* resolve to a local
/// declaration here; an occurrence that binds to a local declaration is a shadow
/// and is left untouched. Returns `None` for languages without a binding table,
/// so the caller can fall back to a plain name scan.
pub(crate) fn cross_file_matches(
    source: &SourceFile,
    name: &str,
    new_name: &str,
) -> Option<CrossFileMatches> {
    let table = binding_table(source.detection.language)?;
    let language = tree_sitter_language(source.detection.language)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source.text, None)?;
    let root = tree.root_node();
    let src = source.text.as_bytes();

    let mut declarations = Vec::new();
    collect_declarations(root, src, table, &mut declarations);

    let mut matches = CrossFileMatches {
        occurrences: Vec::new(),
        conflicts: Vec::new(),
    };
    collect_free_occurrences(
        root,
        src,
        table,
        name,
        new_name,
        &declarations,
        &mut matches,
    );
    matches.occurrences.sort_by_key(|&(start, _)| start);
    matches.conflicts.sort_unstable();
    Some(matches)
}

/// Walk the tree collecting uses of `name` that do not resolve to a local
/// declaration, flagging any where `new_name` already binds locally (capture).
#[allow(clippy::too_many_arguments)]
fn collect_free_occurrences(
    node: Node<'_>,
    src: &[u8],
    table: &BindingTable,
    name: &str,
    new_name: &str,
    declarations: &[Declaration<'_>],
    out: &mut CrossFileMatches,
) {
    if is_identifier_kind(node.kind(), table)
        && node.child_count() == 0
        && node.utf8_text(src) == Ok(name)
        && resolve_node(node, name, declarations).is_none()
    {
        out.occurrences.push((node.start_byte(), node.end_byte()));
        if resolve_node(node, new_name, declarations).is_some() {
            out.conflicts.push(node.start_byte());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_free_occurrences(child, src, table, name, new_name, declarations, out);
    }
}

/// Check whether renaming the binding to `new_name` would capture or be captured.
fn find_conflicts(
    root: Node<'_>,
    new_name: &str,
    occurrences: &[Occurrence],
    declarations: &[Declaration<'_>],
) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    for occurrence in occurrences {
        let byte = occurrence.start_byte;
        let Some(node) = root.descendant_for_byte_range(byte, byte) else {
            continue;
        };
        if resolve_node(node, new_name, declarations).is_some() {
            conflicts.push(Conflict {
                byte,
                reason: format!("`{new_name}` already resolves to a binding here"),
            });
        }
    }
    conflicts
}

/// A declared name together with the identifier node and the scope it lives in.
struct Declaration<'tree> {
    name: String,
    ident: Node<'tree>,
    scope: usize,
}

/// The innermost enclosing scope node's id, or the root id when none applies.
fn scope_of(node: Node<'_>, table: &BindingTable) -> usize {
    let mut current = node.parent();
    while let Some(parent) = current {
        if table.scope_kinds.contains(&parent.kind()) {
            return parent.id();
        }
        current = parent.parent();
    }
    node_root(node).id()
}

fn node_root(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        current = parent;
    }
    current
}

fn collect_declarations<'tree>(
    node: Node<'tree>,
    src: &[u8],
    table: &BindingTable,
    out: &mut Vec<Declaration<'tree>>,
) {
    for ident in (table.declared_idents)(node) {
        if let Ok(name) = ident.utf8_text(src) {
            out.push(Declaration {
                name: name.to_owned(),
                ident,
                scope: scope_of(ident, table),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_declarations(child, src, table, out);
    }
}

/// Resolve an identifier node to the byte offset of its binding declaration.
///
/// Walks enclosing scopes innermost-first and picks the nearest declaration of
/// the same name, so an inner declaration shadows an outer one.
fn resolve_node(node: Node<'_>, name: &str, declarations: &[Declaration<'_>]) -> Option<usize> {
    let use_start = node.start_byte();
    let mut scope = Some(node);
    while let Some(current) = scope {
        // Within a scope, Rust bindings are not hoisted: a use only resolves to a
        // declaration that lexically precedes it (or is the use itself). The last
        // such declaration wins, matching re-binding with the same name.
        if let Some(declaration) = declarations
            .iter()
            .filter(|declaration| {
                declaration.name == name
                    && declaration.scope == current.id()
                    && declaration.ident.start_byte() <= use_start
            })
            .max_by_key(|declaration| declaration.ident.start_byte())
        {
            return Some(declaration.ident.start_byte());
        }
        scope = current.parent();
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn collect_occurrences(
    node: Node<'_>,
    src: &[u8],
    table: &BindingTable,
    name: &str,
    target_def: usize,
    declarations: &[Declaration<'_>],
    out: &mut Vec<Occurrence>,
) {
    if is_identifier_kind(node.kind(), table)
        && node.child_count() == 0
        && node.utf8_text(src) == Ok(name)
    {
        let resolved = resolve_node(node, name, declarations);
        let kind = if resolved == Some(target_def) {
            if node.start_byte() == target_def {
                OccurrenceKind::Definition
            } else {
                OccurrenceKind::Reference
            }
        } else {
            OccurrenceKind::Shadowed
        };
        out.push(Occurrence {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            kind,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_occurrences(child, src, table, name, target_def, declarations, out);
    }
}

fn identifier_leaf<'tree>(node: Node<'tree>, table: &BindingTable) -> Option<Node<'tree>> {
    let mut current = node;
    while current.named_child_count() > 0 {
        let byte = current.start_byte();
        match current.named_descendant_for_byte_range(byte, byte) {
            Some(child) if child.id() != current.id() => current = child,
            _ => break,
        }
    }
    is_identifier_kind(current.kind(), table).then_some(current)
}

fn is_identifier_kind(kind: &str, table: &BindingTable) -> bool {
    table.identifier_kinds.contains(&kind)
}

/// Per-language description of scopes and the identifiers that introduce bindings.
struct BindingTable {
    /// Languages this table applies to.
    languages: &'static [Language],
    /// Node kinds that open a new lexical scope.
    scope_kinds: &'static [&'static str],
    /// Leaf node kinds that count as references to a name.
    identifier_kinds: &'static [&'static str],
    /// Given a node, the identifier nodes that introduce a binding in its scope.
    declared_idents: fn(Node<'_>) -> Vec<Node<'_>>,
}

fn binding_table(language: Language) -> Option<&'static BindingTable> {
    BINDING_TABLES
        .iter()
        .find(|table| table.languages.contains(&language))
}

/// All per-language binding tables, stored in read-only data.
static BINDING_TABLES: &[BindingTable] = &[
    BindingTable {
        languages: &[Language::Rust],
        scope_kinds: &[
            "block",
            "function_item",
            "closure_expression",
            "match_arm",
            "for_expression",
            "while_let_expression",
            "if_let_expression",
        ],
        identifier_kinds: &["identifier"],
        declared_idents: rust_declared_idents,
    },
    BindingTable {
        languages: &[Language::C, Language::Cpp],
        scope_kinds: &[
            "compound_statement",
            "function_definition",
            "for_statement",
            "for_range_loop",
            "lambda_expression",
        ],
        identifier_kinds: &["identifier"],
        declared_idents: c_declared_idents,
    },
];

/// Collect identifiers that `node` introduces as bindings in Rust.
///
/// Covers `let` bindings (including patterns), function parameters, and `for`
/// loop patterns. Other constructs fall through and are simply not treated as
/// declarations, which is safe: the name then stays unresolved.
fn rust_declared_idents(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    match node.kind() {
        "let_declaration" | "for_expression" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_pattern_idents(pattern, &mut out);
            }
        }
        "parameter" | "closure_parameters" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_pattern_idents(pattern, &mut out);
            } else {
                collect_pattern_idents(node, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn collect_pattern_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    if node.kind() == "identifier" {
        out.push(node);
        return;
    }
    // Skip the path of a struct/enum pattern; only its bound fields are bindings.
    if matches!(node.kind(), "scoped_identifier" | "type_identifier") {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_pattern_idents(child, out);
    }
}

/// Collect identifiers that `node` introduces as a local binding in C/C++.
///
/// Covers block-scope variable declarations, function and lambda parameters, and
/// `for` initializers. Type names, struct field names, and function names use
/// distinct node kinds and are intentionally not treated as variable bindings.
fn c_declared_idents(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    match node.kind() {
        "declaration" | "parameter_declaration" => {
            let mut cursor = node.walk();
            for child in node.children_by_field_name("declarator", &mut cursor) {
                if let Some(ident) = c_declarator_ident(child) {
                    out.push(ident);
                }
            }
        }
        _ => {}
    }
    out
}

/// Descend a C/C++ declarator to the bound identifier.
///
/// Unwraps `init_declarator` and pointer/array/reference declarators, but stops
/// at `function_declarator`: a function declarator names a function, not a local
/// variable, so its identifier is not a renameable local binding.
fn c_declarator_ident(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "identifier" => Some(node),
        "function_declarator" => None,
        _ => c_declarator_ident(node.child_by_field_name("declarator")?),
    }
}
