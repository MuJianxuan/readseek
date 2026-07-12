// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! C# binding helpers.

use tree_sitter::Node;

/// Collect identifiers that `node` introduces as a binding in C#.
///
/// Covers local variable declarators (`int x = 1;`), method/constructor
/// parameters, `foreach` iteration variables, and `catch` exception variables.
/// Type names, property/field names, and method names use the `name` field of
/// their declaration nodes (distinct `identifier` positions) but are not
/// local variable bindings and are intentionally not collected here.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "variable_declarator" | "parameter" | "catch_declaration" => {
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "identifier"
            {
                out.push(name);
            }
        }
        "foreach_statement" => {
            if let Some(left) = node.child_by_field_name("left")
                && left.kind() == "identifier"
            {
                out.push(left);
            }
        }
        _ => {}
    }
    out
}

/// Whether a C# `identifier` leaf is a renameable name reference rather than a
/// member access name or object-initializer key.
///
/// The `name` child of a `member_access_expression` (`obj.Prop`) names a
/// property or field, not a local binding. The `left` child of an
/// `assignment_expression` inside an `initializer_expression`
/// (`new Obj { Prop = 2 }`) is a member assignment, not a local binding.
pub(super) fn is_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    match parent.kind() {
        "member_access_expression" => parent.child_by_field_name("name") != Some(node),
        "assignment_expression"
            if parent.child_by_field_name("left") == Some(node)
                && parent
                    .parent()
                    .is_some_and(|gp| gp.kind() == "initializer_expression") =>
        {
            false
        }
        _ => true,
    }
}
