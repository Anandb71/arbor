//! Cross-file symbol resolution.
//!
//! Maps Fully Qualified Names (FQNs) to graph nodes and resolves bare or
//! partially-qualified references against them.
//!
//! # Determinism
//!
//! Every lookup path in this module is order-independent. Candidate lists are
//! sorted by `(file, node index)` before any pick is made, and no decision ever
//! depends on `HashMap` iteration order — Rust seeds `RandomState` per process,
//! so iterating a map to choose a winner would make graph construction vary
//! between runs of the same binary on the same input.
//!
//! # Collisions
//!
//! An FQN can legitimately map to several nodes: `handler` in twelve route
//! files, `new` on forty structs, `process` in both `utils.py` and `helpers.py`.
//! The table keeps every entry and reports ambiguity to the caller rather than
//! silently overwriting, which would orphan the loser and produce a false
//! negative in blast radius.

use crate::graph::NodeId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One definition of a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEntry {
    pub id: NodeId,
    pub file: PathBuf,
}

/// How a reference was matched, ordered from most to least trustworthy.
///
/// The variant determines the confidence stamped on the resulting edge, so a
/// downstream consumer can tell a certain call from an educated guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Exact FQN match with exactly one definition.
    Exact(NodeId),
    /// Matched a symbol defined in the referencing file.
    SameFile(NodeId),
    /// Suffix match with exactly one definition repo-wide.
    UniqueSuffix(NodeId),
    /// Several candidates; the referencing file's own import statement names
    /// exactly one module, and one candidate came from it.
    ///
    /// Stronger evidence than `SameDir`: the author wrote down which module they
    /// meant. Ranked accordingly in `confidence`.
    ViaImport(NodeId),
    /// Several candidates; exactly one lives in the referencing file's directory.
    SameDir(NodeId),
    /// Several equally-plausible candidates, sorted deterministically.
    Ambiguous(Vec<NodeId>),
    /// No definition in this repository (external or stdlib).
    Unresolved,
}

impl Resolution {
    /// Confidence to stamp on an edge created from this resolution.
    ///
    /// `Ambiguous` is deliberately low rather than zero: the reference does
    /// point at *something* in the repo, we just cannot say which. Consumers
    /// that need certainty should filter on this value.
    pub fn confidence(&self) -> f32 {
        match self {
            Resolution::Exact(_) => 1.0,
            Resolution::SameFile(_) => 0.95,
            Resolution::UniqueSuffix(_) => 0.80,
            Resolution::ViaImport(_) => 0.93,
            Resolution::SameDir(_) => 0.55,
            Resolution::Ambiguous(_) => 0.25,
            Resolution::Unresolved => 0.0,
        }
    }

    /// The single resolved node, if this resolution picked one.
    pub fn node(&self) -> Option<NodeId> {
        match self {
            Resolution::Exact(id)
            | Resolution::SameFile(id)
            | Resolution::UniqueSuffix(id)
            | Resolution::ViaImport(id)
            | Resolution::SameDir(id) => Some(*id),
            Resolution::Ambiguous(_) | Resolution::Unresolved => None,
        }
    }

    /// Every candidate this resolution considered plausible.
    pub fn candidates(&self) -> Vec<NodeId> {
        match self {
            Resolution::Ambiguous(ids) => ids.clone(),
            other => other.node().into_iter().collect(),
        }
    }

    pub fn is_resolved(&self) -> bool {
        !matches!(self, Resolution::Unresolved)
    }
}

