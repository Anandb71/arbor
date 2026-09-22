//! Arbor Graph - Code relationship management
//!
//! This crate manages the graph of code entities and their relationships.
//! It provides fast lookups, traversals, and centrality scoring for
//! prioritizing context in AI queries.
//!
//! # Architecture
//!
//! The graph uses petgraph internally with additional indexes for:
//! - Name-based lookups
//! - File-based grouping (for incremental updates)
//! - Kind-based filtering
//!
//! # Example
//!
//! ```no_run
//! use arbor_graph::ArborGraph;
//! use arbor_core::{CodeNode, NodeKind};
//!
//! let mut graph = ArborGraph::new();
//!
//! // Add nodes from parsing
//! let node = CodeNode::new("validate", "UserService.validate", NodeKind::Method, "user.rs");
//! let id = graph.add_node(node);
//!
//! // Query the graph
//! let matches = graph.find_by_name("validate");
//! ```

mod builder;
mod confidence;
mod diff;
mod edge;
mod graph;
mod heuristics;
mod impact;
pub mod lexicon;
mod query;
mod ranking;
mod search_index;
mod slice;

pub mod store;
pub mod symbol_table;

pub use search_index::{MatchKind, SearchHit, SearchIndex};

pub use builder::GraphBuilder;
pub use confidence::{ConfidenceExplanation, ConfidenceLevel, NodeRole};
pub use diff::{
    changed_node_ids, changed_node_ids_for_ranges, compute_blast_radius, node_matches_changed_file,
    parse_unified_diff_ranges, BlastRadiusSummary, ChangedRange, ChangedSymbols,
};
pub use edge::{Edge, EdgeKind, GraphEdge};
pub use graph::{ArborGraph, NodeId};
pub use heuristics::{
    detect_analysis_limitations, AnalysisWarning, HeuristicsMatcher, UncertainEdge,
    UncertainEdgeKind,
};
pub use impact::{AffectedNode, ImpactAnalysis, ImpactDirection, ImpactSeverity};
pub use lexicon::{stem, tokenize_identifier, Lexicon};
pub use query::{DependentInfo, ImpactResult, NodeInfo, QueryResult};
pub use ranking::{
    compute_centrality, compute_centrality_warm, ensure_centrality, is_noise_path, top_hotspots,
    CentralityScores,
};
pub use slice::{ContextNode, ContextSlice, TruncationReason};
pub use store::{GraphStore, StoreError};
pub use symbol_table::{Resolution, SymbolEntry, SymbolTable};
