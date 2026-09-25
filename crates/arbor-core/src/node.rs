//! Code node representation.
//!
//! A CodeNode is our abstraction over raw AST nodes. It captures
//! the semantically meaningful parts of code: what it is, where it lives,
//! and enough metadata to be useful for graph construction.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// The kind of code entity this node represents.
///
/// We intentionally keep this list focused on the entities that matter
/// for understanding code structure. Helper nodes like expressions
/// or statements are filtered out during extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A standalone function (not attached to a class).
    Function,
    /// A method inside a class or impl block.
    Method,
    /// A class definition.
    Class,
    /// An interface, protocol, or trait.
    Interface,
    /// A struct (Rust, Go).
    Struct,
    /// An enum definition.
    Enum,
    /// A module-level variable.
    Variable,
    /// A constant or static value.
    Constant,
    /// A type alias.
    TypeAlias,
    /// The file/module itself as a container.
    Module,
    /// An import statement.
    Import,
    /// An export declaration.
    Export,
    /// A constructor (Java, TypeScript class constructors).
    Constructor,
    /// A class field.
    Field,
    /// A document section or heading (for Markdown knowledge graphs in Lattice).
    Section,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::TypeAlias => "type_alias",
            Self::Module => "module",
            Self::Import => "import",
            Self::Export => "export",
            Self::Constructor => "constructor",
            Self::Field => "field",
            Self::Section => "section",
        };
        write!(f, "{}", s)
    }
}

/// Prefix on [`CodeNode::references`] for a superclass name.
///
/// These are not call targets. The graph builder turns them into `extends`
/// edges and does not feed them to PageRank.
pub const EXTENDS_REF_PREFIX: &str = "extends:";

/// Prefix on [`CodeNode::references`] for an implemented interface or trait.
pub const IMPLEMENTS_REF_PREFIX: &str = "implements:";

/// A type relationship stored on a node until the graph builder resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRelationKind {
    /// `class Middle(Base)` — subclass depends on the base.
    Extends,
    /// `class Foo implements Bar` — implementor depends on the interface.
    Implements,
}

/// Splits an `extends:` / `implements:` reference into its kind and type name.
///
/// Returns `None` for ordinary call references.
pub fn type_relation_ref(reference: &str) -> Option<(TypeRelationKind, &str)> {
    let reference = reference.trim();
    if let Some(name) = reference.strip_prefix(EXTENDS_REF_PREFIX) {
        let name = name.trim();
        if !name.is_empty() {
            return Some((TypeRelationKind::Extends, name));
        }
    }
    if let Some(name) = reference.strip_prefix(IMPLEMENTS_REF_PREFIX) {
        let name = name.trim();
        if !name.is_empty() {
            return Some((TypeRelationKind::Implements, name));
        }
    }
    None
}

/// Strips pointers, references, and generic arguments from a type name.
///
/// `*Base`, `Foo<T>`, and `pkg.Base` become `Base`, `Foo`, and `pkg.Base`.
/// Keywords and anything that is not a type path become empty.
pub fn clean_type_name(raw: &str) -> String {
    let mut raw = raw.trim();
    loop {
        if let Some(rest) = raw.strip_prefix('*').or_else(|| raw.strip_prefix('&')) {
            raw = rest.trim_start();
            continue;
        }
        if let Some(rest) = raw.strip_prefix("mut ") {
            raw = rest.trim_start();
            continue;
        }
        break;
    }
    let raw = raw.split(['<', '[']).next().unwrap_or(raw).trim();
    let raw = raw.trim_end_matches('?').trim();
    if raw.is_empty() || is_type_keyword(raw) || !is_type_path(raw) {
        return String::new();
    }
    raw.to_string()
}

fn is_type_path(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    name.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':')
}

