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
    node: Node<'tree>,
    src: &[u8],
    table: &BindingTable,
    out: &mut Vec<Declaration<'tree>>,
) {
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
    for child in node.children(&mut cursor) {
        collect_declarations(child, src, table, out);
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
    /// Languages this table applies to.
    languages: &'static [Language],
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
        class_scope_kinds: &[],
        identifier_kinds: &["identifier"],
        declared_idents: rust_declared_idents,
        resolution: Resolution::Lexical,
        is_reference: any_reference,
        escapes_scope: never_escapes,
        binds_past: never_binds_past,
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
        class_scope_kinds: &[],
        identifier_kinds: &["identifier"],
        declared_idents: c_declared_idents,
        resolution: Resolution::Lexical,
        is_reference: any_reference,
        escapes_scope: never_escapes,
        binds_past: never_binds_past,
    },
    BindingTable {
        languages: &[Language::Python],
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
        declared_idents: python_declared_idents,
        resolution: Resolution::Hoisted,
        is_reference: python_is_reference,
        escapes_scope: python_escapes_scope,
        binds_past: python_binds_past,
    },
    BindingTable {
        languages: &[
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Jsx,
        ],
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
        declared_idents: ts_declared_idents,
        resolution: Resolution::Lexical,
        is_reference: ts_is_reference,
        escapes_scope: ts_is_hoisted_name,
        binds_past: never_binds_past,
    },
    BindingTable {
        languages: &[Language::Vimscript],
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
        declared_idents: vimscript_declared_idents,
        resolution: Resolution::Lexical,
        is_reference: vimscript_is_reference,
        escapes_scope: vimscript_escapes_scope,
        binds_past: never_binds_past,
    },
];

/// Collect identifiers that `node` introduces as bindings in Rust.
///
/// Covers `let` bindings (including patterns), function parameters, and `for`
/// loop patterns. Other constructs fall through and are simply not treated as
/// declarations, which is safe: the name then stays unresolved.
fn rust_declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
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
fn c_declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    if matches!(node.kind(), "declaration" | "parameter_declaration") {
        let mut cursor = node.walk();
        out.extend(
            node.children_by_field_name("declarator", &mut cursor)
                .filter_map(c_declarator_ident),
        );
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

/// Whether a Python `identifier` leaf is a renameable name reference.
///
/// Python reuses the `identifier` kind for attribute members (`obj.attr`) and
/// keyword-argument names (`f(arg=1)`), which name a member or parameter rather
/// than a binding in scope; excluding them keeps a rename from over-matching.
fn python_is_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    match parent.kind() {
        "attribute" => parent.child_by_field_name("attribute") != Some(node),
        "keyword_argument" => parent.child_by_field_name("name") != Some(node),
        _ => true,
    }
}

/// Collect identifiers that `node` introduces as a binding in Python.
///
/// Covers function and lambda parameters, assignment, `for`, comprehension and
/// `with ... as` targets, walrus bindings, and the names of `def`/`class`
/// statements (which bind in their enclosing scope). Attribute and subscript
/// targets are not bindings and are skipped. A name declared `global` or
/// `nonlocal` in the enclosing function binds in an outer scope, so it is dropped
/// here and resolves outward instead.
fn python_declared_idents<'tree>(node: Node<'tree>, src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "parameters" | "lambda_parameters" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                python_param_ident(child, &mut out);
            }
        }
        "function_definition" | "class_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                out.push(name);
            }
        }
        "assignment" | "for_statement" | "for_in_clause" | "named_expression" | "as_pattern" => {
            let field = match node.kind() {
                "named_expression" => "name",
                "as_pattern" => "alias",
                _ => "left",
            };
            if let Some(target) = node.child_by_field_name(field) {
                python_target_idents(target, &mut out);
            }
        }
        _ => {}
    }
    out.retain(|ident| !python_freed(*ident, src));
    out
}

/// Whether a name is declared `global`/`nonlocal` in its enclosing function and
/// so does not introduce a local binding there.
fn python_freed(ident: Node<'_>, src: &[u8]) -> bool {
    let Ok(name) = ident.utf8_text(src) else {
        return false;
    };
    let mut current = ident.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "function_definition" | "lambda") {
            return python_body_declares_freed(parent, name, src);
        }
        current = parent.parent();
    }
    false
}

