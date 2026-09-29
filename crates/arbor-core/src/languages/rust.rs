//! Rust language parser implementation.
//!
//! Handles .rs files and extracts functions, structs, enums, traits,
//! and impl blocks.

use crate::languages::LanguageParser;
use crate::node::{clean_type_name, CodeNode, NodeKind, TypeRelationKind, Visibility};
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
        attach_rust_inheritance(&root, source, &mut nodes);

        nodes
    }
}

/// `impl Trait for Type` and `trait Child: Parent` are not class clauses.
/// Attach them onto the type or trait node extracted above.
fn attach_rust_inheritance(root: &Node, source: &str, nodes: &mut [CodeNode]) {
    walk_rust_inheritance(root, source, nodes);
}

fn walk_rust_inheritance(node: &Node, source: &str, nodes: &mut [CodeNode]) {
    match node.kind() {
        "impl_item" => {
            if let (Some(trait_node), Some(type_node)) = (
                node.child_by_field_name("trait"),
                node.child_by_field_name("type"),
            ) {
                let trait_name = clean_type_name(&node_text(&trait_node, source));
                let type_name = clean_type_name(&node_text(&type_node, source));
                if !trait_name.is_empty() && !type_name.is_empty() {
                    if let Some(target) = nodes.iter_mut().find(|n| {
                        is_rust_type(n.kind)
                            && (n.name == type_name || clean_type_name(&n.name) == type_name)
                    }) {
                        target.add_type_relation(TypeRelationKind::Implements, &trait_name);
                    }
                }
            }
        }
        "trait_item" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let trait_name = node_text(&name_node, source);
            if let Some(bounds) = node.child_by_field_name("bounds") {
                let supers = super::heritage::type_names(&bounds, source);
                if let Some(target) = nodes
                    .iter_mut()
                    .find(|n| n.kind == NodeKind::Interface && n.name == trait_name)
                {
                    for parent in supers {
                        target.add_type_relation(TypeRelationKind::Extends, &parent);
                    }
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_rust_inheritance(&child, source, nodes);
        }
    }
}

fn is_rust_type(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Struct | NodeKind::Enum | NodeKind::Interface | NodeKind::TypeAlias
    )
}

fn node_text(node: &Node, source: &str) -> String {
    let range = node.byte_range();
    if range.end <= source.len() {
        source[range].to_string()
    } else {
        String::new()
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
                                extract_from_node(
                                    &child,
                                    source,
                                    file_path,
                                    nodes,
                                    Some(&trait_name),
                                );
                            }
                        }
                    }
                    return;
                }
            }

            // Impl blocks
            "impl_item" => {
                // Methods are keyed by the bare type (`impl<T> Foo<T>` → `Foo`)
                // so `Foo::new()` and `self.method()` can find them.
                let impl_target = get_impl_target(node, source).map(|target| {
                    let clean = clean_type_name(&target);
                    if clean.is_empty() {
                        target
                    } else {
                        clean
                    }
                });
                if let Some(body) = find_child_by_kind(node, "declaration_list") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_from_node(
                                &child,
                                source,
                                file_path,
                                nodes,
                                impl_target.as_deref(),
                            );
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

/// Method names so common on std types (`Option`, `Result`, `Vec`, `String`,
/// iterators, maps) that a call on a receiver of unknown type says nothing
/// about which definition is meant. Linking `.len()` to the one `len` a project
/// defines would invent callers, so these are not emitted as unknown-receiver
/// calls. A call on `self` is still emitted: its type is known.
const STD_METHODS: &[&str] = &[
    "all",
    "and",
    "and_then",
    "any",
    "as_bytes",
    "as_mut",
    "as_ref",
    "as_slice",
    "as_str",
    "borrow",
    "borrow_mut",
    "chain",
    "clone",
    "cloned",
    "cmp",
    "collect",
    "contains",
    "contains_key",
    "copied",
    "count",
    "dedup",
    "default",
    "ends_with",
    "entry",
    "enumerate",
    "eq",
    "err",
    "expect",
    "extend",
    "filter",
    "filter_map",
    "find",
    "first",
    "flat_map",
    "flatten",
    "fmt",
    "fold",
    "for_each",
    "from",
    "hash",
    "into",
    "into_iter",
    "is_empty",
    "is_err",
    "is_none",
    "is_ok",
    "is_some",
    "iter",
    "iter_mut",
    "join",
    "keys",
    "last",
    "len",
    "lock",
    "map",
    "map_err",
    "max",
    "min",
    "next",
    "ok",
    "ok_or",
    "ok_or_else",
    "or",
    "or_default",
    "or_else",
    "or_insert",
    "or_insert_with",
    "parse",
    "partial_cmp",
    "pop",
    "push",
    "push_str",
    "retain",
    "rev",
    "skip",
    "sort",
    "sort_by",
    "sort_by_key",
    "sort_unstable",
    "split",
    "starts_with",
    "sum",
    "take",
    "then",
    "to_lowercase",
    "to_owned",
    "to_string",
    "to_uppercase",
    "to_vec",
    "trim",
    "try_from",
    "try_into",
    "unwrap",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
    "values",
    "zip",
];

/// Extracts function call references in the shapes the graph builder
/// resolves: `name`, `module::name`, `Type::name`, `self.name`, and `.name`
/// for a method on a receiver whose type isn't known.
fn extract_call_references(node: &Node, source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    collect_calls(node, source, &mut refs);
    refs.sort();
    refs.dedup();
    refs
}

fn collect_calls(node: &Node, source: &str, refs: &mut Vec<String>) {
    stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
        match node.kind() {
            "call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    refs.extend(call_reference(&function, source));
                }
            }
            // A macro's arguments are unparsed tokens, not expressions, so a
            // call inside `assert!(check(x))` or `format!("{}", f(x))` never
            // appears as a `call_expression`. Scan the tokens instead.
            "macro_invocation" => {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "token_tree" {
                            collect_token_calls(&child, source, refs);
                        }
                    }
                }
                return;
            }
            // A macro body is a template, not code this function runs.
            "macro_definition" => return,
            _ => {}
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                collect_calls(&child, source, refs);
            }
        }
    });
}

