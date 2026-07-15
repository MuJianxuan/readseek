// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Rust-specific binding helpers.

use tree_sitter::Node;

/// Collect identifiers that `node` introduces as bindings in Rust.
///
/// Covers `let` bindings (including patterns), function parameters, loop
/// patterns, and match-arm patterns. Other constructs fall through and are
/// simply not treated as declarations, which is safe: the name then stays
/// unresolved.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "let_declaration" | "for_expression" | "let_condition" | "match_arm" => {
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
