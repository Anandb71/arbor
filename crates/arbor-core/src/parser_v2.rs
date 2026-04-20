//! ArborParser - The Eyes of Arbor
//!
//! This module implements high-performance code parsing using Tree-sitter queries.
//! It extracts symbols (functions, classes, interfaces) and their relationships
//! (imports, calls) to build a comprehensive code graph.
//!
//! The parser is designed for incremental updates - calling it on the same file
//! will update existing nodes rather than creating duplicates.

use crate::error::{ParseError, Result};
use crate::fallback_parser;
use crate::node::{CodeNode, NodeKind};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::warn;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Tree};

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// A relationship between two symbols in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRelation {
    /// The source symbol (caller/importer).
    pub from_id: String,
    /// The target symbol name (what is being called/imported).
    pub to_name: String,
    /// The type of relationship.
    pub kind: RelationType,
    /// Line number where the relationship occurs.
    pub line: u32,
}

/// Types of relationships between code symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationType {
    /// Function/method calls another function.
    Calls,
    /// Module imports another module or symbol.
    Imports,
    /// Class extends another class.
    Extends,
    /// Class/type implements an interface.
    Implements,
}

/// Result of parsing a single file.
#[derive(Debug)]
pub struct ParseResult {
    /// Extracted code symbols.
    pub symbols: Vec<CodeNode>,
    /// Relationships between symbols.
    pub relations: Vec<SymbolRelation>,
    /// File path that was parsed.
    pub file_path: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// ArborParser
// ─────────────────────────────────────────────────────────────────────────────

/// High-performance code parser using Tree-sitter queries.
///
/// The parser caches compiled queries for reuse across multiple files,
/// making it efficient for large codebase indexing.
pub struct ArborParser {
    /// Tree-sitter parser instance.
    parser: Parser,
    /// Compiled queries by language.
    queries: HashMap<String, CompiledQueries>,
}

/// Pre-compiled queries for a specific language.
struct CompiledQueries {
    /// Query for extracting symbols (functions, classes, etc.).
    symbols: Query,
    /// Query for extracting imports.
    imports: Query,
    /// Query for extracting function calls.
    calls: Query,
    /// The language for this query set.
    language: Language,
}

impl Default for ArborParser {
    fn default() -> Self {
        match Self::new() {
            Ok(parser) => parser,
            Err(error) => {
                warn!(
                    "ArborParser::new failed during default init; continuing with empty query registry: {}",
                    error
                );
                Self {
                    parser: Parser::new(),
                    queries: HashMap::new(),
                }
            }
        }
    }
}

impl ArborParser {
    /// Creates a new ArborParser with pre-compiled queries.
    ///
    /// Returns an error if any language queries fail to compile.
    pub fn new() -> Result<Self> {
        let parser = Parser::new();
        let mut queries = HashMap::new();

        // Compile TypeScript/JavaScript queries
        for ext in &["ts", "tsx", "js", "jsx"] {
            let compiled = Self::compile_typescript_queries()?;
            queries.insert(ext.to_string(), compiled);
        }

        // Compile Rust queries
        let rs_queries = Self::compile_rust_queries()?;
        queries.insert("rs".to_string(), rs_queries);

        // Compile Python queries
        let py_queries = Self::compile_python_queries()?;
        queries.insert("py".to_string(), py_queries);

        // Compile Go queries
        let go_queries = Self::compile_go_queries()?;
        queries.insert("go".to_string(), go_queries);

        // Compile Java queries
        let java_queries = Self::compile_java_queries()?;
        queries.insert("java".to_string(), java_queries);

        // Compile C queries
        for ext in &["c", "h"] {
            queries.insert(ext.to_string(), Self::compile_c_queries()?);
        }

        // Compile C++ queries
        for ext in &["cpp", "hpp", "cc", "hh", "cxx", "hxx"] {
            queries.insert(ext.to_string(), Self::compile_cpp_queries()?);
        }

        // Dart remains on legacy path (queries incompatible with tree-sitter-dart v0.0.4). Registry helper enables trivial future addition.
        // Compile C# queries
        let csharp_queries = Self::compile_csharp_queries()?;
        queries.insert("cs".to_string(), csharp_queries);

        Ok(Self { parser, queries })
    }

