//! Rust language parser implementation.
//!
//! Handles .rs files and extracts functions, structs, enums, traits,
//! and impl blocks.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct RustParser;

impl LanguageParser for RustParser {
    fn language(&self) -> Language {
        tree_sitter_rust::language()
    }

    fn extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();

        extract_from_node(&root, source, file_path, &mut nodes, None);

        nodes
    }
}

fn extract_from_node(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    context: Option<&str>,
) {
    stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
    let kind = node.kind();

    match kind {
        // Standalone functions
        "function_item" => {
            if let Some(code_node) = extract_function(node, source, file_path, context) {
                nodes.push(code_node);
            }
        }

        // Structs
        "struct_item" => {
            if let Some(code_node) = extract_struct(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Enums
        "enum_item" => {
            if let Some(code_node) = extract_enum(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Traits (Rust's version of interfaces)
        "trait_item" => {
            if let Some(code_node) = extract_trait(node, source, file_path) {
                let trait_name = code_node.name.clone();
                nodes.push(code_node);

                // Extract trait methods
                if let Some(body) = find_child_by_kind(node, "declaration_list") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(&child, source, file_path, nodes, Some(&trait_name));
                        }
                    }
                }
                return;
            }
        }

        // Impl blocks
        "impl_item" => {
            let impl_target = get_impl_target(node, source);
            if let Some(body) = find_child_by_kind(node, "declaration_list") {
                for i in 0..body.child_count() {
                    if let Some(child) = body.child(i) {
                        extract_from_node(&child, source, file_path, nodes, impl_target.as_deref());
                    }
                }
            }
            return;
        }

        // Module declarations
        "mod_item" => {
            if let Some(code_node) = extract_module(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Use statements (imports)
        "use_declaration" => {
            if let Some(code_node) = extract_use(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Constants and statics
        "const_item" | "static_item" => {
            if let Some(code_node) = extract_const(node, source, file_path) {
                nodes.push(code_node);
            }
        }

        // Type aliases
        "type_item" => {
            if let Some(code_node) = extract_type_alias(node, source, file_path) {
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
    }); // stacker::maybe_grow
}

/// Extracts a function or method.
fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
    context: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let kind = if context.is_some() {
        NodeKind::Method
    } else {
        NodeKind::Function
    };

    let qualified_name = match context {
        Some(ctx) => format!("{}.{}", ctx, name),
        None => name.clone(),
    };

    // Check visibility
    let visibility = detect_visibility(node, source);

    // Check for async
    let is_async = has_modifier(node, "async");

    // Build signature
    let signature = build_function_signature(node, source, &name);

    // Extract references
    let references = extract_call_references(node, source);

    Some(
        CodeNode::new(&name, &qualified_name, kind, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_signature(signature)
            .with_visibility(visibility)
            .with_references(references)
            .with_async_if(is_async),
    )
}

/// Extracts a struct definition.
fn extract_struct(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Struct, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts an enum definition.
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

/// Extracts a trait definition.
fn extract_trait(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
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

/// Extracts a module declaration.
fn extract_module(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Module, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts a use statement.
fn extract_use(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // Get the full use path
    if let Some(arg) = node.child_by_field_name("argument") {
        let path = get_text(&arg, source);

        return Some(
            CodeNode::new(&path, &path, NodeKind::Import, file_path)
                .with_lines(
                    node.start_position().row as u32 + 1,
                    node.end_position().row as u32 + 1,
                )
                .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
        );
    }
    None
}

/// Extracts a const or static item.
fn extract_const(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Constant, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

/// Extracts a type alias.
fn extract_type_alias(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = detect_visibility(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::TypeAlias, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility),
    )
}

// ============================================================================
// Helper functions
// ============================================================================

/// Gets text content of a node.
fn get_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Finds a child node by its kind.
fn find_child_by_kind<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

/// Gets the target type of an impl block (e.g., "UserService" from `impl UserService`).
fn get_impl_target(node: &Node, source: &str) -> Option<String> {
    // The type being implemented for
    if let Some(type_node) = node.child_by_field_name("type") {
        return Some(get_text(&type_node, source));
    }
    None
}

/// Detects visibility from Rust's pub/pub(crate) modifiers.
fn detect_visibility(node: &Node, source: &str) -> Visibility {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "visibility_modifier" {
                let text = get_text(&child, source);
                if text == "pub" {
                    return Visibility::Public;
                } else if text.contains("crate") || text.contains("super") {
                    return Visibility::Internal;
                }
            }
        }
    }
    Visibility::Private
}

/// Checks if a node has a specific modifier.
fn has_modifier(node: &Node, modifier: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == modifier {
                return true;
            }
        }
    }
    false
}

/// Builds a function signature.
fn build_function_signature(node: &Node, source: &str, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());

    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| get_text(&n, source))
        .unwrap_or_default();

    if return_type.is_empty() {
        format!("fn {}{}", name, params)
    } else {
        format!("fn {}{} {}", name, params, return_type)
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

fn collect_calls(root: &Node, source: &str, refs: &mut Vec<String>) {
    let mut cursor = root.walk();

    'outer: loop {
        let node = cursor.node();

        if node.kind() == "call_expression" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let range = func_node.byte_range();
                if range.end <= source.len() {
                    refs.push(source[range].to_string());
                }
            }
        }

        if cursor.goto_first_child() { continue; }
        if cursor.goto_next_sibling() { continue; }
        loop {
            if !cursor.goto_parent() { break 'outer; }
            if cursor.depth() == 0 { break 'outer; }
            if cursor.goto_next_sibling() { break; }
        }
    }
}

// Builder pattern helpers
trait CodeNodeExt {
    fn with_async_if(self, cond: bool) -> Self;
}

impl CodeNodeExt for CodeNode {
    fn with_async_if(self, cond: bool) -> Self {
        if cond {
            self.as_async()
        } else {
            self
        }
    }
}
