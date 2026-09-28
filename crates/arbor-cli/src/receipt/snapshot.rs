//! Whole-working-tree snapshots as git tree objects.
//!
//! A turn's changes must be isolated even when the tree was already dirty, so
//! the start of a turn is recorded as a tree object built through a private
//! index. Nothing touches the user's index, stash, branches or files.

use std::path::Path;
use std::process::Command;

use super::Result;

fn git(root: &Path, args: &[&str], index: Option<&Path>) -> Result<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(root);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let output = command.output()?;
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

/// Write the working tree (tracked and untracked files, honouring .gitignore,
/// without `.arbor/`) as a tree object and return its id.
pub fn snapshot(root: &Path) -> Result<String> {
    // The private index lives in the git directory, never in the working tree,
    // so it can't end up inside the snapshot it builds.
    let git_path = |name: &str| -> Result<std::path::PathBuf> {
        Ok(root.join(git(root, &["rev-parse", "--git-path", name], None)?.trim()))
    };
    let index = git_path(&format!("arbor-snapshot-{}.index", std::process::id()))?;
    // Seeding from the real index keeps its stat cache, so unchanged files are
    // not re-hashed on large repositories.
    let real = git_path("index")?;
    if real.exists() {
        std::fs::copy(&real, &index)?;
    } else {
        let _ = std::fs::remove_file(&index);
    }
    // Naming `.arbor` in an exclude pathspec fails when it is already
    // gitignored, so add everything and then drop it from the private index.
    let tree = git(root, &["add", "-A", "--", "."], Some(&index))
        .and_then(|_| {
            git(
                root,
                &[
                    "rm",
                    "-r",
                    "-f",
                    "--cached",
                    "--quiet",
                    "--ignore-unmatch",
                    "--",
                    ".arbor",
                ],
                Some(&index),
            )
        })
        .and_then(|_| git(root, &["write-tree"], Some(&index)));
    let _ = std::fs::remove_file(&index);
    Ok(tree?.trim().to_string())
}

/// Whether git still has a snapshot. Nothing references these trees, so
/// `git gc` prunes them once they are old enough (two weeks by default).
pub fn exists(root: &Path, tree: &str) -> bool {
    git(root, &["cat-file", "-e", &format!("{tree}^{{tree}}")], None).is_ok()
}

fn blob(root: &Path, tree: &str, file: &str) -> Option<String> {
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tree}:{file}"),
        ],
        None,
    )
    .ok()
    .map(|id| id.trim().to_string())
}

/// True when `file` in the working tree is exactly as the snapshot recorded
/// it, including when neither has it.
pub fn unchanged_since(root: &Path, tree: &str, file: &str) -> Result<bool> {
    let on_disk = root.join(file);
    Ok(match (blob(root, tree, file), on_disk.is_file()) {
        (None, false) => !on_disk.exists(),
        (Some(recorded), true) => git(root, &["hash-object", "--", file], None)?.trim() == recorded,
        _ => false,
    })
}