    /// Parses a file and extracts symbols and relationships.
    ///
    /// This is the main entry point for parsing. It returns a ParseResult
    /// containing all symbols and their relationships, ready to be inserted
    /// into an ArborGraph.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the language is unsupported,
    /// or parsing fails. Syntax errors in the source code are handled gracefully -
    /// the parser will still extract what it can.
    pub fn parse_file(&mut self, path: &Path) -> Result<ParseResult> {
        // Read the file
        let source = fs::read_to_string(path).map_err(|e| ParseError::io(path, e))?;

        if source.is_empty() {
            return Err(ParseError::EmptyFile(path.to_path_buf()));
        }

        // Get the extension (normalize to lowercase)
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| ParseError::UnsupportedLanguage(path.to_path_buf()))?;
        let ext = ext.to_ascii_lowercase();

        // Get compiled queries
        let compiled = match self.queries.get(&ext) {
            Some(compiled) => compiled,
            None => {
                if fallback_parser::is_fallback_supported_extension(&ext) {
                    return Ok(ParseResult {
                        symbols: fallback_parser::parse_fallback_source(
                            &source,
                            &path.to_string_lossy(),
                            &ext,
                        ),
                        relations: Vec::new(),
                        file_path: path.to_string_lossy().to_string(),
                    });
                }
                return Err(ParseError::UnsupportedLanguage(path.to_path_buf()));
            }
        };

        // Configure parser for this language
        self.parser
            .set_language(&compiled.language)
            .map_err(|e| ParseError::ParserError(format!("Failed to set language: {}", e)))?;

        // Parse the source
        let Some(tree) = self.parser.parse(&source, None) else {
            warn!(
                "Tree-sitter returned no tree for file '{}'; returning partial empty parse result",
                path.to_string_lossy()
            );
            return Ok(ParseResult {
                symbols: Vec::new(),
                relations: Vec::new(),
                file_path: path.to_string_lossy().to_string(),
            });
        };

        let file_path = path.to_string_lossy().to_string();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        // Extract symbols
        let symbols = self.extract_symbols(&tree, &source, &file_path, file_name, compiled);

        // Extract relationships
        let relations = self.extract_relations(&tree, &source, &file_path, &symbols, compiled);

        Ok(ParseResult {
            symbols,
            relations,
            file_path,
        })
    }

