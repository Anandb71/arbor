//! Python language parser implementation.
//!
//! Handles .py and .pyi files. Python's AST is relatively
//! straightforward with clear function and class boundaries.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct PythonParser;

impl LanguageParser for PythonParser {
    fn language(&self) -> Language {
        tree_sitter_python::language()
    }

    fn extensions(&self) -> &[&str] {
        &["py", "pyi"]
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
    class_name: Option<&str>,
) {
    stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
        let kind = node.kind();

        match kind {
            "function_definition" => {
                if let Some(code_node) = extract_function(node, source, file_path, class_name) {
                    nodes.push(code_node);
                }
            }

            "class_definition" => {
                if let Some(code_node) = extract_class(node, source, file_path) {
                    let name = code_node.name.clone();
                    nodes.push(code_node);
                    if let Some(body) = node.child_by_field_name("body") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                extract_from_node(&child, source, file_path, nodes, Some(&name));
                            }
                        }
                    }
                    return;
                }
            }

            "import_statement" => {
                if let Some(code_node) = extract_import(node, source, file_path) {
                    nodes.push(code_node);
                }
            }

            "import_from_statement" => {
                if let Some(code_node) = extract_from_import(node, source, file_path) {
                    nodes.push(code_node);
                }
            }

            "expression_statement" if class_name.is_none() => {
                if let Some(assign) = find_child_by_kind(node, "assignment") {
                    if let Some(code_node) = extract_assignment(assign, source, file_path) {
                        nodes.push(code_node);
                    }
                }
            }

            _ => {}
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                extract_from_node(&child, source, file_path, nodes, class_name);
            }
        }
    });
}

fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let kind = if class_name.is_some() {
        NodeKind::Method
    } else {
        NodeKind::Function
    };

    let qualified_name = match class_name {
        Some(cls) => format!("{}.{}", cls, name),
        None => name.clone(),
    };

    let visibility = python_visibility(&name);
    let is_async = has_async_keyword(node, source);
    let is_static =
        has_decorator(node, source, "staticmethod") || has_decorator(node, source, "classmethod");
    let signature = build_function_signature(node, source, &name);
    let docstring = extract_docstring(node, source);
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
            .with_docstring_if(docstring)
            .with_async_if(is_async)
            .with_static_if(is_static),
    )
}

