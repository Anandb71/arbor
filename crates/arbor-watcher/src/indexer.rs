//! Directory indexing.
//!
//! Walks directories to find and parse source files, building
//! the initial code graph.

use arbor_core::{parse_file, CodeNode};
use arbor_graph::{ArborGraph, GraphBuilder, GraphStore};
use ignore::WalkBuilder;
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

    // Track files we've seen (for detecting deleted files)
    let mut seen_files: HashSet<String> = HashSet::new();

    // Walk the directory, respecting .gitignore
    let walker = WalkBuilder::new(root)
        .hidden(true) // Skip hidden files
        .git_ignore(true) // Respect .gitignore
        .git_global(true)
        .git_exclude(true)
        .follow_links(options.follow_symlinks)
        .build();

    for entry in walker.filter_map(Result::ok) {
        let path = entry.path();

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Check if it's a supported file type
        let extension = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => ext,
            None => continue,
        };

        if !arbor_core::languages::is_supported(extension) {
            continue;
        }

        let path_str = path.display().to_string();
        seen_files.insert(path_str.clone());

        // Check cache
        if let Some(ref store) = store {
            // Get file mtime
            let current_mtime = match std::fs::metadata(path) {
                Ok(meta) => meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                Err(_) => 0,
            };

            // Check cached mtime
            if let Ok(Some(cached_mtime)) = store.get_mtime(&path_str) {
                if cached_mtime == current_mtime {
                    // File unchanged, load from cache
                    if let Ok(Some(cached_nodes)) = store.get_file_nodes(&path_str) {
                        debug!("Cache hit: {}", path.display());
                        nodes_extracted += cached_nodes.len();
                        cache_hits += 1;
                        builder.add_nodes(cached_nodes);
                        continue;
                    }
                }
            }

            // Cache miss or stale, parse file
            debug!("Parsing (cache miss): {}", path.display());
            match parse_file(path) {
                Ok(nodes) => {
                    nodes_extracted += nodes.len();
                    files_indexed += 1;
                    // Update cache
                    if let Err(e) = store.update_file(&path_str, &nodes, current_mtime) {
                        warn!("Failed to update cache for {}: {}", path_str, e);
                    }
                    builder.add_nodes(nodes);
                }
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                    errors.push((path_str, e.to_string()));
                }
            }
        } else {
            // No cache, parse directly
            debug!("Parsing {}", path.display());
            match parse_file(path) {
                Ok(nodes) => {
                    nodes_extracted += nodes.len();
                    files_indexed += 1;
                    builder.add_nodes(nodes);
                }
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                    errors.push((path_str, e.to_string()));
                }
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