/// The reference for the `function` of a `call_expression`, if it names one.
fn call_reference(function: &Node, source: &str) -> Option<String> {
    match function.kind() {
        "identifier" => Some(get_text(function, source)),
        "scoped_identifier" => path_reference(&get_text(function, source)),
        "field_expression" => {
            let field = function.child_by_field_name("field")?;
            // `tuple.0(x)` calls a stored closure; there's no name to resolve.
            if field.kind() != "field_identifier" {
                return None;
            }
            let receiver = function.child_by_field_name("value")?;
            method_reference(receiver.kind() == "self", &get_text(&field, source))
        }
        // `parse::<u32>(s)` is a call to `parse`.
        "generic_function" => call_reference(&function.child_by_field_name("function")?, source),
        // Closures, `(self.handler)(x)`, `f()()`: no stable name.
        _ => None,
    }
}

/// `jobs::enqueue`, `Vec::<u8>::new`, `crate::db::get` → a `::` path with
/// generics removed. Qualified-type syntax (`<T as Trait>::f`) has no stable
/// owner and is skipped.
fn path_reference(text: &str) -> Option<String> {
    let text = text.trim();
    if text.starts_with('<') {
        return None;
    }
    let mut segments: Vec<&str> = Vec::new();
    for raw in text.split("::") {
        // A turbofish segment (`<u8>`) contributes nothing.
        let segment = raw.split('<').next().unwrap_or_default().trim();
        if segment.is_empty() {
            continue;
        }
        if !segment.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        segments.push(segment);
    }
    match segments.as_slice() {
        [] => None,
        [name] => Some((*name).to_string()),
        _ => Some(segments.join("::")),
    }
}

fn method_reference(on_self: bool, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if on_self {
        return Some(format!("self.{name}"));
    }
    if STD_METHODS.contains(&name) {
        return None;
    }
    Some(format!(".{name}"))
}

