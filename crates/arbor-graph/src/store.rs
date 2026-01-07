use crate::builder::GraphBuilder;
use crate::graph::ArborGraph;
use arbor_core::CodeNode;
use sled::{Batch, Db};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Sled(#[from] sled::Error),
    #[error("Serialization error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("Corrupted data: {0}")]
    Corrupted(String),
}

pub struct GraphStore {
    db: Db,
}

impl GraphStore {
    /// Opens or creates a graph store at the specified path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let db = sled::open(path)?;
        Ok(Self { db })
    }

    /// Updates the nodes for a specific file.
    ///
    /// This operation is atomic: it removes old nodes associated with the file
    /// and inserts the new ones.
    pub fn update_file(&self, file_path: &str, nodes: &[CodeNode]) -> Result<(), StoreError> {
        let file_key = format!("f:{}", file_path);
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

        // 4. Commit batch
        self.db.apply_batch(batch)?;
        self.db.flush()?; // flushing optional for perf, but good for safety
        Ok(())
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

        // Initial update
        store
            .update_file("test.rs", &[node1.clone(), node2.clone()])
            .unwrap();

        // Verify load
        let graph = store.load_graph().unwrap();
        assert_eq!(graph.node_count(), 2);

        // Update with one node removed
        store.update_file("test.rs", &[node1.clone()]).unwrap();
        let graph2 = store.load_graph().unwrap();
        assert_eq!(graph2.node_count(), 1);
        assert!(graph2.find_by_name("foo").len() > 0);
        assert!(graph2.find_by_name("bar").is_empty());
    }
}
