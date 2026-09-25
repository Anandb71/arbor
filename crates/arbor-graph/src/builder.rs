//! Graph builder for constructing the code graph from parsed nodes.
//!
//! Two-pass construction:
//!   1. Add all nodes — populates symbol table and import map
//!   2. Resolve edges — uses import context to create accurate edges

use crate::edge::{Edge, EdgeKind};
use crate::graph::{ArborGraph, NodeId};
use crate::symbol_table::SymbolTable;
use arbor_core::{clean_type_name, type_relation_ref, CodeNode, NodeKind, TypeRelationKind};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Beyond this many candidate definitions, a bare name carries no information
/// and linking to all of them would swamp the graph with noise.
const MAX_AMBIGUOUS_FANOUT: usize = 4;

/// Tighter fan-out cap for calls whose receiver type is unknown.
const MAX_UNKNOWN_RECEIVER_FANOUT: usize = 2;

/// Confidence multiplier for a call on a receiver we could not type
/// (`userService.findOne()`). The method name is real evidence, but which
/// `findOne` it reaches is a guess.
const UNKNOWN_RECEIVER_PENALTY: f32 = 0.55;

/// Builds an ArborGraph from parsed code nodes.
pub struct GraphBuilder {
    graph: ArborGraph,
    symbol_table: SymbolTable,
    name_to_id: HashMap<String, String>,

    /// Per-file map of locally-bound name → source module specifier.
    /// Built from Import nodes that carry their imported names in `references`.
    ///
    /// Example:
    ///   `import { validate } from '@babel/types'`
    ///   → import_map["file.ts"]["validate"] = "@babel/types"
    ///
    /// Used during edge resolution to verify that a direct call like `validate()`
    /// is indeed an intentional import, not a same-name coincidence.
    import_map: HashMap<String, HashMap<String, String>>,

