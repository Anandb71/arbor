//! Parser module - the heart of code analysis.
//!
//! This module wraps Tree-sitter and provides a clean API for parsing
//! source files into CodeNodes. Language detection is automatic based
//! on file extension.

use crate::error::{ParseError, Result};
use crate::fallback_parser;
use crate::languages::{get_parser, LanguageParser};
use crate::node::CodeNode;
use crate::usages::FileUsages;
use std::fs;
use std::path::Path;

/// Parses a source file and extracts all code nodes.
///
/// This is the main entry point for parsing. It handles:
/// - Reading the file from disk
/// - Detecting the language from the extension
/// - Parsing with Tree-sitter
/// - Extracting meaningful code entities
///
/// # Example
///
/// ```no_run
/// use arbor_core::parse_file;
/// use std::path::Path;
///
/// let nodes = parse_file(Path::new("src/lib.rs")).unwrap();
/// println!("Found {} nodes", nodes.len());
/// ```
pub fn parse_file(path: &Path) -> Result<Vec<CodeNode>> {
    // Read the source file
    let source = fs::read_to_string(path).map_err(|e| ParseError::io(path, e))?;

    if source.is_empty() {
        // Empty __init__.py files are valid Python module indicators
        if path
            .file_name()
            .map(|n| n == "__init__.py")
            .unwrap_or(false)
        {
            return Ok(vec![]); // Return empty nodes, not an error
        }
        return Err(ParseError::EmptyFile(path.to_path_buf()));
    }

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    // Get the appropriate parser for this file type (tree-sitter path first)
    let parser = detect_language(path);

    if parser.is_none() {
        if fallback_parser::is_fallback_supported_extension(extension) {
            let file_path = path.to_string_lossy().to_string();
            return Ok(fallback_parser::parse_fallback_source(
                &source, &file_path, extension,
            ));
        }
        return Err(ParseError::UnsupportedLanguage(path.to_path_buf()));
    }

    let parser = parser.unwrap();

    // Use the file path as a string for node IDs
    let file_path = path.to_string_lossy().to_string();

    parse_source(&source, &file_path, parser.as_ref())
}

/// Parses source code directly (useful for testing or in-memory content).
///
/// You need to provide a language parser explicitly since there's no
/// file extension to detect from.
pub fn parse_source(
    source: &str,
    file_path: &str,
    lang_parser: &dyn LanguageParser,
) -> Result<Vec<CodeNode>> {
    let tree = parse_tree(source, lang_parser)?;

    // Extract nodes using the language-specific extractor
    let nodes = lang_parser.extract_nodes(&tree, source, file_path);

    Ok(nodes)
}

/// Builds the tree-sitter tree every extractor in this module reads.
fn parse_tree(source: &str, lang_parser: &dyn LanguageParser) -> Result<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang_parser.language())
        .map_err(|e| ParseError::ParserError(format!("Failed to set language: {}", e)))?;
    parser
        .parse(source, None)
        .ok_or_else(|| ParseError::ParserError("Tree-sitter returned no tree".into()))
}

/// Parses source once and returns both the graph nodes and the SDK usages
/// (see [`crate::usages`]). The nodes are exactly what [`parse_source`]
/// returns; usages are `None` when the language has no usage extractor.
pub fn parse_source_with_usages(
    source: &str,
    file_path: &str,
    lang_parser: &dyn LanguageParser,
) -> Result<(Vec<CodeNode>, Option<FileUsages>)> {
    let tree = parse_tree(source, lang_parser)?;
    let nodes = lang_parser.extract_nodes(&tree, source, file_path);
    let usages = lang_parser.extract_usages(&tree, source, file_path);
    Ok((nodes, usages))
}

/// SDK usages for in-memory source. `Ok(None)` when the language has no
/// usage extractor (today: every language except Python).
pub fn parse_source_usages(
    source: &str,
    file_path: &str,
    lang_parser: &dyn LanguageParser,
) -> Result<Option<FileUsages>> {
    let tree = parse_tree(source, lang_parser)?;
    Ok(lang_parser.extract_usages(&tree, source, file_path))
}

/// SDK usages for a file on disk, with the path as written used in records.
/// `Ok(None)` when the file's extension has no usage extractor (see
/// [`crate::usages::is_usage_extension`]); the file is then not read.
pub fn parse_file_usages(path: &Path) -> Result<Option<FileUsages>> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if !crate::usages::is_usage_extension(extension) {
        return Ok(None);
    }
    let Some(parser) = detect_language(path) else {
        return Ok(None);
    };
    let source = fs::read_to_string(path).map_err(|e| ParseError::io(path, e))?;
    parse_source_usages(&source, &path.to_string_lossy(), parser.as_ref())
}

