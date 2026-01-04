//! ArborParser - The Eyes of Arbor
//!
//! This module implements high-performance code parsing using Tree-sitter queries.
//! It extracts symbols (functions, classes, interfaces) and their relationships
//! (imports, calls) to build a comprehensive code graph.
//!
//! The parser is designed for incremental updates - calling it on the same file
//! will update existing nodes rather than creating duplicates.

use crate::error::{ParseError, Result};
use crate::node::{CodeNode, NodeKind};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tree_sitter::{Language, Parser, Query, QueryCursor, Tree};

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
        Self::new()
    }
}

impl ArborParser {
    /// Creates a new ArborParser with pre-compiled queries.
    pub fn new() -> Self {
        let parser = Parser::new();
        let mut queries = HashMap::new();

        // Compile TypeScript/JavaScript queries
        if let Ok(compiled) = Self::compile_typescript_queries() {
            queries.insert("ts".to_string(), compiled);
        }
        if let Ok(compiled) = Self::compile_typescript_queries() {
            queries.insert("tsx".to_string(), compiled);
        }
        if let Ok(compiled) = Self::compile_typescript_queries() {
            queries.insert("js".to_string(), compiled);
        }
        if let Ok(compiled) = Self::compile_typescript_queries() {
            queries.insert("jsx".to_string(), compiled);
        }

        // Compile Rust queries
        if let Ok(compiled) = Self::compile_rust_queries() {
            queries.insert("rs".to_string(), compiled);
        }

        // Compile Python queries
        if let Ok(compiled) = Self::compile_python_queries() {
            queries.insert("py".to_string(), compiled);
        }

        Self { parser, queries }
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

        // Get the extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| ParseError::UnsupportedLanguage(path.to_path_buf()))?;

        // Get compiled queries
        let compiled = self
            .queries
            .get(ext)
            .ok_or_else(|| ParseError::UnsupportedLanguage(path.to_path_buf()))?;

        // Configure parser for this language
        self.parser
            .set_language(compiled.language.clone())
            .map_err(|e| ParseError::ParserError(format!("Failed to set language: {}", e)))?;

        // Parse the source
        let tree = self
            .parser
            .parse(&source, None)
            .ok_or_else(|| ParseError::ParserError("Tree-sitter returned no tree".into()))?;

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

        let compiled = self
            .queries
            .get(language)
            .ok_or_else(|| ParseError::UnsupportedLanguage(file_path.into()))?;

        self.parser
            .set_language(compiled.language.clone())
            .map_err(|e| ParseError::ParserError(format!("Failed to set language: {}", e)))?;

        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParserError("Tree-sitter returned no tree".into()))?;

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

        let matches = cursor.matches(&compiled.symbols, tree.root_node(), source.as_bytes());