    /// Namespace import aliases: file → alias → source module.
    ///
    /// Example:
    ///   `import * as types from '@babel/types'`
    ///   → namespace_imports["file.ts"]["types"] = "@babel/types"
    ///
    /// Used to resolve calls like `types.validate()` — though since we now DROP
    /// those calls at parse time, this is reserved for future use when we add
    /// a richer call-site representation.
    namespace_imports: HashMap<String, HashMap<String, String>>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            graph: ArborGraph::new(),
            symbol_table: SymbolTable::new(),
            name_to_id: HashMap::new(),
            import_map: HashMap::new(),
            namespace_imports: HashMap::new(),
        }
    }

    /// Adds nodes from a parsed file to the graph.
    ///
    /// This pass does two things:
    ///   - Adds real code entities (functions, classes, etc.) to the graph and symbol table
    ///   - Processes Import nodes to build the per-file import map
    ///
    /// Call this for all files before calling `build()`.
    pub fn add_nodes(&mut self, nodes: Vec<CodeNode>) {
        for node in nodes {
            // Import nodes carry the import map — process them but don't add to graph
            // (they are not code entities we want in centrality analysis)
            if node.kind == NodeKind::Import {
                let file = node.file.clone();
                let module = node.name.clone();

                for imported_name in &node.references {
                    if let Some(alias) = imported_name.strip_prefix("*as:") {
                        // `import * as alias from 'module'`
                        self.namespace_imports
                            .entry(file.clone())
                            .or_default()
                            .insert(alias.to_string(), module.clone());
                    } else {
                        // `import { name } from 'module'` or `import DefaultName from 'module'`
                        self.import_map
                            .entry(file.clone())
                            .or_default()
                            .insert(imported_name.clone(), module.clone());
                    }
                }
                // Import nodes are intentionally NOT added to the graph.
                // They caused misleading centrality scores (e.g. "psycopg.types.range [Import]
                // 330 callers") because every call to a symbol named after the import path
                // was attributed to the import node itself.
                continue;
            }

            let id_str = node.id.clone();
            let name = node.name.clone();
            let qualified = node.qualified_name.clone();
            let file = PathBuf::from(&node.file);

            let node_idx = self.graph.add_node(node);

            if !qualified.is_empty() {
                self.symbol_table
                    .insert(qualified.clone(), node_idx, file.clone());
            }

            self.name_to_id.insert(name.clone(), id_str.clone());
            self.name_to_id.insert(qualified, id_str);
        }
    }

    /// Resolves references into actual graph edges.
    ///
    /// Each reference `R` from file `F` resolves through
    /// [`SymbolTable::resolve_ref`], which reports *how* it matched. That
    /// quality becomes the edge's confidence, so a downstream consumer can
    /// distinguish a proven call from an educated guess instead of treating
    /// every edge as fact.
    ///
    /// Ambiguous references (several equally-plausible definitions) produce a
    /// low-confidence edge to each candidate, capped at
    /// [`MAX_AMBIGUOUS_FANOUT`]. Dropping them outright would silently hide
    /// real call paths; emitting them unlabelled would overstate certainty.
    ///
    /// References with no definition in the repository are dropped — they are
    /// external or stdlib symbols.
    pub fn resolve_edges(&mut self) {
        let mut edges_to_add: Vec<(NodeId, NodeId, f32)> = Vec::new();

        let node_indices: Vec<NodeId> = self.graph.node_indexes().collect();

        for from_idx in node_indices {
            let (references, from_file) = {
                let node = self.graph.get(from_idx).unwrap();
                (node.references.clone(), PathBuf::from(&node.file))
            };

            let from_file_str = from_file.to_string_lossy().to_string();

            for reference in references {
                // Inheritance is not a call. `extends:` / `implements:` and
                // `self.method` / `super.method` are resolved in
                // [`resolve_inheritance`](Self::resolve_inheritance), where an
                // override can hide a base method. Falling through to a bare
                // name here would attach the edge to the wrong definition.
                if type_relation_ref(&reference).is_some() || receiver_method(&reference).is_some()
                {
                    continue;
                }

                // A leading `.` marks a call on a receiver whose type we could
                // not determine (`userService.findOne()`). Resolve it by method
                // name, but never let it claim the confidence of a real match.
                let (lookup, receiver_unknown) = match reference.strip_prefix('.') {
                    Some(method) => (method, true),
                    None => (reference.as_str(), false),
                };

                if lookup.is_empty() {
                    continue;
                }

                // Hand the referencing file's imports to the resolver so a bare
                // name that matches several modules can be pinned to the one
                // the author actually imported. Without this the resolver
                // falls through to SameDir and attaches the edge to whichever
                // definition happens to sit in the caller's own directory.
                let file_imports = self.import_map.get(&from_file_str);
                let resolution =
                    self.symbol_table
                        .resolve_ref_with_imports(lookup, &from_file, file_imports);

                if !resolution.is_resolved() {
                    // The overwhelming majority of references are stdlib or
                    // third-party and have no definition here. Expected, not
                    // notable.
                    debug!(
                        "Unresolved reference '{}' in {} (likely external/stdlib)",
                        reference,
                        from_file.display()
                    );
                    continue;
                }

                let mut base_confidence = resolution.confidence();
                if receiver_unknown {
                    base_confidence *= UNKNOWN_RECEIVER_PENALTY;
                }
                let candidates = resolution.candidates();

                // A receiver-unknown call to a name shared by many symbols
                // (`get`, `run`, `execute`) is pure noise; hold it to a tighter
                // fan-out than a properly-qualified reference.
                let fanout_limit = if receiver_unknown {
                    MAX_UNKNOWN_RECEIVER_FANOUT
                } else {
                    MAX_AMBIGUOUS_FANOUT
                };

                if candidates.len() > fanout_limit {
                    // A name like `new`, `get`, or `run` with dozens of
                    // definitions carries no information. Linking to all of
                    // them would swamp the graph.
                    // Routine on any real codebase — `.get`, `.encode`, `.match`
                    // are shared by many types. Diagnostic, not a user problem.
                    debug!(
                        "Reference '{}' in {} has {} candidate definitions — too ambiguous to link",
                        reference,
                        from_file.display(),
                        candidates.len()
                    );
                    continue;
                }

                for to_idx in candidates {
                    if from_idx == to_idx {
                        continue;
                    }

                    // Method names are never in the import map — importing
                    // `UserService` does not import `findOne` — so import
                    // validation only applies to bare and qualified references.
                    let confidence = if receiver_unknown {
                        base_confidence
                    } else {
                        self.apply_import_validation(
                            &reference,
                            &from_file,
                            &from_file_str,
                            to_idx,
                            base_confidence,
                        )
                    };

                    if confidence <= 0.0 {
                        continue;
                    }

                    edges_to_add.push((from_idx, to_idx, confidence));
                }
            }
        }

        for (from_id, to_id, confidence) in edges_to_add {
            self.graph.add_edge(
                from_id,
                to_id,
                Edge::new(EdgeKind::Calls).with_confidence(confidence),
            );
        }
    }

    /// Downgrades (rather than drops) an edge whose name is not imported.
    ///
    /// If the referencing file declares imports and this bare name is not among
    /// them, a cross-module match is probably a same-name coincidence. The old
    /// behaviour dropped such edges unless the target was in the same directory
    /// — which meant that in a flat `src/` layout, the common case for JS and
    /// Python projects, the check never fired at all. Confidence-weighting
    /// applies the same suspicion uniformly instead of exempting whole layouts.
    ///
    /// Returns `0.0` to drop the edge entirely.
    fn apply_import_validation(
        &self,
        reference: &str,
        from_file: &Path,
        from_file_str: &str,
        to_idx: NodeId,
        base_confidence: f32,
    ) -> f32 {
        let Some(file_imports) = self.import_map.get(from_file_str) else {
            return base_confidence;
        };
        if file_imports.is_empty() || reference.contains('.') {
            return base_confidence;
        }
        if file_imports.contains_key(reference) {
            // Explicitly imported — the strongest possible corroboration.
            return base_confidence.max(0.95);
        }

        let Some(to_node) = self.graph.get(to_idx) else {
            return 0.0;
        };
        let to_file = PathBuf::from(&to_node.file);

        if to_file == from_file {
            // Defined in this very file; imports are irrelevant.
            return base_confidence;
        }

        if to_file.parent() == from_file.parent() {
            // Same directory but never imported. Plausible in languages with
            // implicit module scope, suspicious everywhere else.
            return base_confidence * 0.6;
        }

        // Different module and not imported: almost certainly a name collision.
        debug!(
            "Downgrading unimported cross-module reference '{}' in {} → {}",
            reference,
            from_file.display(),
            to_file.display()
        );
        base_confidence * 0.3
    }

    /// Emits `extends` / `implements` edges and inherited-method reachability.
    ///
    /// A subclass reaches each base it names. A method the subclass does not
    /// define is reachable from the subclass; an override blocks that path.
    /// `self`/`this` calls bind to the nearest definition, including the
    /// enclosing type. `super`/`base` calls start at the parents, so an
    /// override that calls `super` still reaches the base method.
    ///
    /// These type edges are not `Calls`. PageRank stays on the call graph.
    fn resolve_inheritance(&mut self) {
        let indices: Vec<NodeId> = self.graph.node_indexes().collect();
        let types = inherited_types(&self.graph, &indices);
        let (methods_of, method_owner) = index_methods(&self.graph, &indices, &types);
        let mut pending = Vec::new();
        let parents = self.link_type_parents(&types, &mut pending);
        link_inherited_methods(&types, &parents, &methods_of, &mut pending);
        link_receiver_calls(
            &self.graph,
            &indices,
            &method_owner,
            &parents,
            &methods_of,
            &mut pending,
        );
        self.commit_inheritance_edges(pending);
    }

    /// Resolves each written base to a type node and records the parent list.
    fn link_type_parents(
        &self,
        types: &[InheritedType],
        pending: &mut Vec<PendingEdge>,
    ) -> HashMap<NodeId, Vec<NodeId>> {
        let mut parents: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for ty in types {
            let file = PathBuf::from(&ty.file);
            let mut resolved = Vec::new();
            let relations = ty
                .extends
                .iter()
                .map(|name| (TypeRelationKind::Extends, name))
                .chain(
                    ty.implements
                        .iter()
                        .map(|name| (TypeRelationKind::Implements, name)),
                );
            for (recorded, name) in relations {
                let Some(target) = self.resolve_type_name(name, &file) else {
                    continue;
                };
                if target == ty.id || resolved.contains(&target) {
                    continue;
                }
                resolved.push(target);
                pending.push(type_edge(self, ty, recorded, target));
            }
            if !resolved.is_empty() {
                parents.insert(ty.id, resolved);
            }
        }
        parents
    }

    fn commit_inheritance_edges(&mut self, pending: Vec<PendingEdge>) {
        let mut seen: HashSet<(NodeId, NodeId, u8)> = HashSet::new();
        for (from, to, kind, confidence) in pending {
            if from == to || !seen.insert((from, to, edge_tag(kind))) {
                continue;
            }
            self.graph
                .add_edge(from, to, Edge::new(kind).with_confidence(confidence));
        }
    }

    /// Resolves a base or interface name to one type node.
    ///
    /// A path like `pkg.Base` is tried whole, then as its last segment, so a
    /// class recorded under the simple name `Base` still matches. Several
    /// equally plausible types are dropped rather than guessed.
    fn resolve_type_name(&self, name: &str, file: &Path) -> Option<NodeId> {
        let file_str = file.to_string_lossy().to_string();
        let imports = self.import_map.get(&file_str);
        let mut candidates_names = vec![name.to_string()];
        if let Some(last) = name.rsplit(['.', ':']).find(|segment| !segment.is_empty()) {
            if last != name {
                candidates_names.push(last.to_string());
            }
        }

        for candidate in &candidates_names {
            let resolution = self
                .symbol_table
                .resolve_ref_with_imports(candidate, file, imports);
            let type_ids: Vec<NodeId> = resolution
                .candidates()
                .into_iter()
                .filter(|id| {
                    self.graph
                        .get(*id)
                        .is_some_and(|node| is_inheritable_type(node.kind))
                })
                .collect();
            if type_ids.len() == 1 {
                return Some(type_ids[0]);
            }
            let same_file: Vec<NodeId> = type_ids
                .into_iter()
                .filter(|id| {
                    self.graph
                        .get(*id)
                        .is_some_and(|node| Path::new(&node.file) == file)
                })
                .collect();
            if same_file.len() == 1 {
                return Some(same_file[0]);
            }
        }
        None
    }

    /// Finishes building and returns the graph.
    pub fn build(mut self) -> ArborGraph {
        self.resolve_edges();
        self.resolve_inheritance();
        self.graph
    }

    /// Builds without resolving edges (for incremental updates).
    pub fn build_without_resolve(self) -> ArborGraph {
        self.graph
    }
}

