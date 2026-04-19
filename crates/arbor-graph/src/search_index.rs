//! Search index for fast substring matching.
//!
//! This module provides an inverted index that enables O(k) substring
//! search where k is the number of matches, instead of O(n) linear scan.

use crate::graph::NodeId;
use std::collections::{HashMap, HashSet};

/// Minimum n-gram length for indexing.
const MIN_NGRAM_LEN: usize = 2;

/// Maximum n-gram length for indexing.
const MAX_NGRAM_LEN: usize = 4;

/// An inverted index for fast substring search.
///
/// Uses n-gram indexing to support substring matching. When a name is added,
/// we break it into overlapping n-grams and index each one. During search,
/// we look up the query's n-grams and intersect the results.
#[derive(Debug, Default, Clone)]
pub struct SearchIndex {
    /// Maps lowercased full names to NodeIds for exact match lookup.
    exact_index: HashMap<String, Vec<NodeId>>,
    /// Maps lowercased n-grams to NodeIds for substring search.
    ngram_index: HashMap<String, HashSet<NodeId>>,
}

impl SearchIndex {
    /// Creates a new empty search index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a name into the index.
    pub fn insert(&mut self, name: &str, id: NodeId) {
        let lower = name.to_lowercase();

        // Add to exact index
        self.exact_index.entry(lower.clone()).or_default().push(id);

        // Add to n-gram index
        for ngram in self.generate_ngrams(&lower) {
            self.ngram_index.entry(ngram).or_default().insert(id);
        }
    }

    /// Removes a name from the index.
    pub fn remove(&mut self, name: &str, id: NodeId) {
        let lower = name.to_lowercase();

        // Remove from exact index
        if let Some(ids) = self.exact_index.get_mut(&lower) {
            ids.retain(|&x| x != id);
            if ids.is_empty() {
                self.exact_index.remove(&lower);
            }
        }

        // Remove from n-gram index
        for ngram in self.generate_ngrams(&lower) {
            if let Some(ids) = self.ngram_index.get_mut(&ngram) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.ngram_index.remove(&ngram);
                }
            }
        }
    }

    /// Searches for nodes whose names contain the query substring.
    ///
    /// Returns matching NodeIds sorted for deterministic output.
    pub fn search(&self, query: &str) -> Vec<NodeId> {
        if query.is_empty() {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();

        // For very short queries, fall back to prefix matching on exact index
        if query_lower.len() < MIN_NGRAM_LEN {
            let mut results: Vec<NodeId> = self
                .exact_index
                .iter()
                .filter(|(name, _)| name.starts_with(&query_lower))
                .flat_map(|(_, ids)| ids.iter().copied())
                .collect();
            results.sort();
            results.dedup();
            return results;
        }

        // Generate n-grams for the query
        let query_ngrams: Vec<String> = self.generate_ngrams(&query_lower);

        if query_ngrams.is_empty() {
            return Vec::new();
        }

        // Find candidate nodes by intersecting n-gram matches
        let mut candidates: Option<HashSet<NodeId>> = None;

        for ngram in &query_ngrams {
            if let Some(ids) = self.ngram_index.get(ngram) {
                match &mut candidates {
                    None => candidates = Some(ids.clone()),
                    Some(c) => {
                        c.retain(|id| ids.contains(id));
                    }
                }
            } else {
                // If any n-gram has no matches, the query has no results
                return Vec::new();
            }
        }

        // Filter candidates to ensure full substring match
        // (n-gram intersection can have false positives)
        let mut results: Vec<NodeId> = candidates
            .unwrap_or_default()
            .into_iter()
            .filter(|id| {
                self.exact_index
                    .iter()
                    .any(|(name, ids)| ids.contains(id) && name.contains(&query_lower))
            })
            .collect();

        results.sort();
        results
    }

    /// Generates n-grams for a lowercased string.
    fn generate_ngrams(&self, s: &str) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        let mut ngrams = Vec::new();

        for n in MIN_NGRAM_LEN..=MAX_NGRAM_LEN {
            if chars.len() >= n {
                for i in 0..=(chars.len() - n) {
                    ngrams.push(chars[i..i + n].iter().collect());
                }
            }
        }

        ngrams
    }

    /// Returns the number of unique names indexed.
    pub fn len(&self) -> usize {
        self.exact_index.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.exact_index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use petgraph::graph::NodeIndex;

    fn node_id(n: u32) -> NodeId {
        NodeIndex::new(n as usize)
    }

    #[test]
    fn test_insert_and_search_exact() {
        let mut index = SearchIndex::new();
        index.insert("validate_user", node_id(0));
        index.insert("validate_email", node_id(1));
        index.insert("send_email", node_id(2));

        let results = index.search("validate_user");
        assert_eq!(results, vec![node_id(0)]);
    }

    #[test]
    fn test_search_substring() {
        let mut index = SearchIndex::new();
        index.insert("validate_user", node_id(0));
        index.insert("validate_email", node_id(1));
        index.insert("send_email", node_id(2));

        let results = index.search("validate");
        assert!(results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
        assert!(!results.contains(&node_id(2)));
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut index = SearchIndex::new();
        index.insert("ValidateUser", node_id(0));

        let results = index.search("validateuser");
        assert_eq!(results, vec![node_id(0)]);

        let results = index.search("VALIDATEUSER");
        assert_eq!(results, vec![node_id(0)]);
    }

    #[test]
    fn test_search_middle_substring() {
        let mut index = SearchIndex::new();
        index.insert("get_user_profile", node_id(0));

        let results = index.search("user");
        assert_eq!(results, vec![node_id(0)]);

        let results = index.search("_user_");
        assert_eq!(results, vec![node_id(0)]);
    }

    #[test]
    fn test_remove_from_index() {
        let mut index = SearchIndex::new();
        index.insert("foo", node_id(0));
        index.insert("foobar", node_id(1));

        index.remove("foo", node_id(0));

        let results = index.search("foo");
        assert!(!results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
    }

    #[test]
    fn test_search_no_match() {
        let mut index = SearchIndex::new();
        index.insert("hello", node_id(0));

        let results = index.search("world");
        assert!(results.is_empty());
    }

    #[test]
    fn test_short_query() {
        let mut index = SearchIndex::new();
        index.insert("ab", node_id(0));
        index.insert("abc", node_id(1));
        index.insert("xyz", node_id(2));

        // Query shorter than MIN_NGRAM_LEN uses prefix matching
        let results = index.search("a");
        assert!(results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
        assert!(!results.contains(&node_id(2)));
    }

    #[test]
    fn test_index_len_and_is_empty() {
        let mut index = SearchIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);

        index.insert("foo", node_id(0));
        assert!(!index.is_empty());
        assert_eq!(index.len(), 1);

        index.insert("bar", node_id(1));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn test_duplicate_name_different_ids() {
        let mut index = SearchIndex::new();
        // Two different nodes with the same name (overloads, different files)
        index.insert("process", node_id(0));
        index.insert("process", node_id(1));

        let results = index.search("process");
        assert!(results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
        // len() should still be 1 since it's the same name
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_remove_nonexistent_does_not_panic() {
        let mut index = SearchIndex::new();
        // Removing from empty index should not panic
        index.remove("nonexistent", node_id(99));
        assert!(index.is_empty());
    }

    #[test]
    fn test_search_empty_query() {
        let mut index = SearchIndex::new();
        index.insert("hello", node_id(0));

        // Empty query should return empty (no prefix matches)
        let results = index.search("");
        assert!(results.is_empty());
    }
}
