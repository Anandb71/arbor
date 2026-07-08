//! Git changed-file detection for MCP blast radius.

use std::path::Path;
use std::process::Command;

const GENERATED_PATTERNS: &[&str] = &[
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    ".arbor/",
    "package-lock.json",
    "Cargo.lock",
    "generated/",
    ".min.js",
    ".min.css",
];

fn normalize_slashes(input: &str) -> String {
    input.replace('\\', "/")
}

fn is_generated_or_internal_path(path: &str) -> bool {
    let norm = normalize_slashes(path);
    GENERATED_PATTERNS
        .iter()
        .any(|p| norm.contains(p) || norm.ends_with(p.trim_end_matches('/')))
}

fn run_git(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(|e| format!("git not available: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_git_name_status_output(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let path = if parts[0].starts_with('R') && parts.len() >= 3 {
                    parts[2]
                } else {
                    parts[1]
                };
                Some(normalize_slashes(path))
            } else {
                None
            }
        })
        .collect()
}

fn parse_numstat_files(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let adds: i32 = parts[0].parse().unwrap_or(0);
                let dels: i32 = parts[1].parse().unwrap_or(0);
                if adds > 0 || dels > 0 {
                    Some(normalize_slashes(parts[2]))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

/// List changed files in a git repo (unstaged + staged + untracked).
pub fn list_changed_files(project_root: &Path) -> Result<Vec<String>, String> {
    if !project_root.join(".git").exists() {
        return Err("Not a git repository".to_string());
    }

    if let (Ok(base), Ok(head)) = (
        std::env::var("ARBOR_DIFF_BASE"),
        std::env::var("ARBOR_DIFF_HEAD"),
    ) {
        if !base.is_empty() && !head.is_empty() {
            let diff = run_git(
                project_root,
                &[
                    "diff",
                    "-w",
                    "--name-status",
                    "--find-renames",
                    &format!("{}..{}", base, head),
                ],
            )?;
            let mut files = parse_git_name_status_output(&diff);
            files.retain(|p| !is_generated_or_internal_path(p));
            files.sort();
            files.dedup();
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
        assert!(!is_generated_or_internal_path("src/main.rs"));
    }

    #[test]
    fn parse_rename_line() {
        let out = "R100\tsrc/old.rs\tsrc/new.rs\n";
        let files = parse_git_name_status_output(out);
        assert_eq!(files, vec!["src/new.rs"]);
    }
}
