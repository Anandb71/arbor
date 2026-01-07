use crate::graph::NodeId;
use std::collections::HashMap;
use std::path::PathBuf;

/// A global symbol table for resolving cross-file references.
///
/// Maps Fully Qualified Names (FQNs) to Node IDs.
/// Example FQN: "arbor::graph::SymbolTable" -> NodeId(42)
#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    /// Map of FQN to NodeId
    by_fqn: HashMap<String, NodeId>,

    /// Map of File Path to list of exported symbols (FQNs)
    /// Used to resolve wildcard imports or find all symbols in a file.
    exports_by_file: HashMap<PathBuf, Vec<String>>,
}

impl SymbolTable {
    /// Creates a new empty symbol table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a symbol in the table.
    ///
    /// * `fqn` - Fully Qualified Name (e.g., "pkg.module.function")
    /// * `id` - The Node ID in the graph
    /// * `file` - The file path defining this symbol
    pub fn insert(&mut self, fqn: String, id: NodeId, file: PathBuf) {
        self.by_fqn.insert(fqn.clone(), id);
        self.exports_by_file.entry(file).or_default().push(fqn);
    }

    /// Resolves a Fully Qualified Name to a Node ID.
    pub fn resolve(&self, fqn: &str) -> Option<NodeId> {
        self.by_fqn.get(fqn).copied()
    }

    /// Returns all symbols exported by a file.
    pub fn get_file_exports(&self, file: &PathBuf) -> Option<&Vec<String>> {
        self.exports_by_file.get(file)
    }

    /// Clears the symbol table.
    pub fn clear(&mut self) {
        self.by_fqn.clear();
        self.exports_by_file.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_resolve() {
        let mut table = SymbolTable::new();
        let path = PathBuf::from("main.rs");
        let id = NodeId::new(1);

        table.insert("main::foo".to_string(), id, path.clone());

        assert_eq!(table.resolve("main::foo"), Some(id));
        assert_eq!(table.resolve("main::bar"), None);

        let exports = table.get_file_exports(&path).unwrap();
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0], "main::foo");
    }
}