        for match_ in matches {
            // Extract name and type from captures
            let mut name: Option<&str> = None;
            let mut kind: Option<NodeKind> = None;
            let mut node = match_.captures.first().map(|c| c.node);

            for capture in match_.captures {
                let capture_name = &compiled.symbols.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name.as_str() {
                    "name" | "function.name" | "class.name" | "interface.name" | "method.name" => {
                        name = Some(text);
                    }
                    "function" | "function.def" => {
                        kind = Some(NodeKind::Function);
                        node = Some(capture.node);
                    }
                    "class" | "class.def" => {
                        kind = Some(NodeKind::Class);
                        node = Some(capture.node);
                    }
                    "interface" | "interface.def" => {
                        kind = Some(NodeKind::Interface);
                        node = Some(capture.node);
                    }
                    "method" | "method.def" => {
                        kind = Some(NodeKind::Method);
                        node = Some(capture.node);
                    }
                    "struct" | "struct.def" => {
                        kind = Some(NodeKind::Struct);
                        node = Some(capture.node);
                    }
                    "enum" | "enum.def" => {
                        kind = Some(NodeKind::Enum);
                        node = Some(capture.node);
                    }
                    "trait" | "trait.def" => {
                        kind = Some(NodeKind::Interface);
                        node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(kind), Some(node)) = (name, kind, node) {
                // Build fully qualified name: filename:symbol_name
                let qualified_name = format!("{}:{}", file_name, name);

                // Extract signature (first line of the node)
                let signature = source
                    .lines()
                    .nth(node.start_position().row)
                    .map(|s| s.trim().to_string());

                let mut symbol = CodeNode::new(name, &qualified_name, kind, file_path)
                    .with_lines(
                        node.start_position().row as u32 + 1,
                        node.end_position().row as u32 + 1,
                    )
                    .with_column(node.start_position().column as u32)
                    .with_bytes(node.start_byte() as u32, node.end_byte() as u32);

                if let Some(sig) = signature {
                    symbol = symbol.with_signature(sig);
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

        // Extract imports
        self.extract_imports(tree, source, file_path, &mut relations, compiled);

        // Extract calls
        self.extract_calls(tree, source, file_path, symbols, &mut relations, compiled);

        relations
    }

    fn extract_imports(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &str,
        relations: &mut Vec<SymbolRelation>,
        compiled: &CompiledQueries,
    ) {
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&compiled.imports, tree.root_node(), source.as_bytes());

        for match_ in matches {
            let mut module_name: Option<&str> = None;
            let mut line: u32 = 0;

            for capture in match_.captures {
                let capture_name = &compiled.imports.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name.as_str() {
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
                let file_id = format!("{}:__file__", file_path);
                relations.push(SymbolRelation {
                    from_id: file_id,
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
        relations: &mut Vec<SymbolRelation>,
        compiled: &CompiledQueries,
    ) {
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&compiled.calls, tree.root_node(), source.as_bytes());

        for match_ in matches {
            let mut callee_name: Option<&str> = None;
            let mut call_line: u32 = 0;

            for capture in match_.captures {
                let capture_name = &compiled.calls.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name.as_str() {
                    "callee" | "function" | "call.function" => {
                        // Handle method calls like obj.method()
                        if let Some(dot_pos) = text.rfind('.') {
                            callee_name = Some(&text[dot_pos + 1..]);
                        } else {
                            callee_name = Some(text);
                        }
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
                    .unwrap_or_else(|| format!("{}:__file__", file_path));

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

    // ─────────────────────────────────────────────────────────────────────────
    // Query Compilation
    // ─────────────────────────────────────────────────────────────────────────

    fn compile_typescript_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_typescript::language_typescript();

        // Symbol extraction query
        let symbols_query = r#"
            ; Functions
            (function_declaration
                name: (identifier) @name) @function.def

            ; Arrow functions assigned to variables
            (lexical_declaration
                (variable_declarator
                    name: (identifier) @name
                    value: (arrow_function))) @function.def

            ; Classes
            (class_declaration
                name: (identifier) @name) @class.def

            ; Methods
            (method_definition
                name: (property_identifier) @name) @method.def

            ; Interfaces
            (interface_declaration
                name: (type_identifier) @name) @interface.def

            ; Type aliases
            (type_alias_declaration
                name: (type_identifier) @name) @interface.def

            ; Exported functions
            (export_statement
                declaration: (function_declaration
                    name: (identifier) @name)) @function.def
        "#;

        // Import query
        let imports_query = r#"
            (import_statement
                source: (string) @source)

            (import_statement
                source: (string) @source)
        "#;

        // Call expression query
        let calls_query = r#"
            (call_expression
                function: (identifier) @callee)

            (call_expression
                function: (member_expression
                    property: (property_identifier) @callee))
        "#;

        let symbols = Query::new(language.clone(), symbols_query)
            .map_err(|e| ParseError::QueryError(e.message))?;
        let imports = Query::new(language.clone(), imports_query)
            .map_err(|e| ParseError::QueryError(e.message))?;
        let calls = Query::new(language.clone(), calls_query)
            .map_err(|e| ParseError::QueryError(e.message))?;

        Ok(CompiledQueries {
            symbols,
            imports,
            calls,
            language,
        })
    }

    fn compile_rust_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_rust::language();

        let symbols_query = r#"
            ; Functions
            (function_item
                name: (identifier) @name) @function.def

            ; Structs
            (struct_item
                name: (type_identifier) @name) @struct.def

            ; Enums
            (enum_item
                name: (type_identifier) @name) @enum.def

            ; Traits
            (trait_item
                name: (type_identifier) @name) @trait.def

            ; Impl methods
            (impl_item
                body: (declaration_list
                    (function_item
                        name: (identifier) @name) @method.def))
        "#;

        let imports_query = r#"
            (use_declaration
                argument: (use_tree) @source)
        "#;

        let calls_query = r#"
            (call_expression
                function: (identifier) @callee)

            (call_expression
                function: (field_expression
                    field: (field_identifier) @callee))

            (call_expression
                function: (scoped_identifier
                    name: (identifier) @callee))
        "#;

        let symbols = Query::new(language.clone(), symbols_query)
            .map_err(|e| ParseError::QueryError(e.message))?;
        let imports = Query::new(language.clone(), imports_query)
            .map_err(|e| ParseError::QueryError(e.message))?;
        let calls = Query::new(language.clone(), calls_query)
            .map_err(|e| ParseError::QueryError(e.message))?;

        Ok(CompiledQueries {
            symbols,
            imports,
            calls,
            language,
        })
    }

    fn compile_python_queries() -> Result<CompiledQueries> {
        let language = tree_sitter_python::language();

        let symbols_query = r#"
            ; Functions
            (function_definition
                name: (identifier) @name) @function.def

            ; Classes
            (class_definition
                name: (identifier) @name) @class.def

            ; Methods (functions inside classes)
            (class_definition
                body: (block
                    (function_definition
                        name: (identifier) @name) @method.def))
        "#;

        let imports_query = r#"
            (import_statement
                name: (dotted_name) @source)

            (import_from_statement
                module_name: (dotted_name) @source)
        "#;

        let calls_query = r#"
            (call
                function: (identifier) @callee)

            (call
                function: (attribute
                    attribute: (identifier) @callee))
        "#;

        let symbols = Query::new(language.clone(), symbols_query)
            .map_err(|e| ParseError::QueryError(e.message))?;
        let imports = Query::new(language.clone(), imports_query)
            .map_err(|e| ParseError::QueryError(e.message))?;
        let calls = Query::new(language.clone(), calls_query)
            .map_err(|e| ParseError::QueryError(e.message))?;

        Ok(CompiledQueries {
            symbols,
            imports,
            calls,
            language,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_typescript_symbols() {
        let mut parser = ArborParser::new();

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
        let mut parser = ArborParser::new();

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
        let mut parser = ArborParser::new();

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
        let mut parser = ArborParser::new();

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
        let mut parser = ArborParser::new();

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
}