fn is_type_keyword(name: &str) -> bool {
    matches!(
        name,
        "public"
            | "private"
            | "protected"
            | "virtual"
            | "override"
            | "abstract"
            | "static"
            | "final"
            | "sealed"
            | "partial"
            | "readonly"
            | "const"
            | "class"
            | "struct"
            | "interface"
            | "enum"
            | "extends"
            | "implements"
            | "with"
            | "where"
            | "new"
            | "base"
            | "this"
            | "super"
            | "mut"
            | "pub"
            | "crate"
            | "object"
    )
}

fn push_type_relation(references: &mut Vec<String>, kind: TypeRelationKind, name: &str) {
    let name = clean_type_name(name);
    if name.is_empty() {
        return;
    }
    let prefix = match kind {
        TypeRelationKind::Extends => EXTENDS_REF_PREFIX,
        TypeRelationKind::Implements => IMPLEMENTS_REF_PREFIX,
    };
    let encoded = format!("{prefix}{name}");
    if !references.iter().any(|existing| existing == &encoded) {
        references.push(encoded);
    }
}

/// Visibility of a code entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Public,
    Protected,
    /// Rust's pub(crate) or similar restricted visibility.
    Internal,
}

/// A code entity extracted from source.
///
/// This is the core data type that flows through Arbor. It's designed
/// to be language-agnostic while still capturing the structure we need.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeNode {
    /// Unique identifier, derived from file path + qualified name + kind.
    pub id: String,

    /// The simple name (e.g., "validate_user").
    pub name: String,

    /// Fully qualified name including parent scope (e.g., "UserService.validate_user").
    pub qualified_name: String,

    /// What kind of entity this is.
    pub kind: NodeKind,

    /// Path to the source file, relative to project root.
    pub file: String,

    /// Starting line (1-indexed, like editors show).
    pub line_start: u32,

    /// Ending line (inclusive).
    pub line_end: u32,

    /// Column of the name identifier.
    pub column: u32,

    /// Function/method signature if applicable.
    pub signature: Option<String>,

    /// Visibility modifier.
    pub visibility: Visibility,

    /// Whether this is async.
    pub is_async: bool,

    /// Whether this is static/class-level.
    pub is_static: bool,

    /// Whether this is exported (TS/ES modules).
    pub is_exported: bool,

    /// Docstring or leading comment.
    pub docstring: Option<String>,

    /// Byte offset range in source for incremental updates.
    pub byte_start: u32,
    pub byte_end: u32,

    /// Entities this node references (call targets, type refs, etc).
    /// These are names, not IDs - resolution happens in the graph crate.
    pub references: Vec<String>,
}

impl CodeNode {
    /// Creates a deterministic ID for this node.
    ///
    /// The ID is a hash of (file, qualified_name, kind) so the same
    /// entity always gets the same ID across parses.
    pub fn compute_id(file: &str, qualified_name: &str, kind: NodeKind) -> String {
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        file.hash(&mut hasher);
        qualified_name.hash(&mut hasher);
        kind.hash(&mut hasher);

        format!("{:016x}", hasher.finish())
    }

    /// Creates a new node and automatically computes its ID.
    pub fn new(
        name: impl Into<String>,
        qualified_name: impl Into<String>,
        kind: NodeKind,
        file: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let qualified_name = qualified_name.into();
        let file = file.into();
        let id = Self::compute_id(&file, &qualified_name, kind);

        Self {
            id,
            name,
            qualified_name,
            kind,
            file,
            line_start: 0,
            line_end: 0,
            column: 0,
            signature: None,
            visibility: Visibility::default(),
            is_async: false,
            is_static: false,
            is_exported: false,
            docstring: None,
            byte_start: 0,
            byte_end: 0,
            references: Vec::new(),
        }
    }

    /// Builder pattern: set line range.
    pub fn with_lines(mut self, start: u32, end: u32) -> Self {
        self.line_start = start;
        self.line_end = end;
        self
    }

    /// Builder pattern: set byte range.
    pub fn with_bytes(mut self, start: u32, end: u32) -> Self {
        self.byte_start = start;
        self.byte_end = end;
        self
    }

    /// Builder pattern: set column.
    pub fn with_column(mut self, column: u32) -> Self {
        self.column = column;
        self
    }

