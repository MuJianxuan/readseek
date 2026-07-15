// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Java-specific binding helpers.

use tree_sitter::Node;

/// Collect identifiers that introduce local bindings in Java.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "variable_declarator"
        | "formal_parameter"
        | "catch_formal_parameter"
        | "enhanced_for_statement" => push_name(node, &mut out),
        "lambda_expression" => {
            if let Some(parameters) = node.child_by_field_name("parameters") {
                collect_lambda_parameters(parameters, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn push_name<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    if let Some(name) = node.child_by_field_name("name")
        && name.kind() == "identifier"
    {
        out.push(name);
    }
}

fn collect_lambda_parameters<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    if node.kind() == "identifier" {
        out.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "formal_parameter" {
            push_name(child, out);
        } else {
            collect_lambda_parameters(child, out);
        }
    }
}

/// Exclude member and method names, which are not references to local bindings.
pub(super) fn is_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    match parent.kind() {
        "field_access" => parent.child_by_field_name("field") != Some(node),
        "method_invocation" => parent.child_by_field_name("name") != Some(node),
        _ => true,
    }
}