/// Detects the programming language from a file path.
///
/// Returns None if we don't support the file's extension.
pub fn detect_language(path: &Path) -> Option<Box<dyn LanguageParser>> {
    let extension = path.extension()?.to_str()?;
    get_parser(extension)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeKind;

    #[test]
    fn test_detect_language() {
        assert!(detect_language(Path::new("foo.rs")).is_some());
        assert!(detect_language(Path::new("bar.ts")).is_some());
        assert!(detect_language(Path::new("baz.py")).is_some());
        assert!(detect_language(Path::new("unknown.xyz")).is_none());
    }

    #[test]
    fn test_parse_fallback_language_source() {
        let source = r#"
            fun calculateTax(price: Double): Double = price * 0.18
            class TaxService
        "#;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("billing.kt");
        std::fs::write(&path, source).unwrap();

        let nodes = parse_file(&path).unwrap();
        assert!(nodes.iter().any(|n| n.name == "calculateTax"));
        assert!(nodes.iter().any(|n| n.name == "TaxService"));
    }

    #[test]
    fn test_parse_rust_source() {
        let source = r#"
            fn hello_world() {
                println!("Hello!");
            }

            pub struct User {
                name: String,
            }
        "#;

        let parser = get_parser("rs").unwrap();
        let nodes = parse_source(source, "test.rs", parser.as_ref()).unwrap();

        // Should find at least the function and struct
        assert!(nodes
            .iter()
            .any(|n| n.name == "hello_world" && n.kind == NodeKind::Function));
        assert!(nodes
            .iter()
            .any(|n| n.name == "User" && n.kind == NodeKind::Struct));
    }

    #[test]
    fn usages_share_the_parse_and_leave_graph_nodes_unchanged() {
        let source = "from openai import OpenAI\nclient = OpenAI()\n\ndef ask():\n    return client.chat.completions.create(model=\"gpt-4\", messages=[])\n";
        let parser = get_parser("py").unwrap();
        let plain = parse_source(source, "ask.py", parser.as_ref()).unwrap();
        let (nodes, usages) = parse_source_with_usages(source, "ask.py", parser.as_ref()).unwrap();
        assert_eq!(
            serde_json::to_value(&nodes).unwrap(),
            serde_json::to_value(&plain).unwrap(),
            "usage extraction must not change graph nodes"
        );
        let usages = usages.expect("python has a usage extractor");
        let call = &usages.calls[0];
        assert_eq!(call.symbol.as_deref(), Some("ask"));
        // The enclosing symbol is the graph's own name for that function.
        assert!(nodes.iter().any(|n| n.qualified_name == "ask"));

        let ts = get_parser("ts").unwrap();
        assert!(parse_source_usages("const x = 1;", "x.ts", ts.as_ref())
            .unwrap()
            .is_none());
    }

    #[test]
    fn parse_file_usages_reads_python_sources_only() {
        let dir = tempfile::tempdir().unwrap();
        let py = dir.path().join("llm.py");
        std::fs::write(&py, "import anthropic\nanthropic.Anthropic().messages.create(model=\"claude-2.1\", max_tokens=1, messages=[])\n").unwrap();
        let found = parse_file_usages(&py).unwrap().unwrap();
        assert_eq!(found.calls.len(), 1);
        assert_eq!(found.file, py.to_string_lossy());

        let stub = dir.path().join("llm.pyi");
        std::fs::write(&stub, "import anthropic\n").unwrap();
        assert!(parse_file_usages(&stub).unwrap().is_none());
        // Not read at all: the file does not exist.
        assert!(parse_file_usages(&dir.path().join("main.rs"))
            .unwrap()
            .is_none());
        let empty = dir.path().join("__init__.py");
        std::fs::write(&empty, "").unwrap();
        assert!(parse_file_usages(&empty).unwrap().unwrap().is_empty());
    }

    #[test]
    fn test_parse_typescript_source() {
        let source = r#"
            export function greet(name: string): string {
                return `Hello, ${name}!`;
            }

            export class UserService {
                validate() {}
            }
        "#;

        let parser = get_parser("ts").unwrap();
        let nodes = parse_source(source, "test.ts", parser.as_ref()).unwrap();

        assert!(nodes
            .iter()
            .any(|n| n.name == "greet" && n.kind == NodeKind::Function));
        assert!(nodes
            .iter()
            .any(|n| n.name == "UserService" && n.kind == NodeKind::Class));
    }
}
