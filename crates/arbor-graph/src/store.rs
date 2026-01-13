use crate::builder::GraphBuilder;
use crate::graph::ArborGraph;
use arbor_core::CodeNode;
use sled::{Batch, Db};
use std::path::Path;
use thiserror::Error;

/// Current cache format version. Increment when schema changes.
const CACHE_VERSION: &str = "arbor-1.3";

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Sled(#[from] sled::Error),
    #[error("Serialization error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("Corrupted data: {0}")]
    Corrupted(String),
    #[error("Cache version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: String, found: String },
}

pub struct GraphStore {
    db: Db,
}

impl GraphStore {
    /// Opens or creates a graph store at the specified path.
    /// Returns an error if the cache version doesn't match.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let db = sled::open(path)?;
        let store = Self { db };

        // Check cache version
        if let Some(version_bytes) = store.db.get("meta:version")? {
            let version: String = bincode::deserialize(&version_bytes)?;
            if version != CACHE_VERSION {
                return Err(StoreError::VersionMismatch {
                    expected: CACHE_VERSION.to_string(),
                    found: version,
                });
            }
        } else {
            // New cache, set version
            let version_bytes = bincode::serialize(&CACHE_VERSION.to_string())?;
            store.db.insert("meta:version", version_bytes)?;
        }