struct InheritedType {
    id: NodeId,
    file: String,
    name: String,
    qualified: String,
    extends: Vec<String>,
    implements: Vec<String>,
}

type PendingEdge = (NodeId, NodeId, EdgeKind, f32);

fn inherited_types(graph: &ArborGraph, indices: &[NodeId]) -> Vec<InheritedType> {
    let mut types = Vec::new();
    for id in indices {
        let Some(node) = graph.get(*id) else {
            continue;
        };
        if !is_inheritable_type(node.kind) {
            continue;
        }
        let mut extends = Vec::new();
        let mut implements = Vec::new();
        for reference in &node.references {
            if let Some((kind, name)) = type_relation_ref(reference) {
                match kind {
                    TypeRelationKind::Extends => extends.push(name.to_string()),
                    TypeRelationKind::Implements => implements.push(name.to_string()),
                }
            }
        }
        types.push(InheritedType {
            id: *id,
            file: node.file.clone(),
            name: node.name.clone(),
            qualified: node.qualified_name.clone(),
            extends,
            implements,
        });
    }
    types
}

fn index_methods(
    graph: &ArborGraph,
    indices: &[NodeId],
    types: &[InheritedType],
) -> (
    HashMap<NodeId, BTreeMap<String, NodeId>>,
    HashMap<NodeId, NodeId>,
) {
    let mut methods_of: HashMap<NodeId, BTreeMap<String, NodeId>> = HashMap::new();
    let mut method_owner: HashMap<NodeId, NodeId> = HashMap::new();
    for id in indices {
        let Some(node) = graph.get(*id) else {
            continue;
        };
        if node.kind != NodeKind::Method {
            continue;
        }
        let Some((parent, method_name)) = split_owner(&node.qualified_name) else {
            continue;
        };
        let parent = clean_type_name(parent);
        if parent.is_empty() || method_name.is_empty() {
            continue;
        }
        let owners: Vec<NodeId> = types
            .iter()
            .filter(|ty| {
                ty.file == node.file
                    && (clean_type_name(&ty.name) == parent
                        || clean_type_name(&ty.qualified) == parent)
            })
            .map(|ty| ty.id)
            .collect();
        let Some(owner) = unique_owner(&owners, |id| {
            types
                .iter()
                .find(|ty| ty.id == id)
                .map(|ty| ty.qualified.len())
                .unwrap_or(0)
        }) else {
            continue;
        };
        methods_of
            .entry(owner)
            .or_default()
            .entry(method_name.to_string())
            .or_insert(*id);
        method_owner.insert(*id, owner);
    }
    (methods_of, method_owner)
}