    /// Builder pattern: set signature.
    pub fn with_signature(mut self, sig: impl Into<String>) -> Self {
        self.signature = Some(sig.into());
        self
    }

    /// Builder pattern: set visibility.
    pub fn with_visibility(mut self, vis: Visibility) -> Self {
        self.visibility = vis;
        self
    }

    /// Builder pattern: mark as async.
    pub fn as_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Builder pattern: mark as static.
    pub fn as_static(mut self) -> Self {
        self.is_static = true;
        self
    }

    /// Builder pattern: mark as exported.
    pub fn as_exported(mut self) -> Self {
        self.is_exported = true;
        self
    }

    /// Builder pattern: add references.
    pub fn with_references(mut self, refs: Vec<String>) -> Self {
        self.references = refs;
        self
    }

    /// Records a superclass. Appends to [`references`](Self::references); it
    /// does not replace call references already stored there.
    pub fn add_type_relation(&mut self, kind: TypeRelationKind, name: &str) {
        push_type_relation(&mut self.references, kind, name);
    }

    /// Records superclasses this type extends.
    pub fn with_extends<I, S>(mut self, bases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in bases {
            self.add_type_relation(TypeRelationKind::Extends, name.as_ref());
        }
        self
    }

    /// Records interfaces or traits this type implements.
    pub fn with_implements<I, S>(mut self, interfaces: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in interfaces {
            self.add_type_relation(TypeRelationKind::Implements, name.as_ref());
        }
        self
    }
}

impl PartialEq for CodeNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for CodeNode {}