        Ok(store)
    }

    /// Opens a store, clearing it if version mismatches.
    pub fn open_or_reset<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        match Self::open(path.as_ref()) {
            Ok(store) => Ok(store),
            Err(StoreError::VersionMismatch { .. }) => {
                // Clear and reopen
                let db = sled::open(path.as_ref())?;
                db.clear()?;
                let version_bytes = bincode::serialize(&CACHE_VERSION.to_string())?;
                db.insert("meta:version", version_bytes)?;
                db.flush()?;
                Ok(Self { db })
            }
            Err(e) => Err(e),
        }
    }

    /// Gets the stored mtime for a file.
    pub fn get_mtime(&self, file_path: &str) -> Result<Option<u64>, StoreError> {
        let key = format!("m:{}", file_path);
        match self.db.get(&key)? {
            Some(bytes) => {
                let mtime: u64 = bincode::deserialize(&bytes)?;
                Ok(Some(mtime))
            }
            None => Ok(None),
        }
    }

    /// Gets the stored nodes for a file.
    pub fn get_file_nodes(&self, file_path: &str) -> Result<Option<Vec<CodeNode>>, StoreError> {
        let file_key = format!("f:{}", file_path);
        match self.db.get(&file_key)? {
            Some(index_bytes) => {
                let node_ids: Vec<String> = bincode::deserialize(&index_bytes)?;
                let mut nodes = Vec::with_capacity(node_ids.len());
                for id in node_ids {
                    let node_key = format!("n:{}", id);
                    if let Some(node_bytes) = self.db.get(&node_key)? {
                        let node: CodeNode = bincode::deserialize(&node_bytes)?;
                        nodes.push(node);
                    }
                }
                Ok(Some(nodes))
            }
            None => Ok(None),
        }
    }

    /// Updates the nodes and mtime for a specific file.
    ///
    /// This operation is atomic: it removes old nodes associated with the file
    /// and inserts the new ones.
    pub fn update_file(
        &self,
        file_path: &str,
        nodes: &[CodeNode],
        mtime: u64,
    ) -> Result<(), StoreError> {
        let file_key = format!("f:{}", file_path);
        let mtime_key = format!("m:{}", file_path);
        let mut batch = Batch::default();

        // 1. Get old nodes for this file
        if let Some(old_bytes) = self.db.get(&file_key)? {
            let old_ids: Vec<String> = bincode::deserialize(&old_bytes)?;
            for id in old_ids {
                batch.remove(format!("n:{}", id).as_bytes());
            }
        }

        // 2. Insert new nodes
        let mut new_ids = Vec::with_capacity(nodes.len());
        for node in nodes {
            let node_key = format!("n:{}", node.id);
            let bytes = bincode::serialize(node)?;
            batch.insert(node_key.as_bytes(), bytes);
            new_ids.push(node.id.clone());
        }

        // 3. Update file index
        let index_bytes = bincode::serialize(&new_ids)?;
        batch.insert(file_key.as_bytes(), index_bytes);

        // 4. Update mtime
        let mtime_bytes = bincode::serialize(&mtime)?;
        batch.insert(mtime_key.as_bytes(), mtime_bytes);

        // 5. Commit batch
        self.db.apply_batch(batch)?;
        self.db.flush()?;
        Ok(())
    }

    /// Removes a file from the cache (for deleted files).
    pub fn remove_file(&self, file_path: &str) -> Result<(), StoreError> {
        let file_key = format!("f:{}", file_path);
        let mtime_key = format!("m:{}", file_path);
        let mut batch = Batch::default();

        // Remove nodes
        if let Some(old_bytes) = self.db.get(&file_key)? {
            let old_ids: Vec<String> = bincode::deserialize(&old_bytes)?;
            for id in old_ids {
                batch.remove(format!("n:{}", id).as_bytes());
            }
        }

        batch.remove(file_key.as_bytes());
        batch.remove(mtime_key.as_bytes());

        self.db.apply_batch(batch)?;
        self.db.flush()?;
        Ok(())
    }

    /// Lists all cached file paths.
    pub fn list_cached_files(&self) -> Result<Vec<String>, StoreError> {
        let mut files = Vec::new();
        let prefix = b"f:";
        for item in self.db.scan_prefix(prefix) {
            let (key, _) = item?;
            let key_str = String::from_utf8_lossy(&key);
            if let Some(file_path) = key_str.strip_prefix("f:") {
                files.push(file_path.to_string());
            }
        }
        Ok(files)
    }

    /// Loads the entire graph from the store.
    ///
    /// This iterates over all stored nodes and reconstructs the ArborGraph
    /// using the GraphBuilder (which re-links edges).
    pub fn load_graph(&self) -> Result<ArborGraph, StoreError> {
        let mut builder = GraphBuilder::new();
        let mut nodes = Vec::new();

        // Iterate over all keys starting with "n:"
        let prefix = b"n:";
        for item in self.db.scan_prefix(prefix) {
            let (_key, value) = item?;
            let node: CodeNode = bincode::deserialize(&value)?;
            nodes.push(node);
        }

        if nodes.is_empty() {
            // Return empty graph
            return Ok(ArborGraph::new());
        }

        // Reconstruct graph
        builder.add_nodes(nodes);
        // resolve_edges() is called by build()
        let graph = builder.build();

        Ok(graph)
    }

    /// Clears the stored graph.
    pub fn clear(&self) -> Result<(), StoreError> {
        self.db.clear()?;
        // Re-set version after clear
        let version_bytes = bincode::serialize(&CACHE_VERSION.to_string())?;
        self.db.insert("meta:version", version_bytes)?;
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbor_core::NodeKind;
    use tempfile::tempdir;

    #[test]
    fn test_incremental_updates() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let node1 = CodeNode::new("foo", "foo", NodeKind::Function, "test.rs");
        let node2 = CodeNode::new("bar", "bar", NodeKind::Function, "test.rs");

        // Initial update with mtime
        store
            .update_file("test.rs", &[node1.clone(), node2.clone()], 1000)
            .unwrap();

        // Verify load
        let graph = store.load_graph().unwrap();
        assert_eq!(graph.node_count(), 2);

        // Verify mtime was stored
        assert_eq!(store.get_mtime("test.rs").unwrap(), Some(1000));

        // Update with one node removed
        store
            .update_file("test.rs", &[node1.clone()], 2000)
            .unwrap();
        let graph2 = store.load_graph().unwrap();
        assert_eq!(graph2.node_count(), 1);
        assert!(!graph2.find_by_name("foo").is_empty());
        assert!(graph2.find_by_name("bar").is_empty());

        // Verify mtime was updated
        assert_eq!(store.get_mtime("test.rs").unwrap(), Some(2000));
    }

    #[test]
    fn test_cache_version() {
        let dir = tempdir().unwrap();

        // First open sets version
        let store = GraphStore::open(dir.path()).unwrap();
        drop(store);

        // Second open should succeed with same version
        let store2 = GraphStore::open(dir.path()).unwrap();
        drop(store2);
    }

    #[test]
    fn test_remove_file() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let node = CodeNode::new("foo", "foo", NodeKind::Function, "test.rs");
        store.update_file("test.rs", &[node], 1000).unwrap();

        // Verify file exists
        assert!(store.get_mtime("test.rs").unwrap().is_some());
        assert!(store.get_file_nodes("test.rs").unwrap().is_some());

        // Remove file
        store.remove_file("test.rs").unwrap();

        // Verify file is gone
        assert!(store.get_mtime("test.rs").unwrap().is_none());
        assert!(store.get_file_nodes("test.rs").unwrap().is_none());
    }

    #[test]
    fn test_list_cached_files() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let node1 = CodeNode::new("foo", "foo", NodeKind::Function, "a.rs");
        let node2 = CodeNode::new("bar", "bar", NodeKind::Function, "b.rs");

        store.update_file("a.rs", &[node1], 1000).unwrap();
        store.update_file("b.rs", &[node2], 2000).unwrap();

        let files = store.list_cached_files().unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"a.rs".to_string()));
        assert!(files.contains(&"b.rs".to_string()));
    }
}
