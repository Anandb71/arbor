# Adding a New Language to Arbor

This guide walks through adding parser support for a new programming language.

## Prerequisites

- Familiarity with Rust
- Understanding of the target language's syntax
- A Tree-sitter grammar for the language

## Steps

### 1. Add the Tree-sitter Dependency

Edit `crates/arbor-core/Cargo.toml`:

```toml
[dependencies]
tree-sitter-your-language = "0.20"
```

### 2. Create the Language Module

Create `crates/arbor-core/src/languages/your_language.rs`:

```rust
//! YourLanguage parser implementation.

use crate::languages::LanguageParser;
use crate::node::{CodeNode, NodeKind};
use tree_sitter::{Language, Tree};

pub struct YourLanguageParser;

impl LanguageParser for YourLanguageParser {
    fn language(&self) -> Language {
        tree_sitter_your_language::language()
    }

    fn extensions(&self) -> &[&str] {
        &["ext"]
    }

    fn extract_nodes(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<CodeNode> {
        let mut nodes = Vec::new();
        let root = tree.root_node();
        
        // Implement extraction logic
        extract_from_node(&root, source, file_path, &mut nodes, None);
        
        nodes
    }
}
```

### 3. Register the Parser

Edit `crates/arbor-core/src/languages/mod.rs`:

```rust
mod your_language;

// In get_parser():
"ext" => Some(Box::new(your_language::YourLanguageParser)),
```

### 4. Implement Node Extraction

Study the Tree-sitter grammar for your language to understand:

- Which AST node types represent functions, classes, etc.
- How to extract names and signatures
- How to detect visibility modifiers

### 5. Add Tests

Create `crates/arbor-core/tests/your_language_test.rs`:

```rust
use arbor_core::{parse_source, languages::get_parser, NodeKind};

#[test]
fn test_function_extraction() {
    let source = "...your language code...";
    let parser = get_parser("ext").unwrap();
    let nodes = parse_source(source, "test.ext", parser.as_ref()).unwrap();
    
    assert!(nodes.iter().any(|n| n.kind == NodeKind::Function));
}
```

### 6. Update Documentation

Add your language to:

- `README.md` supported languages table
- `docs/GRAPH_SCHEMA.md` language mappings section

## Tips

- Use `tree-sitter playground` to explore the AST structure
- Start with basic extraction (functions, classes) before adding edge cases
- Look at existing language implementations for patterns
- Test with real-world code from open source projects

## Fast Path: Fallback Parser (v1.6.2+)

If you want immediate indexing support before wiring a full Tree-sitter parser module,
add the extension to the fallback parser:

- `crates/arbor-core/src/fallback_parser.rs`

This path is heuristic-based (line scanning) and is ideal for quickly supporting
additional language families in polyglot repos while full grammar support is in progress.

Recommended workflow:

1. Add extension + symbol patterns in fallback parser
2. Add tests for extraction behavior
3. Wire extension into watcher/sync lists
4. Promote to full Tree-sitter parser in a follow-up milestone

## Example: Adding Go

Here's a condensed example for Go:

```rust
// In go.rs
"function_declaration" => {
    if let Some(name) = node.child_by_field_name("name") {
        nodes.push(CodeNode::new(
            get_text(&name, source),
            get_text(&name, source),
            NodeKind::Function,
            file_path,
        ));
    }
}
```

## Questions?

Open an issue or discussion on GitHub if you get stuck.
