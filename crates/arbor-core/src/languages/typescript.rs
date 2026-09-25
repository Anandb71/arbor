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

            // `export_statement` is deliberately not handled here.
            //
            // It used to recurse into its declaration children explicitly, and
            // then the generic child loop below recursed into those same
            // children again — so every exported symbol was extracted twice.
            // That doubled the node count for the export-heavy files typical of
            // TS/JS, split each symbol's centrality across its duplicate, and
            // double-counted it in blast radius. Two vertices also shared one
            // `CodeNode::id`, since the id is a hash of (file, name, kind).
            //
            // The generic recursion already reaches every declaration, and
            // `is_node_exported` reads the parent, so export status survives.
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
    let (extends, implements) = super::heritage::clause_bases(node, source);

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
            .with_exported_if(is_exported)
            .with_extends(extends)
            .with_implements(implements),
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
///   - Direct call     `foo()`             → `"foo"`
///   - this/super      `this.foo()`        → `"this.foo"` / `"super.foo"`
///   - Static-looking  `MathUtils.add()`   → `"MathUtils.add"` (exact FQN candidate)
///   - Instance call   `userService.get()` → `".get"` (unknown-receiver marker)
///
/// # Why unknown-receiver calls are emitted rather than dropped
///
/// These used to be discarded outright, on the grounds that resolving `obj` needs
/// type inference we do not have. But `obj.method()` is the dominant call shape in
/// real TypeScript and JavaScript, so dropping it left the graph nearly edgeless on
/// the largest ecosystem Arbor supports — and an empty graph reports a blast radius
/// of zero, which reads as "safe" rather than "unknown".
///
/// The leading `.` marks the reference as receiver-unknown. The graph builder
/// resolves it by method name, refuses to link when too many symbols share that
/// name, and stamps a reduced confidence on whatever edge it does create. Precision
/// is now expressed in the edge weight instead of by silence.
fn extract_call_references(root: &Node, source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    'outer: loop {
        let node = cursor.node();

        if node.kind() == "call_expression" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let range = func_node.byte_range();
                if range.end <= source.len() {
                    if let Some(reference) = classify_callee(&source[range]) {
                        refs.push(reference);
                    }
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

/// Turns the callee text of a call expression into a resolvable reference.
///
/// Returns `None` when the callee carries no usable name (an immediately-invoked
/// function, a computed member like `handlers[key]`, or an empty match).
fn classify_callee(raw: &str) -> Option<String> {
    // Optional chaining is a null-guard, not a different call target.
    let call_text = raw.replace("?.", ".");
    let call_text = call_text.trim();

    if call_text.is_empty() {
        return None;
    }

    if !call_text.contains('.') {
        // Direct call: `validate(x)`, `clone(node)`.
        return if is_plain_identifier(call_text) {
            Some(call_text.to_string())
        } else {
            // `(() => {})`, `arr[i]`, `(await f())` — no stable name.
            None
        };
    }

    let (receiver, method) = call_text.rsplit_once('.')?;
    if !is_plain_identifier(method) {
        return None;
    }

    if receiver == "this" || receiver == "super" {
        // Keep the receiver so an override can be told from a super call.
        // A bare method name would bind to the subclass's own method and
        // hide the base the `super` call actually reaches.
        return Some(format!("{receiver}.{method}"));
    }

    // A chained or computed receiver (`this.repo`, `getUser().profile`,
    // `items[0]`) tells us nothing about the type, so fall through to the
    // unknown-receiver marker.
    if is_plain_identifier(receiver) {
        // Capitalised bare receivers are classes, enums, or namespace imports
        // in every mainstream TS/JS convention: `Logger.info`, `MathUtils.add`.
        // Emitting the qualified name lets the symbol table match it exactly.
        if receiver.starts_with(|c: char| c.is_uppercase()) {
            return Some(format!("{receiver}.{method}"));
        }
    }

    Some(format!(".{method}"))
}

/// Whether a string is a single JS/TS identifier (no operators, calls, or indexing).
fn is_plain_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

// Builder pattern helpers as a trait extension
trait CodeNodeExt {
    fn with_async_if(self, cond: bool) -> Self;
    fn with_static_if(self, cond: bool) -> Self;
    fn with_exported_if(self, cond: bool) -> Self;
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
    fn with_exported_if(self, cond: bool) -> Self {
        if cond {
            self.as_exported()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod callee_tests {
    use super::classify_callee;

    #[test]
    fn direct_call_is_bare_name() {
        assert_eq!(classify_callee("validate"), Some("validate".into()));
        assert_eq!(classify_callee("_private"), Some("_private".into()));
        assert_eq!(classify_callee("$jq"), Some("$jq".into()));
    }

    #[test]
    fn this_and_super_keep_the_receiver() {
        assert_eq!(
            classify_callee("this.validate"),
            Some("this.validate".into())
        );
        assert_eq!(classify_callee("super.clone"), Some("super.clone".into()));
    }

    #[test]
    fn capitalised_receiver_keeps_qualifier() {
        // Resolvable as an exact FQN against a class or namespace import.
        assert_eq!(
            classify_callee("MathUtils.add"),
            Some("MathUtils.add".into())
        );
        assert_eq!(classify_callee("Logger.info"), Some("Logger.info".into()));
    }

    #[test]
    fn lowercase_receiver_becomes_unknown_marker() {
        // The shape that used to be dropped entirely, and which dominates real
        // TS/JS: service objects, imported instances, arrays, strings.
        assert_eq!(
            classify_callee("userService.findOne"),
            Some(".findOne".into())
        );
        assert_eq!(classify_callee("arr.push"), Some(".push".into()));
        assert_eq!(classify_callee("str.trim"), Some(".trim".into()));
    }

    #[test]
    fn chained_receiver_becomes_unknown_marker() {
        assert_eq!(
            classify_callee("this.repo.findOne"),
            Some(".findOne".into())
        );
        assert_eq!(classify_callee("a.b.c.d"), Some(".d".into()));
        // The receiver is a call result, but `profile` is still the method
        // being invoked and is worth resolving by name.
        assert_eq!(
            classify_callee("getUser().profile"),
            Some(".profile".into())
        );
    }

    #[test]
    fn optional_chaining_is_normalised() {
        assert_eq!(classify_callee("user?.getName"), Some(".getName".into()));
        assert_eq!(
            classify_callee("this?.validate"),
            Some("this.validate".into())
        );
    }

    #[test]
    fn computed_and_anonymous_callees_are_dropped() {
        // No stable name to resolve against.
        assert_eq!(classify_callee("handlers[key]"), None);
        assert_eq!(classify_callee("(() => {})"), None);
        assert_eq!(classify_callee(""), None);
    }

    #[test]
    fn trailing_dot_is_not_a_method() {
        assert_eq!(classify_callee("obj."), None);
    }
}
