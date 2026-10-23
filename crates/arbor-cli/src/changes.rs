//! What a change set touched, symbol by symbol, and which symbols are new.
//!
//! `arbor diff`, `check` and `summary` used to treat every symbol in a touched
//! file as changed, so adding one function to a sixty-symbol file implicated
//! all sixty and every caller of them, and a purely additive PR rated
//! critical. Here a symbol counts as changed only when a changed line falls
//! inside it. Hunks are taken with no context lines, so the neighbours of an
//! edit aren't pulled in either. A changed symbol is *added* when the base
//! version of its file doesn't define it: nothing existing can call it yet,
//! so it has no blast radius of its own.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use arbor_graph::{ArborGraph, ChangedRange, NodeId};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Git's empty tree, the base of a repository with no commits yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// What to compare against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Uncommitted work (staged, unstaged and untracked) against `HEAD`.
    WorkingTree,
    /// Only what is staged, against `HEAD`.
    Staged,
    /// Everything since this branch left `REF`: committed and uncommitted
    /// work against `git merge-base REF HEAD`, which is what a PR shows.
    Base(String),
}

impl Scope {
    pub(crate) fn from_flags(base: Option<String>, staged: bool) -> Self {
        match (base, staged) {
            (Some(reference), _) => Scope::Base(reference),
            (None, true) => Scope::Staged,
            (None, false) => Scope::WorkingTree,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileStatus {
    Added,
    Modified,
    Renamed,
}

impl FileStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            FileStatus::Added => "Added",
            FileStatus::Modified => "Modified",
            FileStatus::Renamed => "Renamed",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// Changed line ranges in the new version of the file.
    pub ranges: Vec<ChangedRange>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChangeSet {
    pub files: Vec<ChangedFile>,
    pub deleted: Vec<String>,
    /// Human-readable description of what was compared.
    pub description: String,
    /// The revision changes are measured from; `None` before the first commit.
    base: Option<String>,
}

/// Changed symbols, split by whether the base already had them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Symbols {
    /// Symbols that existed at the base and were edited.
    pub modified: Vec<NodeId>,
    /// Symbols the change introduces.
    pub added: Vec<NodeId>,
}

impl ChangeSet {
    /// Collects the change set for `scope`. The `ARBOR_DIFF_BASE` /
    /// `ARBOR_DIFF_HEAD` pair, when both are set, compares those two commits
    /// instead (the form CI integrations already use).
    pub(crate) fn collect(root: &Path, scope: &Scope) -> Result<Self> {
        if let (Ok(base), Ok(head)) = (
            std::env::var("ARBOR_DIFF_BASE"),
            std::env::var("ARBOR_DIFF_HEAD"),
        ) {
            let (base, head) = (base.trim(), head.trim());
            if !base.is_empty() && !head.is_empty() {
                let patch = git(root, &diff_args(&[base, head]))?;
                return Ok(Self::from_patch(
                    root,
                    &patch,
                    Some(base.to_string()),
                    Vec::new(),
                    format!("{} against {}", short(head), short(base)),
                ));
            }
        }

        let head = commit(root, "HEAD");
        let (base, extra, description, with_untracked) = match scope {
            Scope::WorkingTree => (
                head.clone(),
                None,
                "uncommitted changes against HEAD".to_string(),
                true,
            ),
            Scope::Staged => (
                head.clone(),
                Some("--cached"),
                "staged changes against HEAD".to_string(),
                false,
            ),
            Scope::Base(reference) => {
                let target = commit(root, reference)
                    .ok_or_else(|| format!("'{reference}' is not a commit in this repository"))?;
                let head = head
                    .clone()
                    .ok_or("--base needs a commit on the current branch to compare")?;
                let merge_base = git(root, &["merge-base", &target, &head])
                    .map_err(|_| format!("'{reference}' shares no history with HEAD"))?
                    .trim()
                    .to_string();
                (
                    Some(merge_base.clone()),
                    None,
                    format!(
                        "changes since this branch left {reference} (merge base {})",
                        short(&merge_base)
                    ),
                    true,
                )
            }
        };

        let from = base.clone().unwrap_or_else(|| EMPTY_TREE.to_string());
        let mut revs: Vec<&str> = extra.into_iter().collect();
        revs.push(&from);
        let patch = git(root, &diff_args(&revs))?;
        let untracked = if with_untracked {
            untracked_files(root)?
        } else {
            Vec::new()
        };
        Ok(Self::from_patch(root, &patch, base, untracked, description))
    }

    fn from_patch(
        root: &Path,
        patch: &str,
        base: Option<String>,
        untracked: Vec<String>,
        description: String,
    ) -> Self {
        let mut files = Vec::new();
        let mut deleted = Vec::new();
        for entry in parse_patch(patch) {
            match entry {
                PatchEntry::Deleted(path) => deleted.push(path),
                PatchEntry::Changed(file) => files.push(file),
            }
        }
        for path in untracked {
            if files.iter().any(|f| f.path == path) {
                continue;
            }
            let lines = std::fs::read_to_string(root.join(&path))
                .map(|text| text.lines().count().max(1) as u32)
                .unwrap_or(1);
            files.push(ChangedFile {
                ranges: vec![ChangedRange::new(path.clone(), 1, lines)],
                path,
                old_path: None,
                status: FileStatus::Added,
            });
        }

        // `-w` leaves a whitespace-only edit with no hunks: not a change. A
        // rename or a new empty file still is.
        files.retain(|f| {
            !crate::commands::is_generated_or_internal_path(&f.path)
                && (f.status != FileStatus::Modified || !f.ranges.is_empty())
        });
        deleted.retain(|path| !crate::commands::is_generated_or_internal_path(path));
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        deleted.sort();
        deleted.dedup();

        ChangeSet {
            files,
            deleted,
            description,
            base,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub(crate) fn paths(&self) -> Vec<String> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }

    /// The changed symbols, each classified as modified or added.
    pub(crate) fn symbols(&self, graph: &ArborGraph, root: &Path) -> Symbols {
        let mut symbols = Symbols::default();
        for file in &self.files {
            if file.ranges.is_empty() {
                // A pure rename: every symbol moved, none changed.
                continue;
            }
            let hits = arbor_graph::changed_node_ids_for_ranges(graph, &file.ranges, root);
            if hits.node_ids.is_empty() {
                continue;
            }
            // `None` (no base, unreadable or unparseable): count every hit as
            // modified. Overstating is safer than hiding a risky edit.
            let existing = match file.status {
                FileStatus::Added => Some(HashSet::new()),
                _ => self.base_definitions(root, file),
            };
            for id in hits.node_ids {
                let is_new = match (&existing, graph.get(id)) {
                    (Some(names), Some(node)) => !names.contains(&node.qualified_name),
                    _ => false,
                };
                if is_new {
                    symbols.added.push(id);
                } else {
                    symbols.modified.push(id);
                }
            }
        }
        symbols
    }

    /// Qualified names defined in the base version of `file`.
    fn base_definitions(&self, root: &Path, file: &ChangedFile) -> Option<HashSet<String>> {
        let base = self.base.as_ref()?;
        let old = file.old_path.as_deref().unwrap_or(&file.path);
        let source = git(root, &["show", &format!("{base}:{old}")]).ok()?;
        let parser = arbor_core::detect_language(Path::new(old))?;
        let nodes = arbor_core::parse_source(&source, old, parser.as_ref()).ok()?;
        Some(nodes.into_iter().map(|n| n.qualified_name).collect())
    }
}

fn diff_args<'a>(revs: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "diff",
        "-U0",
        "-w",
        "--no-color",
        "--no-ext-diff",
        "--find-renames",
    ];
    args.extend_from_slice(revs);
    args
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The commit `spec` names, if it names one.
fn commit(root: &Path, spec: &str) -> Option<String> {
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{spec}^{{commit}}"),
        ],
    )
    .ok()
    .map(|sha| sha.trim().to_string())
    .filter(|sha| !sha.is_empty())
}