fn type_edge(
    builder: &GraphBuilder,
    ty: &InheritedType,
    recorded: TypeRelationKind,
    target: NodeId,
) -> PendingEdge {
    let target_kind = builder.graph.get(target).map(|node| node.kind);
    let edge_kind = match (recorded, target_kind) {
        (TypeRelationKind::Implements, _) | (_, Some(NodeKind::Interface)) => EdgeKind::Implements,
        _ => EdgeKind::Extends,
    };
    let same_file = builder
        .graph
        .get(target)
        .is_some_and(|node| node.file == ty.file);
    let confidence = if same_file { 0.95 } else { 0.85 };
    (ty.id, target, edge_kind, confidence)
}

fn link_inherited_methods(
    types: &[InheritedType],
    parents: &HashMap<NodeId, Vec<NodeId>>,
    methods_of: &HashMap<NodeId, BTreeMap<String, NodeId>>,
    pending: &mut Vec<PendingEdge>,
) {
    for ty in types {
        let own = methods_of.get(&ty.id);
        for (name, method_id) in inherited_methods(ty.id, parents, methods_of) {
            if own.and_then(|methods| methods.get(&name)).is_some() {
                continue;
            }
            pending.push((ty.id, method_id, EdgeKind::References, 0.9));
        }
    }
}

fn link_receiver_calls(
    graph: &ArborGraph,
    indices: &[NodeId],
    method_owner: &HashMap<NodeId, NodeId>,
    parents: &HashMap<NodeId, Vec<NodeId>>,
    methods_of: &HashMap<NodeId, BTreeMap<String, NodeId>>,
    pending: &mut Vec<PendingEdge>,
) {
    for id in indices {
        let Some(node) = graph.get(*id) else {
            continue;
        };
        if node.kind != NodeKind::Method {
            continue;
        }
        let Some(&owner) = method_owner.get(id) else {
            continue;
        };
        for reference in &node.references {
            let Some((is_super, method_name)) = receiver_method(reference) else {
                continue;
            };
            let Some(target) = nearest_method(owner, method_name, is_super, parents, methods_of)
            else {
                continue;
            };
            if target != *id {
                pending.push((*id, target, EdgeKind::Calls, 0.95));
            }
        }
    }
}

fn is_inheritable_type(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Class | NodeKind::Struct | NodeKind::Interface | NodeKind::Enum
    )
}

/// `Class.method` or `Class::method` → `("Class", "method")`.
fn split_owner(qualified: &str) -> Option<(&str, &str)> {
    if let Some((parent, method)) = qualified.rsplit_once('.') {
        return Some((parent, method));
    }
    qualified.rsplit_once("::")
}

fn unique_owner(owners: &[NodeId], qualified_len: impl Fn(NodeId) -> usize) -> Option<NodeId> {
    match owners {
        [] => None,
        [only] => Some(*only),
        _ => {
            let mut best_len = 0usize;
            let mut best: Option<NodeId> = None;
            let mut tied = false;
            for id in owners {
                let len = qualified_len(*id);
                if len > best_len {
                    best_len = len;
                    best = Some(*id);
                    tied = false;
                } else if len == best_len {
                    tied = true;
                }
            }
            if tied {
                None
            } else {
                best
            }
        }
    }
}

fn inherited_methods(
    start: NodeId,
    parents: &HashMap<NodeId, Vec<NodeId>>,
    methods_of: &HashMap<NodeId, BTreeMap<String, NodeId>>,
) -> BTreeMap<String, NodeId> {
    let mut found = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    if let Some(bases) = parents.get(&start) {
        queue.extend(bases.iter().copied());
    }
    while let Some(current) = queue.pop_front() {
        if !seen.insert(current) {
            continue;
        }
        if let Some(methods) = methods_of.get(&current) {
            for (name, id) in methods {
                found.entry(name.clone()).or_insert(*id);
            }
        }
        if let Some(bases) = parents.get(&current) {
            for base in bases {
                if !seen.contains(base) {
                    queue.push_back(*base);
                }
            }
        }
    }
    found
}