fn extract_class(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let visibility = python_visibility(&name);
    let docstring = extract_docstring(node, source);

    Some(
        CodeNode::new(&name, &name, NodeKind::Class, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(visibility)
            .with_docstring_if(docstring),
    )
}

fn extract_import(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let text = get_text(node, source);
    let module_name = text.strip_prefix("import ")?.trim();

    Some(
        CodeNode::new(module_name, module_name, NodeKind::Import, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32),
    )
}

/// Extracts a from...import statement and captures the imported names.
///
/// The imported names are stored in `references` so the graph builder can
/// build an import map for import-aware edge resolution.
///
/// Examples:
///   `from django.http import HttpResponse` → references: ["HttpResponse"]
///   `from .utils import helper, format_output` → references: ["helper", "format_output"]
///   `from typing import *` → references: ["*"]
fn extract_from_import(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let module_node = node.child_by_field_name("module_name")?;
    let module_name = get_text(&module_node, source);

    let mut imported_names: Vec<String> = Vec::new();
    let mut past_import_kw = false;

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let child_text = get_text(&child, source);

            if child_text == "import" {
                past_import_kw = true;
                continue;
            }
            if !past_import_kw {
                continue;
            }

            match child.kind() {
                // Wildcard: `from X import *`
                "wildcard_import" => {
                    imported_names.push("*".to_string());
                }
                // Single name or dotted name
                "dotted_name" | "identifier" => {
                    imported_names.push(child_text);
                }
                // `from X import Y as Z` — use the local name Z
                "aliased_import" => {
                    let local = child
                        .child_by_field_name("alias")
                        .map(|n| get_text(&n, source))
                        .or_else(|| {
                            child
                                .child_by_field_name("name")
                                .map(|n| get_text(&n, source))
                        });
                    if let Some(name) = local {
                        imported_names.push(name);
                    }
                }
                // Parenthesised list: `from X import (A, B, C)`
                _ if child.kind().contains("list") || child.kind() == "import_list" => {
                    for j in 0..child.child_count() {
                        if let Some(item) = child.child(j) {
                            match item.kind() {
                                "dotted_name" | "identifier" => {
                                    imported_names.push(get_text(&item, source));
                                }
                                "aliased_import" => {
                                    let local = item
                                        .child_by_field_name("alias")
                                        .map(|n| get_text(&n, source))
                                        .or_else(|| {
                                            item.child_by_field_name("name")
                                                .map(|n| get_text(&n, source))
                                        });
                                    if let Some(name) = local {
                                        imported_names.push(name);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Some(
        CodeNode::new(&module_name, &module_name, NodeKind::Import, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_references(imported_names),
    )
}

fn extract_assignment(node: Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let left = node.child_by_field_name("left")?;
    if left.kind() != "identifier" {
        return None;
    }
    let name = get_text(&left, source);
    let kind = if name.chars().all(|c| c.is_uppercase() || c == '_') {
        NodeKind::Constant
    } else {
        NodeKind::Variable
    };

    Some(
        CodeNode::new(&name, &name, kind, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(left.start_position().column as u32),
    )
}

// ============================================================================
// Helper functions
// ============================================================================

fn get_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

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

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && !name.ends_with("__") {
        Visibility::Private
    } else if name.starts_with('_') {
        Visibility::Protected
    } else {
        Visibility::Public
    }
}

fn has_async_keyword(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if get_text(&child, source) == "async" {
                return true;
            }
        }
    }
    false
}

fn has_decorator(node: &Node, source: &str, decorator_name: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "decorator" {
                let text = get_text(&child, source);
                if text.contains(decorator_name) {
                    return true;
                }
            }
        }
    }
    false
}

fn build_function_signature(node: &Node, source: &str, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| format!(" -> {}", get_text(&n, source)))
        .unwrap_or_default();
    format!("def {}{}{}", name, params, return_type)
}

fn extract_docstring(node: &Node, source: &str) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "expression_statement" {
                if let Some(string_node) = child.child(0) {
                    if string_node.kind() == "string" {
                        let text = get_text(&string_node, source);
                        let doc = text
                            .trim_start_matches("\"\"\"")
                            .trim_start_matches("'''")
                            .trim_end_matches("\"\"\"")
                            .trim_end_matches("'''")
                            .trim();
                        return Some(doc.to_string());
                    }
                }
            }
            break;
        }
    }
    None
}

/// Extracts function call references using iterative TreeCursor traversal.
///
/// For Python, we keep the full call text (including dotted paths) because:
///   - `self.method()`  → "self.method" → suffix-matched to same-class method ✓
///   - `HttpResponse()` → "HttpResponse" → resolved directly ✓
///   - `os.path.join()` → "os.path.join" → fails to resolve (stdlib) → dropped ✓
///
/// Python doesn't suffer from the JS name-collision problem because Python
/// call expressions rarely strip the receiver object.
fn extract_call_references(root: &Node, source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    'outer: loop {
        let node = cursor.node();

        if node.kind() == "call" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let range = func_node.byte_range();
                if range.end <= source.len() {
                    refs.push(source[range].to_string());
                }
            }
        }

        if cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            continue;
        }
        loop {
            if !cursor.goto_parent() {
                break 'outer;
            }
            if cursor.depth() == 0 {
                break 'outer;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }
    }

    refs.sort();
    refs.dedup();
    refs
}

// Builder pattern helpers
trait CodeNodeExt {
    fn with_async_if(self, cond: bool) -> Self;
    fn with_static_if(self, cond: bool) -> Self;
    fn with_docstring_if(self, docstring: Option<String>) -> Self;
}

impl CodeNodeExt for CodeNode {
    fn with_async_if(self, cond: bool) -> Self {
        if cond {
            self.as_async()
        } else {
            self
        }
    }
    fn with_static_if(self, cond: bool) -> Self {
        if cond {
            self.as_static()
        } else {
            self
        }
    }
    fn with_docstring_if(mut self, docstring: Option<String>) -> Self {
        self.docstring = docstring;
        self
    }
}
