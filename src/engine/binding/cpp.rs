// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! C and C++ binding helpers.

use tree_sitter::Node;

/// Collect identifiers that `node` introduces as a local binding in C/C++.
///
/// Covers block-scope variable declarations, function and lambda parameters, and
/// `for` initializers. Type names, struct field names, and function names use
/// distinct node kinds and are intentionally not treated as variable bindings.
pub(super) fn declared_idents<'tree>(node: Node<'tree>, _src: &[u8]) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    if matches!(node.kind(), "declaration" | "parameter_declaration") {
        let mut cursor = node.walk();
        out.extend(
            node.children_by_field_name("declarator", &mut cursor)
                .filter_map(declarator_ident),
        );
    }
    out
}

/// Descend a C/C++ declarator to the bound identifier.
///
/// Unwraps `init_declarator` and pointer/array/reference declarators, but stops
/// at `function_declarator`: a function declarator names a function, not a local
/// variable, so its identifier is not a renameable local binding.
fn declarator_ident(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "identifier" => Some(node),
        "function_declarator" => None,
        _ => declarator_ident(node.child_by_field_name("declarator")?),
    }
}
