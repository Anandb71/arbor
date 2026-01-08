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
mod edge;
mod graph;
mod impact;
mod query;
mod ranking;

pub mod store;
pub mod symbol_table;

pub use builder::GraphBuilder;
pub use edge::{Edge, EdgeKind, GraphEdge};
pub use graph::{ArborGraph, NodeId};
pub use impact::{AffectedNode, ImpactAnalysis, ImpactDirection, ImpactSeverity};
pub use query::{DependentInfo, ImpactResult, NodeInfo, QueryResult};
pub use ranking::{compute_centrality, CentralityScores};
pub use store::{GraphStore, StoreError};
pub use symbol_table::SymbolTable;
