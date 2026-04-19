//! C# language parser implementation.
//!
//! Handles .cs files and extracts classes, interfaces, structs, methods,
//! constructors, properties, and enums.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct CSharpParser;

impl LanguageParser for CSharpParser {
    fn language(&self) -> Language {
        tree_sitter_c_sharp::language()
    }

    fn extensions(&self) -> &[&str] {
        &["cs"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes, None);

        nodes
    }
}

/// Recursively extracts nodes from the C# AST.
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
            if let Some(code_node) = extract_type_decl(node, source, file_path, NodeKind::Class) {
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
            if let Some(code_node) = extract_type_decl(node, source, file_path, NodeKind::Interface)
            {
                let iface_name = code_node.name.clone();
                nodes.push(code_node);

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

        // Struct declarations
        "struct_declaration" => {
            if let Some(code_node) = extract_type_decl(node, source, file_path, NodeKind::Struct) {
                let struct_name = code_node.name.clone();
                nodes.push(code_node);

                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(&child, source, file_path, nodes, Some(&struct_name));
                        }
                    }
                }
                return;
            }
        }

        // Enum declarations
        "enum_declaration" => {
            if let Some(code_node) = extract_type_decl(node, source, file_path, NodeKind::Enum) {
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

        // Property declarations
        "property_declaration" => {
            if let Some(code_node) = extract_property(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Field declarations
        "field_declaration" => {
            extract_fields(node, source, file_path, nodes, context);
        }

        // Using directives (imports)
        "using_directive" => {
            if let Some(code_node) = extract_using(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Namespace declarations
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = get_text(&name_node, source);
                nodes.push(
                    CodeNode::new(&name, &name, NodeKind::Module, file_path)
                        .with_lines(
                            node.start_position().row as u32 + 1,
                            node.end_position().row as u32 + 1,
                        )
                        .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
                );
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

/// Extracts a type declaration (class, interface, struct, enum).
fn extract_type_decl(
    node: &Node,
    source: &str,
    file_path: &str,
    kind: NodeKind,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

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

/// Extracts a property declaration.
fn extract_property(
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

    Some(
        CodeNode::new(&name, &qualified_name, NodeKind::Field, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
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

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "variable_declaration" {
                for j in 0..child.child_count() {
                    if let Some(declarator) = child.child(j) {
                        if declarator.kind() == "variable_declarator" {
                            if let Some(name_node) = declarator.child_by_field_name("name") {
                                let name = get_text(&name_node, source);
                                let qualified_name = match context {
                                    Some(ctx) => format!("{}.{}", ctx, name),
                                    None => name.clone(),
                                };

                                nodes.push(
                                    CodeNode::new(
                                        &name,
                                        &qualified_name,
                                        NodeKind::Field,
                                        file_path,
                                    )
                                    .with_lines(
                                        declarator.start_position().row as u32 + 1,
                                        declarator.end_position().row as u32 + 1,
                                    )
                                    .with_bytes(
                                        declarator.start_byte() as u32,
                                        declarator.end_byte() as u32,
                                    )
                                    .with_column(name_node.start_position().column as u32)
                                    .with_visibility(visibility),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Extracts a using directive.
fn extract_using(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "qualified_name" || child.kind() == "identifier" {
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

/// Detects visibility from C# modifiers. If none is explicitly stated,
/// it infers the default visibility based on the context (parent node).
fn detect_visibility(node: &Node, source: &str) -> Visibility {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "modifier" {
                let text = get_text(&child, source);
                match text.as_str() {
                    "public" => return Visibility::Public,
                    "protected" => return Visibility::Protected,
                    "private" => return Visibility::Private,
                    "internal" => return Visibility::Internal,
                    _ => {}
                }
            }
        }
    }

    // Determine default visibility based on C# rules for the parent container
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "interface_declaration" => return Visibility::Public,
            "class_declaration" | "struct_declaration" => return Visibility::Private,
            "namespace_declaration" | "file_scoped_namespace_declaration" | "compilation_unit" => {
                return Visibility::Internal
            }
            _ => {
                parent = p.parent();
            }
        }
    }

    Visibility::Private
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
    if node.kind() == "invocation_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            match func.kind() {
                "identifier" => {
                    refs.push(get_text(&func, source));
                }
                "member_access_expression" => {
                    if let Some(name_node) = func.child_by_field_name("name") {
                        refs.push(get_text(&name_node, source));
                    }
                }
                _ => {}
            }
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
    fn test_parse_csharp_class_and_method() {
        let source = r#"
using System;

namespace MyApp
{
    public class UserService
    {
        public string GetUser(int id)
        {
            return "user";
        }
    }
}
"#;

        let parser = CSharpParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "UserService.cs");

        assert!(nodes
            .iter()
            .any(|n| n.name == "UserService" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "GetUser" && matches!(n.kind, NodeKind::Method)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "System" && matches!(n.kind, NodeKind::Import)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "MyApp" && matches!(n.kind, NodeKind::Module)));
    }

    #[test]
    fn test_parse_csharp_interface() {
        let source = r#"
public interface IRepository
{
    void Save(string data);
}
"#;

        let parser = CSharpParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "IRepository.cs");

        assert!(nodes
            .iter()
            .any(|n| n.name == "IRepository" && matches!(n.kind, NodeKind::Interface)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "Save" && matches!(n.kind, NodeKind::Method)));
    }

    #[test]
    fn test_parse_csharp_struct_and_enum() {
        let source = r#"
public struct Point
{
    public int X;
    public int Y;
}

public enum Color
{
    Red,
    Green,
    Blue
}
"#;

        let parser = CSharpParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "Types.cs");

        assert!(nodes
            .iter()
            .any(|n| n.name == "Point" && matches!(n.kind, NodeKind::Struct)));
        assert!(nodes
            .iter()
            .any(|n| n.name == "Color" && matches!(n.kind, NodeKind::Enum)));
    }
}