/// A global symbol table for resolving cross-file references.
#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    /// FQN → every node defining it.
    by_fqn: HashMap<String, Vec<SymbolEntry>>,

    /// Segment-aligned suffix → FQNs ending with it.
    ///
    /// `pkg.Utils.helper` is indexed under `helper`, `Utils.helper`, and
    /// `pkg.Utils.helper`, so resolving a bare `helper` is an O(1) lookup
    /// instead of a scan over every FQN in the repository.
    by_suffix: HashMap<String, Vec<String>>,

    /// File → FQNs it defines.
    exports_by_file: HashMap<PathBuf, Vec<String>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a symbol definition.
    ///
    /// Repeated FQNs accumulate instead of overwriting.
    pub fn insert(&mut self, fqn: String, id: NodeId, file: PathBuf) {
        let entry = SymbolEntry {
            id,
            file: file.clone(),
        };

        let entries = self.by_fqn.entry(fqn.clone()).or_default();
        // Guard against the same definition being indexed twice (incremental
        // re-parse of an unchanged file).
        if !entries.contains(&entry) {
            entries.push(entry);
        }

        for suffix in segment_suffixes(&fqn) {
            let fqns = self.by_suffix.entry(suffix.to_string()).or_default();
            if !fqns.iter().any(|f| f == &fqn) {
                fqns.push(fqn.clone());
            }
        }

        let exports = self.exports_by_file.entry(file).or_default();
        if !exports.contains(&fqn) {
            exports.push(fqn);
        }
    }

    /// Resolves an exact FQN, but only when the definition is unambiguous.
    ///
    /// Returns `None` for a colliding FQN so the caller can fall back to
    /// context-aware resolution rather than picking a definition at random.
    pub fn resolve(&self, fqn: &str) -> Option<NodeId> {
        match self.by_fqn.get(fqn) {
            Some(entries) if entries.len() == 1 => Some(entries[0].id),
            _ => None,
        }
    }

    /// All nodes defining this exact FQN, in deterministic order.
    pub fn resolve_all(&self, fqn: &str) -> Vec<NodeId> {
        let Some(entries) = self.by_fqn.get(fqn) else {
            return Vec::new();
        };
        let mut entries: Vec<&SymbolEntry> = entries.iter().collect();
        sort_entries(&mut entries);
        entries.into_iter().map(|e| e.id).collect()
    }

    /// Returns all symbols exported by a file.
    pub fn get_file_exports(&self, file: &PathBuf) -> Option<&Vec<String>> {
        self.exports_by_file.get(file)
    }

    pub fn clear(&mut self) {
        self.by_fqn.clear();
        self.by_suffix.clear();
        self.exports_by_file.clear();
    }

    /// Resolves a reference, preferring locality and reporting ambiguity.
    ///
    /// Order:
    ///   1. Exact FQN, unique → `Exact`
    ///   2. Candidate gathering: exact-FQN entries plus every segment-aligned
    ///      suffix match, deduplicated and sorted
    ///   3. Single candidate → `Exact` / `UniqueSuffix`
    ///   4. Exactly one candidate in the referencing file → `SameFile`
    ///   5. Exactly one candidate in the referencing directory → `SameDir`
    ///   6. Otherwise → `Ambiguous`
    pub fn resolve_ref(&self, name: &str, context_file: &Path) -> Resolution {
        self.resolve_ref_with_imports(name, context_file, None)
    }

    /// Resolve a reference, allowing the referencing file's imports to break ties.
    ///
    /// Without `imports`, a bare name matching definitions in several modules
    /// falls through to `Ambiguous` whenever no candidate shares a file or a
    /// directory with the caller — and the builder drops ambiguous edges. On a
    /// codebase that reuses a name across modules that is the common case, not
    /// the rare one: every call to it is discarded, and the module those calls
    /// belonged to reports no callers at all.
    ///
    /// The disambiguating fact is usually written at the top of the file.
    /// `from deep.l0.m14 import value_m14` says exactly which `value_m14` is
    /// meant here. `import_map` already carries that, but only
    /// `apply_import_validation` consumed it, and that runs *after* a candidate
    /// has been chosen — so it never saw the references that were being thrown
    /// away. Consulting it during disambiguation instead of after it is the
    /// whole change.
    ///
    /// `imports` maps a locally-visible name to the module it came from, so an
    /// aliased import (`from x import y as z`) is keyed on `z`.
    pub fn resolve_ref_with_imports(
        &self,
        name: &str,
        context_file: &Path,
        imports: Option<&HashMap<String, String>>,
    ) -> Resolution {
        // A call never crosses languages: a Rust `enqueue` isn't called by a
        // TypeScript `enqueue(...)`. Definitions in another language are not
        // candidates at all, so they can neither win nor make a match ambiguous.
        let family = language_family(context_file);
        let visible = |e: &SymbolEntry| same_family(family, &e.file);

        let exact: Option<Vec<&SymbolEntry>> = self
            .by_fqn
            .get(name)
            .map(|entries| entries.iter().filter(|e| visible(e)).collect::<Vec<_>>())
            .filter(|entries| !entries.is_empty());
        if let Some(entries) = &exact {
            if entries.len() == 1 {
                return Resolution::Exact(entries[0].id);
            }
        }
        let had_exact = exact.is_some();

        let mut candidates: Vec<&SymbolEntry> = Vec::new();
        let mut seen: Vec<NodeId> = Vec::new();

        if let Some(entries) = exact {
            for e in entries {
                if !seen.contains(&e.id) {
                    seen.push(e.id);
                    candidates.push(e);
                }
            }
        }

        if let Some(fqns) = self.by_suffix.get(name) {
            // `by_suffix` values are insertion-ordered Vecs, but sort the FQN
            // list anyway so candidate order never depends on parse order.
            let mut fqns: Vec<&String> = fqns.iter().collect();
            fqns.sort();
            for fqn in fqns {
                if let Some(entries) = self.by_fqn.get(fqn) {
                    for e in entries {
                        if visible(e) && !seen.contains(&e.id) {
                            seen.push(e.id);
                            candidates.push(e);
                        }
                    }
                }
            }
        }

        if candidates.is_empty() {
            return Resolution::Unresolved;
        }

        sort_entries(&mut candidates);

        if candidates.len() == 1 {
            return if had_exact {
                Resolution::Exact(candidates[0].id)
            } else {
                Resolution::UniqueSuffix(candidates[0].id)
            };
        }

        let same_file: Vec<&SymbolEntry> = candidates
            .iter()
            .copied()
            .filter(|e| e.file == context_file)
            .collect();
        if same_file.len() == 1 {
            return Resolution::SameFile(same_file[0].id);
        }

        // The referencing file said which module it meant. Believe it.
        //
        // Checked after same-file (a local definition shadows an import) and
        // before same-directory (a written import is better evidence than mere
        // adjacency). Matching is done on the module path so it works whether
        // the source records `deep.l0.m14` or a path-like `deep/l0/m14`.
        if let Some(map) = imports {
            if let Some(source_module) = map.get(name) {
                let wanted = normalize_module_path(source_module);
                if !wanted.is_empty() {
                    let from_import: Vec<&SymbolEntry> = candidates
                        .iter()
                        .copied()
                        .filter(|e| entry_belongs_to_module(e, &wanted))
                        .collect();
                    if from_import.len() == 1 {
                        return Resolution::ViaImport(from_import[0].id);
                    }
                }
            }
        }

        let context_dir = context_file.parent();
        let same_dir: Vec<&SymbolEntry> = candidates
            .iter()
            .copied()
            .filter(|e| e.file.parent() == context_dir)
            .collect();
        if same_dir.len() == 1 {
            return Resolution::SameDir(same_dir[0].id);
        }

        Resolution::Ambiguous(candidates.into_iter().map(|e| e.id).collect())
    }

    /// Resolves a `::` path reference: `jobs::enqueue`, `Provider::new`,
    /// `crate::db::queries::get_user`, `my_app::app::create`.
    ///
    /// The path says where the definition lives, so it is matched on that
    /// rather than on the bare name:
    ///
    /// - a path some parser recorded verbatim (C++ `ns::fn`) matches directly;
    /// - a capitalised owner is a type, and `Type::new` is stored as `Type.new`;
    /// - anything else is a module, matched against the definition's file
    ///   (`jobs.rs`, `app/mod.rs`). Leading segments are dropped until one
    ///   matches, so a crate-name prefix (`my_app::app::…`) works,
    ///   and a definition re-exported from a submodule is found at lower
    ///   confidence.
    ///
    /// A path into an external crate (`sqlx::query`, `std::fs::read`) matches
    /// nothing here and stays unresolved, instead of binding to an unrelated
    /// local function that shares the last name.
    pub fn resolve_path(&self, path: &str, context_file: &Path) -> Resolution {
        let family = language_family(context_file);
        let visible = |e: &&SymbolEntry| same_family(family, &e.file);

        let verbatim = self.entries_with_suffix(path, &visible);
        if !verbatim.is_empty() {
            return pick(verbatim, context_file, Resolution::Exact);
        }

        let segments: Vec<&str> = path
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty() && !matches!(*s, "crate" | "self" | "super"))
            .collect();
        let Some((name, qualifiers)) = segments.split_last() else {
            return Resolution::Unresolved;
        };
        let Some(owner) = qualifiers.last() else {
            return self.resolve_ref(name, context_file);
        };

        if owner.starts_with(|c: char| c.is_uppercase()) {
            let mut entries = self.entries_with_suffix(&format!("{owner}.{name}"), &visible);
            entries.extend(self.entries_with_suffix(&format!("{owner}::{name}"), &visible));
            // Nothing: an enum variant (`Error::NotFound(e)`) or an external type.
            return pick(entries, context_file, Resolution::Exact);
        }

        // Free functions are stored under their bare name.
        let named: Vec<&SymbolEntry> = self
            .by_fqn
            .get(*name)
            .into_iter()
            .flatten()
            .filter(|e| visible(e))
            .collect();
        for start in 0..qualifiers.len() {
            let wanted = normalize_module_path(&qualifiers[start..].join("."));
            if wanted.is_empty() {
                continue;
            }
            let in_module: Vec<&SymbolEntry> = named
                .iter()
                .copied()
                .filter(|e| module_is(e, &wanted))
                .collect();
            if !in_module.is_empty() {
                return pick(in_module, context_file, Resolution::ViaImport);
            }
            let re_exported: Vec<&SymbolEntry> = named
                .iter()
                .copied()
                .filter(|e| entry_belongs_to_module(e, &wanted))
                .collect();
            if !re_exported.is_empty() {
                return pick(re_exported, context_file, Resolution::UniqueSuffix);
            }
        }
        Resolution::Unresolved
    }

    /// Entries whose FQN is `fqn` or ends with it on a segment boundary.
    fn entries_with_suffix(
        &self,
        fqn: &str,
        visible: &dyn Fn(&&SymbolEntry) -> bool,
    ) -> Vec<&SymbolEntry> {
        let mut fqns: Vec<&String> = self.by_suffix.get(fqn).into_iter().flatten().collect();
        fqns.sort();
        fqns.into_iter()
            .filter_map(|f| self.by_fqn.get(f))
            .flatten()
            .filter(|e| visible(e))
            .collect()
    }

    /// Back-compatible wrapper returning only a confidently-resolved node.
    pub fn resolve_with_context(&self, name: &str, context_file: &Path) -> Option<NodeId> {
        self.resolve_ref(name, context_file).node()
    }

    /// Number of distinct FQNs registered.
    pub fn len(&self) -> usize {
        self.by_fqn.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_fqn.is_empty()
    }
}