    /// Parses source code directly (for testing or in-memory content).
    pub fn parse_source(
        &mut self,
        source: &str,
        file_path: &str,
        language: &str,
    ) -> Result<ParseResult> {
        if source.is_empty() {
            return Err(ParseError::EmptyFile(file_path.into()));
        }

        // Normalize language/extension to lowercase
        let language = language.to_ascii_lowercase();
        let compiled = match self.queries.get(&language) {
            Some(compiled) => compiled,
            None => {
            if fallback_parser::is_fallback_supported_extension(&language) {
                return Ok(ParseResult {
                    symbols: fallback_parser::parse_fallback_source(source, file_path, &language),
                    relations: Vec::new(),
                    file_path: file_path.to_string(),
                });
            }
            return Err(ParseError::UnsupportedLanguage(file_path.into()));
            }
        };

        self.parser
            .set_language(&compiled.language)
            .map_err(|e| ParseError::ParserError(format!("Failed to set language: {}", e)))?;

        let Some(tree) = self.parser.parse(source, None) else {
            warn!(
                "Tree-sitter returned no tree for in-memory source '{}'; returning partial empty parse result",
                file_path
            );
            return Ok(ParseResult {
                symbols: Vec::new(),
                relations: Vec::new(),
                file_path: file_path.to_string(),
            });
        };

        let file_name = Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let symbols = self.extract_symbols(&tree, source, file_path, file_name, compiled);
        let relations = self.extract_relations(&tree, source, file_path, &symbols, compiled);

        Ok(ParseResult {
            symbols,
            relations,
            file_path: file_path.to_string(),
        })
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Symbol Extraction
    // ─────────────────────────────────────────────────────────────────────────

    fn extract_symbols(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &str,
        file_name: &str,
        compiled: &CompiledQueries,
    ) -> Vec<CodeNode> {
        let mut symbols = Vec::new();
        let mut cursor = QueryCursor::new();
        let symbol_capture_names = compiled.symbols.capture_names();
        let source_bytes = source.as_bytes();

        let matches = cursor.matches(&compiled.symbols, tree.root_node(), source_bytes);

        for match_ in matches {
            // Extract name and type from captures
            let mut name: Option<&str> = None;
            let mut kind: Option<NodeKind> = None;
            let mut node = match_.captures.first().map(|c| c.node);

            for capture in match_.captures {
                let Some(capture_name) = symbol_capture_names.get(capture.index as usize) else {
                    warn!(
                        "Symbol capture index out of bounds (index={} file='{}')",
                        capture.index,
                        file_path
                    );
                    continue;
                };

                let Some(text) = Self::node_text(capture.node, source_bytes, file_path) else {
                    continue;
                };

                let capture_name = *capture_name;
                match capture_name {
                    "name" | "function.name" | "class.name" | "interface.name" | "method.name" => {
                        name = Some(text);
                    }
                    "function" | "function_def" => {
                        kind = Some(NodeKind::Function);
                        node = Some(capture.node);
                    }
                    "class" | "class_def" => {
                        kind = Some(NodeKind::Class);
                        node = Some(capture.node);
                    }
                    "interface" | "interface_def" => {
                        kind = Some(NodeKind::Interface);
                        node = Some(capture.node);
                    }
                    "method" | "method_def" => {
                        kind = Some(NodeKind::Method);
                        node = Some(capture.node);
                    }
                    "struct" | "struct_def" => {
                        kind = Some(NodeKind::Struct);
                        node = Some(capture.node);
                    }
                    "enum" | "enum_def" => {
                        kind = Some(NodeKind::Enum);
                        node = Some(capture.node);
                    }
                    "trait" | "trait_def" => {
                        kind = Some(NodeKind::Interface);
                        node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(kind), Some(node)) = (name, kind, node) {
                // Build fully qualified name: filename:symbol_name
                let qualified_name = format!("{}:{}", file_name, name);

                // Extract signature from the node slice (avoids scanning from start each time)
                let signature = Self::first_line_signature(source, node);

                let mut symbol = CodeNode::new(name, &qualified_name, kind, file_path)
                    .with_lines(
                        node.start_position().row as u32 + 1,
                        node.end_position().row as u32 + 1,
                    )
                    .with_column(node.start_position().column as u32)
                    .with_bytes(node.start_byte() as u32, node.end_byte() as u32);

                if let Some(sig) = signature {
                    symbol = symbol.with_signature(sig.to_owned());
                }

                symbols.push(symbol);
            }
        }

        symbols
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Relationship Extraction
    // ─────────────────────────────────────────────────────────────────────────

    fn extract_relations(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &str,
        symbols: &[CodeNode],
        compiled: &CompiledQueries,
    ) -> Vec<SymbolRelation> {
        let mut relations = Vec::new();
        let mut cursor = QueryCursor::new();

        // Extract imports
        self.extract_imports(tree, source, file_path, &mut cursor, &mut relations, compiled);

        // Extract calls
        self.extract_calls(
            tree,
            source,
            file_path,
            symbols,
            &mut cursor,
            &mut relations,
            compiled,
        );

        relations
    }

    fn extract_imports(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &str,
        cursor: &mut QueryCursor,
        relations: &mut Vec<SymbolRelation>,
        compiled: &CompiledQueries,
    ) {
        let import_capture_names = compiled.imports.capture_names();
        let source_bytes = source.as_bytes();
        let file_id = format!("{}:__file__", file_path);
        let matches = cursor.matches(&compiled.imports, tree.root_node(), source_bytes);

        for match_ in matches {
            let mut module_name: Option<&str> = None;
            let mut line: u32 = 0;

            for capture in match_.captures {
                let Some(capture_name) = import_capture_names.get(capture.index as usize) else {
                    warn!(
                        "Import capture index out of bounds (index={} file='{}')",
                        capture.index,
                        file_path
                    );
                    continue;
                };

                let Some(text) = Self::node_text(capture.node, source_bytes, file_path) else {
                    continue;
                };

                let capture_name = *capture_name;
                match capture_name {
                    "source" | "module" | "import.source" => {
                        // Remove quotes from module name
                        module_name = Some(text.trim_matches(|c| c == '"' || c == '\''));
                        line = capture.node.start_position().row as u32 + 1;
                    }
                    _ => {}
                }
            }

            if let Some(module) = module_name {
                // Create a file-level import relation
                relations.push(SymbolRelation {
                    from_id: file_id.clone(),
                    to_name: module.to_string(),
                    kind: RelationType::Imports,
                    line,
                });
            }
        }
    }

    fn extract_calls(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &str,
        symbols: &[CodeNode],
        cursor: &mut QueryCursor,
        relations: &mut Vec<SymbolRelation>,
        compiled: &CompiledQueries,
    ) {
        let call_capture_names = compiled.calls.capture_names();
        let source_bytes = source.as_bytes();
        let file_id = format!("{}:__file__", file_path);
        let matches = cursor.matches(&compiled.calls, tree.root_node(), source_bytes);

        for match_ in matches {
            let mut callee_name: Option<&str> = None;
            let mut call_line: u32 = 0;

            for capture in match_.captures {
                let Some(capture_name) = call_capture_names.get(capture.index as usize) else {
                    warn!(
                        "Call capture index out of bounds (index={} file='{}')",
                        capture.index,
                        file_path
                    );
                    continue;
                };

                let Some(text) = Self::node_text(capture.node, source_bytes, file_path) else {
                    continue;
                };

                let capture_name = *capture_name;
                match capture_name {
                    "callee" | "function" | "call.function" => {
                        // Handle method calls like obj.method()
                        callee_name = Some(text.rsplit('.').next().unwrap_or(text));
                        call_line = capture.node.start_position().row as u32 + 1;
                    }
                    _ => {}
                }
            }

            if let Some(callee) = callee_name {
                // Find the enclosing function/method
                let caller_id = self
                    .find_enclosing_symbol(call_line, symbols)
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|| file_id.clone());

                relations.push(SymbolRelation {
                    from_id: caller_id,
                    to_name: callee.to_string(),
                    kind: RelationType::Calls,
                    line: call_line,
                });
            }
        }
    }

    fn find_enclosing_symbol<'a>(
        &self,
        line: u32,
        symbols: &'a [CodeNode],
    ) -> Option<&'a CodeNode> {
        symbols
            .iter()
            .filter(|s| s.line_start <= line && s.line_end >= line)
            .min_by_key(|s| s.line_end - s.line_start) // Smallest enclosing
    }

    #[inline]
    fn node_text<'a>(node: Node<'a>, source_bytes: &'a [u8], file_path: &str) -> Option<&'a str> {
        match node.utf8_text(source_bytes) {
            Ok(text) => Some(text),
            Err(error) => {
                warn!(
                    "Skipping invalid UTF-8 capture in file '{}' at row {}: {}",
                    file_path,
                    node.start_position().row,
                    error
                );
                None
            }
        }
    }

    #[inline]
    fn first_line_signature<'a>(source: &'a str, node: Node<'_>) -> Option<&'a str> {
        let tail = source.get(node.start_byte()..)?;
        let signature = tail.lines().next()?.trim();
        if signature.is_empty() {
            None
        } else {
            Some(signature)
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Query Registry Helper
    // Extracts S-expressions into reusable strings. Eliminates duplication across
    // compile functions. Makes adding new languages trivial (just add compile_xxx).
    // Aligns with Accessibility (easy extension) and Affordability (less code).
    // ─────────────────────────────────────────────────────────────────────────

    fn compile_queries(
        language: Language,
        symbols_query: &str,
        imports_query: &str,
        calls_query: &str,
    ) -> Result<CompiledQueries> {
        let symbols = Query::new(&language, symbols_query)
            .map_err(|e| ParseError::QueryError(e.to_string()))?;
        let imports = Query::new(&language, imports_query)
            .map_err(|e| ParseError::QueryError(e.to_string()))?;
        let calls = Query::new(&language, calls_query)
            .map_err(|e| ParseError::QueryError(e.to_string()))?;

        Ok(CompiledQueries {
            symbols,
            imports,
            calls,
            language,
        })
    }

    fn compile_typescript_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_typescript::language_typescript();

        let symbols_query = r#"
            (function_declaration name: (identifier) @name) @function_def
            (class_declaration name: (type_identifier) @name) @class_def
            (method_definition name: (property_identifier) @name) @method_def
            (interface_declaration name: (type_identifier) @name) @interface_def
            (type_alias_declaration name: (type_identifier) @name) @interface_def
        "#;

        let imports_query = r#"
            (import_statement
                source: (string) @source)
        "#;

        let calls_query = r#"
            (call_expression
                function: (identifier) @callee)

            (call_expression
                function: (member_expression
                    property: (property_identifier) @callee))
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_rust_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_rust::language();

        let symbols_query = r#"
            (function_item name: (identifier) @name) @function_def
            (struct_item name: (type_identifier) @name) @struct_def
            (enum_item name: (type_identifier) @name) @enum_def
            (trait_item name: (type_identifier) @name) @trait_def
        "#;

        let imports_query = r#"
            (use_declaration) @source
        "#;

        let calls_query = r#"
            (call_expression function: (identifier) @callee)
            (call_expression function: (field_expression field: (field_identifier) @callee))
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_python_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_python::language();

        let symbols_query = r#"
            (function_definition name: (identifier) @name) @function_def
            (class_definition name: (identifier) @name) @class_def
        "#;

        let imports_query = r#"
            (import_statement) @source
            (import_from_statement) @source
        "#;

        let calls_query = r#"
            (call function: (identifier) @callee)
            (call function: (attribute attribute: (identifier) @callee))
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_go_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_go::language();

        let symbols_query = r#"
            (function_declaration name: (identifier) @name) @function_def
            (method_declaration name: (field_identifier) @name) @method_def
            (type_declaration (type_spec name: (type_identifier) @name type: (struct_type))) @struct_def
            (type_declaration (type_spec name: (type_identifier) @name type: (interface_type))) @interface_def
        "#;

        let imports_query = r#"
            (import_spec path: (interpreted_string_literal) @source)
        "#;

        let calls_query = r#"
            (call_expression function: (identifier) @callee)
            (call_expression function: (selector_expression field: (field_identifier) @callee))
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_java_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_java::language();

        let symbols_query = r#"
            (method_declaration name: (identifier) @name) @method_def
            (class_declaration name: (identifier) @name) @class_def
            (interface_declaration name: (identifier) @name) @interface_def
            (constructor_declaration name: (identifier) @name) @function_def
        "#;

        let imports_query = r#"
            (import_declaration) @source
        "#;

        let calls_query = r#"
            (method_invocation name: (identifier) @callee)
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_c_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_c::language();

        let symbols_query = r#"
            (function_definition declarator: (function_declarator declarator: (identifier) @name)) @function_def
            (struct_specifier name: (type_identifier) @name) @struct_def
            (enum_specifier name: (type_identifier) @name) @enum_def
        "#;

        let imports_query = r#"
            (preproc_include path: (string_literal) @source)
            (preproc_include path: (system_lib_string) @source)
        "#;

        let calls_query = r#"
            (call_expression function: (identifier) @callee)
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_cpp_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_cpp::language();

        let symbols_query = r#"
            (function_definition declarator: (function_declarator declarator: (identifier) @name)) @function_def
            (function_definition declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @name))) @method_def
            (class_specifier name: (type_identifier) @name) @class_def
            (struct_specifier name: (type_identifier) @name) @struct_def
        "#;

        let imports_query = r#"
            (preproc_include path: (string_literal) @source)
            (preproc_include path: (system_lib_string) @source)
        "#;

        let calls_query = r#"
            (call_expression function: (identifier) @callee)
            (call_expression function: (field_expression field: (field_identifier) @callee))
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }

