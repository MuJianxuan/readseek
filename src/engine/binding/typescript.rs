// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! TypeScript/JavaScript binding helpers.

use tree_sitter::Node;

/// Collect identifiers that `node` introduces as a binding in TypeScript or
/// JavaScript (the two share a node vocabulary).
///
/// Covers `let`/`const`/`var` declarators, function and arrow parameters,
/// `catch` parameters, `for`/`for-in`/`for-of` loop bindings, and the names of
/// `function`/`class` declarations (which bind in their enclosing scope).
/// Destructuring patterns descend to their bound identifiers. Type-level names,
/// object property keys, and member accesses use distinct kinds or positions and
/// are intentionally not treated as bindings.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    match node.kind() {
        "variable_declarator" | "for_in_statement" => {
            if let Some(name) = node.child_by_field_name("left") {
                pattern_idents(name, &mut out);
            } else if let Some(name) = node.child_by_field_name("name") {
                pattern_idents(name, &mut out);
            }
        }
        "required_parameter" | "optional_parameter" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                pattern_idents(pattern, &mut out);
            }
        }
        "arrow_function" | "catch_clause" => {
            if let Some(param) = node.child_by_field_name("parameter") {
                pattern_idents(param, &mut out);
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
fn pattern_idents<'tree>(node: Node<'tree>, out: &mut Vec<Node<'tree>>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => out.push(node),
        "object_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "pair_pattern" => {
                        if let Some(value) = child.child_by_field_name("value") {
                            pattern_idents(value, out);
                        }
                    }
                    _ => pattern_idents(child, out),
                }
            }
        }
        "array_pattern" | "rest_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                pattern_idents(child, out);
            }
        }
        "assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                pattern_idents(left, out);
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
pub(super) fn is_reference(node: Node<'_>) -> bool {
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
pub(super) fn is_hoisted_name(node: Node<'_>) -> bool {
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