/// Reduce a module reference to dot-separated segments for comparison.
///
/// `deep.l0.m14`, `deep/l0/m14`, `./deep/l0/m14.py` and `crate::deep::l0::m14`
/// all normalize to `deep.l0.m14`, so the same check works across the languages
/// this engine parses without a per-language branch.
fn normalize_module_path(raw: &str) -> String {
    let cleaned = raw.replace("::", ".").replace(['/', '\\'], ".");
    let cleaned = cleaned
        .trim_start_matches('.')
        .trim_end_matches(".py")
        .trim_end_matches(".ts")
        .trim_end_matches(".js")
        .trim_end_matches(".rs")
        .trim_end_matches(".go");
    cleaned
        .split('.')
        .filter(|seg| {
            !seg.is_empty()
                && *seg != "index"
                && *seg != "mod"
                && *seg != "crate"
                && *seg != "self"
                && *seg != "super"
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Picks among entries that all match the reference equally well: one is
/// `unique`, otherwise the referencing file, then its directory, break ties.
fn pick(
    mut entries: Vec<&SymbolEntry>,
    context_file: &Path,
    unique: fn(NodeId) -> Resolution,
) -> Resolution {
    sort_entries(&mut entries);
    entries.dedup_by_key(|e| e.id);
    match entries.as_slice() {
        [] => Resolution::Unresolved,
        [only] => unique(only.id),
        _ => {
            let same_file: Vec<&&SymbolEntry> =
                entries.iter().filter(|e| e.file == context_file).collect();
            if same_file.len() == 1 {
                return Resolution::SameFile(same_file[0].id);
            }
            let dir = context_file.parent();
            let same_dir: Vec<&&SymbolEntry> =
                entries.iter().filter(|e| e.file.parent() == dir).collect();
            if same_dir.len() == 1 {
                return Resolution::SameDir(same_dir[0].id);
            }
            Resolution::Ambiguous(entries.into_iter().map(|e| e.id).collect())
        }
    }
}

/// Is this symbol defined in exactly `module` (`jobs.rs`, `jobs/mod.rs`)?
fn module_is(entry: &SymbolEntry, module: &str) -> bool {
    let file_norm = normalize_module_path(&entry.file.to_string_lossy());
    file_norm == module || file_norm.ends_with(&format!(".{module}"))
}

/// Source languages that can call each other directly. Calls are never
/// resolved across families; a name shared by a Rust and a TypeScript
/// definition is two symbols, not one.
fn language_family(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => "js",
        "py" | "pyi" => "python",
        "go" => "go",
        "java" | "kt" | "kts" | "scala" => "jvm",
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "m" | "mm" => "c",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" | "phtml" => "php",
        "swift" => "swift",
        "dart" => "dart",
        "sh" | "bash" | "zsh" => "shell",
        _ => return None,
    })
}

/// Unknown extensions on either side don't filter anything out.
/// Whether two files are in the same language family (an unknown extension
/// matches anything).
pub(crate) fn same_language(a: &Path, b: &Path) -> bool {
    same_family(language_family(a), b)
}

fn same_family(family: Option<&'static str>, file: &Path) -> bool {
    match (family, language_family(file)) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// Did this symbol come from `module`?
///
/// A `SymbolEntry` carries only its id and its file, so the file path is the
/// module identity. Requires a segment-aligned match, so a reference to
/// `l0.m14` is never satisfied by `l0.m1`.
fn entry_belongs_to_module(entry: &SymbolEntry, module: &str) -> bool {
    let file_norm = normalize_module_path(&entry.file.to_string_lossy());
    segment_aligned_contains(&file_norm, module)
}

/// True when `needle` appears in `haystack` on segment boundaries.
fn segment_aligned_contains(haystack: &str, needle: &str) -> bool {
    if haystack == needle {
        return true;
    }
    haystack.starts_with(&format!("{needle}."))
        || haystack.ends_with(&format!(".{needle}"))
        || haystack.contains(&format!(".{needle}."))
}

/// Sorts entries by `(file, node index)` for a total, parse-order-independent order.
fn sort_entries(entries: &mut [&SymbolEntry]) {
    entries.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.id.index().cmp(&b.id.index()))
    });
}

