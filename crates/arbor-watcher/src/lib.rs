//! Arbor Watcher - File watching and incremental indexing
//!
//! This crate handles the file system side of things:
//! - Walking directories to find source files
//! - Watching for changes
//! - Triggering incremental re-indexing
//!
//! It respects .gitignore and other ignore patterns.

mod indexer;
pub mod usages;
mod watcher;

pub use indexer::{
    index_directory, mtime_nanos, sources_newer_than, IgnoreMatcher, IndexOptions, IndexResult,
};
pub use usages::{scan_usages, UsageReport, UsageScanOptions};
pub use watcher::{FileChange, FileWatcher};