/// Put `file` back as `tree` recorded it, removing it when the snapshot does
/// not have it. A rename is undone by removing the new path and restoring the
/// old one. Only the working tree changes; the index is left alone.
pub fn restore(root: &Path, tree: &str, file: &str, old_path: Option<&str>) -> Result<()> {
    for path in old_path.into_iter().chain(std::iter::once(file)) {
        if blob(root, tree, path).is_some() {
            git(
                root,
                &[
                    "restore",
                    &format!("--source={tree}"),
                    "--worktree",
                    "--",
                    &format!(":(literal){path}"),
                ],
                None,
            )?;
        } else if root.join(path).is_file() {
            std::fs::remove_file(root.join(path))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
}

/// Files that differ between two snapshots, with line counts.
pub fn changes(root: &Path, from: &str, to: &str) -> Result<Vec<FileChange>> {
    let status = git(root, &["diff", "--name-status", "-M", "-z", from, to], None)?;
    let numstat = git(root, &["diff", "--numstat", "-M", "-z", from, to], None)?;
    let counts = parse_numstat(&numstat);
    Ok(parse_name_status(&status)
        .into_iter()
        .map(|mut change| {
            if let Some((additions, deletions)) = counts.get(&change.path) {
                change.additions = *additions;
                change.deletions = *deletions;
            }
            change
        })
        .collect())
}

/// The unified diff of one file between two snapshots.
pub fn patch(root: &Path, from: &str, to: &str, file: &str) -> Result<String> {
    git(root, &["diff", "-M", from, to, "--", file], None)
}

fn parse_name_status(raw: &str) -> Vec<FileChange> {
    let mut fields = raw.split('\0').filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        let kind = match status.chars().next() {
            Some('A') => ChangeKind::Added,
            Some('D') => ChangeKind::Deleted,
            Some('R') | Some('C') => ChangeKind::Renamed,
            _ => ChangeKind::Modified,
        };
        let (old_path, path) = if kind == ChangeKind::Renamed {
            let old = fields.next().map(str::to_string);
            (old, fields.next())
        } else {
            (None, fields.next())
        };
        if let Some(path) = path {
            changes.push(FileChange {
                path: path.replace('\\', "/"),
                old_path,
                kind,
                additions: 0,
                deletions: 0,
            });
        }
    }
    changes
}

fn parse_numstat(raw: &str) -> std::collections::HashMap<String, (u32, u32)> {
    let mut counts = std::collections::HashMap::new();
    let mut fields = raw.split('\0');
    while let Some(record) = fields.next() {
        let mut parts = record.splitn(3, '\t');
        let (Some(additions), Some(deletions), Some(path)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // A rename prints an empty path, then the old and new paths as fields.
        let path = if path.is_empty() {
            let _old = fields.next();
            fields.next().unwrap_or_default().to_string()
        } else {
            path.to_string()
        };
        // Binary files report "-".
        counts.insert(
            path.replace('\\', "/"),
            (
                additions.parse().unwrap_or(0),
                deletions.parse().unwrap_or(0),
            ),
        );
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_handles_renames_and_spaces() {
        let raw = "M\0src/app page.ts\0R087\0old/a.ts\0new/a.ts\0A\0b.ts\0D\0c.ts\0";
        let changes = parse_name_status(raw);
        assert_eq!(changes.len(), 4);
        assert_eq!(changes[0].path, "src/app page.ts");
        assert_eq!(changes[1].kind, ChangeKind::Renamed);
        assert_eq!(changes[1].old_path.as_deref(), Some("old/a.ts"));
        assert_eq!(changes[1].path, "new/a.ts");
        assert_eq!(changes[2].kind, ChangeKind::Added);
        assert_eq!(changes[3].kind, ChangeKind::Deleted);
    }

    #[test]
    fn numstat_reads_counts_renames_and_binaries() {
        let raw = "3\t1\tsrc/a.ts\x002\t0\t\0old.ts\0new.ts\0-\t-\tlogo.png\0";
        let counts = parse_numstat(raw);
        assert_eq!(counts["src/a.ts"], (3, 1));
        assert_eq!(counts["new.ts"], (2, 0));
        assert_eq!(counts["logo.png"], (0, 0));
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec!["config", "core.autocrlf", "false"],
        ] {
            git(dir.path(), &args, None).unwrap();
        }
        dir
    }

    #[test]
    fn works_when_arbor_is_gitignored() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(
            root.join(".gitignore"),
            ".arbor/
",
        )
        .unwrap();
        std::fs::write(
            root.join("a.txt"),
            "a
",
        )
        .unwrap();
        git(root, &["add", "."], None).unwrap();
        git(root, &["commit", "-qm", "base"], None).unwrap();
        std::fs::create_dir_all(root.join(".arbor").join("receipts")).unwrap();
        let start = snapshot(root).unwrap();
        std::fs::write(
            root.join("a.txt"),
            "a
b
",
        )
        .unwrap();
        let end = snapshot(root).unwrap();
        let changed = changes(root, &start, &end).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].path, "a.txt");
    }

    #[test]
    fn undo_restores_each_kind_of_change_and_notices_later_edits() {
        let dir = repo();
        let root = dir.path();
        let read = |name: &str| std::fs::read_to_string(root.join(name)).ok();
        std::fs::write(root.join("kept.txt"), "one\n").unwrap();
        std::fs::write(root.join("moved.txt"), "same\n").unwrap();
        git(root, &["add", "."], None).unwrap();
        git(root, &["commit", "-qm", "base"], None).unwrap();
        // Untracked before the turn: undo must still bring it back.
        std::fs::write(root.join("gone [1].txt"), "bye\n").unwrap();
        let before = snapshot(root).unwrap();

        std::fs::write(root.join("kept.txt"), "two\n").unwrap();
        std::fs::remove_file(root.join("gone [1].txt")).unwrap();
        std::fs::write(root.join("new.txt"), "hi\n").unwrap();
        std::fs::rename(root.join("moved.txt"), root.join("renamed.txt")).unwrap();
        let after = snapshot(root).unwrap();

        for file in [
            "kept.txt",
            "gone [1].txt",
            "new.txt",
            "renamed.txt",
            "moved.txt",
        ] {
            assert!(unchanged_since(root, &after, file).unwrap(), "{file}");
        }
        std::fs::write(root.join("kept.txt"), "three\n").unwrap();
        assert!(!unchanged_since(root, &after, "kept.txt").unwrap());
        std::fs::write(root.join("moved.txt"), "recreated\n").unwrap();
        assert!(!unchanged_since(root, &after, "moved.txt").unwrap());
        std::fs::remove_file(root.join("moved.txt")).unwrap();

        restore(root, &before, "kept.txt", None).unwrap();
        restore(root, &before, "gone [1].txt", None).unwrap();
        restore(root, &before, "new.txt", None).unwrap();
        restore(root, &before, "renamed.txt", Some("moved.txt")).unwrap();
        assert_eq!(read("kept.txt").as_deref(), Some("one\n"));
        assert_eq!(read("gone [1].txt").as_deref(), Some("bye\n"));
        assert_eq!(read("new.txt"), None);
        assert_eq!(read("moved.txt").as_deref(), Some("same\n"));
        assert_eq!(read("renamed.txt"), None);
        // The user's index is untouched.
        let staged = git(root, &["diff", "--cached", "--name-only"], None).unwrap();
        assert!(staged.trim().is_empty(), "undo staged files: {staged}");

        assert!(exists(root, &before));
        assert!(!exists(root, "0123456789012345678901234567890123456789"));
    }

    #[test]
    fn a_turn_is_isolated_from_changes_made_before_it() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("before.txt"), "one\n").unwrap();
        git(root, &["add", "."], None).unwrap();
        git(root, &["commit", "-qm", "base"], None).unwrap();
        // Dirty before the turn starts: this must not appear in the receipt.
        std::fs::write(root.join("before.txt"), "one\ntwo\n").unwrap();
        std::fs::create_dir_all(root.join(".arbor").join("receipts")).unwrap();
        let start = snapshot(root).unwrap();
        // The agent's turn.
        std::fs::write(root.join("new file.txt"), "hello\n").unwrap();
        std::fs::write(root.join(".arbor").join("ignored.json"), "{}").unwrap();
        let end = snapshot(root).unwrap();
        let changed = changes(root, &start, &end).unwrap();
        assert_eq!(changed.len(), 1, "{changed:?}");
        assert_eq!(changed[0].path, "new file.txt");
        assert_eq!(changed[0].kind, ChangeKind::Added);
        assert_eq!(changed[0].additions, 1);
        // The user's own index is untouched.
        let staged = git(root, &["diff", "--cached", "--name-only"], None).unwrap();
        assert!(staged.trim().is_empty(), "snapshot staged files: {staged}");
    }
}