fn nearest_method(
    start: NodeId,
    name: &str,
    skip_self: bool,
    parents: &HashMap<NodeId, Vec<NodeId>>,
    methods_of: &HashMap<NodeId, BTreeMap<String, NodeId>>,
) -> Option<NodeId> {
    let own = if skip_self {
        None
    } else {
        methods_of.get(&start).and_then(|methods| methods.get(name))
    };
    if let Some(id) = own {
        return Some(*id);
    }
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    if let Some(bases) = parents.get(&start) {
        queue.extend(bases.iter().copied());
    }
    while let Some(current) = queue.pop_front() {
        if !seen.insert(current) {
            continue;
        }
        if let Some(id) = methods_of
            .get(&current)
            .and_then(|methods| methods.get(name))
        {
            return Some(*id);
        }
        if let Some(bases) = parents.get(&current) {
            for base in bases {
                if !seen.contains(base) {
                    queue.push_back(*base);
                }
            }
        }
    }
    None
}

/// `self.compute` / `this.compute` search from the enclosing type.
/// `super().compute` / `super.compute` / `base.compute` start at the parents.
fn receiver_method(reference: &str) -> Option<(bool, &str)> {
    const SUPER_PREFIXES: &[&str] = &["super().", "super.", "base."];
    const SAME_PREFIXES: &[&str] = &["self.", "this->", "this.", "Self::"];
    let reference = reference.trim();
    for prefix in SUPER_PREFIXES {
        if let Some(method) = reference.strip_prefix(prefix) {
            if is_plain_method(method) {
                return Some((true, method));
            }
        }
    }
    for prefix in SAME_PREFIXES {
        if let Some(method) = reference.strip_prefix(prefix) {
            if is_plain_method(method) {
                return Some((false, method));
            }
        }
    }
    None
}

