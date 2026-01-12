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

    /// Resolves a symbol name with context-aware matching.
    ///
    /// Resolution order:
    /// 1. Exact FQN match
    /// 2. Suffix match (e.g., "helper" matches "pkg.Utils.helper")
    ///    - Only matches if unambiguous OR in same directory as `context_file`
    ///
    /// Returns None if:
    /// - No match found
    /// - Multiple matches exist and none are in the same directory (ambiguous)
    pub fn resolve_with_context(
        &self,
        name: &str,
        context_file: &std::path::Path,
    ) -> Option<NodeId> {
        // 1. Try exact match first
        if let Some(id) = self.by_fqn.get(name) {
            return Some(*id);
        }

        // 2. Suffix match
        let context_dir = context_file.parent();
        let mut candidates: Vec<(&String, NodeId, bool)> = Vec::new();

        for (fqn, &id) in &self.by_fqn {
            // Check if FQN ends with the name (with separator)
            if fqn.ends_with(name) {
                // Ensure it's a proper suffix (preceded by separator or start)
                let prefix_len = fqn.len() - name.len();
                if prefix_len == 0
                    || fqn.chars().nth(prefix_len - 1) == Some('.')
                    || fqn.chars().nth(prefix_len - 1) == Some(':')
                {
                    // Check if in same directory
                    let same_dir = self
                        .exports_by_file
                        .iter()
                        .find(|(_, exports)| exports.contains(fqn))
                        .map(|(file, _)| file.parent() == context_dir)
                        .unwrap_or(false);

                    candidates.push((fqn, id, same_dir));
                }
            }
        }

        match candidates.len() {
            0 => None,
            1 => Some(candidates[0].1),
            _ => {
                // Multiple candidates: only resolve if exactly one is in same directory
                let same_dir_candidates: Vec<_> =
                    candidates.iter().filter(|(_, _, same)| *same).collect();
                if same_dir_candidates.len() == 1 {
                    Some(same_dir_candidates[0].1)
                } else {
                    // Ambiguous: don't auto-link
                    None
                }
            }
        }
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

    #[test]
    fn test_resolve_with_context_exact_match() {
        let mut table = SymbolTable::new();
        let path = PathBuf::from("src/utils.rs");
        let id = NodeId::new(1);

        table.insert("pkg.utils.helper".to_string(), id, path.clone());

        // Exact match works from any context
        let result =
            table.resolve_with_context("pkg.utils.helper", &PathBuf::from("other/file.rs"));
        assert_eq!(result, Some(id));
    }

    #[test]
    fn test_resolve_with_context_suffix_match() {
        let mut table = SymbolTable::new();
        let path = PathBuf::from("src/utils.rs");
        let id = NodeId::new(1);

        table.insert("pkg.utils.helper".to_string(), id, path.clone());

        // Suffix match works when unambiguous
        let result = table.resolve_with_context("helper", &PathBuf::from("other/file.rs"));
        assert_eq!(result, Some(id));
    }

    #[test]
    fn test_resolve_with_context_ambiguous_returns_none() {
        let mut table = SymbolTable::new();
        let id1 = NodeId::new(1);
        let id2 = NodeId::new(2);

        // Two helpers in different directories
        table.insert(
            "pkg.a.helper".to_string(),
            id1,
            PathBuf::from("src/a/mod.rs"),
        );
        table.insert(
            "pkg.b.helper".to_string(),
            id2,
            PathBuf::from("src/b/mod.rs"),
        );

        // Ambiguous: from unrelated directory, should return None
        let result = table.resolve_with_context("helper", &PathBuf::from("src/c/caller.rs"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_with_context_locality_preference() {
        let mut table = SymbolTable::new();
        let id1 = NodeId::new(1);
        let id2 = NodeId::new(2);

        // Two helpers in different directories
        table.insert(
            "pkg.a.helper".to_string(),
            id1,
            PathBuf::from("src/a/mod.rs"),
        );
        table.insert(
            "pkg.b.helper".to_string(),
            id2,
            PathBuf::from("src/b/mod.rs"),
        );

        // From src/a/, should resolve to id1 (same directory)
        let result = table.resolve_with_context("helper", &PathBuf::from("src/a/caller.rs"));
        assert_eq!(result, Some(id1));

        // From src/b/, should resolve to id2 (same directory)
        let result = table.resolve_with_context("helper", &PathBuf::from("src/b/caller.rs"));
        assert_eq!(result, Some(id2));
    }
}
