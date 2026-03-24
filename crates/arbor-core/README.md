<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="60" height="60" />
</p>

<h1 align="center">arbor-core</h1>

<p align="center">
  <strong>AST parsing engine for Arbor</strong><br>
  <em>Tree-sitter powered multi-language code analysis</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-core"><img src="https://img.shields.io/crates/v/arbor-core?style=flat-square&color=blue" alt="Crates.io" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## Overview

`arbor-core` is the parsing foundation of the [Arbor](https://github.com/Anandb71/arbor) ecosystem. It uses **Tree-sitter** to parse source code into Abstract Syntax Trees, extracting:

- **Nodes**: Functions, classes, structs, variables, imports
- **Edges**: Calls, inheritance, implementations, references

## Supported Languages

| Language | Parser | Entities |
|----------|--------|----------|
| Rust | `tree-sitter-rust` | fn, struct, trait, impl, macro |
| TypeScript | `tree-sitter-typescript` | class, interface, method, type |
| JavaScript | `tree-sitter-javascript` | function, class, var, import |
| Python | `tree-sitter-python` | class, def, decorator, import |
| Go | `tree-sitter-go` | struct, interface, func, method |
| Java | `tree-sitter-java` | class, interface, method, field |
| C/C++ | `tree-sitter-c/cpp` | struct, class, function, template |
| C# | `tree-sitter-c-sharp` | class, method, property, interface |
| Dart | `tree-sitter-dart` | class, mixin, method, widget |

Additional fallback parser support (v1.6.2+):

- Kotlin (`.kt`, `.kts`)
- Swift (`.swift`)
- Ruby (`.rb`)
- PHP (`.php`, `.phtml`)
- Shell (`.sh`, `.bash`, `.zsh`)

## Usage

This crate is primarily used internally by `arbor-graph` and `arbor-watcher`. For most use cases, install `arbor-graph-cli` instead:

```bash
cargo install arbor-graph-cli
```

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)
