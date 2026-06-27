// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Vimscript-specific binding helpers.

use tree_sitter::Node;

/// Collect identifiers that `node` introduces as a binding in Vimscript.
///
/// Covers `let`/`const` declarations (including scoped identifiers and list
/// destructuring), `for` loop variables, function parameters (simple, default,
/// and spread), and the names of `function` declarations (which bind in the
/// enclosing scope). Lambda parameters are not tracked because tree-sitter-vim
/// does not separate them from body references.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "let_statement" | "const_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" | "name" => out.push(child),
                    "scoped_identifier" => {
                        if let Some(ident) = scoped_ident(child) {
                            out.push(ident);
                        }
                    }
                    "list_assignment" => list_target_idents(child, &mut out),
                    _ => {}
                }
            }
        }
        "for_loop" => {
            if let Some(variable) = node.child_by_field_name("variable") {
                match variable.kind() {
                    "identifier" | "name" => out.push(variable),
                    "scoped_identifier" => {
                        if let Some(ident) = scoped_ident(variable) {
                            out.push(ident);
                        }
                    }
                    "list_assignment" => list_target_idents(variable, &mut out),
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
            if let Some(name) = node.child_by_field_name("name")
                && (name.kind() == "identifier" || name.kind() == "name")
            {
                out.push(name);
            }
        }
        _ => {}
    }
    out
}

/// Descend a Vimscript `scoped_identifier` (e.g. `l:count`) to its name.
fn scoped_ident(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "identifier")
}

/// Collect the identifiers bound by a Vimscript `list_assignment` (`[a, b]`).
fn list_target_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "name" => out.push(child),
            "scoped_identifier" => {
                if let Some(ident) = scoped_ident(child) {
                    out.push(ident);
                }
            }
            "list_assignment" => list_target_idents(child, out),
            _ => {}
        }
    }
}

/// Whether a Vimscript `identifier` or `name` leaf is a renameable reference.
///
/// Excludes the `field` child of a `field_expression` (`dict.field`), which names
/// a dictionary key rather than a variable binding.
pub(super) fn is_reference(node: Node<'_>) -> bool {
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
pub(super) fn escapes_scope(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "function_declaration" && parent.child_by_field_name("name") == Some(node)
    })
}
