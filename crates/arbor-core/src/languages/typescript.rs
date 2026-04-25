//! TypeScript/JavaScript parser implementation.
//!
//! This handles TS, TSX, JS, and JSX files. Tree-sitter's TypeScript
//! grammar is comprehensive enough to handle most JS patterns too.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language(&self) -> Language {
        tree_sitter_typescript::language_typescript()
    }

    fn extensions(&self) -> &[&str] {
        &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();
        extract_from_node(&root, source, file_path, &mut nodes, None);
        nodes
    }
}

/// Recursively extracts nodes from the AST.
/// Uses stacker::maybe_grow to prevent stack overflow on deeply-nested files
/// (e.g. TypeScript compiler's checker.ts which is 50k+ lines).
fn extract_from_node(
    node: &Node,
    source: &str,
    file_path: &str,
    nodes: &mut Vec<CodeNode>,
    parent_name: Option<&str>,
) {
    stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
        let kind = node.kind();

        match kind {
            "function_declaration" | "function" => {
                if let Some(code_node) = extract_function(node, source, file_path, parent_name) {
                    nodes.push(code_node);
                }
            }

            "lexical_declaration" | "variable_declaration" => {
                if let Some(code_node) = extract_arrow_function(node, source, file_path) {
                    nodes.push(code_node);
                }
            }

            "class_declaration" | "class" => {
                if let Some(code_node) = extract_class(node, source, file_path) {
                    let class_name = code_node.name.clone();
                    nodes.push(code_node);
                    if let Some(body) = node.child_by_field_name("body") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                extract_from_node(
                                    &child,
                                    source,
                                    file_path,
                                    nodes,
                                    Some(&class_name),
                                );
                            }
                        }
                    }
                    return;
                }
            }

            "method_definition" => {
                if let Some(code_node) = extract_method(node, source, file_path, parent_name) {
                    nodes.push(code_node);
                }
            }

            "interface_declaration" => {
                if let Some(code_node) = extract_interface(node, source, file_path) {
                    nodes.push(code_node);
                }
            }

            "type_alias_declaration" => {
                if let Some(code_node) = extract_type_alias(node, source, file_path) {
                    nodes.push(code_node);
                }
            }

            "import_statement" => {
                if let Some(code_node) = extract_import(node, source, file_path) {
                    nodes.push(code_node);
                }
            }

            "export_statement" => {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        let child_kind = child.kind();
                        if matches!(
                            child_kind,
                            "function_declaration" | "class_declaration" | "lexical_declaration"
                        ) {
                            extract_from_node(&child, source, file_path, nodes, parent_name);
                        }
                    }
                }
            }

            _ => {}
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                extract_from_node(&child, source, file_path, nodes, parent_name);
            }
        }
    });
}

fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
    parent_name: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let qualified_name = match parent_name {
        Some(parent) => format!("{}.{}", parent, name),
        None => name.clone(),
    };

    let kind = if parent_name.is_some() {
        NodeKind::Method
    } else {
        NodeKind::Function
    };

    let is_async = has_modifier(node, source, "async");
    let is_exported = is_node_exported(node);
    let signature = build_function_signature(node, source);
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
            .with_visibility(if is_exported {
                Visibility::Public
            } else {
                Visibility::Private
            })
            .with_references(references)
            .with_async_if(is_async)
            .with_exported_if(is_exported),
    )
}

fn extract_arrow_function(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    for i in 0..node.child_count() {
        if let Some(declarator) = node.child(i) {
            if declarator.kind() == "variable_declarator" {
                let name_node = declarator.child_by_field_name("name")?;
                let value_node = declarator.child_by_field_name("value")?;

                if value_node.kind() == "arrow_function" {
                    let name = get_text(&name_node, source);
                    let is_async = has_modifier(&value_node, source, "async");
                    let is_exported = is_node_exported(node);
                    let signature = build_arrow_signature(&value_node, source, &name);
                    let references = extract_call_references(&value_node, source);

                    return Some(
                        CodeNode::new(&name, &name, NodeKind::Function, file_path)
                            .with_lines(
                                node.start_position().row as u32 + 1,
                                node.end_position().row as u32 + 1,
                            )
                            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
                            .with_column(name_node.start_position().column as u32)
                            .with_signature(signature)
                            .with_references(references)
                            .with_async_if(is_async)
                            .with_exported_if(is_exported),
                    );
                }
            }
        }
    }
    None
}

