//! Java language parser implementation.
//!
//! Handles .java files and extracts classes, interfaces, methods,
//! constructors, and fields.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct JavaParser;

impl LanguageParser for JavaParser {
    fn language(&self) -> Language {
        tree_sitter_java::language()
    }

    fn extensions(&self) -> &[&str] {
        &["java"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes, None);

        nodes
    }
}

/// Recursively extracts nodes from the Java AST.
fn extract_from_node(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    context: Option<&str>,
) {
    let kind = node.kind();

    match kind {
        // Class declarations
        "class_declaration" => {
            if let Some(code_node) = extract_class(node, source, file_path) {
                let class_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract class members
                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(&child, source, file_path, nodes, Some(&class_name));
                        }
                    }
                }
                return;
            }
        }

        // Interface declarations
        "interface_declaration" => {
            if let Some(code_node) = extract_interface(node, source, file_path) {
                let iface_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract interface methods
                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(&child, source, file_path, nodes, Some(&iface_name));
                        }
                    }
                }
                return;
            }
        }

        // Enum declarations
        "enum_declaration" => {
            if let Some(code_node) = extract_enum(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Method declarations
        "method_declaration" => {
            if let Some(code_node) = extract_method(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Constructor declarations
        "constructor_declaration" => {
            if let Some(code_node) = extract_constructor(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Field declarations
        "field_declaration" => {
            extract_fields(node, source, file_path, nodes, context);
        }

        // Package declarations
        "package_declaration" => {
            if let Some(code_node) = extract_package(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Import declarations
        "import_declaration" => {
            if let Some(code_node) = extract_import(node, source, file_path) {
                nodes.push(code_node);
            }
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

/// Extracts a class declaration.
fn extract_class(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Class, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts an interface declaration.
fn extract_interface(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Interface, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts an enum declaration.
fn extract_enum(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Enum, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts a method declaration.
fn extract_method(
    node: &Node,
    source: &str,
    file_path: &str,
    context: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let qualified_name = match context {
        Some(ctx) => format!("{}.{}", ctx, name),
        None => name.clone(),
    };

    let visibility = detect_visibility(node, source);
    let signature = build_method_signature(node, source, &name);
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

/// Extracts a constructor declaration.
fn extract_constructor(
    node: &Node,
    source: &str,
    file_path: &str,
    context: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let qualified_name = match context {
        Some(ctx) => format!("{}.{}", ctx, name),
        None => name.clone(),
    };

    let visibility = detect_visibility(node, source);
    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());
    let signature = format!("{}{}", name, params);

    Some(
        CodeNode::new(&name, &qualified_name, NodeKind::Constructor, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_signature(signature)
            .with_visibility(visibility),
    )
}

/// Extracts field declarations.
fn extract_fields(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    context: Option<&str>,
) {
    let visibility = detect_visibility(node, source);

    // Look for variable declarators
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_text(&name_node, source);
                    let qualified_name = match context {
                        Some(ctx) => format!("{}.{}", ctx, name),
                        None => name.clone(),
                    };

                    nodes.push(
                        CodeNode::new(&name, &qualified_name, NodeKind::Field, file_path)
                            .with_lines(
                                child.start_position().row as u32 + 1,
                                child.end_position().row as u32 + 1,
                            )
                            .with_bytes(child.start_byte() as u32, child.end_byte() as u32)
                            .with_column(name_node.start_position().column as u32)
                            .with_visibility(visibility.clone()),
                    );
                }
            }
        }
    }
}

/// Extracts package declaration.
fn extract_package(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // Find the scope identifier in the package declaration
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
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

/// Extracts import declaration.
fn extract_import(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // Find the import path
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
                let name = get_text(&child, source);
                return Some(
                    CodeNode::new(&name, &name, NodeKind::Import, file_path)
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

/// Detects visibility from Java modifiers.
fn detect_visibility(node: &Node, source: &str) -> Visibility {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "modifiers" {
                let text = get_text(&child, source);
                if text.contains("public") {
                    return Visibility::Public;
                } else if text.contains("protected") {
                    return Visibility::Protected;
                } else if text.contains("private") {
                    return Visibility::Private;
                }
            }
        }
    }
    // Default package-private visibility
    Visibility::Internal
}

/// Builds a method signature.
fn build_method_signature(node: &Node, source: &str, name: &str) -> String {
    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "void".to_string());

    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());

    format!("{} {}{}", return_type, name, params)
}

/// Extracts method call references.
fn extract_call_references(node: &Node, source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    collect_calls(node, source, &mut refs);
    refs.sort();
    refs.dedup();
    refs
}

/// Recursively collects method call names.
fn collect_calls(node: &Node, source: &str, refs: &mut Vec<String>) {
    if node.kind() == "method_invocation" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let call_name = get_text(&name_node, source);
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
    fn test_parse_simple_class() {
        let source = r#"
package com.example;

public class HelloWorld {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}
"#;

        let parser = JavaParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "HelloWorld.java");

        assert!(nodes
            .iter()
            .any(|n| n.name == "com.example" && matches!(n.kind, NodeKind::Module)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "HelloWorld" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "main" && matches!(n.kind, NodeKind::Method)));
    }

    #[test]
    fn test_parse_interface() {
        let source = r#"
public interface Greeting {
    String greet(String name);
}
"#;

        let parser = JavaParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "Greeting.java");

        assert!(nodes
            .iter()
            .any(|n| n.name == "Greeting" && matches!(n.kind, NodeKind::Interface)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "greet" && matches!(n.kind, NodeKind::Method)));
    }

    #[test]
    fn test_visibility_detection() {
        let source = r#"
public class Example {
    public void publicMethod() {}
    protected void protectedMethod() {}
    private void privateMethod() {}
    void packageMethod() {}
}
"#;

        let parser = JavaParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "Example.java");

        let public_method = nodes.iter().find(|n| n.name == "publicMethod").unwrap();
        let protected_method = nodes.iter().find(|n| n.name == "protectedMethod").unwrap();
        let private_method = nodes.iter().find(|n| n.name == "privateMethod").unwrap();
        let package_method = nodes.iter().find(|n| n.name == "packageMethod").unwrap();

        assert!(matches!(public_method.visibility, Visibility::Public));
        assert!(matches!(protected_method.visibility, Visibility::Protected));
        assert!(matches!(private_method.visibility, Visibility::Private));
        assert!(matches!(package_method.visibility, Visibility::Internal));
    }
}