/// Calls inside a macro's token tree: `name(`, `path::name(`, `.name(` and
/// `self.name(`. Nested groups and nested macros are scanned too.
fn collect_token_calls(tree: &Node, source: &str, refs: &mut Vec<String>) {
    stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
        let tokens: Vec<Node> = (0..tree.child_count())
            .filter_map(|i| tree.child(i))
            .collect();
        for (i, token) in tokens.iter().enumerate() {
            if token.kind() != "token_tree" {
                continue;
            }
            collect_token_calls(token, source, refs);
            let opens_call = token.child(0).is_some_and(|open| open.kind() == "(");
            if !opens_call || i == 0 || tokens[i - 1].kind() != "identifier" {
                continue;
            }
            let mut start = i - 1;
            let mut path = vec![get_text(&tokens[start], source)];
            while start >= 2
                && tokens[start - 1].kind() == "::"
                && matches!(
                    tokens[start - 2].kind(),
                    "identifier" | "self" | "super" | "crate"
                )
            {
                path.insert(0, get_text(&tokens[start - 2], source));
                start -= 2;
            }
            let reference = if path.len() == 1 && start >= 1 && tokens[start - 1].kind() == "." {
                let on_self = start >= 2 && tokens[start - 2].kind() == "self";
                method_reference(on_self, &path[0])
            } else {
                path_reference(&path.join("::"))
            };
            refs.extend(reference);
        }
    });
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

#[cfg(test)]
mod call_tests {
    use super::RustParser;
    use crate::languages::LanguageParser;
    use crate::node::CodeNode;

    fn parse(source: &str) -> Vec<CodeNode> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&RustParser.language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error(), "fixture must parse cleanly");
        RustParser.extract_nodes(&tree, source, "src/billing.rs")
    }

    fn references_of<'a>(nodes: &'a [CodeNode], name: &str) -> &'a [String] {
        &nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no node {name}"))
            .references
    }

    const SOURCE: &str = r#"
struct Provider;
struct Wrapper<T>(T);
impl<T> Wrapper<T> {
    fn make() {}
}
impl Provider {
    fn order_history(&self, store: Store) {
        self.owner_of(1);
        jobs::enqueue(pool, actor);
        crate::app::create_project(1);
        Provider::new();
        let v: Vec<u8> = Vec::with_capacity(3);
        store.fetch_ledger(1).unwrap();
        items.iter().map(|x| x.len()).collect::<Vec<_>>();
        parse::<u32>("1");
        assert!(verify_webhook_signature(&body, sig));
        assert_eq!(self.total(), money::round_cents(2.0));
        println!("{}", format_money(x));
        tokio::spawn(async move { worker::run().await });
        (self.handler)(1);
        <T as Default>::default();
    }
}
"#;

    #[test]
    fn calls_are_normalized_to_resolvable_shapes() {
        let nodes = parse(SOURCE);
        let refs = references_of(&nodes, "order_history");
        for expected in [
            "self.owner_of",
            "jobs::enqueue",
            "crate::app::create_project",
            "Provider::new",
            "Vec::with_capacity",
            ".fetch_ledger",
            "parse",
            "tokio::spawn",
            "worker::run",
        ] {
            assert!(
                refs.iter().any(|r| r == expected),
                "missing {expected} in {refs:?}"
            );
        }
        // Whole chains and generics never leak into a reference.
        assert!(
            refs.iter().all(|r| !r.contains('(') && !r.contains('<')),
            "{refs:?}"
        );
    }

    #[test]
    fn calls_inside_macros_are_found() {
        let nodes = parse(SOURCE);
        let refs = references_of(&nodes, "order_history");
        for expected in [
            "verify_webhook_signature",
            "self.total",
            "money::round_cents",
            "format_money",
        ] {
            assert!(
                refs.iter().any(|r| r == expected),
                "missing {expected} in {refs:?}"
            );
        }
    }

    #[test]
    fn std_methods_on_unknown_receivers_are_not_references() {
        let nodes = parse(SOURCE);
        let refs = references_of(&nodes, "order_history");
        for noise in [".unwrap", ".iter", ".map", ".len", ".collect", "default"] {
            assert!(!refs.iter().any(|r| r == noise), "{noise} in {refs:?}");
        }
    }

    #[test]
    fn generic_impls_key_methods_by_the_bare_type() {
        let nodes = parse(SOURCE);
        let make = nodes.iter().find(|n| n.name == "make").unwrap();
        assert_eq!(make.qualified_name, "Wrapper.make");
        let history = nodes.iter().find(|n| n.name == "order_history").unwrap();
        assert_eq!(history.qualified_name, "Provider.order_history");
    }
}