fn short(sha: &str) -> &str {
    if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        &sha[..7]
    } else {
        sha
    }
}

fn untracked_files(root: &Path) -> Result<Vec<String>> {
    Ok(
        git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(|p| p.replace('\\', "/"))
            .collect(),
    )
}

enum PatchEntry {
    Changed(ChangedFile),
    Deleted(String),
}

/// Splits a `git diff -U0` patch into files with their changed ranges.
fn parse_patch(patch: &str) -> Vec<PatchEntry> {
    struct Current {
        old: Option<String>,
        new: Option<String>,
        rename_from: Option<String>,
        rename_to: Option<String>,
        created: bool,
        deleted: bool,
        hunks: Vec<crate::hunk_lines::Hunk>,
        /// Inside a hunk body, where `--- x` is a removed line, not a header.
        in_hunk: bool,
    }
    fn finish(current: Option<Current>, out: &mut Vec<PatchEntry>) {
        let Some(c) = current else {
            return;
        };
        if c.deleted {
            if let Some(path) = c.old.or(c.rename_from) {
                out.push(PatchEntry::Deleted(path));
            }
            return;
        }
        let Some(path) = c.new.or(c.rename_to.clone()) else {
            return;
        };
        let status = if c.created {
            FileStatus::Added
        } else if c.rename_from.is_some() {
            FileStatus::Renamed
        } else {
            FileStatus::Modified
        };
        // Only lines that change code: a hunk adding a doc comment, an
        // attribute and a blank line around a new function touches the
        // function, not the module that holds it.
        let ranges = c
            .hunks
            .iter()
            .flat_map(|hunk| crate::hunk_lines::code_ranges(hunk, &path))
            .collect();
        let old_path = c.rename_from.or(c.old).filter(|old| old != &path);
        out.push(PatchEntry::Changed(ChangedFile {
            path,
            old_path,
            status,
            ranges,
        }));
    }

    let mut out = Vec::new();
    let mut current: Option<Current> = None;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            finish(current.take(), &mut out);
            current = Some(Current {
                old: None,
                new: None,
                rename_from: None,
                rename_to: None,
                created: false,
                deleted: false,
                hunks: Vec::new(),
                in_hunk: false,
            });
            continue;
        }
        let Some(c) = current.as_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            c.in_hunk = true;
            c.hunks.push(crate::hunk_lines::Hunk {
                header: line.to_string(),
                ..Default::default()
            });
        } else if c.in_hunk {
            if let Some(hunk) = c.hunks.last_mut() {
                if let Some(added) = line.strip_prefix('+') {
                    hunk.added.push(added.to_string());
                } else if let Some(removed) = line.strip_prefix('-') {
                    hunk.removed.push(removed.to_string());
                }
            }
        } else if line.starts_with("new file mode") {
            c.created = true;
        } else if line.starts_with("deleted file mode") {
            c.deleted = true;
        } else if let Some(path) = line.strip_prefix("rename from ") {
            c.rename_from = Some(unquote(path));
        } else if let Some(path) = line.strip_prefix("rename to ") {
            c.rename_to = Some(unquote(path));
        } else if let Some(path) = line.strip_prefix("--- ") {
            c.old = side_path(path, "a/");
        } else if let Some(path) = line.strip_prefix("+++ ") {
            c.new = side_path(path, "b/");
        }
    }
    finish(current, &mut out);
    out
}

