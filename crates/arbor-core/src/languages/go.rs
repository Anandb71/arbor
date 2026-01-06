//! Go language parser implementation.
//!
//! Handles .go files and extracts functions, methods, structs, interfaces,
//! and type definitions.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct GoParser;

impl LanguageParser for GoParser {
    fn language(&self) -> Language {
        tree_sitter_go::language()
    }

    fn extensions(&self) -> &[&str] {
        &["go"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes, None);

        nodes
    }
}

/// Recursively extracts nodes from the Go AST.
fn extract_from_node(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    context: Option<&str>,
) {
    let kind = node.kind();

    match kind {
        // Functions
        "function_declaration" => {
            if let Some(code_node) = extract_function(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Methods (functions with receivers)
        "method_declaration" => {
            if let Some(code_node) = extract_method(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Type declarations (struct, interface, type alias)
        "type_declaration" => {
            extract_type_declaration(node, source, file_path, nodes);
        }

        // Package declaration
        "package_clause" => {
            if let Some(code_node) = extract_package(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Import declarations
        "import_declaration" => {
            extract_imports(node, source, file_path, nodes);
        }

        // Constants
        "const_declaration" => {
            extract_constants(node, source, file_path, nodes);
        }

        // Variables
        "var_declaration" => {
            extract_variables(node, source, file_path, nodes);
        }

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_from_node(&child, source, file_path, nodes, context);
        }
    }
}

/// Extracts a standalone function.
fn extract_function(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let visibility = if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let signature = build_function_signature(node, source, &name);
    let references = extract_call_references(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Function, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_signature(signature)
            .with_visibility(visibility)
            .with_references(references),
    )
}

/// Extracts a method (function with receiver).
fn extract_method(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    // Get receiver type for qualified name
    let receiver_type = node
        .child_by_field_name("receiver")
        .and_then(|r| {
            // Look for type identifier in receiver
            for i in 0..r.child_count() {
                if let Some(child) = r.child(i) {
                    if child.kind() == "parameter_declaration" {
                        if let Some(type_node) = child.child_by_field_name("type") {
                            let type_text = get_text(&type_node, source);
                            // Strip pointer marker
                            return Some(type_text.trim_start_matches('*').to_string());
                        }
                    }
                }
            }
            None
        })
        .unwrap_or_default();

    let qualified_name = if receiver_type.is_empty() {
        name.clone()
    } else {
        format!("{}.{}", receiver_type, name)
    };

    let visibility = if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let signature = build_function_signature(node, source, &name);
    let references = extract_call_references(node, source);

    Some(
        CodeNode::new(&name, &qualified_name, NodeKind::Method, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_signature(signature)
            .with_visibility(visibility)
            .with_references(references),
    )
}

/// Extracts type declarations (struct, interface, type alias).
fn extract_type_declaration(node: &Node, source: &str, file_path: &str, nodes: &mut Vec<CodeNode>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "type_spec" => {
                    if let Some(code_node) = extract_type_spec(&child, source, file_path) {
                        nodes.push(code_node);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Extracts a type spec (struct, interface, or type alias).
fn extract_type_spec(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let visibility = if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    // Determine the kind based on the type definition
    let type_node = node.child_by_field_name("type")?;
    let kind = match type_node.kind() {
        "struct_type" => NodeKind::Struct,
        "interface_type" => NodeKind::Interface,
        _ => NodeKind::TypeAlias,
    };

    Some(
        CodeNode::new(&name, &name, kind, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts package declaration.
fn extract_package(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "package_identifier" {
                let name = get_text(&child, source);
                return Some(
                    CodeNode::new(&name, &name, NodeKind::Module, file_path)
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

/// Extracts import declarations.
fn extract_imports(node: &Node, source: &str, file_path: &str, nodes: &mut Vec<CodeNode>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "import_spec" || child.kind() == "import_spec_list" {
                extract_import_specs(&child, source, file_path, nodes);
            }
        }
    }
}

fn extract_import_specs(node: &Node, source: &str, file_path: &str, nodes: &mut Vec<CodeNode>) {
    match node.kind() {
        "import_spec" => {
            if let Some(path_node) = node.child_by_field_name("path") {
                let path = get_text(&path_node, source);
                let clean_path = path.trim_matches('"');
                nodes.push(
                    CodeNode::new(clean_path, clean_path, NodeKind::Import, file_path)
                        .with_lines(
                            node.start_position().row as u32 + 1,
                            node.end_position().row as u32 + 1,
                        )
                        .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
                );
            }
        }
        "import_spec_list" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_import_specs(&child, source, file_path, nodes);
                }
            }
        }
        _ => {}
    }
}

/// Extracts constant declarations.
fn extract_constants(node: &Node, source: &str, file_path: &str, nodes: &mut Vec<CodeNode>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "const_spec" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_text(&name_node, source);
                    let visibility =
                        if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                            Visibility::Public
                        } else {
                            Visibility::Private
                        };

                    nodes.push(
                        CodeNode::new(&name, &name, NodeKind::Constant, file_path)
                            .with_lines(
                                child.start_position().row as u32 + 1,
                                child.end_position().row as u32 + 1,
                            )
                            .with_bytes(child.start_byte() as u32, child.end_byte() as u32)
                            .with_column(name_node.start_position().column as u32)
                            .with_visibility(visibility),
                    );
                }
            }
        }
    }
}

/// Extracts variable declarations.
fn extract_variables(node: &Node, source: &str, file_path: &str, nodes: &mut Vec<CodeNode>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "var_spec" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_text(&name_node, source);
                    let visibility =
                        if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                            Visibility::Public
                        } else {
                            Visibility::Private
                        };

                    nodes.push(
                        CodeNode::new(&name, &name, NodeKind::Variable, file_path)
                            .with_lines(
                                child.start_position().row as u32 + 1,
                                child.end_position().row as u32 + 1,
                            )
                            .with_bytes(child.start_byte() as u32, child.end_byte() as u32)
                            .with_column(name_node.start_position().column as u32)
                            .with_visibility(visibility),
                    );
                }
            }
        }
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Gets text content of a node.
fn get_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Builds a function signature.
fn build_function_signature(node: &Node, source: &str, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());

    let return_type = node
        .child_by_field_name("result")
        .map(|n| get_text(&n, source))
        .unwrap_or_default();

    if return_type.is_empty() {
        format!("func {}{}", name, params)
    } else {
        format!("func {}{} {}", name, params, return_type)
    }
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
package main

func Hello(name string) string {
    return "Hello, " + name
}
"#;

        let parser = GoParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();
        
        let nodes = parser.extract_nodes(&tree, source, "test.go");
        
        assert!(nodes.iter().any(|n| n.name == "main" && matches!(n.kind, NodeKind::Module)));
        assert!(nodes.iter().any(|n| n.name == "Hello" && matches!(n.kind, NodeKind::Function)));
    }

    #[test]
    fn test_parse_struct_and_method() {
        let source = r#"
package main

type User struct {
    Name string
    Age  int
}

func (u *User) Greet() string {
    return "Hello, " + u.Name
}
"#;

        let parser = GoParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();
        
        let nodes = parser.extract_nodes(&tree, source, "test.go");
        
        assert!(nodes.iter().any(|n| n.name == "User" && matches!(n.kind, NodeKind::Struct)));
        assert!(nodes.iter().any(|n| n.name == "Greet" && matches!(n.kind, NodeKind::Method)));
    }

    #[test]
    fn test_visibility_detection() {
        let source = r#"
package main

func PublicFunc() {}
func privateFunc() {}

type PublicStruct struct {}
type privateStruct struct {}
"#;

        let parser = GoParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();
        
        let nodes = parser.extract_nodes(&tree, source, "test.go");
        
        let public_func = nodes.iter().find(|n| n.name == "PublicFunc").unwrap();
        let private_func = nodes.iter().find(|n| n.name == "privateFunc").unwrap();
        
        assert!(matches!(public_func.visibility, Visibility::Public));
        assert!(matches!(private_func.visibility, Visibility::Private));
    }
}