/// Every segment-aligned suffix of an FQN, including the FQN itself.
///
/// `a.b.c` → `["a.b.c", "b.c", "c"]`; `a::b::c` → `["a::b::c", "b::c", "c"]`.
///
/// Only positions immediately after a `.` or `:` qualify, so `helper` never
/// matches the tail of `my_helper`. A suffix that would itself start with a
/// separator (the middle of Rust's `::`) is skipped.
fn segment_suffixes(fqn: &str) -> Vec<&str> {
    let mut out = vec![fqn];
    let bytes = fqn.as_bytes();

    for (i, c) in fqn.char_indices().skip(1) {
        if c == '.' || c == ':' {
            continue;
        }
        // `i` is a char boundary, so `bytes[i - 1]` is either the whole
        // previous char (ASCII) or a UTF-8 continuation byte (>= 0x80),
        // which can never equal `.` or `:`.
        let prev = bytes[i - 1];
        if prev == b'.' || prev == b':' {
            out.push(&fqn[i..]);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nid(n: usize) -> NodeId {
        petgraph::graph::NodeIndex::new(n)
    }

    #[test]
    fn insert_resolve() {
        let mut table = SymbolTable::new();
        let path = PathBuf::from("main.rs");
        table.insert("main::foo".to_string(), nid(1), path.clone());

        assert_eq!(table.resolve("main::foo"), Some(nid(1)));
        assert_eq!(table.resolve("main::bar"), None);
        assert_eq!(table.get_file_exports(&path).unwrap(), &vec!["main::foo"]);
    }

    #[test]
    fn exact_match_from_any_context() {
        let mut table = SymbolTable::new();
        table.insert(
            "pkg.utils.helper".to_string(),
            nid(1),
            PathBuf::from("src/utils.rs"),
        );

        let r = table.resolve_ref("pkg.utils.helper", Path::new("other/file.rs"));
        assert_eq!(r, Resolution::Exact(nid(1)));
        assert_eq!(r.confidence(), 1.0);
    }

    #[test]
    fn unique_suffix_match() {
        let mut table = SymbolTable::new();
        table.insert(
            "pkg.utils.helper".to_string(),
            nid(1),
            PathBuf::from("src/utils.rs"),
        );

        let r = table.resolve_ref("helper", Path::new("other/file.rs"));
        assert_eq!(r, Resolution::UniqueSuffix(nid(1)));
    }

    #[test]
    fn multi_segment_suffix_match() {
        let mut table = SymbolTable::new();
        table.insert(
            "pkg.Utils.helper".to_string(),
            nid(1),
            PathBuf::from("src/utils.rs"),
        );

        let r = table.resolve_ref("Utils.helper", Path::new("other/file.rs"));
        assert_eq!(r, Resolution::UniqueSuffix(nid(1)));
    }

    #[test]
    fn suffix_must_be_segment_aligned() {
        let mut table = SymbolTable::new();
        table.insert(
            "pkg.my_helper".to_string(),
            nid(1),
            PathBuf::from("src/utils.rs"),
        );

        // `helper` is a character-suffix of `my_helper` but not a segment.
        assert_eq!(
            table.resolve_ref("helper", Path::new("other/file.rs")),
            Resolution::Unresolved
        );
    }

    #[test]
    fn ambiguous_reports_all_candidates() {
        let mut table = SymbolTable::new();
        table.insert(
            "pkg.a.helper".to_string(),
            nid(1),
            PathBuf::from("src/a/mod.rs"),
        );
        table.insert(
            "pkg.b.helper".to_string(),
            nid(2),
            PathBuf::from("src/b/mod.rs"),
        );

        let r = table.resolve_ref("helper", Path::new("src/c/caller.rs"));
        assert_eq!(r, Resolution::Ambiguous(vec![nid(1), nid(2)]));
        assert!(r.node().is_none());
        assert_eq!(r.candidates().len(), 2);
    }

    #[test]
    fn locality_preference_same_dir() {
        let mut table = SymbolTable::new();
        table.insert(
            "pkg.a.helper".to_string(),
            nid(1),
            PathBuf::from("src/a/mod.rs"),
        );
        table.insert(
            "pkg.b.helper".to_string(),
            nid(2),
            PathBuf::from("src/b/mod.rs"),
        );

        assert_eq!(
            table.resolve_ref("helper", Path::new("src/a/caller.rs")),
            Resolution::SameDir(nid(1))
        );
        assert_eq!(
            table.resolve_ref("helper", Path::new("src/b/caller.rs")),
            Resolution::SameDir(nid(2))
        );
    }

    #[test]
    fn same_file_beats_same_dir() {
        let mut table = SymbolTable::new();
        table.insert("A.run".to_string(), nid(1), PathBuf::from("src/a.rs"));
        table.insert("B.run".to_string(), nid(2), PathBuf::from("src/b.rs"));

        // Both live in `src/`, so same-dir cannot disambiguate — same-file must.
        assert_eq!(
            table.resolve_ref("run", Path::new("src/a.rs")),
            Resolution::SameFile(nid(1))
        );
    }

    #[test]
    fn colliding_fqn_keeps_both_definitions() {
        let mut table = SymbolTable::new();
        table.insert("process".to_string(), nid(1), PathBuf::from("src/utils.py"));
        table.insert(
            "process".to_string(),
            nid(2),
            PathBuf::from("src/helpers.py"),
        );

        // The old table overwrote here, orphaning one node entirely.
        assert_eq!(table.resolve_all("process"), vec![nid(2), nid(1)]);
        assert_eq!(
            table.resolve("process"),
            None,
            "collision is not unambiguous"
        );

        let r = table.resolve_ref("process", Path::new("src/other.py"));
        assert_eq!(r, Resolution::Ambiguous(vec![nid(2), nid(1)]));
    }

    #[test]
    fn duplicate_insert_is_idempotent() {
        let mut table = SymbolTable::new();
        table.insert("a.b".to_string(), nid(1), PathBuf::from("x.rs"));
        table.insert("a.b".to_string(), nid(1), PathBuf::from("x.rs"));

        assert_eq!(table.resolve_all("a.b"), vec![nid(1)]);
        assert_eq!(
            table
                .get_file_exports(&PathBuf::from("x.rs"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn resolution_confidence_is_ordered() {
        assert!(Resolution::Exact(nid(0)).confidence() > Resolution::SameFile(nid(0)).confidence());
        assert!(
            Resolution::SameFile(nid(0)).confidence()
                > Resolution::UniqueSuffix(nid(0)).confidence()
        );
        assert!(
            Resolution::UniqueSuffix(nid(0)).confidence()
                > Resolution::SameDir(nid(0)).confidence()
        );
        assert!(
            Resolution::SameDir(nid(0)).confidence() > Resolution::Ambiguous(vec![]).confidence()
        );
        assert_eq!(Resolution::Unresolved.confidence(), 0.0);
    }

    #[test]
    fn resolution_is_order_independent() {
        // Same definitions inserted in opposite orders must resolve identically.
        let mut a = SymbolTable::new();
        a.insert("p.x.run".into(), nid(1), PathBuf::from("src/x/mod.rs"));
        a.insert("p.y.run".into(), nid(2), PathBuf::from("src/y/mod.rs"));
        a.insert("p.z.run".into(), nid(3), PathBuf::from("src/z/mod.rs"));

        let mut b = SymbolTable::new();
        b.insert("p.z.run".into(), nid(3), PathBuf::from("src/z/mod.rs"));
        b.insert("p.y.run".into(), nid(2), PathBuf::from("src/y/mod.rs"));
        b.insert("p.x.run".into(), nid(1), PathBuf::from("src/x/mod.rs"));

        let ctx = Path::new("src/other/caller.rs");
        assert_eq!(a.resolve_ref("run", ctx), b.resolve_ref("run", ctx));
    }

    #[test]
    fn segment_suffixes_shapes() {
        assert_eq!(segment_suffixes("a.b.c"), vec!["a.b.c", "b.c", "c"]);
        assert_eq!(segment_suffixes("a::b::c"), vec!["a::b::c", "b::c", "c"]);
        assert_eq!(segment_suffixes("solo"), vec!["solo"]);
    }

    #[test]
    fn unicode_fqn_does_not_panic() {
        let mut table = SymbolTable::new();
        table.insert("módulo.función".to_string(), nid(1), PathBuf::from("a.py"));
        assert_eq!(
            table.resolve_ref("función", Path::new("b.py")),
            Resolution::UniqueSuffix(nid(1))
        );
    }
}

#[cfg(test)]
mod import_resolution_tests {
    use super::*;

    fn nid(i: usize) -> NodeId {
        NodeId::new(i)
    }

    /// The exact shape that returned zero on the torture fixture: one bare name
    /// defined in ten modules, called from an eleventh that imported one of them.
    #[test]
    fn import_breaks_a_tie_that_would_otherwise_be_ambiguous() {
        let mut table = SymbolTable::default();
        for layer in 0usize..10 {
            table.insert(
                format!("deep.l{layer}.m14.value_m14"),
                nid(layer),
                PathBuf::from(format!("/repo/deep/l{layer}/m14.py")),
            );
        }

        let caller = Path::new("/repo/deep/l1/m03.py");

        // Without imports the resolver does NOT report ambiguity — it picks the
        // sibling sitting in the caller's own directory and reports SameDir at
        // 0.55 confidence. That is the real defect: the edge is not dropped, it
        // is confidently attached to the wrong module. Every caller in `l1/`
        // resolves to `l1/m14.py`, which is why `l0/m14.py` ends up with no
        // callers at all while an unrelated layer inherits its centrality.
        let bare = table.resolve_ref("value_m14", caller);
        assert_eq!(
            bare,
            Resolution::SameDir(nid(1)),
            "expected the same-directory sibling to be mis-picked, got {bare:?}"
        );

        // With the import the caller actually wrote, exactly one survives.
        let mut imports = HashMap::new();
        imports.insert("value_m14".to_string(), "deep.l0.m14".to_string());
        let resolved = table.resolve_ref_with_imports("value_m14", caller, Some(&imports));

        match resolved {
            Resolution::ViaImport(id) => {
                assert_eq!(id, nid(0), "resolved to the wrong layer");
            }
            other => panic!("expected ViaImport(l0), got {other:?}"),
        }
    }

    /// A definition in the caller's own file outranks an import of the same name.
    #[test]
    fn local_definition_still_shadows_an_import() {
        let mut table = SymbolTable::default();
        table.insert(
            "a.mod.process".to_string(),
            nid(0),
            PathBuf::from("/repo/a/mod.py"),
        );
        table.insert(
            "b.other.process".to_string(),
            nid(1),
            PathBuf::from("/repo/b/other.py"),
        );

        let caller = Path::new("/repo/a/mod.py");
        let mut imports = HashMap::new();
        imports.insert("process".to_string(), "b.other".to_string());

        let r = table.resolve_ref_with_imports("process", caller, Some(&imports));
        assert!(
            matches!(r, Resolution::SameFile(_)),
            "a local definition must win over an import, got {r:?}"
        );
    }

    #[test]
    fn module_paths_normalize_across_separator_styles() {
        assert_eq!(normalize_module_path("deep/l0/m14.py"), "deep.l0.m14");
        assert_eq!(normalize_module_path("crate::deep::l0::m14"), "deep.l0.m14");
        assert_eq!(normalize_module_path("./deep/l0/m14"), "deep.l0.m14");
    }

    /// `l0.m1` must never satisfy a request for `l0.m14`.
    #[test]
    fn module_matching_is_segment_aligned() {
        assert!(segment_aligned_contains("x.deep.l0.m14", "deep.l0.m14"));
        assert!(!segment_aligned_contains("x.deep.l0.m140", "deep.l0.m14"));
        assert!(!segment_aligned_contains("x.deep.l0.m1", "deep.l0.m14"));
    }
}
