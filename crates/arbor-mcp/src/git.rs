//! Git changed-file detection shared by CLI and MCP.

use std::path::Path;
use std::process::Command;

const GENERATED_SEGMENTS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".arbor",
    ".dart_tool",
    "generated",
];

const GENERATED_SUFFIXES: &[&str] = &[
    ".min.js",
    ".min.css",
    ".g.dart",
    ".generated.rs",
    ".pb.go",
    ".designer.cs",
];

const GENERATED_FILENAMES: &[&str] = &["package-lock.json", "Cargo.lock"];

fn normalize_slashes(input: &str) -> String {
    input.replace('\\', "/")
}

pub fn is_generated_or_internal_path(path: &str) -> bool {
    let normalized = normalize_slashes(path).to_lowercase();
    let with_boundary = format!("/{normalized}/");

    if GENERATED_FILENAMES
        .iter()
        .any(|name| normalized.ends_with(name))
    {
        return true;
    }

    GENERATED_SEGMENTS
        .iter()
        .any(|segment| with_boundary.contains(&format!("/{segment}/")))
        || GENERATED_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
}

fn run_git(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(|e| format!("git not available: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Whether `path` is inside a git work tree (including linked worktrees).
pub fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn parse_git_name_status_output(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }

            let parts: Vec<&str> = trimmed.split('\t').collect();
            if parts.len() < 2 {
                return None;
            }

            let status = parts[0];
            let path = if status.starts_with('R') || status.starts_with('C') {
                parts.get(2).copied().unwrap_or(parts[1])
            } else if status.starts_with('D') {
                return None;
            } else {
                parts[1]
            };

            let normalized = normalize_slashes(path.trim());
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn parse_numstat_files(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            let adds: i32 = parts[0].parse().unwrap_or(0);
            let dels: i32 = parts[1].parse().unwrap_or(0);
            if adds == 0 && dels == 0 {
                return None;
            }
            let normalized = normalize_slashes(parts[2].trim());
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn retain_real_and_source(files: &mut Vec<String>, numstat: &str) {
    let has_real_diff: std::collections::HashSet<String> =
        parse_numstat_files(numstat).into_iter().collect();
    files.retain(|f| has_real_diff.contains(f));
    files.retain(|path| !is_generated_or_internal_path(path));
    files.sort();
    files.dedup();
}

/// List changed files in a git repo (unstaged + staged + untracked).
///
/// Honours `ARBOR_DIFF_BASE` / `ARBOR_DIFF_HEAD` for CI ranged diffs.
pub fn list_changed_files(project_root: &Path) -> Result<Vec<String>, String> {
    if !is_git_repo(project_root) {
        return Err("Not a git repository".to_string());
    }

    let range_base = std::env::var("ARBOR_DIFF_BASE").ok();
    let range_head = std::env::var("ARBOR_DIFF_HEAD").ok();

    if let (Some(base), Some(head)) = (range_base, range_head) {
        let base = base.trim();
        let head = head.trim();

        if !base.is_empty() && !head.is_empty() {
            let ranged = run_git(
                project_root,
                &["diff", "-w", "--name-status", "--find-renames", base, head],
            )?;
            let mut files = parse_git_name_status_output(&ranged);
            let numstat = run_git(
                project_root,
                &["diff", "-w", "--numstat", "--find-renames", base, head],
            )?;
            retain_real_and_source(&mut files, &numstat);
            return Ok(files);
        }
    }

    let mut files = Vec::new();

    let unstaged = run_git(
        project_root,
        &["diff", "-w", "--name-status", "--find-renames", "HEAD"],
    )?;
    files.extend(parse_git_name_status_output(&unstaged));

    let staged = run_git(
        project_root,
        &[
            "diff",
            "--cached",
            "-w",
            "--name-status",
            "--find-renames",
            "HEAD",
        ],
    )?;
    files.extend(parse_git_name_status_output(&staged));

    let numstat_unstaged = run_git(
        project_root,
        &["diff", "-w", "--numstat", "--find-renames", "HEAD"],
    )?;
    let numstat_staged = run_git(
        project_root,
        &[
            "diff",
            "--cached",
            "-w",
            "--numstat",
            "--find-renames",
            "HEAD",
        ],
    )?;
    let has_real_diff: std::collections::HashSet<String> = parse_numstat_files(&numstat_unstaged)
        .into_iter()
        .chain(parse_numstat_files(&numstat_staged))
        .collect();
    files.retain(|f| has_real_diff.contains(f));

    let untracked = run_git(
        project_root,
        &["ls-files", "--others", "--exclude-standard"],
    )?;
    files.extend(
        untracked
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(normalize_slashes),
    );

    files.retain(|path| !is_generated_or_internal_path(path));
    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_generated_paths() {
        assert!(is_generated_or_internal_path("node_modules/foo/bar.js"));
        assert!(is_generated_or_internal_path("target/debug/foo"));
        assert!(is_generated_or_internal_path(".arbor/config.json"));
        assert!(is_generated_or_internal_path("src/models/user.g.dart"));
        assert!(is_generated_or_internal_path("pkg/generated/client.rs"));
        assert!(is_generated_or_internal_path("web/app.min.js"));
        assert!(!is_generated_or_internal_path("src/main.rs"));
        assert!(!is_generated_or_internal_path("src/lib.rs"));
    }

    #[test]
    fn parse_rename_modify_delete() {
        let out = "R100\tsrc/old.rs\tsrc/new.rs\nM\tsrc/lib.rs\nD\tsrc/dead.rs\n";
        let files = parse_git_name_status_output(out);
        assert!(files.contains(&"src/new.rs".to_string()));
        assert!(files.contains(&"src/lib.rs".to_string()));
        assert!(!files.contains(&"src/old.rs".to_string()));
        assert!(!files.contains(&"src/dead.rs".to_string()));
    }
}
