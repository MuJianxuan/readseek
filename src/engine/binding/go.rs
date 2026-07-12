// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Go-specific binding helpers.

use tree_sitter::Node;

/// Collect identifiers that `node` introduces as a binding in Go.
///
/// Covers short-variable declarations (`:=`), `var`/`const` specs, `range`
/// clauses, `for`-clause initializers, channel `receive` statements, switch
/// type-guard variables, function/method parameters (including variadic), and
/// import aliases. Struct field names, package names, and type names use
/// distinct node kinds and are not treated as variable bindings. The blank
/// identifier (`_`) is never a binding.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "short_var_declaration" | "range_clause" | "receive_statement" => {
            if let Some(left) = node.child_by_field_name("left") {
                left_idents(left, &mut out);
            }
        }
        "var_spec" | "const_spec" => {
            let mut cursor = node.walk();
            for name in node.children_by_field_name("name", &mut cursor) {
                if name.kind() == "identifier" {
                    out.push(name);
                }
            }
        }
        "parameter_declaration" | "variadic_parameter_declaration" => {
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "identifier"
            {
                out.push(name);
            }
        }
        "type_switch_statement" => {
            if let Some(alias) = node.child_by_field_name("alias") {
                left_idents(alias, &mut out);
            }
        }
        "import_spec" => {
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "package_identifier"
            {
                out.push(name);
            }
        }
        _ => {}
    }
    out
}

/// Collect bound identifiers from the left-hand side of a `:=` / range / receive.
fn left_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            out.push(child);
        }
    }
}

/// Whether a Go `identifier` leaf is a renameable name reference rather than a
/// struct-literal key.
///
/// The key of a `keyed_element` (`Config{Port: Port}`) names a struct field, not
/// a binding; the value side is a real reference and stays included. The key is
/// detected via the grandparent `keyed_element`'s field name, since
/// tree-sitter-go wraps both sides in `literal_element` children. Import aliases
/// and package names use `package_identifier`, a distinct kind, so they are never
/// matched as ordinary `identifier` references.
pub(super) fn is_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    if parent.kind() == "literal_element"
        && let Some(grandparent) = parent.parent()
        && grandparent.kind() == "keyed_element"
        && grandparent
            .child_by_field_name("key")
            .is_some_and(|key| key.id() == parent.id())
    {
        return false;
    }
    true
}
