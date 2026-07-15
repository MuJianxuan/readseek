// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Swift-specific binding helpers.

use tree_sitter::Node;

/// Collect identifiers that introduce bindings in Swift.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "property_declaration" => {
            let mut cursor = node.walk();
            for name in node.children_by_field_name("name", &mut cursor) {
                collect_pattern_idents(name, &mut out);
            }
        }
        "function_declaration" | "parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                collect_pattern_idents(name, &mut out);
            }
        }
        "for_statement" => {
            if let Some(item) = node.child_by_field_name("item") {
                collect_pattern_idents(item, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn collect_pattern_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    if node.kind() == "simple_identifier" {
        out.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_pattern_idents(child, out);
    }
}

/// Exclude member names in navigation expressions from local binding resolution.
pub(super) fn is_reference(node: Node<'_>) -> bool {
    node.parent()
        .is_none_or(|parent| parent.kind() != "navigation_suffix")
}

/// Function declarations bind in their enclosing scope, not their function body.
pub(super) fn escapes_scope(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "function_declaration" && parent.child_by_field_name("name") == Some(node)
    })
}