    fn compile_csharp_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_c_sharp::language();

        let symbols_query = r#"
            (method_declaration name: (identifier) @name) @method_def
            (class_declaration name: (identifier) @name) @class_def
            (interface_declaration name: (identifier) @name) @interface_def
            (struct_declaration name: (identifier) @name) @struct_def
            (constructor_declaration name: (identifier) @name) @function_def
            (property_declaration name: (identifier) @name) @method_def
        "#;

        let imports_query = r#"
            (using_directive (identifier) @source)
            (using_directive (qualified_name) @source)
        "#;

        let calls_query = r#"
            (invocation_expression function: (identifier) @callee)
            (invocation_expression function: (member_access_expression name: (identifier) @callee))
        "#;

        Self::compile_queries(language, symbols_query, imports_query, calls_query)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_initialization() {
        // This test will show us the actual error if query compilation fails
        match ArborParser::new() {
            Ok(_) => println!("Parser initialized successfully!"),
            Err(e) => panic!("Parser failed to initialize: {}", e),
        }
    }

    #[test]
    fn test_parse_typescript_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
            function greet(name: string): string {
                return `Hello, ${name}!`;
            }

            export class UserService {
                validate(user: User): boolean {
                    return true;
                }
            }

            interface User {
                name: string;
                email: string;
            }
        "#;

