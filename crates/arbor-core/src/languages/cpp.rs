//! C++ language parser implementation.
//!
//! Handles .cpp, .hpp, .cc, .hh, .cxx files and extracts classes,
//! namespaces, methods, functions, and structs.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct CppParser;

impl LanguageParser for CppParser {
    fn language(&self) -> Language {
        tree_sitter_cpp::language()
    }

    fn extensions(&self) -> &[&str] {
        &["cpp", "hpp", "cc", "hh", "cxx", "hxx"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes, None);

        nodes
    }
}

/// Recursively extracts nodes from the C++ AST.
fn extract_from_node(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    context: Option<&str>,
) {
    let kind = node.kind();

    match kind {
        // Class definitions
        "class_specifier" => {
            if let Some(code_node) = extract_class(node, source, file_path) {
                let class_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract class members from body or field_declaration_list
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "field_declaration_list"
                            || child.kind() == "declaration_list"
                        {
                            for j in 0..child.child_count() {
                                if let Some(member) = child.child(j) {
                                    extract_from_node(
                                        &member,
                                        source,
                                        file_path,
                                        nodes,
                                        Some(&class_name),
                                    );
                                }
                            }
                        }
                    }
                }
                return;
            }
        }

        // Struct definitions (C++ adds methods to structs)
        "struct_specifier" => {
            if let Some(code_node) = extract_struct(node, source, file_path) {
                let struct_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract struct members from body or field_declaration_list
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "field_declaration_list"
                            || child.kind() == "declaration_list"
                        {
                            for j in 0..child.child_count() {
                                if let Some(member) = child.child(j) {
                                    extract_from_node(
                                        &member,
                                        source,
                                        file_path,
                                        nodes,
                                        Some(&struct_name),
                                    );
                                }
                            }
                        }
                    }
                }
                return;
            }
        }

        // Namespace definitions
        "namespace_definition" => {
            if let Some(code_node) = extract_namespace(node, source, file_path) {
                let ns_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract namespace members from body or declaration_list
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "declaration_list"
                            || child.kind() == "compound_statement"
                        {
                            for j in 0..child.child_count() {
                                if let Some(member) = child.child(j) {
                                    extract_from_node(
                                        &member,
                                        source,
                                        file_path,
                                        nodes,
                                        Some(&ns_name),
                                    );
                                }
                            }
                        }
                    }
                }
                return;
            }
        }

        // Function definitions
        "function_definition" => {
            if let Some(code_node) = extract_function(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Field declarations in class
        "field_declaration" => {
            if context.is_some() {
                extract_fields(node, source, file_path, nodes, context);
            }
        }

        // Template declarations
        "template_declaration" => {
            // Recurse into template body
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_from_node(&child, source, file_path, nodes, context);
                }
            }
            return;
        }

        // Enum definitions
        "enum_specifier" => {
            if let Some(code_node) = extract_enum(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Include directives
        "preproc_include" => {
            if let Some(code_node) = extract_include(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Using directives
        "using_declaration" => {
            if let Some(code_node) = extract_using(node, source, file_path) {
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

/// Extracts a class definition.
fn extract_class(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Class, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(Visibility::Public),
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

/// Extracts a namespace definition.
fn extract_namespace(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Module, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(Visibility::Public),
    )
}

/// Extracts a function or method definition.
fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
    context: Option<&str>,
) -> Option<CodeNode> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = find_function_name(&declarator, source)?;

    let kind = if context.is_some() {
        NodeKind::Method
    } else {
        NodeKind::Function
    };

    let qualified_name = match context {
        Some(ctx) => format!("{}::{}", ctx, name),
        None => name.clone(),
    };

    let visibility = detect_visibility(node, source);
    let signature = build_function_signature(node, source, &name);
    let references = extract_call_references(node, source);

    Some(
        CodeNode::new(&name, &qualified_name, kind, file_path)
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

/// Extracts field declarations.
fn extract_fields(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    context: Option<&str>,
) {
    let visibility = detect_visibility(node, source);

    // Look for declarators
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "field_identifier" {
                let name = get_text(&child, source);
                let qualified_name = match context {
                    Some(ctx) => format!("{}::{}", ctx, name),
                    None => name.clone(),
                };

                nodes.push(
                    CodeNode::new(&name, &qualified_name, NodeKind::Field, file_path)
                        .with_lines(
                            node.start_position().row as u32 + 1,
                            node.end_position().row as u32 + 1,
                        )
                        .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
                        .with_column(child.start_position().column as u32)
                        .with_visibility(visibility.clone()),
                );
            }
        }
    }
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

/// Extracts a using declaration.
fn extract_using(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let text = get_text(node, source);
    let import_name = text
        .trim_start_matches("using ")
        .trim_end_matches(';')
        .trim();

    Some(
        CodeNode::new(import_name, import_name, NodeKind::Import, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
    )
}

// ============================================================================
// Helper functions
// ============================================================================

/// Gets text content of a node.
fn get_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Finds the function name from a declarator.
fn find_function_name(node: &Node, source: &str) -> Option<String> {
    // Handle qualified names (Class::method)
    if node.kind() == "qualified_identifier" {
        if let Some(name_node) = node.child_by_field_name("name") {
            return Some(get_text(&name_node, source));
        }
    }

    if node.kind() == "function_declarator" {
        if let Some(name_node) = node.child_by_field_name("declarator") {
            return find_function_name(&name_node, source);
        }
    }

    if node.kind() == "identifier" || node.kind() == "destructor_name" {
        return Some(get_text(node, source));
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

/// Detects visibility from C++ access specifiers.
fn detect_visibility(node: &Node, source: &str) -> Visibility {
    // Check for explicit access specifier
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "access_specifier" {
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
    // Default for class members is private
    Visibility::Private
}

/// Builds a function signature.
fn build_function_signature(node: &Node, source: &str, name: &str) -> String {
    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "void".to_string());

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
    fn test_parse_class() {
        let source = r#"
#include <iostream>

class MyClass {
public:
    void hello() {
        std::cout << "Hello" << std::endl;
    }
};
"#;

        let parser = CppParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "myclass.cpp");

        // Class is detected
        assert!(nodes
            .iter()
            .any(|n| n.name == "MyClass" && matches!(n.kind, NodeKind::Class)));
        // Include is detected
        assert!(nodes
            .iter()
            .any(|n| n.name == "iostream" && matches!(n.kind, NodeKind::Import)));
    }

    #[test]
    fn test_parse_namespace() {
        let source = r#"
namespace MyLib {
}
"#;

        let parser = CppParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "mylib.cpp");

        assert!(nodes
            .iter()
            .any(|n| n.name == "MyLib" && matches!(n.kind, NodeKind::Module)));
    }

    #[test]
    fn test_parse_struct() {
        let source = r#"
struct Point {
    int x;
    int y;
};
"#;

        let parser = CppParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "point.hpp");

        assert!(nodes
            .iter()
            .any(|n| n.name == "Point" && matches!(n.kind, NodeKind::Struct)));
    }

    #[test]
    fn test_parse_standalone_function() {
        let source = r#"
void myFunction(int x) {
    return;
}
"#;

        let parser = CppParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "main.cpp");

        assert!(nodes
            .iter()
            .any(|n| n.name == "myFunction" && matches!(n.kind, NodeKind::Function)));
    }
}
