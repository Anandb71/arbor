//! Directory indexing.
//!
//! Walks directories to find and parse source files, building
//! the initial code graph.

use arbor_core::{parse_file, CodeNode};
use arbor_graph::{ArborGraph, GraphBuilder, GraphStore};
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{debug, info, warn};

/// Result of indexing a directory.
pub struct IndexResult {
    /// The built graph.
    pub graph: ArborGraph,

    /// Number of files processed (parsed fresh).
    pub files_indexed: usize,

    /// Number of files loaded from cache.
    pub cache_hits: usize,

    /// Number of nodes extracted.
    pub nodes_extracted: usize,

    /// Time taken in milliseconds.
    pub duration_ms: u64,

    /// Files that failed to parse.
    pub errors: Vec<(String, String)>,
}

/// Options for directory indexing.
#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// Follow symbolic links when walking directories.
    pub follow_symlinks: bool,

    /// Path to cache directory (e.g., `.arbor/cache`).
    /// If None, caching is disabled.
    pub cache_path: Option<PathBuf>,
}

/// Indexes a directory and returns the code graph.
///
/// This walks all source files, parses them, and builds the
/// relationship graph. It respects .gitignore patterns.
///
/// If `options.cache_path` is set, files are cached with their mtimes.
/// Only files with changed mtimes are re-parsed.
///
/// # Example
///
/// ```no_run
/// use arbor_watcher::{index_directory, IndexOptions};
/// use std::path::Path;
///
/// let result = index_directory(Path::new("./src"), IndexOptions::default()).unwrap();
/// println!("Indexed {} files, {} nodes", result.files_indexed, result.nodes_extracted);
/// ```
pub fn index_directory(root: &Path, options: IndexOptions) -> Result<IndexResult, std::io::Error> {
    let start = Instant::now();
    let mut builder = GraphBuilder::new();
    let mut files_indexed = 0;
    let mut cache_hits = 0;
    let mut nodes_extracted = 0;
    let mut errors = Vec::new();

    info!("Starting index of {}", root.display());

    // Open cache if configured
    let store =
        options
            .cache_path
            .as_ref()
            .and_then(|path| match GraphStore::open_or_reset(path) {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!("Failed to open cache: {}, proceeding without cache", e);
                    None
                }
            });

    // Walk the directory, respecting .gitignore, collecting supported files
    let walker = WalkBuilder::new(root)
        .hidden(true) // Skip hidden files
        .git_ignore(true) // Respect .gitignore
        .git_global(true)
        .git_exclude(true)
        .follow_links(options.follow_symlinks)
        .build();

    let candidates: Vec<PathBuf> = walker
        .filter_map(Result::ok)
        .filter(|entry| {
            let path = entry.path();
            if path.is_dir() {
                return false;
            }
            match path.extension().and_then(|e| e.to_str()) {
                Some(ext) => arbor_core::languages::is_supported(ext),
                None => false,
            }
        })
        .map(|entry| entry.into_path())
        .collect();

    // Track files we've seen (for detecting deleted files)
    let seen_files: HashSet<String> = candidates.iter().map(|p| p.display().to_string()).collect();

    // Parse files in parallel. parse_file creates its own tree-sitter parser
    // per call and sled handles concurrent reads/writes, so both the cache
    // check and the parse can fan out. Results collect in walk order, keeping
    // graph construction deterministic.
    enum Outcome {
        CacheHit(Vec<CodeNode>),
        Parsed(Vec<CodeNode>),
        Failed(String),
    }

    let store_ref = store.as_ref();
    let outcomes: Vec<(String, Outcome)> = candidates
        .par_iter()
        .map(|path| {
            let path_str = path.display().to_string();

            let current_mtime = match std::fs::metadata(path) {
                Ok(meta) => meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                Err(_) => 0,
            };

            if let Some(store) = store_ref {
                if let Ok(Some(cached_mtime)) = store.get_mtime(&path_str) {
                    if cached_mtime == current_mtime {
                        // File unchanged, load from cache
                        if let Ok(Some(cached_nodes)) = store.get_file_nodes(&path_str) {
                            debug!("Cache hit: {}", path.display());
                            return (path_str, Outcome::CacheHit(cached_nodes));
                        }
                    }
                }
            }

            debug!("Parsing: {}", path.display());
            match parse_file(path) {
                Ok(nodes) => {
                    if let Some(store) = store_ref {
                        if let Err(e) = store.update_file(&path_str, &nodes, current_mtime) {
                            warn!("Failed to update cache for {}: {}", path_str, e);
                        }
                    }
                    (path_str, Outcome::Parsed(nodes))
                }
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                    (path_str, Outcome::Failed(e.to_string()))
                }
            }
        })
        .collect();

    for (path_str, outcome) in outcomes {
        match outcome {
            Outcome::CacheHit(nodes) => {
                nodes_extracted += nodes.len();
                cache_hits += 1;
                builder.add_nodes(nodes);
            }
            Outcome::Parsed(nodes) => {
                nodes_extracted += nodes.len();
                files_indexed += 1;
                builder.add_nodes(nodes);
            }
            Outcome::Failed(error) => {
                errors.push((path_str, error));
            }
        }
    }

    // Handle deleted files: remove from cache any files that no longer exist
    if let Some(ref store) = store {
        if let Ok(cached_files) = store.list_cached_files() {
            for cached_file in cached_files {
                if !seen_files.contains(&cached_file) {
                    debug!("Removing deleted file from cache: {}", cached_file);
                    if let Err(e) = store.remove_file(&cached_file) {
                        warn!("Failed to remove {} from cache: {}", cached_file, e);
                    }
                }
            }
        }
    }

    let graph = builder.build();
    let duration = start.elapsed();

    info!(
        "Indexed {} files, {} cache hits ({} nodes) in {:?}",
        files_indexed, cache_hits, nodes_extracted, duration
    );

    Ok(IndexResult {
        graph,
        files_indexed,
        cache_hits,
        nodes_extracted,
        duration_ms: duration.as_millis() as u64,
        errors,
    })
}

