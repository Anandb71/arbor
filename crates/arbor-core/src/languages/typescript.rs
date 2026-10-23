//! TypeScript/JavaScript parser implementation.
//!
//! `.ts` files use the TypeScript grammar. `.tsx`, `.jsx` and JavaScript use
//! the TSX grammar: the plain TypeScript grammar cannot parse JSX, so every
//! component body in a React file used to come back as error nodes. The only
//! construct TSX rejects, `<T>value` type assertions, is not valid in those
//! files anyway.

use crate::languages::LanguageParser;
use crate::node::{import_alias, import_alias_ref, CodeNode, NodeKind, Visibility};
use tree_sitter::{Language, Node, Tree};

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language(&self) -> Language {
        tree_sitter_typescript::language_typescript()
    }

    fn extensions(&self) -> &[&str] {
        &["ts", "mts", "cts"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();
        extract_from_node(&root, source, file_path, &mut nodes, None);
        distinguish_repeated_tests(&mut nodes, file_path);
        let pinned = extract_bindings_from_imported_values(&root, source, file_path, &nodes);
        nodes.extend(pinned);
        nodes
    }
}

/// Two tests in one file may share a title. Give each repeat its line, so
/// ids stay distinct while a unique title keeps its plain name.
fn distinguish_repeated_tests(nodes: &mut [CodeNode], file_path: &str) {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for node in nodes.iter() {
        if is_test_block_name(&node.name) {
            *seen.entry(node.qualified_name.clone()).or_default() += 1;
        }
    }
    for node in nodes.iter_mut() {
        if is_test_block_name(&node.name)
            && seen
                .get(&node.qualified_name)
                .is_some_and(|count| *count > 1)
        {
            node.qualified_name = format!("{} (line {})", node.qualified_name, node.line_start);
            node.id = CodeNode::compute_id(file_path, &node.qualified_name, node.kind);
        }
    }
}

fn is_test_block_name(name: &str) -> bool {
    TEST_BLOCKS
        .iter()
        .any(|block| name == *block || name.starts_with(&format!("{block}: ")))
}

/// TSX, JSX and JavaScript: the same extraction on the grammar that parses JSX.
pub struct TsxParser;

impl LanguageParser for TsxParser {
    fn language(&self) -> Language {
        tree_sitter_typescript::language_tsx()
    }