impl Hash for CodeNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_node_kind_display_all_variants() {
        // Exhaustive check of Display for every NodeKind variant
        assert_eq!(NodeKind::Function.to_string(), "function");
        assert_eq!(NodeKind::Method.to_string(), "method");
        assert_eq!(NodeKind::Class.to_string(), "class");
        assert_eq!(NodeKind::Struct.to_string(), "struct");
        assert_eq!(NodeKind::Interface.to_string(), "interface");
        assert_eq!(NodeKind::Enum.to_string(), "enum");
        assert_eq!(NodeKind::Module.to_string(), "module");
        assert_eq!(NodeKind::Field.to_string(), "field");
        assert_eq!(NodeKind::Constant.to_string(), "constant");
        assert_eq!(NodeKind::Constructor.to_string(), "constructor");
        assert_eq!(NodeKind::Import.to_string(), "import");
        assert_eq!(NodeKind::Export.to_string(), "export");
        assert_eq!(NodeKind::TypeAlias.to_string(), "type_alias");
        assert_eq!(NodeKind::Variable.to_string(), "variable");
    }

    #[test]
    fn test_visibility_default_is_private() {
        let vis = Visibility::default();
        assert!(matches!(vis, Visibility::Private));
    }

    #[test]
    fn test_builder_pattern_chain() {
        // Verify that all builder methods compose correctly
        let node = CodeNode::new("foo", "pkg.foo", NodeKind::Function, "main.rs")
            .with_lines(10, 20)
            .with_bytes(100, 300)
            .with_column(4)
            .with_signature("fn foo(x: i32) -> bool")
            .with_visibility(Visibility::Public)
            .as_async()
            .as_static()
            .as_exported()
            .with_references(vec!["bar".to_string(), "baz".to_string()]);

        assert_eq!(node.name, "foo");
        assert_eq!(node.qualified_name, "pkg.foo");
        assert_eq!(node.file, "main.rs");
        assert_eq!(node.line_start, 10);
        assert_eq!(node.line_end, 20);
        assert_eq!(node.byte_start, 100);
        assert_eq!(node.byte_end, 300);
        assert_eq!(node.column, 4);
        assert_eq!(node.signature.as_deref(), Some("fn foo(x: i32) -> bool"));
        assert!(matches!(node.visibility, Visibility::Public));
        assert!(node.is_async);
        assert!(node.is_static);
        assert!(node.is_exported);
        assert_eq!(node.references.len(), 2);
    }

    #[test]
    fn test_code_node_equality_by_id() {
        // PartialEq compares by ID only, not by other fields
        let node1 = CodeNode::new("foo", "foo", NodeKind::Function, "a.rs");
        let node2 = CodeNode::new("foo", "foo", NodeKind::Function, "a.rs");
        // Same inputs → same ID → equal
        assert_eq!(node1, node2);

        // Different kind → different ID → not equal
        let node3 = CodeNode::new("foo", "foo", NodeKind::Method, "a.rs");
        assert_ne!(node1, node3);
    }

    #[test]
    fn test_code_node_hash_consistency() {
        // Same node should hash identically, and be usable in HashSet
        let node1 = CodeNode::new("foo", "foo", NodeKind::Function, "main.rs");
        let node2 = CodeNode::new("foo", "foo", NodeKind::Function, "main.rs");

        let mut set = HashSet::new();
        set.insert(node1.clone());
        assert!(set.contains(&node2));
        // Inserting duplicate should not increase size
        set.insert(node2);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_compute_id_deterministic() {
        // Same inputs must always produce the same ID
        let id1 = CodeNode::compute_id("test.rs", "main", NodeKind::Function);
        let id2 = CodeNode::compute_id("test.rs", "main", NodeKind::Function);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_compute_id_different_kinds_differ() {
        // A function and a struct with the same name should have different IDs
        let id_fn = CodeNode::compute_id("test.rs", "Foo", NodeKind::Function);
        let id_struct = CodeNode::compute_id("test.rs", "Foo", NodeKind::Struct);
        assert_ne!(id_fn, id_struct);
    }

    #[test]
    fn test_compute_id_different_files_differ() {
        let id1 = CodeNode::compute_id("a.rs", "main", NodeKind::Function);
        let id2 = CodeNode::compute_id("b.rs", "main", NodeKind::Function);
        assert_ne!(id1, id2);
    }

    #[test]
    fn type_relation_refs_round_trip() {
        assert_eq!(
            type_relation_ref("extends:Base"),
            Some((TypeRelationKind::Extends, "Base"))
        );
        assert_eq!(
            type_relation_ref("implements:Reader"),
            Some((TypeRelationKind::Implements, "Reader"))
        );
        assert!(type_relation_ref("helper").is_none());
        assert!(type_relation_ref("extends:").is_none());
    }

    #[test]
    fn clean_type_name_strips_generics_and_pointers() {
        assert_eq!(clean_type_name("*Base"), "Base");
        assert_eq!(clean_type_name("Foo<T>"), "Foo");
        assert_eq!(clean_type_name("pkg.Base"), "pkg.Base");
        assert_eq!(clean_type_name("std::fmt::Display"), "std::fmt::Display");
        assert_eq!(clean_type_name("object"), "");
        assert_eq!(clean_type_name("public"), "");
    }

    #[test]
    fn with_extends_appends_without_dropping_calls() {
        let node = CodeNode::new("Middle", "Middle", NodeKind::Class, "a.py")
            .with_references(vec!["helper".to_string()])
            .with_extends(["Base"])
            .with_implements(["Proto"]);
        assert!(node.references.iter().any(|r| r == "helper"));
        assert!(node.references.iter().any(|r| r == "extends:Base"));
        assert!(node.references.iter().any(|r| r == "implements:Proto"));
    }

    #[test]
    fn test_node_default_values() {
        // Verify sensible defaults for a freshly created node
        let node = CodeNode::new("f", "f", NodeKind::Function, "x.rs");
        assert_eq!(node.line_start, 0);
        assert_eq!(node.line_end, 0);
        assert_eq!(node.byte_start, 0);
        assert_eq!(node.byte_end, 0);
        assert_eq!(node.column, 0);
        assert!(node.signature.is_none());
        assert!(!node.is_async);
        assert!(!node.is_static);
        assert!(!node.is_exported);
        assert!(node.references.is_empty());
        assert!(matches!(node.visibility, Visibility::Private));
    }
}