        let result = parser.parse_source(source, "test.ts", "ts").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "greet"));
        assert!(result.symbols.iter().any(|s| s.name == "UserService"));
        assert!(result.symbols.iter().any(|s| s.name == "validate"));
        assert!(result.symbols.iter().any(|s| s.name == "User"));
    }

    #[test]
    fn test_parse_typescript_imports() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
            import { useState } from 'react';
            import lodash from 'lodash';

            function Component() {
                const [count, setCount] = useState(0);
            }
        "#;

        let result = parser.parse_source(source, "test.ts", "ts").unwrap();

        let imports: Vec<_> = result
            .relations
            .iter()
            .filter(|r| r.kind == RelationType::Imports)
            .collect();

        assert!(imports.iter().any(|i| i.to_name.contains("react")));
        assert!(imports.iter().any(|i| i.to_name.contains("lodash")));
    }

    #[test]
    fn test_parse_typescript_calls() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
            function outer() {
                inner();
                helper.process();
            }

            function inner() {
                console.log("Hello");
            }
        "#;

        let result = parser.parse_source(source, "test.ts", "ts").unwrap();

        let calls: Vec<_> = result
            .relations
            .iter()
            .filter(|r| r.kind == RelationType::Calls)
            .collect();

        assert!(calls.iter().any(|c| c.to_name == "inner"));
        assert!(calls.iter().any(|c| c.to_name == "process"));
        assert!(calls.iter().any(|c| c.to_name == "log"));
    }

    #[test]
    fn test_parse_rust_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
            fn main() {
                println!("Hello!");
            }

            pub struct User {
                name: String,
            }

            impl User {
                fn new(name: &str) -> Self {
                    Self { name: name.to_string() }
                }
            }

            enum Status {
                Active,
                Inactive,
            }
        "#;

        let result = parser.parse_source(source, "test.rs", "rs").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "main"));
        assert!(result.symbols.iter().any(|s| s.name == "User"));
        assert!(result.symbols.iter().any(|s| s.name == "new"));
        assert!(result.symbols.iter().any(|s| s.name == "Status"));
    }

    #[test]
    fn test_parse_python_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
