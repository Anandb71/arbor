<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="60" height="60" />
</p>

<h1 align="center">arbor-watcher</h1>

<p align="center">
  <strong>File watcher for Arbor</strong><br>
  <em>Real-time incremental indexing</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-watcher"><img src="https://img.shields.io/crates/v/arbor-watcher?style=flat-square&color=blue" alt="Crates.io" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## Overview

`arbor-watcher` provides the file system watching and incremental indexing layer for [Arbor](https://github.com/Anandb71/arbor).

## Features

- **Cross-platform**: Uses `notify` for Windows, macOS, and Linux
- **Debounced Events**: Prevents rapid re-indexing (100ms threshold)
- **Gitignore Aware**: Respects `.gitignore` patterns via `ignore` crate
- **Incremental**: Only re-parses changed files

## How It Works

```
File Change → notify → Debouncer → Index Queue → arbor-core → arbor-graph
                                       ↓
                              Only "dirty" nodes updated
```

## Usage

This crate is used internally. For most use cases:

```bash
cargo install arbor-graph-cli
arbor setup
arbor watch  # Live re-indexing
```

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)