fn extract_class(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let is_exported = is_node_exported(node);

    Some(
        CodeNode::new(&name, &name, NodeKind::Class, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(if is_exported {
                Visibility::Public
            } else {
                Visibility::Private
            })
            .with_exported_if(is_exported),
    )
}

fn extract_method(
    node: &Node,
    source: &str,
    file_path: &str,
    parent_name: Option<&str>,
) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);

    let qualified_name = match parent_name {
        Some(parent) => format!("{}.{}", parent, name),
        None => name.clone(),
    };

    let is_async = has_modifier(node, source, "async");
    let is_static = has_modifier(node, source, "static");
    let signature = build_function_signature(node, source);
    let references = extract_call_references(node, source);
    let visibility = detect_visibility(node, source);

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
            .with_references(references)
            .with_async_if(is_async)
            .with_static_if(is_static),
    )
}

fn extract_interface(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let is_exported = is_node_exported(node);

    Some(
        CodeNode::new(&name, &name, NodeKind::Interface, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_visibility(if is_exported {
                Visibility::Public
            } else {
                Visibility::Private
            })
            .with_exported_if(is_exported),
    )
}

fn extract_type_alias(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_text(&name_node, source);
    let is_exported = is_node_exported(node);

    Some(
        CodeNode::new(&name, &name, NodeKind::TypeAlias, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(name_node.start_position().column as u32)
            .with_exported_if(is_exported),
    )
}

/// Extracts an import statement, capturing both the source module and what was imported.
///
/// The imported names are stored in `references` so the graph builder can build an
/// import map for import-aware edge resolution. Format:
///   - Named import  `{ X }`       → "X"
///   - Default import `import X`   → "X"
///   - Namespace     `* as X`      → "*as:X"  (graph builder resolves X.method() calls)
fn extract_import(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let source_node = node.child_by_field_name("source")?;
    let raw = get_text(&source_node, source);
    let module_path = raw.trim_matches(|c| c == '"' || c == '\'');

    let mut imported_names: Vec<String> = Vec::new();

    // Walk the import_clause to find what was imported
    for i in 0..node.child_count() {
        if let Some(clause) = node.child(i) {
            if clause.kind() != "import_clause" {
                continue;
            }
            for j in 0..clause.child_count() {
                if let Some(child) = clause.child(j) {
                    match child.kind() {
                        // Default import: `import Foo from './mod'`
                        "identifier" => {
                            imported_names.push(get_text(&child, source));
                        }
                        // Named imports: `import { A, B as C } from './mod'`
                        "named_imports" => {
                            for k in 0..child.child_count() {
                                if let Some(spec) = child.child(k) {
                                    if spec.kind() == "import_specifier" {
                                        // Use the local alias if present, otherwise the original name
                                        let local = spec
                                            .child_by_field_name("alias")
                                            .or_else(|| spec.child_by_field_name("name"))
                                            .map(|n| get_text(&n, source));
                                        if let Some(n) = local {
                                            imported_names.push(n);
                                        }
                                    }
                                }
                            }
                        }
                        // Namespace import: `import * as types from '@babel/types'`
                        "namespace_import" => {
                            // Find the identifier (the alias) — it follows the `as` keyword
                            for k in 0..child.child_count() {
                                if let Some(ns_child) = child.child(k) {
                                    if ns_child.kind() == "identifier" {
                                        let alias = get_text(&ns_child, source);
                                        imported_names.push(format!("*as:{}", alias));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            break; // only one import_clause per statement
        }
    }

    Some(
        CodeNode::new(module_path, module_path, NodeKind::Import, file_path)
            .with_lines(
                node.start_position().row as u32 + 1,
                node.end_position().row as u32 + 1,
            )
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_references(imported_names),
    )
}

// ============================================================================
// Helper functions
// ============================================================================

fn get_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn has_modifier(node: &Node, source: &str, modifier: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let text = get_text(&child, source);
            if text == modifier {
                return true;
            }
        }
    }
    false
}

fn is_node_exported(node: &Node) -> bool {
    if let Some(parent) = node.parent() {
        return parent.kind() == "export_statement";
    }
    false
}

fn detect_visibility(node: &Node, source: &str) -> Visibility {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let text = get_text(&child, source);
            match text.as_str() {
                "public" => return Visibility::Public,
                "private" => return Visibility::Private,
                "protected" => return Visibility::Protected,
                _ => {}
            }
        }
    }
    Visibility::Public
}

fn build_function_signature(node: &Node, source: &str) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_text(&n, source))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| get_text(&n, source))
        .unwrap_or_default();

    if return_type.is_empty() {
        format!("{}{}", name, params)
    } else {
        format!("{}{}{}", name, params, return_type)
    }
}

fn build_arrow_signature(node: &Node, source: &str, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .or_else(|| node.child_by_field_name("parameter"))
        .map(|n| get_text(&n, source))
        .unwrap_or_else(|| "()".to_string());
    format!("{}{}", name, params)
}

/// Extracts function call references from a node's body.
///
/// Uses an iterative TreeCursor traversal to prevent stack overflow on deeply-nested
/// ASTs (e.g. TypeScript compiler, large generated files).
///
/// Resolution strategy:
///   - Direct call   `foo()`         → "foo"         (resolvable via symbol table)
///   - this-call     `this.foo()`    → "foo"          (resolvable via same-class lookup)
///   - super-call    `super.foo()`   → "foo"          (resolvable via parent class)
///   - Other dotted  `arr.push()`    → DROPPED        (method on unknown object type;
///                                                     can't resolve without type inference,
///                                                     and would cause false name collisions)
fn extract_call_references(root: &Node, source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    'outer: loop {
        let node = cursor.node();

        if node.kind() == "call_expression" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let range = func_node.byte_range();
                if range.end <= source.len() {
                    let call_text = &source[range];

                    if !call_text.contains('.') {
                        // Direct call: validate(x), clone(node) — always track
                        refs.push(call_text.to_string());
                    } else if call_text.starts_with("this.") || call_text.starts_with("super.") {
                        // this.validate() / super.clone() — strip prefix, track method name
                        if let Some(method) = call_text.splitn(2, '.').nth(1) {
                            if !method.is_empty() && !method.contains('.') {
                                refs.push(method.to_string());
                            }
                        }
                    }
                    // All other dotted calls (arr.push, path.resolve, str.trim, obj.method)
                    // are DROPPED. Without type inference we cannot know what type `arr`,
                    // `path`, `str`, or `obj` are, so any edge would be a false positive.
                }
            }
        }

        // Iterative depth-first traversal — no recursion, no stack overflow
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
            // depth() is relative to the node root.walk() was called on, so 0 = back at root
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

// Builder pattern helpers as a trait extension
trait CodeNodeExt {
    fn with_async_if(self, cond: bool) -> Self;
    fn with_static_if(self, cond: bool) -> Self;
    fn with_exported_if(self, cond: bool) -> Self;
}

impl CodeNodeExt for CodeNode {
    fn with_async_if(self, cond: bool) -> Self {
        if cond { self.as_async() } else { self }
    }
    fn with_static_if(self, cond: bool) -> Self {
        if cond { self.as_static() } else { self }
    }
    fn with_exported_if(self, cond: bool) -> Self {
        if cond { self.as_exported() } else { self }
    }
}