def greet(name):
    return f"Hello, {name}!"

class UserService:
    def validate(self, user):
        return True
        "#;

        let result = parser.parse_source(source, "test.py", "py").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "greet"));
        assert!(result.symbols.iter().any(|s| s.name == "UserService"));
        assert!(result.symbols.iter().any(|s| s.name == "validate"));
    }

    #[test]
    fn test_parse_fallback_kotlin_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
            class BillingService
            fun computeInvoiceTotal(amount: Double): Double = amount
        "#;

        let result = parser.parse_source(source, "billing.kt", "kt").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "BillingService"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "computeInvoiceTotal"));
        assert!(result.relations.is_empty());
    }

    #[test]
    fn test_parse_go_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
package main

import "fmt"

func greet(name string) string {
    return fmt.Sprintf("Hello, %s!", name)
}

type User struct {
    Name string
    Age  int
}

type Service interface {
    Process(data []byte) error
}
"#;

        let result = parser.parse_source(source, "main.go", "go").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "greet"));
        assert!(result.symbols.iter().any(|s| s.name == "User"));
        assert!(result.symbols.iter().any(|s| s.name == "Service"));
    }

    #[test]
    fn test_parse_java_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
package com.example;

import java.util.List;

public class OrderService {
    public void processOrder(String orderId) {
        validate(orderId);
    }