/// Parses a single file and returns its nodes.
#[allow(dead_code)]
pub fn parse_single_file(path: &Path) -> Result<Vec<CodeNode>, arbor_core::ParseError> {
    parse_file(path)
}

/// Returns true if any supported source file under `root` is newer than `cache_mtime`.
///
/// Used by read commands to detect a stale `graph.bin` before trusting it.
/// Walks the same gitignore-respecting tree as [`index_directory`] but only
/// stats files — no parsing — and early-exits on the first newer file.
///
/// `cache_mtime` is the modified time of the cache file, in seconds since the
/// UNIX epoch. Catches edits and additions; a lone deletion leaves no newer
/// file, so it is picked up on the next edit instead.
pub fn sources_newer_than(root: &Path, cache_mtime: u64, follow_symlinks: bool) -> bool {
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(follow_symlinks)
        .build();

    for entry in walker.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if arbor_core::languages::is_supported(ext) => {}
            _ => continue,
        }
        let mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
            Ok(t) => t
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            Err(_) => continue,
        };
        if mtime > cache_mtime {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_index_empty_directory() {
        let dir = tempdir().unwrap();
        let result = index_directory(dir.path(), IndexOptions::default()).unwrap();
        assert_eq!(result.files_indexed, 0);
        assert_eq!(result.nodes_extracted, 0);
    }

    #[test]
    fn test_sources_newer_than_detects_fresh_edit() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "pub fn a() {}").unwrap();

        // Cache mtime far in the past → the source file is newer → stale.
        assert!(sources_newer_than(dir.path(), 0, false));

        // Cache mtime far in the future → nothing newer → fresh.
        let far_future = u64::MAX;
        assert!(!sources_newer_than(dir.path(), far_future, false));
    }

    #[test]
    fn test_sources_newer_than_ignores_unsupported_files() {
        let dir = tempdir().unwrap();
        // Only an unsupported file exists; it must not mark the cache stale.
        fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        assert!(!sources_newer_than(dir.path(), 0, false));
    }

    #[test]
    fn test_index_with_rust_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        fs::write(
            &file_path,
            r#"
            pub fn hello() {
                println!("Hello!");
            }
        "#,
        )
        .unwrap();

        let result = index_directory(dir.path(), IndexOptions::default()).unwrap();
        assert_eq!(result.files_indexed, 1);
        assert!(result.nodes_extracted > 0);
    }

    /// Helper to create a directory symlink cross-platform.
    /// Returns None if symlink creation fails (e.g., no privileges on Windows).
    fn create_dir_symlink(original: &std::path::Path, link: &std::path::Path) -> Option<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(original, link).ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(original, link).ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            None
        }
    }

    #[test]
    fn test_index_does_not_follow_symlinks_by_default() {
        let dir = tempdir().unwrap();
        let linked_dir = tempdir().unwrap();

        // Create a file in the linked directory
        let linked_file = linked_dir.path().join("linked.rs");
        fs::write(&linked_file, "pub fn linked_func() {}").unwrap();

        // Create a symlink to the linked directory
        let symlink_path = dir.path().join("linked");
        if create_dir_symlink(linked_dir.path(), &symlink_path).is_none() {
            // Skip test if symlinks not supported (e.g., Windows without privileges)
            return;
        }

        // Index without following symlinks (default)
        let result = index_directory(dir.path(), IndexOptions::default()).unwrap();
        assert_eq!(result.files_indexed, 0);
    }

    #[test]
    fn test_index_follows_symlinks_when_enabled() {
        let dir = tempdir().unwrap();
        let linked_dir = tempdir().unwrap();

        // Create a file in the linked directory
        let linked_file = linked_dir.path().join("linked.rs");
        fs::write(&linked_file, "pub fn linked_func() {}").unwrap();

        // Create a symlink to the linked directory
        let symlink_path = dir.path().join("linked");
        if create_dir_symlink(linked_dir.path(), &symlink_path).is_none() {
            // Skip test if symlinks not supported (e.g., Windows without privileges)
            return;
        }

        // Index with follow_symlinks enabled
        let options = IndexOptions {
            follow_symlinks: true,
            cache_path: None,
        };
        let result = index_directory(dir.path(), options).unwrap();
        assert_eq!(result.files_indexed, 1);
        assert!(result.nodes_extracted > 0);
    }
}