/// `a/src/lib.rs` → `src/lib.rs`; `/dev/null` → none.
fn side_path(raw: &str, prefix: &str) -> Option<String> {
    let path = unquote(raw.trim_end());
    if path == "/dev/null" {
        return None;
    }
    Some(path.strip_prefix(prefix).unwrap_or(&path).to_string())
}

/// Git quotes a path with unusual characters: `"a/we\"ird.rs"`.
fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return raw.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -3,0 +4,5 @@ fn existing() {
+fn added() {}
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+fn fresh() {}
diff --git a/src/old.rs b/src/gone.rs
deleted file mode 100644
--- a/src/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn old() {}
diff --git a/src/a.rs b/src/b.rs
similarity index 100%
rename from src/a.rs
rename to src/b.rs
diff --git a/src/space.rs b/src/space.rs
index 4444444..5555555 100644
--- a/src/space.rs
+++ b/src/space.rs
diff --git \"a/src/we\\\"ird.rs\" \"b/src/we\\\"ird.rs\"
--- \"a/src/we\\\"ird.rs\"
+++ \"b/src/we\\\"ird.rs\"
@@ -1 +1 @@
";

    #[test]
    fn hunk_bodies_are_not_headers_and_count_only_code() {
        // A removed line `-- old` appears as `--- old`, and an added `++ x`
        // as `+++ x`: neither is a file header inside a hunk.
        let patch = "\
diff --git a/src/q.rs b/src/q.rs
--- a/src/q.rs
+++ b/src/q.rs
@@ -2 +2 @@ mod tests {
--- old
+++ new
@@ -9,0 +10,4 @@ mod tests {
+    /// Documents the test.
+    #[test]
+    fn added() {}
+
";
        let entries = parse_patch(patch);
        let [PatchEntry::Changed(file)] = entries.as_slice() else {
            panic!("one changed file expected, got {}", entries.len());
        };
        assert_eq!(file.path, "src/q.rs");
        assert_eq!(file.old_path, None);
        let ranges: Vec<(u32, u32)> = file
            .ranges
            .iter()
            .map(|r| (r.start_line, r.end_line))
            .collect();
        // The first hunk replaces code; the second counts the function, not
        // its doc comment, its attribute or the blank line.
        assert_eq!(ranges, [(2, 2), (12, 12)]);
    }

    #[test]
    fn patches_split_into_files_statuses_and_ranges() {
        let set = ChangeSet::from_patch(
            Path::new("."),
            PATCH,
            Some("HEAD".into()),
            Vec::new(),
            String::new(),
        );
        type Row = (String, FileStatus, Vec<(u32, u32)>);
        let summary: Vec<Row> = set
            .files
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    f.status,
                    f.ranges
                        .iter()
                        .map(|r| (r.start_line, r.end_line))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("src/b.rs".into(), FileStatus::Renamed, vec![]),
                ("src/lib.rs".into(), FileStatus::Modified, vec![(4, 8)]),
                ("src/new.rs".into(), FileStatus::Added, vec![(1, 2)]),
                ("src/we\"ird.rs".into(), FileStatus::Modified, vec![(1, 1)]),
            ],
            "the whitespace-only src/space.rs has no hunks and is dropped"
        );
        assert_eq!(set.deleted, vec!["src/old.rs".to_string()]);
        let renamed = set.files.iter().find(|f| f.path == "src/b.rs").unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn flags_pick_the_scope() {
        assert_eq!(Scope::from_flags(None, false), Scope::WorkingTree);
        assert_eq!(Scope::from_flags(None, true), Scope::Staged);
        assert_eq!(
            Scope::from_flags(Some("origin/main".into()), false),
            Scope::Base("origin/main".into())
        );
    }
}
