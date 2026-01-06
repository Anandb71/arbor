//! C language parser implementation.
//!
//! Handles .c and .h files and extracts functions, structs, enums,
//! typedefs, and global variables.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct CParser;

impl LanguageParser for CParser {
    fn language(&self) -> Language {
        tree_sitter_c::language()
    }

    fn extensions(&self) -> &[&str] {
        &["c", "h"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes);

        nodes
    }
}

/// Recursively extracts nodes from the C AST.
fn extract_from_node(node: &Node, source: &str, file_path: &str, nodes: &mut Vec<CodeNode>) {
    let kind = node.kind();

    match kind {
        // Function definitions
        "function_definition" => {
            if let Some(code_node) = extract_function(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Function declarations (prototypes)
        "declaration" => {
            if has_function_declarator(node) {
                if let Some(code_node) = extract_function_declaration(node, source, file_path) {
                    nodes.push(code_node);
                }
            }
        }

        // Struct definitions
        "struct_specifier" => {
            if let Some(code_node) = extract_struct(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Enum definitions
        "enum_specifier" => {
            if let Some(code_node) = extract_enum(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Typedef declarations
        "type_definition" => {
            if let Some(code_node) = extract_typedef(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Include directives
        "preproc_include" => {
            if let Some(code_node) = extract_include(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_from_node(&child, source, file_path, nodes);
        }
    }
}

/// Extracts a function definition.
fn extract_function(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = find_function_name(&declarator, source)?;

    let signature = build_function_signature(node, source, &name);
    let references = extract_call_references(node, source);

    // C functions are typically public unless static
    let visibility = if is_static(node, source) {
        Visibility::Private
    } else {
        Visibility::Public
    };

    Some(
        CodeNode::new(&name, &name, NodeKind::Function, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_signature(signature)
            .with_visibility(visibility)
            .with_references(references),
    )
}

/// Extracts a function declaration (prototype).
fn extract_function_declaration(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let declarator = find_declarator(node)?;
    let name = find_function_name(&declarator, source)?;

    let visibility = if is_static(node, source) {
        Visibility::Private
    } else {
        Visibility::Public
    };

    Some(
        CodeNode::new(&name, &name, NodeKind::Function, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_visibility(visibility),
    )
}

/// Extracts a struct definition.
fn extract_struct(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Struct, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(Visibility::Public),
    )
}

/// Extracts an enum definition.
fn extract_enum(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Enum, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(Visibility::Public),
    )
}

/// Extracts a typedef declaration.
fn extract_typedef(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // Look for the type name (usually at the end)
    let declarator = find_declarator(node)?;
    let name = find_type_name(&declarator, source)?;

    Some(
        CodeNode::new(&name, &name, NodeKind::TypeAlias, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_visibility(Visibility::Public),
    )
}

/// Extracts an include directive.
fn extract_include(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "string_literal" || child.kind() == "system_lib_string" {
                let path = get_text(&child, source);
                let clean_path = path.trim_matches(|c| c == '"' || c == '<' || c == '>');
                return Some(
                    CodeNode::new(clean_path, clean_path, NodeKind::Import, file_path)
                        .with_lines(
                            node.start_position().row as u32 + 1,
                            node.end_position().row as u32 + 1,
                        )
                        .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
                );
            }
        }
    }
    None
}

// ============================================================================
// Helper functions
// ============================================================================

/// Gets text content of a node.
fn get_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Checks if a declaration has a function declarator.
fn has_function_declarator(node: &Node) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "function_declarator" {
                return true;
            }
            if has_function_declarator(&child) {
                return true;
            }
        }
    }
    false
}

/// Finds the declarator in a declaration.
fn find_declarator<'a>(node: &'a Node) -> Option<Node<'a>> {
    node.child_by_field_name("declarator")
}

/// Finds the function name from a declarator.
fn find_function_name(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "function_declarator" {
        if let Some(name_node) = node.child_by_field_name("declarator") {
            return Some(get_text(&name_node, source));
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(name) = find_function_name(&child, source) {
                return Some(name);
            }
        }
    }
    None
}

/// Finds type name from a typedef declarator.
fn find_type_name(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "type_identifier" || node.kind() == "identifier" {
        return Some(get_text(node, source));
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(name) = find_type_name(&child, source) {
                return Some(name);
            }
        }
    }
    None
}

/// Checks if a declaration is static.
fn is_static(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "storage_class_specifier" {
                let text = get_text(&child, source);
                if text == "static" {
                    return true;
                }
            }
        }
    }
    false
}

/// Builds a function signature.
fn build_function_signature(node: &Node, source: &str, name: &str) -> String {
    // Get return type
    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "void".to_string());

    // Get parameters from declarator
    let params = node
        .child_by_field_name("declarator")
        .and_then(|d| find_params(&d, source))
        .unwrap_or_else(|| "()".to_string());

    format!("{} {}{}", return_type, name, params)
}

/// Finds function parameters.
fn find_params(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "function_declarator" {
        if let Some(params) = node.child_by_field_name("parameters") {
            return Some(get_text(&params, source));
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(params) = find_params(&child, source) {
                return Some(params);
            }
        }
    }
    None
}

/// Extracts function call references.
fn extract_call_references(node: &Node, source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    collect_calls(node, source, &mut refs);
    refs.sort();
    refs.dedup();
    refs
}

/// Recursively collects function call names.
fn collect_calls(node: &Node, source: &str, refs: &mut Vec<String>) {
    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let call_name = get_text(&func_node, source);
            refs.push(call_name);
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_calls(&child, source, refs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_function() {
        let source = r#"
#include <stdio.h>

int main(int argc, char *argv[]) {
    printf("Hello, World!\n");
    return 0;
}
"#;

        let parser = CParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "main.c");

        assert!(nodes
            .iter()
            .any(|n| n.name == "main" && matches!(n.kind, NodeKind::Function)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "stdio.h" && matches!(n.kind, NodeKind::Import)));
    }

    #[test]
    fn test_parse_struct() {
        let source = r#"
struct Point {
    int x;
    int y;
};
"#;

        let parser = CParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "point.h");

        assert!(nodes
            .iter()
            .any(|n| n.name == "Point" && matches!(n.kind, NodeKind::Struct)));
    }

    #[test]
    fn test_static_visibility() {
        let source = r#"
static void helper() {}
void public_func() {}
"#;

        let parser = CParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "test.c");

        let helper = nodes.iter().find(|n| n.name == "helper").unwrap();
        let public_func = nodes.iter().find(|n| n.name == "public_func").unwrap();

        assert!(matches!(helper.visibility, Visibility::Private));
        assert!(matches!(public_func.visibility, Visibility::Public));
    }
}
