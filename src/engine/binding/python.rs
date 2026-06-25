// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Python-specific binding helpers.

use tree_sitter::Node;

/// Whether a Python `identifier` leaf is a renameable name reference.
///
/// Python reuses the `identifier` kind for attribute members (`obj.attr`) and
/// keyword-argument names (`f(arg=1)`), which name a member or parameter rather
/// than a binding in scope; excluding them keeps a rename from over-matching.
pub(super) fn is_reference(node: Node<'_>) -> bool {
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
pub(super) fn declared_idents<'tree>(node: Node<'tree>, src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "parameters" | "lambda_parameters" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                param_ident(child, &mut out);
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
                target_idents(target, &mut out);
            }
        }
        _ => {}
    }
    out.retain(|ident| !freed(*ident, src));
    out
}

/// Whether a name is declared `global`/`nonlocal` in its enclosing function and
/// so does not introduce a local binding there.
fn freed(ident: Node<'_>, src: &[u8]) -> bool {
    let Ok(name) = ident.utf8_text(src) else {
        return false;
    };
    let mut current = ident.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "function_definition" | "lambda") {
            return body_declares_freed(parent, name, src);
        }
        current = parent.parent();
    }
    false
}

/// Search a function body for a `global`/`nonlocal` statement naming `name`,
/// without descending into nested function, lambda, or class scopes.
fn body_declares_freed(node: Node<'_>, name: &str, src: &[u8]) -> bool {
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
                if body_declares_freed(child, name, src) {
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
pub(super) fn escapes_scope(node: Node<'_>) -> bool {
    is_definition_name(node) || in_param_default(node) || in_leading_iterable(node)
}

/// Whether `node` is the name of a `def` or `class` statement.
fn is_definition_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(parent.kind(), "function_definition" | "class_definition")
            && parent.child_by_field_name("name") == Some(node)
    })
}

/// Whether a declaration binds past `scope_kind`. A walrus (`:=`) target binds in
/// the nearest enclosing function rather than the comprehension it appears in.
pub(super) fn binds_past(node: Node<'_>, scope_kind: &str) -> bool {
    matches!(
        scope_kind,
        "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    ) && is_walrus_target(node)
}

/// Whether `node` is the target of a walrus (`name := value`) expression.
fn is_walrus_target(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "named_expression" && parent.child_by_field_name("name") == Some(node)
    })
}

/// Whether `node` lies in the `value` of a parameter default, not separated from
/// it by a nested scope.
fn in_param_default(node: Node<'_>) -> bool {
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
fn in_leading_iterable(node: Node<'_>) -> bool {
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
fn param_ident<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    match node.kind() {
        "identifier" => out.push(node),
        "default_parameter" | "typed_default_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                target_idents(name, out);
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
fn target_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
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
                target_idents(child, out);
            }
        }
        _ => {}
    }
}
