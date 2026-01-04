//! Error types for the parsing module.
//!
//! We keep errors simple and actionable. Each variant tells you
//! exactly what went wrong and (usually) how to fix it.

use std::path::PathBuf;
use thiserror::Error;

/// Convenience type for functions that can fail during parsing.
pub type Result<T> = std::result::Result<T, ParseError>;

/// Things that can go wrong when parsing source files.
#[derive(Error, Debug)]
pub enum ParseError {
    /// Couldn't read the file from disk.
    #[error("failed to read file '{path}': {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// File extension doesn't map to any supported language.
    #[error("unsupported language for file '{0}'")]
    UnsupportedLanguage(PathBuf),

    /// Tree-sitter failed to parse the source. Usually means
    /// the file has syntax errors or the parser hit an edge case.
    #[error("parser error: {0}")]
    ParserError(String),

    /// Tree-sitter query compilation failed. Usually means
    /// the query pattern is invalid for the language grammar.
    #[error("query error: {0}")]
    QueryError(String),

    /// The file exists but is empty. Not really an error,
    /// but we surface it so callers can handle it gracefully.
    #[error("file is empty: '{0}'")]
    EmptyFile(PathBuf),
}

impl ParseError {
    /// Creates an IO error with the path for context.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::IoError {
            path: path.into(),
            source,
        }
    }
}
