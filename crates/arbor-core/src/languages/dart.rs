//! Dart language parser implementation.
//!
//! Handles .dart files and extracts classes, mixins, extensions,
//! functions, methods, and imports.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct DartParser;

/// ABI Compatibility Shim for tree-sitter-dart
///
/// # Safety
/// `tree-sitter-dart` v0.0.4 is built against tree-sitter 0.20 ABI, while
/// Arbor uses tree-sitter 0.22. The ABI is compatible for our use case
/// (parsing and node traversal), but the Rust types differ.
///
/// This transmute is safe because:
/// 1. Both ABIs use the same C representation for TSLanguage
/// 2. We only use the language for parsing/tree traversal (no query syntax)
/// 3. This shim is isolated for easy removal when tree-sitter-dart updates
///
/// TODO: Remove this when tree-sitter-dart releases a 0.22+ compatible version.
#[inline]
fn dart_language_compat() -> Language {
    // SAFETY: See module-level documentation above
    unsafe { std::mem::transmute(tree_sitter_dart::language()) }
}

impl LanguageParser for DartParser {
    fn language(&self) -> Language {
        // Use the ABI compatibility shim for tree-sitter-dart 0.0.4
        dart_language_compat()
    }

    fn extensions(&self) -> &[&str] {
        &["dart"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes, None);

        nodes
    }
}

/// Recursively extracts nodes from the Dart AST.
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
        "class_definition" => {
            if let Some(code_node) = extract_class(node, source, file_path) {
                let class_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract class members from body
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

        // Mixin declarations
        "mixin_declaration" => {
            if let Some(code_node) = extract_mixin(node, source, file_path) {
                let mixin_name = code_node.name.clone();
                nodes.push(code_node);

                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(&child, source, file_path, nodes, Some(&mixin_name));
                        }
                    }
                }
                return;
            }
        }

        // Extension declarations
        "extension_declaration" => {
            if let Some(code_node) = extract_extension(node, source, file_path) {
                let ext_name = code_node.name.clone();
                nodes.push(code_node);

                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(&child, source, file_path, nodes, Some(&ext_name));
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

        // Function declarations (top-level)
        "function_signature" | "function_definition" => {
            if let Some(code_node) = extract_function(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Method declarations (inside class)
        "method_signature" | "method_definition" => {
            if let Some(code_node) = extract_method(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Constructor declarations
        "constructor_signature" => {
            if let Some(code_node) = extract_constructor(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Getter/Setter declarations
        "getter_signature" | "setter_signature" => {
            if let Some(code_node) = extract_accessor(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Import statements
        "import_or_export" | "import_specification" => {
            if let Some(code_node) = extract_import(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Library declaration
        "library_name" => {
            if let Some(code_node) = extract_library(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Top-level variable declarations
        "top_level_definition" => {
            // Check if it's a variable
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "initialized_variable_definition"
                        || child.kind() == "static_final_declaration"
                    {
                        if let Some(code_node) = extract_variable(&child, source, file_path) {
                            nodes.push(code_node);
                        }
                    }
                }
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
    let visibility = detect_visibility(&name);

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

/// Extracts a mixin declaration.
fn extract_mixin(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(&name);

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

/// Extracts an extension declaration.
fn extract_extension(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // Extensions can be named or anonymous
    let name = node
        .child_by_field_name("name")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "<anonymous>".to_string());

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

/// Extracts an enum declaration.
fn extract_enum(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(&name);

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

/// Extracts a top-level function.
fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
    context: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    // Skip if we're inside a class context (those are methods)
    if context.is_some() {
        return None;
    }

    let visibility = detect_visibility(&name);
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

/// Extracts a method (inside a class).
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

    let visibility = detect_visibility(&name);
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

/// Extracts a constructor.
fn extract_constructor(
    node: &Node,
    source: &str,
    file_path: &str,
    context: Option<&str>,
) -> Option<CodeNode> {
    // Find constructor name
    let name = find_constructor_name(node, source)?;

    let qualified_name = match context {
        Some(ctx) => format!("{}.{}", ctx, name),
        None => name.clone(),
    };

    let visibility = detect_visibility(&name);

    Some(
        CodeNode::new(&name, &qualified_name, NodeKind::Constructor, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_visibility(visibility),
    )
}

/// Extracts a getter or setter.
fn extract_accessor(
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

    let visibility = detect_visibility(&name);

    Some(
        CodeNode::new(&name, &qualified_name, NodeKind::Method, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts an import statement.
fn extract_import(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let text = get_text(node, source);

    // Extract the import path from the statement
    if let Some(start) = text.find('\'').or_else(|| text.find('"')) {
        if let Some(end) = text[start + 1..]
            .find('\'')
            .or_else(|| text[start + 1..].find('"'))
        {
            let import_path = &text[start + 1..start + 1 + end];
            return Some(
                CodeNode::new(import_path, import_path, NodeKind::Import, file_path)
                    .with_lines(
                        node.start_position().row as u32 + 1,
                        node.end_position().row as u32 + 1,
                    )
                    .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
            );
        }
    }
    None
}

/// Extracts a library declaration.
fn extract_library(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let text = get_text(node, source);
    let name = text
        .trim_start_matches("library ")
        .trim_end_matches(';')
        .trim();

    Some(
        CodeNode::new(name, name, NodeKind::Module, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
    )
}

/// Extracts a variable declaration.
fn extract_variable(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // Find the variable name
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "identifier" {
                let name = get_text(&child, source);
                let visibility = detect_visibility(&name);
                return Some(
                    CodeNode::new(&name, &name, NodeKind::Variable, file_path)
                        .with_lines(
                            node.start_position().row as u32 + 1,
                            node.end_position().row as u32 + 1,
                        )
                        .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
                        .with_visibility(visibility),
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

/// Finds constructor name in a constructor signature.
fn find_constructor_name(node: &Node, source: &str) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "identifier" {
                return Some(get_text(&child, source));
            }
        }
    }
    None
}

/// Dart visibility: underscore prefix means private.
fn detect_visibility(name: &str) -> Visibility {
    if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

/// Builds a function signature.
fn build_function_signature(node: &Node, source: &str, name: &str) -> String {
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "void".to_string());

    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());

    format!("{} {}{}", return_type, name, params)
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
    if node.kind() == "call_expression" || node.kind() == "selector" {
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
    fn test_parse_simple_class() {
        let source = r#"
class MyClass {
  void hello() {
    print('Hello');
  }
}
"#;

        let parser = DartParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "my_class.dart");

        // Should find the class
        assert!(nodes
            .iter()
            .any(|n| n.name == "MyClass" && matches!(n.kind, NodeKind::Class)));
    }

    #[test]
    fn test_visibility_detection() {
        let source = r#"
class Example {
  void publicMethod() {}
  void _privateMethod() {}
}
"#;

        let parser = DartParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "example.dart");

        // Class should be public (no underscore)
        let example = nodes.iter().find(|n| n.name == "Example");
        assert!(example.is_some());
        assert!(matches!(example.unwrap().visibility, Visibility::Public));
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
enum Color { red, green, blue }
"#;

        let parser = DartParser;
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&parser.language()).unwrap();
        let tree = ts_parser.parse(source, None).unwrap();

        let nodes = parser.extract_nodes(&tree, source, "color.dart");

        assert!(nodes
            .iter()
            .any(|n| n.name == "Color" && matches!(n.kind, NodeKind::Enum)));
    }
}