    fn extensions(&self) -> &[&str] {
        &["tsx", "jsx", "js", "mjs", "cjs"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        TypeScriptParser.extract_nodes(tree, source, file_path)
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
                nodes.extend(extract_require_bindings(node, source, file_path));
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

            // A test or hook body is an anonymous callback, so without a node
            // of its own its calls belonged to nothing and every Jest, Vitest
            // and Mocha test was invisible to `callers` (#235).
            "call_expression" => {
                if let Some(code_node) = extract_test_block(node, source, file_path) {
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

/// Test and hook calls whose callback runs as a test. Containers such as
/// `describe` are left out: their bodies hold the tests, which get nodes of
/// their own, and counting both would report each test twice.
const TEST_BLOCKS: &[&str] = &[
    "it",
    "test",
    "specify",
    "bench",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "before",
    "after",
];

/// The test-runner function a call goes through: `it`, `it.only`,
/// `test.skip`, or `test.each(rows)` all name a test block.
fn test_callee(callee: &Node, source: &str) -> Option<&'static str> {
    let mut current = *callee;
    loop {
        match current.kind() {
            "identifier" => {
                let name = get_text(&current, source);
                return TEST_BLOCKS.iter().find(|block| **block == name).copied();
            }
            "member_expression" => current = current.child_by_field_name("object")?,
            "call_expression" => current = current.child_by_field_name("function")?,
            _ => return None,
        }
    }
}

/// `it("accepts two", () => { ... })` becomes a function named
/// `it: accepts two` that owns the calls in its callback.
fn extract_test_block(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    let callee = node.child_by_field_name("function")?;
    let block = test_callee(&callee, source)?;
    let arguments = node.child_by_field_name("arguments")?;
    let mut title = None;
    let mut callback = None;
    for i in 0..arguments.named_child_count() {
        let argument = arguments.named_child(i)?;
        match argument.kind() {
            "string" | "template_string" if title.is_none() => {
                let text = get_text(&argument, source);
                let trimmed = text.trim_matches(|c| matches!(c, '"' | '\'' | '`'));
                title = Some(trimmed.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            "arrow_function" | "function" | "function_expression" => callback = Some(argument),
            _ => {}
        }
    }
    let callback = callback?;
    let label = match title.filter(|title| !title.is_empty()) {
        Some(title) => {
            let title: String = title.chars().take(80).collect();
            format!("{block}: {title}")
        }
        None => block.to_string(),
    };
    let line = node.start_position().row as u32 + 1;
    Some(
        CodeNode::new(&label, &label, NodeKind::Function, file_path)
            .with_lines(line, node.end_position().row as u32 + 1)
            .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
            .with_column(callee.start_position().column as u32)
            .with_visibility(Visibility::Private)
            .with_references(extract_call_references(&callback, source)),
    )
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
///   - Aliased       `{ X as Y }`  → "alias:Y:X"
///   - Default import `import X`   → "X"
///   - Namespace     `* as X`      → "*as:X"  (graph builder resolves X.method() calls)
///   - TypeScript    `import X = require('m')` → "X", "*as:X" (see [`extract_require_bindings`])
fn extract_import(node: &Node, source: &str, file_path: &str) -> Option<CodeNode> {
    // TypeScript's CommonJS form keeps its module on the clause, not on the
    // statement.
    if let Some(clause) = named_child_of_kind(node, "import_require_clause") {
        let local = named_child_of_kind(&clause, "identifier")?;
        let module = string_literal(&clause.child_by_field_name("source")?, source)?;
        let local = get_text(&local, source);
        return Some(import_node(
            &module,
            vec![local.clone(), format!("*as:{local}")],
            node,
            file_path,
        ));
    }

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
                                        if let Some(n) = import_specifier(&spec, source) {
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

    Some(import_node(module_path, imported_names, node, file_path))
}

/// An `Import` node for `module`, spanning `node`, with the names it binds.
fn import_node(module: &str, names: Vec<String>, node: &Node, file_path: &str) -> CodeNode {
    CodeNode::new(module, module, NodeKind::Import, file_path)
        .with_lines(
            node.start_position().row as u32 + 1,
            node.end_position().row as u32 + 1,
        )
        .with_bytes(node.start_byte() as u32, node.end_byte() as u32)
        .with_references(names)
}

/// `A as B` in `import { A as B }`: calls use `B`, and the module exports
/// `A`, so both are recorded (`alias:B:A`). `{ A }`, `{ A as A }` and
/// `{ default as B }` record the local name, as a default import does.
fn import_specifier(spec: &Node, source: &str) -> Option<String> {
    let imported = spec
        .child_by_field_name("name")
        .map(|n| get_text(&n, source));
    let local = spec
        .child_by_field_name("alias")
        .map(|n| get_text(&n, source));
    match (local, imported) {
        (Some(local), Some(imported)) => Some(import_binding(&local, &imported)),
        (local, imported) => local.or(imported),
    }
}

/// `local` bound to the module's export `exported`: `alias:local:exported`,
/// or the local name alone when the two match or the export is `default`.
fn import_binding(local: &str, exported: &str) -> String {
    if local != exported && exported != "default" {
        import_alias_ref(local, exported)
    } else {
        local.to_string()
    }
}

/// CommonJS `require` and awaited `import()` bindings, recorded in the same
/// format as `import` statements so the graph builder sees every name a file
/// takes from another module. One `Import` node per declarator:
///   - `const { A, B: C } = require('m')` → "A", "alias:C:B"
///   - `const C = require('m').B`         → "alias:C:B"
///   - `const M = require('m')`           → "M", "*as:M"
///   - `const { A } = await import('m')`  → "A"
///   - `const M = await import('m')`      → "*as:M"
///   - `const { A } = require('m').make(deps)`, `require('m')(deps)` → "alias:A:A"
///
/// A required module is `module.exports`, which may be the function itself,
/// so `M()` is a call to it as with a default import. An ES module namespace
/// is never callable, so a dynamic import binds only the namespace. Names
/// destructured from a value computed from the module (a factory it exports,
/// the module called, a property of an export) are pinned to the module, see
/// [`destructured_names`]; an identifier bound to such a value is not one of
/// its names. Only a string-literal module is read, and nested or `...rest`
/// patterns bind objects rather than exports, so they record nothing.
fn extract_require_bindings(node: &Node, source: &str, file_path: &str) -> Vec<CodeNode> {
    let mut imports = Vec::new();
    for i in 0..node.named_child_count() {
        let Some(declarator) = node.named_child(i) else {
            continue;
        };
        let Some((pattern, value)) = declarator_parts(&declarator) else {
            continue;
        };
        let Some(chain) = value_chain(&value, source) else {
            continue;
        };
        let ChainRoot::Module { module, dynamic } = chain.root else {
            continue;
        };
        let names = match (pattern.kind(), chain.steps, chain.outer_property) {
            ("identifier", 0, _) => {
                let local = get_text(&pattern, source);
                if dynamic {
                    vec![format!("*as:{local}")]
                } else {
                    vec![local.clone(), format!("*as:{local}")]
                }
            }
            ("identifier", 1, Some(export)) => {
                vec![import_binding(&get_text(&pattern, source), &export)]
            }
            ("object_pattern", 0, _) => destructured_names(&pattern, source, false),
            ("object_pattern", _, _) => destructured_names(&pattern, source, true),
            _ => continue,
        };
        if !names.is_empty() {
            imports.push(import_node(&module, names, &declarator, file_path));
        }
    }
    imports
}

/// Names destructured from a value computed from something the file already
/// binds from another module: `const { setFilters } = useListFilters()` after
/// `import { useListFilters } from './hooks'`, `const { load } = config.make()`
/// after `const config = require('./config')`, `const { get } = api` after
/// `import * as api from './api'`. One `Import` node per declarator, pinned to
/// that module (see [`destructured_names`]).
///
/// It runs after the main pass, over the `Import` nodes that pass found, so a
/// binding is recognised wherever the import sits in the file. The tree is
/// walked with a cursor, not recursion.
fn extract_bindings_from_imported_values(
    root: &Node,
    source: &str,
    file_path: &str,
    nodes: &[CodeNode],
) -> Vec<CodeNode> {
    let mut modules: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for import in nodes.iter().filter(|n| n.kind == NodeKind::Import) {
        for reference in &import.references {
            let local = match (reference.strip_prefix("*as:"), import_alias(reference)) {
                (Some(namespace), _) => namespace,
                (None, Some((local, _))) => local,
                (None, None) => reference.as_str(),
            };
            modules.insert(local, import.name.as_str());
        }
    }
    let mut imports = Vec::new();
    if modules.is_empty() {
        return imports;
    }
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if let Some((pattern, value)) = declarator_parts(&node) {
            if pattern.kind() == "object_pattern" {
                if let Some(Chain {
                    root: ChainRoot::Identifier(binding),
                    ..
                }) = value_chain(&value, source)
                {
                    if let Some(module) = modules.get(binding.as_str()) {
                        let names = destructured_names(&pattern, source, true);
                        if !names.is_empty() {
                            imports.push(import_node(module, names, &node, file_path));
                        }
                    }
                }
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return imports;
            }
        }
    }
}

/// The pattern and value of a `variable_declarator`: `const <pattern> = <value>`.
fn declarator_parts<'tree>(node: &Node<'tree>) -> Option<(Node<'tree>, Node<'tree>)> {
    if node.kind() != "variable_declarator" {
        return None;
    }
    Some((
        node.child_by_field_name("name")?,
        node.child_by_field_name("value")?,
    ))
}

/// Where a value's chain of member accesses, calls, `await`s and parentheses
/// starts, and how many steps it takes from there.
struct Chain {
    root: ChainRoot,
    /// Member and call steps between the root and the value.
    steps: usize,
    /// `.B` when the value's outermost step is a property access.
    outer_property: Option<String>,
}

enum ChainRoot {
    /// `require('m')` or `import('m')` with a string-literal module.
    Module { module: String, dynamic: bool },
    /// A plain identifier: `useListFilters()`, `config.make()`, `api`.
    Identifier(String),
}

/// Walks a declarator's value down its chain to the root, in a loop rather
/// than recursion so a very long chain can't exhaust the stack. `None` for any
/// other root (`this`, a literal, `require(name)`, ...).
fn value_chain(value: &Node, source: &str) -> Option<Chain> {
    let mut node = *value;
    let mut steps = 0usize;
    let mut outer_property = None;
    loop {
        match node.kind() {
            "parenthesized_expression" | "await_expression" => node = node.named_child(0)?,
            "member_expression" => {
                if steps == 0 {
                    outer_property = node
                        .child_by_field_name("property")
                        .filter(|property| property.kind() == "property_identifier")
                        .map(|property| get_text(&property, source));
                }
                steps += 1;
                node = node.child_by_field_name("object")?;
            }
            "call_expression" => {
                let function = node.child_by_field_name("function")?;
                let dynamic = match function.kind() {
                    "import" => true,
                    "identifier" if get_text(&function, source) == "require" => false,
                    _ => {
                        steps += 1;
                        node = function;
                        continue;
                    }
                };
                let module = sole_string_argument(&node, source)?;
                return Some(Chain {
                    root: ChainRoot::Module { module, dynamic },
                    steps,
                    outer_property,
                });
            }
            "identifier" => {
                return Some(Chain {
                    root: ChainRoot::Identifier(get_text(&node, source)),
                    steps,
                    outer_property,
                });
            }
            _ => return None,
        }
    }
}

/// The module a `require` or `import()` call names: its one argument, when
/// that is a string literal. Comments (webpack's magic comments) are not
/// arguments.
fn sole_string_argument(call: &Node, source: &str) -> Option<String> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut values = (0..arguments.named_child_count())
        .filter_map(|i| arguments.named_child(i))
        .filter(|argument| argument.kind() != "comment");
    let module = values.next()?;
    if values.next().is_some() {
        return None;
    }
    string_literal(&module, source)
}

/// The names an object pattern binds: `{ A }`, `{ A = d }`, `{ B: C }`,
/// `{ 'B': C }` and `{ B: C = d }`.
///
/// Destructuring a module itself (`require('m')`, `await import('m')`) reads
/// its exports, recorded as `import` statements record them. A `pinned`
/// pattern destructures a value computed from the module instead (a hook's or
/// factory's return value, a property of an export), whose names need not be
/// the module's top-level ones. Each is recorded as an alias even when the
/// local name matches (`alias:A:A`), so the graph builder links the call only
/// to a definition of that name in that module, never to a same-named
/// function elsewhere.
fn destructured_names(pattern: &Node, source: &str, pinned: bool) -> Vec<String> {
    let binding = |local: &str, exported: &str| {
        if pinned {
            import_alias_ref(local, exported)
        } else {
            import_binding(local, exported)
        }
    };
    let mut names = Vec::new();
    for i in 0..pattern.named_child_count() {
        let Some(property) = pattern.named_child(i) else {
            continue;
        };
        match property.kind() {
            "shorthand_property_identifier_pattern" => {
                let name = get_text(&property, source);
                names.push(binding(&name, &name));
            }
            "object_assignment_pattern" => {
                if let Some(left) = property
                    .child_by_field_name("left")
                    .filter(|left| left.kind() == "shorthand_property_identifier_pattern")
                {
                    let name = get_text(&left, source);
                    names.push(binding(&name, &name));
                }
            }
            "pair_pattern" => {
                let local =
                    property
                        .child_by_field_name("value")
                        .and_then(|value| match value.kind() {
                            "identifier" => Some(value),
                            "assignment_pattern" => value
                                .child_by_field_name("left")
                                .filter(|left| left.kind() == "identifier"),
                            _ => None,
                        });
                let exported =
                    property
                        .child_by_field_name("key")
                        .and_then(|key| match key.kind() {
                            "property_identifier" => Some(get_text(&key, source)),
                            "string" => string_literal(&key, source),
                            _ => None,
                        });
                if let (Some(local), Some(exported)) = (local, exported) {
                    names.push(binding(&get_text(&local, source), &exported));
                }
            }
            _ => {}
        }
    }
    names
}

/// The text of a `string` node without its quotes.
fn string_literal(node: &Node, source: &str) -> Option<String> {
    (node.kind() == "string").then(|| {
        get_text(node, source)
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string()
    })
}

fn named_child_of_kind<'tree>(node: &Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    (0..node.named_child_count())
        .filter_map(|i| node.named_child(i))
        .find(|child| child.kind() == kind)
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

        // Rendering `<PricingButton />` uses that component the way a call
        // does. In React code this is most of the dependency graph.
        if matches!(
            node.kind(),
            "jsx_opening_element" | "jsx_self_closing_element"
        ) {
            if let Some(name_node) = node.child_by_field_name("name") {
                let range = name_node.byte_range();
                if range.end <= source.len() {
                    if let Some(reference) = component_reference(&source[range]) {
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

/// A JSX tag that names a component. `<div>` and `<motion.div>` are host
/// elements; a component starts with a capital letter.
fn component_reference(tag: &str) -> Option<String> {
    let last = tag.rsplit('.').next()?;
    if !last.chars().next()?.is_ascii_uppercase() {
        return None;
    }
    classify_callee(tag)
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
mod jsx_tests {
    use super::{component_reference, TsxParser, TypeScriptParser};
    use crate::languages::LanguageParser;

    fn parse(parser: &dyn LanguageParser, source: &str, file: &str) -> Vec<crate::node::CodeNode> {
        let mut ts = tree_sitter::Parser::new();
        ts.set_language(&parser.language()).unwrap();
        let tree = ts.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{file} did not parse cleanly"
        );
        parser.extract_nodes(&tree, source, file)
    }

    #[test]
    fn rendering_a_component_references_it() {
        let source = r#"
import { PricingButton } from "./PricingButton";
import * as Tabs from "./tabs";

export default function PricingPage({ plans }: { plans: string[] }) {
  return (
    <main className="p-4">
      <PricingButton plan={plans[0]} />
      <Tabs.Panel>{plans.map(p => <span key={p}>{p}</span>)}</Tabs.Panel>
    </main>
  );
}
"#;
        let nodes = parse(&TsxParser, source, "app/pricing/page.tsx");
        let page = nodes
            .iter()
            .find(|n| n.name == "PricingPage")
            .expect("page component");
        assert!(
            page.references.contains(&"PricingButton".to_string()),
            "{:?}",
            page.references
        );
        assert!(
            page.references.contains(&"Tabs.Panel".to_string()),
            "{:?}",
            page.references
        );
        assert!(
            !page
                .references
                .iter()
                .any(|r| r == "main" || r == "span" || r == "div"),
            "{:?}",
            page.references
        );
    }

    #[test]
    fn plain_typescript_keeps_angle_bracket_assertions() {
        let source = "export function toCount(value: unknown) { return <number>value; }\n";
        let nodes = parse(&TypeScriptParser, source, "lib/count.ts");
        assert!(nodes.iter().any(|n| n.name == "toCount"));
    }

    #[test]
    fn host_elements_are_not_components() {
        assert_eq!(component_reference("div"), None);
        assert_eq!(component_reference("motion.div"), None);
        assert_eq!(
            component_reference("PricingButton"),
            Some("PricingButton".into())
        );
        assert_eq!(component_reference("ui.Button"), Some(".Button".into()));
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

#[cfg(test)]
mod test_block_tests {
    use super::TypeScriptParser;
    use crate::languages::LanguageParser;
    use crate::node::{CodeNode, NodeKind};

    fn parse(source: &str, file: &str) -> Vec<CodeNode> {
        let parser = TypeScriptParser;
        let mut ts = tree_sitter::Parser::new();
        ts.set_language(&parser.language()).unwrap();
        let tree = ts.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{file} did not parse cleanly"
        );
        parser.extract_nodes(&tree, source, file)
    }

    fn block<'a>(nodes: &'a [CodeNode], name: &str) -> &'a CodeNode {
        nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "{name} missing: {:?}",
                    nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
                )
            })
    }

    /// The repro from #235: calls inside `it` and `test` callbacks belong to the test.
    #[test]
    fn calls_inside_test_callbacks_belong_to_the_test() {
        let source = r#"
import { decide } from './policy';

describe('decide', () => {
  it('accepts two', () => {
    expect(decide(2)).toBe(true);
  });
});

test('rejects zero', function () {
  expect(decide(0)).toBe(false);
});

function namedHelper() {
  return decide(5);
}
"#;
        let nodes = parse(source, "src/policy.test.ts");
        let accepts = block(&nodes, "it: accepts two");
        assert_eq!(accepts.kind, NodeKind::Function);
        assert!(
            accepts.references.contains(&"decide".to_string()),
            "{:?}",
            accepts.references
        );
        assert_eq!(accepts.line_start, 5);
        let rejects = block(&nodes, "test: rejects zero");
        assert!(rejects.references.contains(&"decide".to_string()));
        assert!(block(&nodes, "namedHelper")
            .references
            .contains(&"decide".to_string()));
        // `describe` holds tests; it is not a test itself.
        assert!(nodes.iter().all(|node| !node.name.starts_with("describe")));
    }

    #[test]
    fn runner_variants_hooks_and_repeated_titles_are_distinct_tests() {
        let source = r#"
beforeEach(() => { resetDatabase(); });
it.only('works', async () => { await load(); });
test.skip('works', () => { load(); });
test.each([[1], [2]])('handles %i', (n) => { handle(n); });
it(`template ${name}`, () => { render(); });
it('has no callback');
it('works', () => { again(); });
"#;
        let nodes = parse(source, "src/a.test.ts");
        assert!(block(&nodes, "beforeEach")
            .references
            .contains(&"resetDatabase".to_string()));
        // `it.only('works')` and `it('works')` share a name; `test.skip('works')` doesn't.
        let works: Vec<_> = nodes
            .iter()
            .filter(|node| node.name == "it: works")
            .collect();
        assert_eq!(works.len(), 2);
        assert_ne!(
            works[0].id, works[1].id,
            "same title on different lines keeps distinct ids"
        );
        assert_eq!(works[0].qualified_name, "it: works (line 3)");
        assert_eq!(works[1].qualified_name, "it: works (line 8)");
        assert_eq!(block(&nodes, "test: works").qualified_name, "test: works");
        assert_eq!(
            block(&nodes, "beforeEach").qualified_name,
            "beforeEach",
            "a unique test keeps its plain name"
        );
        assert!(block(&nodes, "test: handles %i")
            .references
            .contains(&"handle".to_string()));
        assert!(block(&nodes, "it: template ${name}")
            .references
            .contains(&"render".to_string()));
        assert!(
            nodes.iter().all(|node| node.name != "it: has no callback"),
            "a call without a callback runs nothing"
        );
    }

    #[test]
    fn ordinary_calls_with_callbacks_are_not_tests() {
        let source =
            "items.forEach((item) => { use(item); });\nsetTimeout(() => { tick(); }, 10);\n";
        let nodes = parse(source, "src/a.ts");
        assert!(
            nodes.is_empty(),
            "{:?}",
            nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
        );
    }

    /// #258: `{ formatName as fmt }` recorded only `fmt`.
    #[test]
    fn aliased_named_imports_keep_the_imported_name() {
        let source = "\
import { formatName as fmt, parse } from './format';
import { a as a, default as Widget } from '../ui/widget';
import Default, { b as c } from './lib';
import * as types from '@babel/types';
";
        let imports: Vec<(String, Vec<String>)> = parse(source, "src/app.ts")
            .into_iter()
            .filter(|n| n.kind == NodeKind::Import)
            .map(|n| (n.name, n.references))
            .collect();
        let owned = |refs: &[&str]| refs.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        assert_eq!(
            imports,
            vec![
                (
                    "./format".to_string(),
                    owned(&["alias:fmt:formatName", "parse"])
                ),
                // `a as a` binds the imported name; `default` names no export.
                ("../ui/widget".to_string(), owned(&["a", "Widget"])),
                ("./lib".to_string(), owned(&["Default", "alias:c:b"])),
                ("@babel/types".to_string(), owned(&["*as:types"])),
            ]
        );
    }

    /// Names destructured from a value computed from a required module (a
    /// factory call, a property of an export) come from that module too, but
    /// not necessarily from its top level, so each is pinned to the module as
    /// an alias. An identifier bound to such a value is not one of its names.
    #[test]
    fn names_destructured_from_a_required_module_value_are_recorded() {
        let source = "\
const { getOrderPage, getOrderDetail: detail } = require('./paged').createPagedReads(prisma);
const { a } = require('./factory')(deps);
const made = require('./factory')(deps);
const { x } = require('./m').b;
const deep = require('./m').a.b;
export async function load() {
  const c = (await import('./p')).q;
  const { t } = await import('./p').then((m) => m.default);
  return [c, t];
}
";
        let imports: Vec<(String, Vec<String>)> = parse(source, "src/app.ts")
            .into_iter()
            .filter(|n| n.kind == NodeKind::Import)
            .map(|n| (n.name, n.references))
            .collect();
        let owned = |refs: &[&str]| refs.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        assert_eq!(
            imports,
            vec![
                (
                    "./paged".to_string(),
                    owned(&[
                        "alias:getOrderPage:getOrderPage",
                        "alias:detail:getOrderDetail"
                    ])
                ),
                ("./factory".to_string(), owned(&["alias:a:a"])),
                ("./m".to_string(), owned(&["alias:x:x"])),
                ("./p".to_string(), owned(&["alias:c:q"])),
                ("./p".to_string(), owned(&["alias:t:t"])),
            ]
        );
    }

    /// Names destructured from a value computed from an imported binding (a
    /// hook, a factory, an object) are pinned to that binding's module.
    #[test]
    fn names_destructured_from_an_imported_binding_are_pinned_to_its_module() {
        let source = "\
import { useListFilters } from './hooks';
import * as api from './api';
const config = require('./config');
const { setFilters, reset: clear } = useListFilters();
const { load } = config.make();
const { get } = api;
const { other } = notImported();
const value = useListFilters();
";
        let imports: Vec<(String, Vec<String>)> = parse(source, "src/app.ts")
            .into_iter()
            .filter(|n| n.kind == NodeKind::Import)
            .map(|n| (n.name, n.references))
            .collect();
        let owned = |refs: &[&str]| refs.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        assert_eq!(
            imports,
            vec![
                ("./hooks".to_string(), owned(&["useListFilters"])),
                ("./api".to_string(), owned(&["*as:api"])),
                ("./config".to_string(), owned(&["config", "*as:config"])),
                (
                    "./hooks".to_string(),
                    owned(&["alias:setFilters:setFilters", "alias:clear:reset"])
                ),
                ("./config".to_string(), owned(&["alias:load:load"])),
                ("./api".to_string(), owned(&["alias:get:get"])),
            ]
        );
    }

    /// A comment beside the module, such as webpack's magic comments, is
    /// not a second argument.
    #[test]
    fn a_comment_in_the_arguments_does_not_hide_the_module() {
        let source = "\
const b = require(/* webpackIgnore: true */ './b');
export async function load() {
  const { a } = await import(/* webpackChunkName: \"a\" */ './a');
  return a;
}
const two = require('./c', './d');
";
        let imports: Vec<(String, Vec<String>)> = parse(source, "src/app.ts")
            .into_iter()
            .filter(|n| n.kind == NodeKind::Import)
            .map(|n| (n.name, n.references))
            .collect();
        let owned = |refs: &[&str]| refs.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        assert_eq!(
            imports,
            vec![
                ("./b".to_string(), owned(&["b", "*as:b"])),
                ("./a".to_string(), owned(&["a"])),
            ]
        );
    }

    /// The chain from a declarator's value down to `require` is walked in a
    /// loop, so a very long member chain can't exhaust the stack.
    #[test]
    fn a_long_member_chain_is_not_a_require_binding() {
        let source = format!("const x = a{};\n", ".b".repeat(20_000));
        let imports = parse(&source, "src/long.ts")
            .into_iter()
            .filter(|n| n.kind == NodeKind::Import)
            .count();
        assert_eq!(imports, 0);
    }

    /// `require`, `import x = require()` and an awaited `import()` bind
    /// names as `import` statements do, in the same format.
    #[test]
    fn require_bindings_are_recorded_as_imports() {
        let source = "\
const { a, b: c, d = 1, e: f = 2, 'g': h, default: dflt, i: { j }, ...rest } = require('./m');
const whole = require(\"./whole\"), other = require('./other').helper;
var same = require('./same').same;
import helper = require('./helper');
const notAModule = require(name);
const called = require('./factory')('arg');
const resolved = require.resolve('./path');
export async function load() {
  const { lazy, x: y } = await import('./lazy.js');
  const ns = await import('./ns');
  return [lazy, y, ns];
}
";
        let imports: Vec<(String, Vec<String>)> = parse(source, "src/app.ts")
            .into_iter()
            .filter(|n| n.kind == NodeKind::Import)
            .map(|n| (n.name, n.references))
            .collect();
        let owned = |refs: &[&str]| refs.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        assert_eq!(
            imports,
            vec![
                // Nested `{ j }` and `...rest` bind objects, not exports.
                (
                    "./m".to_string(),
                    owned(&["a", "alias:c:b", "d", "alias:f:e", "alias:h:g", "dflt"])
                ),
                ("./whole".to_string(), owned(&["whole", "*as:whole"])),
                ("./other".to_string(), owned(&["alias:other:helper"])),
                ("./same".to_string(), owned(&["same"])),
                ("./helper".to_string(), owned(&["helper", "*as:helper"])),
                ("./lazy.js".to_string(), owned(&["lazy", "alias:y:x"])),
                ("./ns".to_string(), owned(&["*as:ns"])),
            ]
        );
    }
}