    private void validate(String id) {
    }
}
"#;

        let result = parser
            .parse_source(source, "OrderService.java", "java")
            .unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "OrderService"));
        assert!(result.symbols.iter().any(|s| s.name == "processOrder"));
        assert!(result.symbols.iter().any(|s| s.name == "validate"));
    }

    #[test]
    fn test_parse_c_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
#include <stdio.h>

struct Point {
    int x;
    int y;
};

void print_point(struct Point p) {
    printf("(%d, %d)\n", p.x, p.y);
}

int add(int a, int b) {
    return a + b;
}
"#;

        let result = parser.parse_source(source, "math.c", "c").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "Point"));
        assert!(result.symbols.iter().any(|s| s.name == "print_point"));
        assert!(result.symbols.iter().any(|s| s.name == "add"));
    }

    #[test]
    fn test_parse_cpp_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
#include <iostream>

class Calculator {
public:
    int add(int a, int b) {
        return a + b;
    }
};

struct Config {
    int timeout;
};

void helpers() {
    std::cout << "ok" << std::endl;
}
"#;

        let result = parser.parse_source(source, "calc.cpp", "cpp").unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "Calculator"));
        assert!(result.symbols.iter().any(|s| s.name == "Config"));
        assert!(result.symbols.iter().any(|s| s.name == "helpers"));
    }

    #[test]
    fn test_parse_csharp_symbols() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
using System;

namespace MyApp
{
    public class UserController
    {
        public string GetUser(int id)
        {
            return "user";
        }
    }

    public interface IRepository
    {
        void Save(string data);
    }
}
"#;

        let result = parser
            .parse_source(source, "UserController.cs", "cs")
            .unwrap();

        assert!(result.symbols.iter().any(|s| s.name == "UserController"));
        assert!(result.symbols.iter().any(|s| s.name == "GetUser"));
        assert!(result.symbols.iter().any(|s| s.name == "IRepository"));
        assert!(result.symbols.iter().any(|s| s.name == "Save"));
    }

    #[test]
    fn test_parse_unsupported_extension_errors() {
        let mut parser = ArborParser::new().unwrap();
        let result = parser.parse_source("anything", "test.xyz", "xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_result_file_path() {
        let mut parser = ArborParser::new().unwrap();
        let result = parser
            .parse_source("fn main() {}", "test.rs", "rs")
            .unwrap();
        assert_eq!(result.file_path, "test.rs");
    }

    #[test]
    fn test_parse_python_imports_detected() {
        let mut parser = ArborParser::new().unwrap();

        let source = r#"
import os
from pathlib import Path

def read_file(path):
    with open(path) as f:
        return f.read()
"#;

        let result = parser.parse_source(source, "utils.py", "py").unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "read_file"));
        assert!(result
            .relations
            .iter()
            .any(|r| r.kind == RelationType::Imports));
    }
}