/// Search a function body for a `global`/`nonlocal` statement naming `name`,
/// without descending into nested function, lambda, or class scopes.
fn python_body_declares_freed(node: Node<'_>, name: &str, src: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "global_statement" | "nonlocal_statement" => {
                let mut inner = child.walk();
                if child
                    .children(&mut inner)
                    .any(|id| id.kind() == "identifier" && id.utf8_text(src) == Ok(name))
                {
                    return true;
                }
            }
            "function_definition" | "lambda" | "class_definition" => {}
            _ => {
                if python_body_declares_freed(child, name, src) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether a Python identifier binds or is evaluated in the scope enclosing its
/// nearest syntactic scope: a `def`/`class` name (which binds in the enclosing
/// scope), a parameter default value, or a comprehension's leading iterable.
fn python_escapes_scope(node: Node<'_>) -> bool {
    python_is_definition_name(node)
        || python_in_param_default(node)
        || python_in_leading_iterable(node)
}

/// Whether `node` is the name of a `def` or `class` statement.
fn python_is_definition_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(parent.kind(), "function_definition" | "class_definition")
            && parent.child_by_field_name("name") == Some(node)
    })
}

/// Whether a declaration binds past `scope_kind`. A walrus (`:=`) target binds in
/// the nearest enclosing function rather than the comprehension it appears in.
fn python_binds_past(node: Node<'_>, scope_kind: &str) -> bool {
    matches!(
        scope_kind,
        "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    ) && python_is_walrus_target(node)
}

/// Whether `node` is the target of a walrus (`name := value`) expression.
fn python_is_walrus_target(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "named_expression" && parent.child_by_field_name("name") == Some(node)
    })
}

/// Whether `node` lies in the `value` of a parameter default, not separated from
/// it by a nested scope.
fn python_in_param_default(node: Node<'_>) -> bool {
    let start = node.start_byte();
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "default_parameter" | "typed_default_parameter" => {
                return parent
                    .child_by_field_name("value")
                    .is_some_and(|value| value.start_byte() <= start && start < value.end_byte());
            }
            "function_definition"
            | "lambda"
            | "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression" => return false,
            _ => {}
        }
        current = parent.parent();
    }
    false
}

/// Whether `node` lies in the iterable of a comprehension's first `for` clause,
/// which Python evaluates in the enclosing scope.
fn python_in_leading_iterable(node: Node<'_>) -> bool {
    let start = node.start_byte();
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "for_in_clause" => {
                let Some(right) = parent.child_by_field_name("right") else {
                    return false;
                };
                if !(right.start_byte() <= start && start < right.end_byte()) {
                    return false;
                }
                let Some(comprehension) = parent.parent() else {
                    return false;
                };
                let mut cursor = comprehension.walk();
                return comprehension
                    .children(&mut cursor)
                    .find(|child| child.kind() == "for_in_clause")
                    .is_some_and(|first| first.id() == parent.id());
            }
            "function_definition"
            | "lambda"
            | "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression" => return false,
            _ => {}
        }
        current = parent.parent();
    }
    false
}

/// Collect the bound identifier of a single Python parameter node.
fn python_param_ident<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    match node.kind() {
        "identifier" => out.push(node),
        "default_parameter" | "typed_default_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                python_target_idents(name, out);
            }
        }
        "typed_parameter" | "list_splat_pattern" | "dictionary_splat_pattern" => {
            let mut cursor = node.walk();
            if let Some(ident) = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "identifier")
            {
                out.push(ident);
            }
        }
        _ => {}
    }
}

/// Collect the identifiers bound by a Python assignment-style target.
fn python_target_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    match node.kind() {
        "identifier" => out.push(node),
        "pattern_list"
        | "tuple_pattern"
        | "list_pattern"
        | "as_pattern_target"
        | "list_splat_pattern"
        | "dictionary_splat_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                python_target_idents(child, out);
            }
        }
        _ => {}
    }
}

/// Collect identifiers that `node` introduces as a binding in TypeScript or
/// JavaScript (the two share a node vocabulary).
///
/// Covers `let`/`const`/`var` declarators, function and arrow parameters,
/// `catch` parameters, `for`/`for-in`/`for-of` loop bindings, and the names of
/// `function`/`class` declarations (which bind in their enclosing scope).
/// Destructuring patterns descend to their bound identifiers. Type-level names,
/// object property keys, and member accesses use distinct kinds or positions and
/// are intentionally not treated as bindings.
fn ts_declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "variable_declarator" | "for_in_statement" => {
            if let Some(name) = node.child_by_field_name("left") {
                ts_pattern_idents(name, &mut out);
            } else if let Some(name) = node.child_by_field_name("name") {
                ts_pattern_idents(name, &mut out);
            }
        }
        "required_parameter" | "optional_parameter" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                ts_pattern_idents(pattern, &mut out);
            }
        }
        "arrow_function" | "catch_clause" => {
            if let Some(param) = node.child_by_field_name("parameter") {
                ts_pattern_idents(param, &mut out);
            }
        }
        "function_declaration"
        | "generator_function_declaration"
        | "function_expression"
        | "class_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                if name.kind() == "identifier" || name.kind() == "type_identifier" {
                    out.push(name);
                }
            }
        }
        _ => {}
    }
    out
}

