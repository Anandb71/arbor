//! Language parsers module.
//!
//! Each supported language has its own submodule that implements
//! the LanguageParser trait. This keeps language-specific quirks
//! isolated and makes it straightforward to add new languages.

mod c;
mod cpp;
mod dart;
mod go;
mod java;
mod python;
mod rust;
mod typescript;

use crate::node::CodeNode;

/// Trait for language-specific parsing logic.
///
/// Each language needs to implement this to handle its unique AST
/// structure and idioms. The trait provides the Tree-sitter language
/// and the extraction logic.
pub trait LanguageParser: Send + Sync {
    /// Returns the Tree-sitter language for this parser.
    fn language(&self) -> tree_sitter::Language;

    /// File extensions this parser handles.
    fn extensions(&self) -> &[&str];

    /// Extracts CodeNodes from a parsed Tree-sitter tree.
    ///
    /// This is where the magic happens. Each language traverses
    /// its AST differently to find functions, classes, etc.
    fn extract_nodes(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &str,
    ) -> Vec<CodeNode>;
}

/// Gets a parser for the given file extension.
///
/// Returns None if we don't support this extension.
pub fn get_parser(extension: &str) -> Option<Box<dyn LanguageParser>> {
    match extension.to_lowercase().as_str() {
        // TypeScript and JavaScript
        "ts" | "tsx" | "mts" | "cts" => Some(Box::new(typescript::TypeScriptParser)),
        "js" | "jsx" | "mjs" | "cjs" => Some(Box::new(typescript::TypeScriptParser)),

        // Rust
        "rs" => Some(Box::new(rust::RustParser)),

        // Python
        "py" | "pyi" => Some(Box::new(python::PythonParser)),

        // Go
        "go" => Some(Box::new(go::GoParser)),

        // Java
        "java" => Some(Box::new(java::JavaParser)),

        // C
        "c" | "h" => Some(Box::new(c::CParser)),

        // C++
        "cpp" | "hpp" | "cc" | "hh" | "cxx" | "hxx" => Some(Box::new(cpp::CppParser)),

        // Dart
        "dart" => Some(Box::new(dart::DartParser)),

        _ => None,
    }
}

/// Lists all supported file extensions.
pub fn supported_extensions() -> &'static [&'static str] {
    &[
        "ts", "tsx", "mts", "cts", // TypeScript
        "js", "jsx", "mjs", "cjs", // JavaScript
        "rs",  // Rust
        "py", "pyi",  // Python
        "go",   // Go
        "java", // Java
        "c", "h", // C
        "cpp", "hpp", "cc", "hh", "cxx", "hxx",  // C++
        "dart", // Dart
    ]
}

/// Checks if a file extension is supported.
pub fn is_supported(extension: &str) -> bool {
    get_parser(extension).is_some()
}
