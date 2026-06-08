//! `arbor hook <harness>` — install Arbor agent directives + hooks into a
//! coding-agent harness (CLAUDE.md, settings.json, etc.).
//!
//! Built to grow: each harness (claude, opencode, codex, cursor, ...) is a
//! [`Harness`] implementation. Today only `claude` exists.

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

    h.apply(&scope)
}
