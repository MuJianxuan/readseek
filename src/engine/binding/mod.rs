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

use crate::engine::lang::Language;
use crate::engine::source::SourceFile;
use crate::engine::symbols::tree_sitter_language;
use serde::Serialize;
use tree_sitter::{Node, Parser, Tree};

mod cpp;
mod python;
mod rust;
mod typescript;
mod vimscript;

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

/// Parse `source` and return its binding table and syntax tree, or `None` when
/// the language has no binding support or the parse fails.
fn parse_source(source: &SourceFile) -> Option<(&'static BindingTable, Tree)> {
    let table = binding_table(source.detection.language)?;
    let language = tree_sitter_language(source.detection.language)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source.text, None)?;
    Some((table, tree))
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
    let (table, tree) = parse_source(source)?;
    let root = tree.root_node();
    let src = source.text.as_bytes();

    let lookup = byte.min(source.text.len().saturating_sub(1));
    let cursor = identifier_leaf(root.descendant_for_byte_range(lookup, lookup)?, table)?;
    let name = cursor.utf8_text(src).ok()?.to_owned();

    let mut declarations = Vec::new();
    collect_declarations(root, src, table, &mut declarations);

    let target_def = resolve_node(cursor, &name, &declarations, table)?;

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
        .map(|new_name| find_conflicts(root, new_name, &occurrences, &declarations, table))
        .unwrap_or_default();

    Some((Binding { name, occurrences }, conflicts))
}