/// Descend a TypeScript/JavaScript binding pattern to its bound identifiers.
///
/// Handles plain identifiers, array and object destructuring (including
/// shorthand, renamed, defaulted, and rest elements). The key of a non-shorthand
/// object pattern entry names a property, not a binding, so only its value is
/// descended.
fn ts_pattern_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => out.push(node),
        "object_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "pair_pattern" => {
                        if let Some(value) = child.child_by_field_name("value") {
                            ts_pattern_idents(value, out);
                        }
                    }
                    _ => ts_pattern_idents(child, out),
                }
            }
        }
        "array_pattern" | "rest_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                ts_pattern_idents(child, out);
            }
        }
        "assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                ts_pattern_idents(left, out);
            }
        }
        _ => {}
    }
}

/// Whether a TypeScript/JavaScript `identifier` leaf is a renameable name
/// reference rather than a property access or object-literal key.
///
/// Member properties (`obj.prop`), object-literal keys (`{ key: value }`), and
/// labeled-statement labels name a member or label, not a binding in scope;
/// excluding them keeps a rename from over-matching. Shorthand object properties
/// (`{ x }`) *are* references to the binding `x` and stay included.
fn ts_is_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    match parent.kind() {
        "member_expression" => parent.child_by_field_name("property") != Some(node),
        "pair" => parent.child_by_field_name("key") != Some(node),
        "labeled_statement" => parent.child_by_field_name("label") != Some(node),
        _ => true,
    }
}

/// Whether `node` names a `function` or `class` declaration, which binds in the
/// scope enclosing the declaration's own body rather than inside it.
fn ts_is_hoisted_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "class_declaration"
        ) && parent.child_by_field_name("name") == Some(node)
    })
}

/// Collect identifiers that `node` introduces as a binding in Vimscript.
///
/// Covers `let`/`const` declarations (including scoped identifiers and list
/// destructuring), `for` loop variables, function parameters (simple, default,
/// and spread), and the names of `function` declarations (which bind in the
/// enclosing scope). Lambda parameters are not tracked because tree-sitter-vim
/// does not separate them from body references.
fn vimscript_declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "let_statement" | "const_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" | "name" => out.push(child),
                    "scoped_identifier" => {
                        if let Some(ident) = vimscript_scoped_ident(child) {
                            out.push(ident);
                        }
                    }
                    "list_assignment" => vimscript_list_target_idents(child, &mut out),
                    _ => {}
                }
            }
        }
        "for_loop" => {
            if let Some(variable) = node.child_by_field_name("variable") {
                match variable.kind() {
                    "identifier" | "name" => out.push(variable),
                    "scoped_identifier" => {
                        if let Some(ident) = vimscript_scoped_ident(variable) {
                            out.push(ident);
                        }
                    }
                    "list_assignment" => vimscript_list_target_idents(variable, &mut out),
                    _ => {}
                }
            }
        }
        "parameters" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => out.push(child),
                    "default_parameter" => {
                        if let Some(name) = child.child_by_field_name("name") {
                            out.push(name);
                        }
                    }
                    _ => {}
                }
            }
        }
        "function_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                if name.kind() == "identifier" || name.kind() == "name" {
                    out.push(name);
                }
            }
        }
        _ => {}
    }
    out
}

/// Descend a Vimscript `scoped_identifier` (e.g. `l:count`) to its name.
fn vimscript_scoped_ident(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "identifier")
}

/// Collect the identifiers bound by a Vimscript `list_assignment` (`[a, b]`).
fn vimscript_list_target_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "name" => out.push(child),
            "scoped_identifier" => {
                if let Some(ident) = vimscript_scoped_ident(child) {
                    out.push(ident);
                }
            }
            "list_assignment" => vimscript_list_target_idents(child, out),
            _ => {}
        }
    }
}

/// Whether a Vimscript `identifier` or `name` leaf is a renameable reference.
///
/// Excludes the `field` child of a `field_expression` (`dict.field`), which names
/// a dictionary key rather than a variable binding.
fn vimscript_is_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    if parent.kind() == "field_expression" && parent.child_by_field_name("field") == Some(node) {
        return false;
    }
    true
}

/// Whether an identifier names a `function_declaration`, which binds in the
/// enclosing scope rather than inside the function body.
fn vimscript_escapes_scope(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "function_declaration" && parent.child_by_field_name("name") == Some(node)
    })
}
