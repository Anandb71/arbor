//! Language parsers module.
//!
//! Each supported language has its own submodule that implements
//! the LanguageParser trait. This keeps language-specific quirks
//! isolated and makes it straightforward to add new languages.

mod c;
mod cpp;
mod csharp;
mod dart;
mod go;
mod heritage;
mod java;
mod python;
mod rust;
mod typescript;

use crate::fallback_parser::is_fallback_supported_extension;
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

        // C#
        "cs" => Some(Box::new(csharp::CSharpParser)),

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
        "cs",   // C#
        "dart", // Dart
        "kt", "kts",   // Kotlin (fallback parser)
        "swift", // Swift (fallback parser)
        "rb",    // Ruby (fallback parser)
        "php", "phtml", // PHP (fallback parser)
        "sh", "bash", "zsh", // Shell (fallback parser)
    ]
}

/// Lists all supported language families (for API metadata).
pub fn supported_language_names() -> &'static [&'static str] {
    &[
        "typescript",
        "javascript",
        "rust",
        "python",
        "go",
        "java",
        "c",
        "cpp",
        "csharp",
        "dart",
        "kotlin",
        "swift",
        "ruby",
        "php",
        "shell",
    ]
}

/// Checks if a file extension is supported.
pub fn is_supported(extension: &str) -> bool {
    get_parser(extension).is_some() || is_fallback_supported_extension(extension)
}

#[cfg(test)]
mod inheritance_emission_tests {
    use super::get_parser;

    fn references(ext: &str, source: &str, name: &str) -> Vec<String> {
        let parser = get_parser(ext).unwrap_or_else(|| panic!("no parser for {ext}"));
        let mut ts = tree_sitter::Parser::new();
        ts.set_language(&parser.language()).unwrap();
        let tree = ts.parse(source, None).unwrap();
        let path = format!("Fixture.{ext}");
        let nodes = parser.extract_nodes(&tree, source, &path);
        nodes
            .into_iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("{ext}: missing {name}"))
            .references
    }

    #[test]
    fn each_language_records_its_bases() {
        let cases = [
            (
                "ts",
                "class Middle extends Base implements Reader { run() { return super.run(); } }",
                "Middle",
                vec!["extends:Base", "implements:Reader"],
            ),
            (
                "java",
                "class Middle extends Base implements Runnable { public void run() {} }",
                "Middle",
                vec!["extends:Base", "implements:Runnable"],
            ),
            (
                "cs",
                "class Middle : Base, IReader { public void Run() {} }",
                "Middle",
                vec!["extends:Base", "extends:IReader"],
            ),
            (
                "cpp",
                "class Middle : public Base, public Reader { void run(); };",
                "Middle",
                vec!["extends:Base", "extends:Reader"],
            ),
            (
                "dart",
                "class Middle extends Base implements Reader { void run() {} }",
                "Middle",
                vec!["extends:Base", "implements:Reader"],
            ),
            (
                "go",
                "package p\ntype Middle struct {\n\tBase\n\t*Reader\n\tnamed int\n}\n",
                "Middle",
                vec!["extends:Base", "extends:Reader"],
            ),
            (
                "rs",
                "struct Middle;\nimpl Reader for Middle {}\ntrait Child: Parent {}",
                "Middle",
                vec!["implements:Reader"],
            ),
        ];

        for (ext, source, name, expected) in cases {
            let mut refs: Vec<String> = references(ext, source, name)
                .into_iter()
                .filter(|reference| {
                    reference.starts_with("extends:") || reference.starts_with("implements:")
                })
                .collect();
            refs.sort();
            let mut expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            expected.sort();
            assert_eq!(refs, expected, "{ext} {name}");
        }

        let rust_refs = references(
            "rs",
            "struct Middle;\nimpl Reader for Middle {}\ntrait Child: Parent {}",
            "Child",
        );
        assert!(
            rust_refs
                .iter()
                .any(|reference| reference == "extends:Parent"),
            "trait supertrait missing, got {rust_refs:?}"
        );

        let go_refs = references(
            "go",
            "package p\ntype Middle struct {\n\tBase\n\t*Reader\n\tnamed int\n}\n",
            "Middle",
        );
        assert!(
            go_refs
                .iter()
                .all(|reference| !reference.contains("int") && !reference.contains("named")),
            "named fields are not parents, got {go_refs:?}"
        );
    }
}