fn is_plain_method(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn edge_tag(kind: EdgeKind) -> u8 {
    match kind {
        EdgeKind::Calls => 1,
        EdgeKind::Extends => 2,
        EdgeKind::Implements => 3,
        EdgeKind::References => 4,
        EdgeKind::Imports => 5,
        EdgeKind::UsesType => 6,
        EdgeKind::Contains => 7,
        EdgeKind::FlowsTo => 8,
        EdgeKind::DataDependency => 9,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbor_core::NodeKind;
    use petgraph::visit::{EdgeRef, IntoEdgeReferences};

    #[test]
    fn test_builder_adds_nodes() {
        let mut builder = GraphBuilder::new();
        let node1 = CodeNode::new("foo", "foo", NodeKind::Function, "test.rs");
        let node2 = CodeNode::new("bar", "bar", NodeKind::Function, "test.rs");
        builder.add_nodes(vec![node1, node2]);
        let graph = builder.build();
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn test_builder_resolves_edges() {
        let mut builder = GraphBuilder::new();
        let caller = CodeNode::new("caller", "caller", NodeKind::Function, "test.rs")
            .with_references(vec!["callee".to_string()]);
        let callee = CodeNode::new("callee", "callee", NodeKind::Function, "test.rs");
        builder.add_nodes(vec![caller, callee]);
        let graph = builder.build();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_cross_file_resolution() {
        let mut builder = GraphBuilder::new();
        let caller = CodeNode::new("main", "main", NodeKind::Function, "main.rs")
            .with_references(vec!["pkg.Utils.helper".to_string()]);
        let mut callee = CodeNode::new("helper", "helper", NodeKind::Method, "utils.rs");
        callee.qualified_name = "pkg.Utils.helper".to_string();
        builder.add_nodes(vec![caller]);
        builder.add_nodes(vec![callee]);
        let graph = builder.build();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(
            graph.edge_count(),
            1,
            "Should resolve cross-file edge via FQN"
        );
    }

    #[test]
    fn test_unresolved_references_no_false_edges() {
        let mut builder = GraphBuilder::new();
        let node = CodeNode::new("caller", "caller", NodeKind::Function, "a.rs")
            .with_references(vec!["nonexistent_function".to_string()]);
        builder.add_nodes(vec![node]);
        let graph = builder.build();
        assert_eq!(graph.node_count(), 1);
        assert_eq!(
            graph.edge_count(),
            0,
            "Unresolved references must not create edges"
        );
    }

    #[test]
    fn test_import_nodes_not_added_to_graph() {
        let mut builder = GraphBuilder::new();
        let import_node = CodeNode::new("./utils", "./utils", NodeKind::Import, "main.ts")
            .with_references(vec!["validate".to_string()]);
        let func = CodeNode::new("main", "main", NodeKind::Function, "main.ts");
        builder.add_nodes(vec![import_node, func]);
        let graph = builder.build();
        // Only the function should be in the graph, not the import node
        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn test_import_map_built_correctly() {
        let mut builder = GraphBuilder::new();
        let import_node =
            CodeNode::new("@babel/types", "@babel/types", NodeKind::Import, "file.ts")
                .with_references(vec!["validate".to_string(), "clone".to_string()]);
        builder.add_nodes(vec![import_node]);
        assert_eq!(
            builder
                .import_map
                .get("file.ts")
                .and_then(|m| m.get("validate")),
            Some(&"@babel/types".to_string())
        );
        assert_eq!(
            builder
                .import_map
                .get("file.ts")
                .and_then(|m| m.get("clone")),
            Some(&"@babel/types".to_string())
        );
    }

    #[test]
    fn test_namespace_import_map() {
        let mut builder = GraphBuilder::new();
        let import_node =
            CodeNode::new("@babel/types", "@babel/types", NodeKind::Import, "file.ts")
                .with_references(vec!["*as:types".to_string()]);
        builder.add_nodes(vec![import_node]);
        assert_eq!(
            builder
                .namespace_imports
                .get("file.ts")
                .and_then(|m| m.get("types")),
            Some(&"@babel/types".to_string())
        );
    }

    #[test]
    fn test_build_empty_graph() {
        let builder = GraphBuilder::new();
        let graph = builder.build();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    /// Confidence of the single edge in the graph.
    fn only_edge_confidence(graph: &ArborGraph) -> f32 {
        let edges = graph.export_edges();
        assert_eq!(edges.len(), 1, "expected exactly one edge");
        graph
            .graph
            .edge_weights()
            .next()
            .expect("edge weight")
            .confidence
    }

    #[test]
    fn exact_fqn_match_is_full_confidence() {
        let mut b = GraphBuilder::new();
        let caller = CodeNode::new("main", "main", NodeKind::Function, "main.rs")
            .with_references(vec!["pkg.Utils.helper".to_string()]);
        let mut callee = CodeNode::new("helper", "helper", NodeKind::Method, "utils.rs");
        callee.qualified_name = "pkg.Utils.helper".to_string();
        b.add_nodes(vec![caller, callee]);
        let graph = b.build();

        assert_eq!(only_edge_confidence(&graph), 1.0);
    }

    #[test]
    fn unknown_receiver_call_creates_a_weak_edge() {
        // `.findOne` — the shape the TS parser used to discard entirely.
        let mut b = GraphBuilder::new();
        let caller = CodeNode::new("handler", "handler", NodeKind::Function, "api.ts")
            .with_references(vec![".findOne".to_string()]);
        let callee = CodeNode::new("findOne", "UserRepo.findOne", NodeKind::Method, "repo.ts");
        b.add_nodes(vec![caller, callee]);
        let graph = b.build();

        let confidence = only_edge_confidence(&graph);
        assert!(
            confidence > 0.0 && confidence < Edge::CONFIDENT_THRESHOLD,
            "edge should exist but not be treated as proven, got {confidence}"
        );
    }

    #[test]
    fn unknown_receiver_with_many_candidates_is_not_linked() {
        // `.get` defined on three different types is noise, not evidence.
        let mut b = GraphBuilder::new();
        let caller = CodeNode::new("handler", "handler", NodeKind::Function, "api.ts")
            .with_references(vec![".get".to_string()]);
        b.add_nodes(vec![caller]);
        for (i, owner) in ["Cache", "Store", "Config"].iter().enumerate() {
            b.add_nodes(vec![CodeNode::new(
                "get",
                format!("{owner}.get"),
                NodeKind::Method,
                format!("m{i}.ts"),
            )]);
        }
        let graph = b.build();

        assert_eq!(
            graph.edge_count(),
            0,
            "over the fan-out cap, so no edge should be guessed"
        );
    }

    #[test]
    fn colliding_symbols_both_remain_reachable() {
        // The old table overwrote one of these, orphaning it with zero callers.
        let mut b = GraphBuilder::new();
        let caller = CodeNode::new("main", "main", NodeKind::Function, "src/main.py")
            .with_references(vec!["process".to_string()]);
        b.add_nodes(vec![caller]);
        b.add_nodes(vec![CodeNode::new(
            "process",
            "process",
            NodeKind::Function,
            "src/utils.py",
        )]);
        b.add_nodes(vec![CodeNode::new(
            "process",
            "process",
            NodeKind::Function,
            "src/helpers.py",
        )]);
        let graph = b.build();

        // Both candidates are linked, each flagged as uncertain.
        assert_eq!(graph.edge_count(), 2);
        for weight in graph.graph.edge_weights() {
            assert!(
                !weight.is_confident(),
                "ambiguous resolution must not claim certainty"
            );
        }
    }

    #[test]
    fn unimported_cross_module_reference_is_downgraded_not_dropped() {
        let mut b = GraphBuilder::new();
        let import = CodeNode::new("./other", "./other", NodeKind::Import, "src/a/main.ts")
            .with_references(vec!["somethingElse".to_string()]);
        let caller = CodeNode::new("main", "main", NodeKind::Function, "src/a/main.ts")
            .with_references(vec!["validate".to_string()]);
        let callee = CodeNode::new("validate", "validate", NodeKind::Function, "src/z/far.ts");
        b.add_nodes(vec![import, caller, callee]);
        let graph = b.build();

        let confidence = only_edge_confidence(&graph);
        assert!(
            confidence > 0.0 && confidence < 0.5,
            "not imported and in another module — weak, got {confidence}"
        );
    }

    #[test]
    fn explicitly_imported_reference_is_strong() {
        let mut b = GraphBuilder::new();
        let import = CodeNode::new("./far", "./far", NodeKind::Import, "src/a/main.ts")
            .with_references(vec!["validate".to_string()]);
        let caller = CodeNode::new("main", "main", NodeKind::Function, "src/a/main.ts")
            .with_references(vec!["validate".to_string()]);
        let callee = CodeNode::new("validate", "validate", NodeKind::Function, "src/z/far.ts");
        b.add_nodes(vec![import, caller, callee]);
        let graph = b.build();

        assert!(
            only_edge_confidence(&graph) >= Edge::CONFIDENT_THRESHOLD,
            "an explicit import is strong corroboration"
        );
    }

    #[test]
    fn same_dir_unimported_reference_is_no_longer_a_free_pass() {
        // Flat `src/` layouts made the old filter a no-op, so same-name
        // collisions sailed through at full strength.
        let mut b = GraphBuilder::new();
        let import = CodeNode::new("./x", "./x", NodeKind::Import, "src/main.ts")
            .with_references(vec!["other".to_string()]);
        let caller = CodeNode::new("main", "main", NodeKind::Function, "src/main.ts")
            .with_references(vec!["validate".to_string()]);
        let callee = CodeNode::new("validate", "validate", NodeKind::Function, "src/sibling.ts");
        b.add_nodes(vec![import, caller, callee]);
        let graph = b.build();

        let confidence = only_edge_confidence(&graph);
        assert!(
            confidence < Edge::CONFIDENT_THRESHOLD,
            "same-dir but unimported should carry doubt, got {confidence}"
        );
    }

    #[test]
    fn build_is_deterministic_across_insertion_orders() {
        let make = |reverse: bool| {
            let mut b = GraphBuilder::new();
            let mut defs = vec![
                CodeNode::new("run", "A.run", NodeKind::Method, "src/a/a.ts"),
                CodeNode::new("run", "B.run", NodeKind::Method, "src/b/b.ts"),
            ];
            if reverse {
                defs.reverse();
            }
            b.add_nodes(vec![CodeNode::new(
                "main",
                "main",
                NodeKind::Function,
                "src/c/main.ts",
            )
            .with_references(vec!["run".to_string()])]);
            b.add_nodes(defs);
            let graph = b.build();
            let mut sig: Vec<String> = graph
                .export_edges()
                .into_iter()
                .map(|e| format!("{}->{}", e.source, e.target))
                .collect();
            sig.sort();
            sig
        };

        assert_eq!(make(false), make(true));
    }

    #[test]
    fn test_qualified_static_call_resolves_to_correct_class() {
        // Regression: `MathUtils.add()` in Calc must link to MathUtils.add, NOT to a
        // same-named `add` on a sibling class. Relies on the parser keeping the class
        // qualifier so the builder gets an exact FQN match.
        let mut b = GraphBuilder::new();
        let caller = CodeNode::new("compute", "Calc.compute", NodeKind::Method, "src/Calc.java")
            .with_references(vec!["MathUtils.add".to_string()]);
        // Same-dir sibling with a colliding bare name — the old bug linked here.
        let sibling = CodeNode::new("add", "Sibling.add", NodeKind::Method, "src/Sibling.java");
        let target = CodeNode::new(
            "add",
            "MathUtils.add",
            NodeKind::Method,
            "src/util/MathUtils.java",
        );
        b.add_nodes(vec![caller, sibling, target]);
        let graph = b.build();

        let compute_idx = graph
            .node_indexes()
            .find(|&i| graph.get(i).unwrap().name == "compute")
            .unwrap();
        let callees = graph.get_callees(compute_idx);
        assert_eq!(callees.len(), 1, "exactly one resolved callee");
        assert_eq!(
            callees[0].qualified_name, "MathUtils.add",
            "static call must resolve to the qualified class, not a same-named sibling"
        );
    }

    fn kinds_between(graph: &ArborGraph, from_q: &str, to_q: &str) -> Vec<EdgeKind> {
        let mut kinds = Vec::new();
        for edge in graph.graph.edge_references() {
            let source = graph.get(edge.source()).unwrap();
            let target = graph.get(edge.target()).unwrap();
            if source.qualified_name == from_q && target.qualified_name == to_q {
                kinds.push(edge.weight().kind);
            }
        }
        kinds
    }

    fn inheritance_fixture() -> ArborGraph {
        let file = "hard/inheritance.py";
        let mut b = GraphBuilder::new();
        b.add_nodes(vec![
            CodeNode::new("Base", "Base", NodeKind::Class, file),
            CodeNode::new("compute", "Base.compute", NodeKind::Method, file),
            CodeNode::new("shared", "Base.shared", NodeKind::Method, file),
            CodeNode::new("Middle", "Middle", NodeKind::Class, file).with_extends(["Base"]),
            CodeNode::new("compute", "Middle.compute", NodeKind::Method, file)
                .with_references(vec!["super().compute".to_string()]),
            CodeNode::new("Derived", "Derived", NodeKind::Class, file).with_extends(["Middle"]),
            CodeNode::new("shared", "Derived.shared", NodeKind::Method, file),
            CodeNode::new("Leaf", "Leaf", NodeKind::Class, file).with_extends(["Derived"]),
            CodeNode::new("run", "Leaf.run", NodeKind::Method, file).with_references(vec![
                "self.compute".to_string(),
                "self.shared".to_string(),
                "len".to_string(),
            ]),
            // Same simple name, no superclass. Must not become an extender.
            CodeNode::new("UserService", "UserService", NodeKind::Class, file),
        ]);
        b.build()
    }

    #[test]
    fn subclass_extends_its_base() {
        let graph = inheritance_fixture();
        assert_eq!(
            kinds_between(&graph, "Middle", "Base"),
            vec![EdgeKind::Extends]
        );
        assert_eq!(
            kinds_between(&graph, "Derived", "Middle"),
            vec![EdgeKind::Extends]
        );
        assert_eq!(
            kinds_between(&graph, "Leaf", "Derived"),
            vec![EdgeKind::Extends]
        );
        assert!(
            kinds_between(&graph, "UserService", "Base").is_empty(),
            "a class with no base is not an extender"
        );

        let base = graph
            .node_indexes()
            .find(|&id| graph.get(id).unwrap().qualified_name == "Base")
            .unwrap();
        let reachers: Vec<_> = graph
            .direct_dependents(base)
            .into_iter()
            .map(|node| node.qualified_name.as_str())
            .collect();
        assert!(
            reachers.contains(&"Middle"),
            "Base is reached by Middle, got {reachers:?}"
        );
        assert!(graph.get_callers(base).is_empty(), "extends is not a call");
    }

    #[test]
    fn inherited_method_reaches_the_defining_class_until_overridden() {
        let graph = inheritance_fixture();

        // Leaf.run's self.compute binds to Middle.compute, not Base.compute.
        assert_eq!(
            kinds_between(&graph, "Leaf.run", "Middle.compute"),
            vec![EdgeKind::Calls]
        );
        assert!(kinds_between(&graph, "Leaf.run", "Base.compute").is_empty());

        // super() in the override still reaches the base method.
        assert_eq!(
            kinds_between(&graph, "Middle.compute", "Base.compute"),
            vec![EdgeKind::Calls]
        );

        // Leaf inherits compute from Middle (the nearest definition).
        assert_eq!(
            kinds_between(&graph, "Leaf", "Middle.compute"),
            vec![EdgeKind::References]
        );
        assert!(kinds_between(&graph, "Leaf", "Base.compute").is_empty());

        // Derived overrides shared, so neither Derived nor Leaf reaches Base.shared.
        assert!(kinds_between(&graph, "Derived", "Base.shared").is_empty());
        assert!(kinds_between(&graph, "Leaf", "Base.shared").is_empty());
        assert_eq!(
            kinds_between(&graph, "Leaf.run", "Derived.shared"),
            vec![EdgeKind::Calls]
        );
        assert!(kinds_between(&graph, "Leaf.run", "Base.shared").is_empty());

        // Middle does not override shared, so it reaches Base.shared.
        assert_eq!(
            kinds_between(&graph, "Middle", "Base.shared"),
            vec![EdgeKind::References]
        );

        let base_compute = graph
            .node_indexes()
            .find(|&id| graph.get(id).unwrap().qualified_name == "Base.compute")
            .unwrap();
        let impact = graph.analyze_impact(base_compute, 5);
        let upstream: Vec<_> = impact
            .upstream
            .iter()
            .map(|node| node.node_info.qualified_name.as_str())
            .collect();
        assert!(
            upstream.contains(&"Leaf"),
            "changing Base.compute must reach Leaf, got {upstream:?}"
        );

        let base_shared = graph
            .node_indexes()
            .find(|&id| graph.get(id).unwrap().qualified_name == "Base.shared")
            .unwrap();
        let shared_impact = graph.analyze_impact(base_shared, 5);
        let direct: Vec<_> = shared_impact
            .direct_only()
            .into_iter()
            .map(|node| node.node_info.qualified_name.clone())
            .collect();
        assert!(
            direct.iter().any(|name| name == "Middle"),
            "Middle inherits shared, got {direct:?}"
        );
        assert!(
            direct
                .iter()
                .all(|name| name != "Derived" && name != "Leaf" && name != "Leaf.run"),
            "Derived's override must not make Base.shared a direct dependency, got {direct:?}"
        );
    }

    #[test]
    fn diamond_inherits_the_nearest_override() {
        let file = "diamond.py";
        let mut b = GraphBuilder::new();
        b.add_nodes(vec![
            CodeNode::new("A", "A", NodeKind::Class, file),
            CodeNode::new("f", "A.f", NodeKind::Method, file),
            CodeNode::new("B", "B", NodeKind::Class, file).with_extends(["A"]),
            CodeNode::new("C", "C", NodeKind::Class, file).with_extends(["A"]),
            CodeNode::new("f", "C.f", NodeKind::Method, file),
            CodeNode::new("D", "D", NodeKind::Class, file).with_extends(["B", "C"]),
        ]);
        let graph = b.build();
        assert_eq!(
            kinds_between(&graph, "D", "C.f"),
            vec![EdgeKind::References],
            "D(B, C) sees C.f before A.f"
        );
        assert!(kinds_between(&graph, "D", "A.f").is_empty());
    }

    #[test]
    fn super_stops_at_the_intermediate_override() {
        let file = "super_chain.py";
        let mut b = GraphBuilder::new();
        b.add_nodes(vec![
            CodeNode::new("A", "A", NodeKind::Class, file),
            CodeNode::new("foo", "A.foo", NodeKind::Method, file),
            CodeNode::new("B", "B", NodeKind::Class, file).with_extends(["A"]),
            CodeNode::new("foo", "B.foo", NodeKind::Method, file),
            CodeNode::new("C", "C", NodeKind::Class, file).with_extends(["B"]),
            CodeNode::new("foo", "C.foo", NodeKind::Method, file)
                .with_references(vec!["super().foo".to_string()]),
        ]);
        let graph = b.build();
        assert_eq!(
            kinds_between(&graph, "C.foo", "B.foo"),
            vec![EdgeKind::Calls],
            "super() in C must reach B's override"
        );
        assert!(
            kinds_between(&graph, "C.foo", "A.foo").is_empty(),
            "super() must not skip B and land on A"
        );
    }
}
