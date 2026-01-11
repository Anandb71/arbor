//! Directory indexing.
//!
//! Walks directories to find and parse source files, building
//! the initial code graph.

use arbor_core::{parse_file, CodeNode};
use arbor_graph::{ArborGraph, GraphBuilder};
use ignore::WalkBuilder;
use std::path::Path;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Result of indexing a directory.
pub struct IndexResult {
    /// The built graph.
    pub graph: ArborGraph,

    /// Number of files processed.
    pub files_indexed: usize,

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
}

/// Indexes a directory and returns the code graph.
///
/// This walks all source files, parses them, and builds the
/// relationship graph. It respects .gitignore patterns.
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
    let mut nodes_extracted = 0;
    let mut errors = Vec::new();

    info!("Starting index of {}", root.display());

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

        debug!("Parsing {}", path.display());

        match parse_file(path) {
            Ok(nodes) => {
                nodes_extracted += nodes.len();
                files_indexed += 1;
                builder.add_nodes(nodes);
            }
            Err(e) => {
                warn!("Failed to parse {}: {}", path.display(), e);
                errors.push((path.display().to_string(), e.to_string()));
            }
        }
    }

    let graph = builder.build();
    let duration = start.elapsed();

    info!(
        "Indexed {} files ({} nodes) in {:?}",
        files_indexed, nodes_extracted, duration
    );

    Ok(IndexResult {
        graph,
        files_indexed,
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
        };
        let result = index_directory(dir.path(), options).unwrap();
        assert_eq!(result.files_indexed, 1);
        assert!(result.nodes_extracted > 0);
    }
}
