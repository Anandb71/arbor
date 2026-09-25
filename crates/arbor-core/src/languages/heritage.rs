//! Shared walk for `extends` / `implements` clauses.
//!
//! Python argument lists and Go embeddings are shaped differently and stay in
//! their own parsers. This walker covers the clause form used by Java, C#,
//! C++, TypeScript, and Dart: a keyword or a named clause, then one or more
//! type names, stopping at the type body.

use crate::node::clean_type_name;
use tree_sitter::Node;

/// Superclass names and interface names declared on `type_node`.
pub(crate) fn clause_bases(type_node: &Node, source: &str) -> (Vec<String>, Vec<String>) {
    let mut extends = Vec::new();
    let mut implements = Vec::new();
    let mut mode: Option<bool> = None; // Some(true) = extends, Some(false) = implements
    walk(
        type_node,
        source,
        &mut extends,
        &mut implements,
        &mut mode,
        true,
    );
    dedup(&mut extends);
    dedup(&mut implements);
    (extends, implements)
}

fn walk(
    node: &Node,
    source: &str,
    extends: &mut Vec<String>,
    implements: &mut Vec<String>,
    mode: &mut Option<bool>,
    at_type_root: bool,
) {
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else {
            continue;
        };
        if at_type_root && node.field_name_for_child(i as u32) == Some("body") {
            continue;
        }
        let kind = child.kind();
        if skip_subtree(kind) {
            continue;
        }

        if is_extends_clause(kind) {
            collect_type_names(&child, source, extends);
            *mode = Some(true);
            continue;
        }
        if is_implements_clause(kind) {
            collect_type_names(&child, source, implements);
            *mode = Some(false);
            continue;
        }
        // C# `base_list` and C++ `base_class_clause` hold the parent types
        // directly. The `:` is not its own node, so there is no keyword to
        // switch modes. Interface targets are reclassified when the edge is built.
        if kind == "base_list" || kind == "base_class_clause" {
            collect_type_names(&child, source, extends);
            *mode = Some(true);
            continue;
        }
        if kind == "class_heritage" {
            walk(&child, source, extends, implements, mode, false);
            continue;
        }

        if child.child_count() == 0 {
            let text = child_text(&child, source);
            match text {
                // `with` is a mixin: inherited implementation, same direction as extends.
                "extends" | ":" | "with" => *mode = Some(true),
                "implements" => *mode = Some(false),
                _ => {}
            }
            continue;
        }

        if is_type_node(kind) {
            if let Some(extends_mode) = *mode {
                if let Some(name) = type_name_of(&child, source) {
                    if extends_mode {
                        extends.push(name);
                    } else {
                        implements.push(name);
                    }
                }
            }
            continue;
        }

        walk(&child, source, extends, implements, mode, false);
    }
}

/// Every type name under `node`, generics stripped. Used for Rust trait bounds.
pub(crate) fn type_names(node: &Node, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_type_names(node, source, &mut out);
    dedup(&mut out);
    out
}

fn collect_type_names(node: &Node, source: &str, out: &mut Vec<String>) {
    let kind = node.kind();
    if skip_subtree(kind) || kind == "type_arguments" || kind == "type_parameters" {
        return;
    }
    if is_type_node(kind) {
        if let Some(name) = type_name_of(node, source) {
            out.push(name);
        }
        return;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_type_names(&child, source, out);
        }
    }
}

fn type_name_of(node: &Node, source: &str) -> Option<String> {
    let cleaned = clean_type_name(child_text(node, source));
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn child_text<'a>(node: &Node, source: &'a str) -> &'a str {
    let range = node.byte_range();
    if range.end <= source.len() {
        source[range].trim()
    } else {
        ""
    }
}

fn is_extends_clause(kind: &str) -> bool {
    matches!(
        kind,
        "superclass" | "extends_clause" | "extends_type_clause" | "mixins"
    )
}

fn is_implements_clause(kind: &str) -> bool {
    matches!(
        kind,
        "super_interfaces" | "implements_clause" | "interfaces"
    )
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "scoped_identifier"
            | "scoped_type_identifier"
            | "namespace_type"
            | "qualified_identifier"
            | "qualified_name"
            | "generic_name"
            | "generic_type"
            | "template_type"
            | "member_expression"
            | "nested_type_identifier"
            | "qualified_type"
    )
}

fn skip_subtree(kind: &str) -> bool {
    matches!(
        kind,
        "class_body"
            | "declaration_list"
            | "block"
            | "field_declaration_list"
            | "enum_body"
            | "interface_body"
            | "modifiers"
            | "attribute_list"
            | "annotation"
            | "marker_annotation"
            | "decorator"
            | "type_parameters"
            | "type_parameter"
            | "parameter_list"
            | "formal_parameters"
            | "arguments"
    )
}

fn dedup(names: &mut Vec<String>) {
    let mut seen = Vec::new();
    names.retain(|name| {
        if seen.iter().any(|existing: &String| existing == name) {
            false
        } else {
            seen.push(name.clone());
            true
        }
    });
}
