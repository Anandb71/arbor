//! `arbor hook <harness>` — install Arbor agent directives + hooks into a
//! coding-agent harness (CLAUDE.md, settings.json, etc.).
//!
//! Built to grow: each harness (claude, opencode, codex, cursor, ...) is a
//! [`Harness`] implementation. Today only `claude` exists.

use crate::commands::GitAvailability;
use colored::Colorize;
use std::path::{Path, PathBuf};

mod claude;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Where to install directives: a single project, or the user's global config.
#[derive(Debug, Clone)]
pub enum Scope {
    /// Install into a project directory (resolved workspace root).
    Project(PathBuf),
    /// Install into the user's home config (e.g. `~/.claude/`).
    Global,
}

/// A coding-agent harness Arbor can wire itself into.
pub trait Harness {
    /// Apply Arbor directives + hooks for the given scope.
    fn apply(&self, scope: &Scope) -> Result<()>;
}

/// Resolve a harness name to its implementation.
fn lookup(name: &str) -> Option<Box<dyn Harness>> {
    match name.to_lowercase().as_str() {
        "claude" => Some(Box::new(claude::Claude)),
        _ => None,
    }
}

/// Entry point for `arbor hook <harness> [path] [--global]`.
pub fn run(harness: &str, path: &Path, global: bool) -> Result<()> {
    let Some(h) = lookup(harness) else {
        return Err(format!("unknown harness '{harness}'. supported: claude").into());
    };

    let scope = if global {
        Scope::Global
    } else {
        Scope::Project(crate::commands::resolve_project_path(path)?)
    };

    h.apply(&scope)?;

    // Receipts need git; say so once here rather than record nothing on
    // every turn without explanation.
    let probe = match &scope {
        Scope::Project(root) => root.clone(),
        Scope::Global => dirs::home_dir().unwrap_or_default(),
    };
    if let Some(warning) = receipt_warning(&scope, crate::commands::git_availability(&probe)) {
        eprintln!("{} {warning}", "⚠".yellow());
    }
    Ok(())
}

/// Why receipts won't be recorded for this install, if they won't.
fn receipt_warning(scope: &Scope, git: GitAvailability) -> Option<String> {
    match (scope, git) {
        (_, GitAvailability::Repository) => None,
        (_, GitAvailability::Missing) => Some(
            "Receipts need git, but no `git` executable was found on PATH.\n  \
             The hooks are installed and start recording once Git is installed."
                .to_string(),
        ),
        (Scope::Project(root), GitAvailability::NotARepository) => Some(format!(
            "{} is not inside a git repository, so receipts will record nothing there.\n  \
             Run `git init` in it, or run this command in your project's repository.",
            root.display()
        )),
        // A global install runs in whichever project the agent opens.
        (Scope::Global, GitAvailability::NotARepository) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{receipt_warning, Scope};
    use crate::commands::GitAvailability;
    use std::path::PathBuf;

    #[test]
    fn a_project_install_warns_when_receipts_cannot_record() {
        let project = Scope::Project(PathBuf::from("app"));
        assert!(receipt_warning(&project, GitAvailability::Repository).is_none());
        let outside = receipt_warning(&project, GitAvailability::NotARepository).unwrap();
        assert!(
            outside.starts_with("app is not inside a git repository"),
            "{outside}"
        );
        assert_eq!(
            outside.lines().nth(1),
            Some("  Run `git init` in it, or run this command in your project's repository.")
        );
        let missing = receipt_warning(&project, GitAvailability::Missing).unwrap();
        assert!(missing.contains("PATH"), "{missing}");
    }

    #[test]
    fn a_global_install_only_warns_when_git_is_missing() {
        assert!(receipt_warning(&Scope::Global, GitAvailability::NotARepository).is_none());
        assert!(receipt_warning(&Scope::Global, GitAvailability::Missing).is_some());
    }
}
