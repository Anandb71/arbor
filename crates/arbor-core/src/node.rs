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
        };
        write!(f, "{}", s)
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