/// The identifier text under `byte`, if the cursor sits on one.
///
/// Unlike [`resolve`], this does not require the name to bind to a local
/// declaration, so it also names top-level symbols (functions, types) the
/// resolver does not track but a cross-file rename still targets.
pub(crate) fn identifier_at(source: &SourceFile, byte: usize) -> Option<String> {
    if let Some((table, tree)) = parse_source(source) {
        let root = tree.root_node();
        let lookup = byte.min(source.text.len().saturating_sub(1));
        let leaf = identifier_leaf(root.descendant_for_byte_range(lookup, lookup)?, table)?;
        return leaf
            .utf8_text(source.text.as_bytes())
            .ok()
            .map(str::to_owned);
    }
    crate::engine::symbols::token_at(source, byte).map(|token| token.text)
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
    let (table, tree) = parse_source(source)?;
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
    root: Node<'_>,
    src: &[u8],
    table: &BindingTable,
    name: &str,
    new_name: &str,
    declarations: &[Declaration<'_>],
    out: &mut CrossFileMatches,
) {
    let mut stack: Vec<Node<'_>> = vec![root];
    while let Some(node) = stack.pop() {
        if is_identifier_kind(node.kind(), table)
            && node.child_count() == 0
            && (table.is_reference)(node)
            && node.utf8_text(src) == Ok(name)
            && resolve_node(node, name, declarations, table).is_none()
        {
            out.occurrences.push((node.start_byte(), node.end_byte()));
            if resolve_node(node, new_name, declarations, table).is_some() {
                out.conflicts.push(node.start_byte());
            }
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

/// Check whether renaming the binding to `new_name` would capture or be captured.
fn find_conflicts(
    root: Node<'_>,
    new_name: &str,
    occurrences: &[Occurrence],
    declarations: &[Declaration<'_>],
    table: &BindingTable,
) -> Vec<Conflict> {
    occurrences
        .iter()
        .filter_map(|occurrence| {
            let byte = occurrence.start_byte;
            let node = root.descendant_for_byte_range(byte, byte)?;
            resolve_node(node, new_name, declarations, table)
                .is_some()
                .then(|| Conflict {
                    byte,
                    reason: format!("`{new_name}` already resolves to a binding here"),
                })
        })
        .collect()
}

/// A declared name together with the identifier node and the scope it lives in.
struct Declaration<'tree> {
    name: String,
    ident: Node<'tree>,
    scope: usize,
}

/// The innermost enclosing scope node's id, or the root id when none applies.
///
/// An identifier that escapes its scope (a parameter default or leading
/// comprehension iterable) is placed in the scope enclosing its nearest
/// syntactic scope, matching where Python evaluates it.
fn scope_of(node: Node<'_>, table: &BindingTable) -> usize {
    let mut current = if (table.escapes_scope)(node) {
        enclosing_scope(node, table).and_then(|scope| scope.parent())
    } else {
        node.parent()
    };
    while let Some(parent) = current {
        if table.scope_kinds.contains(&parent.kind()) && !(table.binds_past)(node, parent.kind()) {
            return parent.id();
        }
        current = parent.parent();
    }
    node_root(node).id()
}

/// The nearest ancestor of `node` that opens a scope, if any.
fn enclosing_scope<'tree>(node: Node<'tree>, table: &BindingTable) -> Option<Node<'tree>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if table.scope_kinds.contains(&parent.kind()) {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn node_root(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        current = parent;
    }
    current
}

fn collect_declarations<'tree>(
    root: Node<'tree>,
    src: &[u8],
    table: &BindingTable,
    out: &mut Vec<Declaration<'tree>>,
) {
    let mut stack: Vec<Node<'tree>> = vec![root];
    while let Some(node) = stack.pop() {
        for ident in (table.declared_idents)(node, src) {
            if let Ok(name) = ident.utf8_text(src) {
                out.push(Declaration {
                    name: name.to_owned(),
                    ident,
                    scope: scope_of(ident, table),
                });
            }
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

/// Resolve an identifier node to the byte offset of its binding declaration.
///
/// Walks enclosing scopes innermost-first and, within the nearest scope that
/// declares the name, picks the binding according to `table.resolution`: under
/// [`Resolution::Lexical`] the nearest lexically-preceding declaration (so a
/// re-declaration shadows), under [`Resolution::Hoisted`] the first declaration
/// in the scope (so all same-name declarations are one binding). Class scopes
/// are consulted only for uses in their own direct body, and an escaping
/// identifier resolves from the scope enclosing its nearest syntactic scope.
fn resolve_node(
    node: Node<'_>,
    name: &str,
    declarations: &[Declaration<'_>],
    table: &BindingTable,
) -> Option<usize> {
    let use_start = node.start_byte();
    let start = if (table.escapes_scope)(node) {
        enclosing_scope(node, table).and_then(|scope| scope.parent())?
    } else {
        node
    };
    let mut scope = Some(start);
    let mut left_innermost_scope = false;
    while let Some(current) = scope {
        let is_scope = current.parent().is_none() || table.scope_kinds.contains(&current.kind());
        let hidden_class =
            left_innermost_scope && table.class_scope_kinds.contains(&current.kind());
        if !hidden_class {
            let scoped = declarations.iter().filter(|declaration| {
                declaration.name == name && declaration.scope == current.id()
            });
            let resolved = match table.resolution {
                Resolution::Lexical => scoped
                    .filter(|declaration| declaration.ident.start_byte() <= use_start)
                    .max_by_key(|declaration| declaration.ident.start_byte()),
                Resolution::Hoisted => {
                    scoped.min_by_key(|declaration| declaration.ident.start_byte())
                }
            };
            if let Some(declaration) = resolved {
                return Some(declaration.ident.start_byte());
            }
        }
        if is_scope {
            left_innermost_scope = true;
        }
        scope = current.parent();
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn collect_occurrences(
    root: Node<'_>,
    src: &[u8],
    table: &BindingTable,
    name: &str,
    target_def: usize,
    declarations: &[Declaration<'_>],
    out: &mut Vec<Occurrence>,
) {
    let mut stack: Vec<Node<'_>> = vec![root];
    while let Some(node) = stack.pop() {
        if is_identifier_kind(node.kind(), table)
            && node.child_count() == 0
            && (table.is_reference)(node)
            && node.utf8_text(src) == Ok(name)
        {
            let resolved = resolve_node(node, name, declarations, table);
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
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
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
    (is_identifier_kind(current.kind(), table) && (table.is_reference)(current)).then_some(current)
}

fn is_identifier_kind(kind: &str, table: &BindingTable) -> bool {
    table.identifier_kinds.contains(&kind)
}

/// How repeated declarations of one name within a scope relate to each other.
#[derive(Clone, Copy)]
enum Resolution {
    /// A later declaration shadows earlier ones; a use binds to the nearest
    /// declaration that lexically precedes it (Rust, C/C++).
    Lexical,
    /// All declarations of a name in a scope are the same binding; a use binds to
    /// the first one regardless of order (Python reassignment).
    Hoisted,
}

/// Per-language description of scopes and the identifiers that introduce bindings.
struct BindingTable {
    /// Node kinds that open a new lexical scope.
    scope_kinds: &'static [&'static str],
    /// Scope kinds whose declarations are visible only to their own direct body,
    /// not to any nested scope (Python class bodies).
    class_scope_kinds: &'static [&'static str],
    /// Leaf node kinds that count as references to a name.
    identifier_kinds: &'static [&'static str],
    /// Given a node and the source bytes, the identifier nodes that introduce a
    /// binding in its scope.
    declared_idents: for<'a, 'b> fn(Node<'a>, &'b [u8]) -> Vec<Node<'a>>,
    /// How a use resolves among same-name declarations sharing a scope.
    resolution: Resolution,
    /// Whether an identifier leaf is a renameable reference rather than, for
    /// example, an attribute member or keyword-argument name.
    is_reference: fn(Node<'_>) -> bool,
    /// Whether an identifier is evaluated in the scope enclosing its nearest
    /// syntactic scope (a parameter default or a leading comprehension iterable).
    escapes_scope: fn(Node<'_>) -> bool,
    /// Whether a declaration binds past a scope of the given kind rather than in
    /// it (a Python walrus target binds past the comprehension it appears in).
    binds_past: fn(Node<'_>, &str) -> bool,
}

/// Default reference filter: every identifier-kind leaf is a reference.
fn any_reference(_node: Node<'_>) -> bool {
    true
}

/// Default scope-escape predicate: identifiers stay in their syntactic scope.
fn never_escapes(_node: Node<'_>) -> bool {
    false
}

/// Default bind-past predicate: declarations bind in their nearest scope.
fn never_binds_past(_node: Node<'_>, _scope_kind: &str) -> bool {
    false
}

fn binding_table(language: Language) -> Option<&'static BindingTable> {
    match language {
        Language::Rust => Some(&RUST_TABLE),
        Language::C | Language::Cpp => Some(&CPP_TABLE),
        Language::Python => Some(&PYTHON_TABLE),
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            Some(&TYPESCRIPT_TABLE)
        }
        Language::Vimscript => Some(&VIMSCRIPT_TABLE),
        _ => None,
    }
}

static RUST_TABLE: BindingTable = BindingTable {
    scope_kinds: &[
        "block",
        "function_item",
        "closure_expression",
        "match_arm",
        "for_expression",
        "while_let_expression",
        "if_let_expression",
    ],
    class_scope_kinds: &[],
    identifier_kinds: &["identifier"],
    declared_idents: rust::declared_idents,
    resolution: Resolution::Lexical,
    is_reference: any_reference,
    escapes_scope: never_escapes,
    binds_past: never_binds_past,
};

static CPP_TABLE: BindingTable = BindingTable {
    scope_kinds: &[
        "compound_statement",
        "function_definition",
        "for_statement",
        "for_range_loop",
        "lambda_expression",
    ],
    class_scope_kinds: &[],
    identifier_kinds: &["identifier"],
    declared_idents: cpp::declared_idents,
    resolution: Resolution::Lexical,
    is_reference: any_reference,
    escapes_scope: never_escapes,
    binds_past: never_binds_past,
};

static PYTHON_TABLE: BindingTable = BindingTable {
    scope_kinds: &[
        "function_definition",
        "lambda",
        "class_definition",
        "list_comprehension",
        "set_comprehension",
        "dictionary_comprehension",
        "generator_expression",
    ],
    class_scope_kinds: &["class_definition"],
    identifier_kinds: &["identifier"],
    declared_idents: python::declared_idents,
    resolution: Resolution::Hoisted,
    is_reference: python::is_reference,
    escapes_scope: python::escapes_scope,
    binds_past: python::binds_past,
};

static TYPESCRIPT_TABLE: BindingTable = BindingTable {
    scope_kinds: &[
        "statement_block",
        "function_declaration",
        "function_expression",
        "generator_function_declaration",
        "arrow_function",
        "method_definition",
        "class_declaration",
        "for_statement",
        "for_in_statement",
        "catch_clause",
    ],
    class_scope_kinds: &["class_declaration"],
    identifier_kinds: &[
        "identifier",
        "shorthand_property_identifier",
        "shorthand_property_identifier_pattern",
    ],
    declared_idents: typescript::declared_idents,
    resolution: Resolution::Lexical,
    is_reference: typescript::is_reference,
    escapes_scope: typescript::is_hoisted_name,
    binds_past: never_binds_past,
};

static VIMSCRIPT_TABLE: BindingTable = BindingTable {
    scope_kinds: &[
        "function_definition",
        "lambda_expression",
        "if_statement",
        "while_loop",
        "for_loop",
        "try_statement",
    ],
    class_scope_kinds: &[],
    identifier_kinds: &["identifier", "name"],
    declared_idents: vimscript::declared_idents,
    resolution: Resolution::Lexical,
    is_reference: vimscript::is_reference,
    escapes_scope: vimscript::escapes_scope,
    binds_past: never_binds_past,
};
