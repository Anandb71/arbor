//! Arbor Watcher - File watching and incremental indexing
//!
//! This crate handles the file system side of things:
//! - Walking directories to find source files
//! - Watching for changes
//! - Triggering incremental re-indexing
//!
//! It respects .gitignore and other ignore patterns.

mod indexer;
mod watcher;

pub use indexer::{index_directory, IndexOptions, IndexResult};
pub use watcher::{FileChange, FileWatcher};
