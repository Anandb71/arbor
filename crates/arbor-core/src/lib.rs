//! Arbor Core - AST parsing and code analysis
//!
//! This crate provides the foundational parsing capabilities for Arbor.
//! It uses Tree-sitter to parse source files into ASTs and extract
//! meaningful code entities like functions, classes, and their relationships.
//!
//! # Example
//!
//! ```no_run
//! use arbor_core::{parse_file, CodeNode};
//! use std::path::Path;
//!
//! let nodes = parse_file(Path::new("src/main.rs")).unwrap();
//! for node in nodes {
//!     println!("{}: {} (line {})", node.kind, node.name, node.line_start);
//! }
//! ```

pub mod error;
pub mod languages;
pub mod node;
pub mod parser;
pub mod parser_v2;

pub use error::{ParseError, Result};
pub use languages::LanguageParser;
pub use node::{CodeNode, NodeKind, Visibility};
pub use parser::{detect_language, parse_file, parse_source};
pub use parser_v2::{ArborParser, ParseResult, RelationType, SymbolRelation};
