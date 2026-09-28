//! `arbor receipt` — a plain-English account of what one agent turn changed.
//!
//! An agent hook calls `begin` when you send a request and `end` when the
//! agent finishes. The receipt says what changed, what it could affect in the
//! app's own terms (pages, API routes, sign in, payments), what it touched that
//! the request did not mention, and what to try by hand before shipping.
//! Everything is computed locally from git and the Arbor graph; no model is
//! called and nothing leaves the machine.

mod app;
mod render;
mod scope;
mod snapshot;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub use app::{Area, RouteKind};
use snapshot::ChangeKind;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How far up the call graph "could affect" looks.
const IMPACT_DEPTH: usize = 4;
const MAX_PROMPT_CHARS: usize = 2000;

#[derive(Debug, Default, Deserialize)]
struct HookInput {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Turn {
    tree: String,
    prompt: String,
    started_at: String,
    agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub change: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
    pub functions: Vec<String>,
    pub areas: Vec<Area>,
    /// `None` when the request was too vague to judge.
    pub in_scope: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Surface {
    pub kind: RouteKind,
    pub route: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Affects {
    pub routes: Vec<Surface>,
    pub areas: Vec<Area>,
    pub entry_points: Vec<String>,
    /// Functions elsewhere that call into the changed code.
    pub callers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub id: String,
    pub agent: String,
    pub prompt: String,
    pub started_at: String,
    pub finished_at: String,
    pub files: Vec<ChangedFile>,
    /// Whether the request could be matched to files at all.
    pub scope_known: bool,
    pub affects: Affects,
    pub tests: Vec<String>,
    /// What this receipt could not check, stated plainly.
    pub limits: Vec<String>,
    /// Snapshot trees from the start and end of the turn, for `arbor receipt undo`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub before: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after: String,
}

impl Receipt {
    pub fn unasked(&self) -> impl Iterator<Item = &ChangedFile> {
        self.files
            .iter()
            .filter(|file| file.in_scope == Some(false))
    }
}

fn receipts_dir(root: &Path) -> PathBuf {
    root.join(".arbor").join("receipts")
}

fn turn_path(root: &Path, session: &str) -> PathBuf {
    let safe: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    receipts_dir(root)
        .join("turns")
        .join(format!("{safe}.json"))
}

fn read_hook_input() -> HookInput {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return HookInput::default();
    }
    let mut raw = String::new();
    if stdin.read_to_string(&mut raw).is_err() {
        return HookInput::default();
    }
    serde_json::from_str(&raw).unwrap_or_default()
}

fn project_root(path: &Path, input: &HookInput) -> Result<PathBuf> {
    // Trust the hook's working directory only when it exists here.
    let start = input
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| path.to_path_buf());
    crate::commands::resolve_project_path(&start)
}

/// Record the start of a turn. Silent by design: it runs on every request.
pub fn begin(path: &Path, agent: &str) -> Result<()> {
    let input = read_hook_input();
    let root = project_root(path, &input)?;
    if !crate::commands::is_git_repo(&root) {
        return Ok(());
    }
    let session = input.session_id.clone().unwrap_or_else(|| "manual".into());
    let tree = snapshot::snapshot(&root)?;
    let prompt: String = input
        .prompt
        .unwrap_or_default()
        .chars()
        .take(MAX_PROMPT_CHARS)
        .collect();
    let turn = Turn {
        tree,
        prompt,
        started_at: now_rfc3339(),
        agent: agent.into(),
    };
    let path = turn_path(&root, &session);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, serde_json::to_vec(&turn)?)?;
    Ok(())
}

/// Finish a turn: compute, save and print its receipt. In hook mode the text
/// goes out as the hook's `systemMessage` so the user sees it, and any failure
/// stays silent rather than interrupting the agent.
pub fn end(path: &Path, hook: bool, json: bool) -> Result<()> {
    let input = read_hook_input();
    let outcome = (|| -> Result<Option<Receipt>> {
        let root = project_root(path, &input)?;
        let session = input.session_id.clone().unwrap_or_else(|| "manual".into());
        let turn_file = turn_path(&root, &session);
        let Ok(raw) = std::fs::read(&turn_file) else {
            return Ok(None);
        };
        // A turn is explained once, even if the agent stops again.
        let _ = std::fs::remove_file(&turn_file);
        let turn: Turn = serde_json::from_slice(&raw)?;
        let receipt = build(&root, &turn)?;
        if let Some(receipt) = &receipt {
            save(&root, receipt)?;
        }
        Ok(receipt)
    })();

    match (outcome, hook) {
        (Ok(Some(receipt)), true) => {
            println!(
                "{}",
                serde_json::json!({ "systemMessage": render::text(&receipt) })
            );
        }
        (Ok(Some(receipt)), false) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            } else {
                println!("{}", render::text(&receipt));
            }
        }
        (Ok(None), false) if !json => println!("No agent turn in progress, or nothing changed."),
        (Err(error), false) => return Err(error),
        (Err(error), true) => eprintln!("arbor receipt end: {error}"),
        _ => {}
    }
    Ok(())
}

fn save(root: &Path, receipt: &Receipt) -> Result<()> {
    let dir = receipts_dir(root);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(format!("{}.json", receipt.id)),
        serde_json::to_vec_pretty(receipt)?,
    )?;
    Ok(())
}

fn supporting_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    [
        "package.json",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lockb",
        "bun.lock",
        "cargo.toml",
        "cargo.lock",
        "requirements.txt",
        "poetry.lock",
        "pyproject.toml",
        "uv.lock",
        "go.mod",
        "go.sum",
        "gemfile",
        "gemfile.lock",
        "composer.json",
        "composer.lock",
        "pubspec.yaml",
        "pubspec.lock",
    ]
    .contains(&name.as_str())
}

fn build(root: &Path, turn: &Turn) -> Result<Option<Receipt>> {
    let now_tree = snapshot::snapshot(root)?;
    if now_tree == turn.tree {
        return Ok(None);
    }
    let changes: Vec<_> = snapshot::changes(root, &turn.tree, &now_tree)?
        .into_iter()
        .filter(|change| !crate::commands::is_generated_or_internal_path(&change.path))
        .collect();
    if changes.is_empty() {
        return Ok(None);
    }

    let mut limits = Vec::new();
    let graph = if crate::commands::project_is_indexed(root)
        || crate::commands::auto_index_enabled()
    {
        match crate::commands::refresh_graph(root) {
            Ok(graph) => Some(graph),
            Err(error) => {
                limits.push(format!(
                    "Couldn't read the code graph ({error}), so only files are listed."
                ));
                None
            }
        }
    } else {
        limits.push("This project isn't indexed yet, so only files are listed. Run `arbor setup` for function-level detail.".into());
        None
    };

    // Changed functions per file, from the lines the turn actually touched.
    let mut functions: HashMap<String, Vec<(String, arbor_graph::NodeId)>> = HashMap::new();
    let mut unparsed = Vec::new();
    if let Some(graph) = &graph {
        let mut ranges = Vec::new();
        for change in changes.iter().filter(|c| c.kind != ChangeKind::Deleted) {
            let patch = snapshot::patch(root, &turn.tree, &now_tree, &change.path)?;
            ranges.extend(arbor_graph::parse_unified_diff_ranges(&patch, &change.path));
        }
        let hits = arbor_graph::changed_node_ids_for_ranges(graph, &ranges, root);
        for id in hits.node_ids {
            if let Some(node) = graph.get(id) {
                let file = changes
                    .iter()
                    .find(|c| arbor_graph::node_matches_changed_file(&node.file, &c.path, root))
                    .map(|c| c.path.clone())
                    .unwrap_or_else(|| node.file.clone());
                functions
                    .entry(file)
                    .or_default()
                    .push((node.name.clone(), id));
            }
        }
        unparsed = hits.files_without_symbol_hits;
    }

    // What the changed functions reach, and which changed files call each other.
    let file_of: HashMap<arbor_graph::NodeId, usize> = changes
        .iter()
        .enumerate()
        .flat_map(|(index, change)| {
            functions
                .get(&change.path)
                .into_iter()
                .flatten()
                .map(move |(_, id)| (*id, index))
        })
        .collect();
    let mut neighbours: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); changes.len()];
    let mut reached_files: BTreeSet<String> = BTreeSet::new();
    let mut callers: BTreeSet<String> = BTreeSet::new();
    let mut reached_names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut entry_points: BTreeSet<String> = BTreeSet::new();
    if let Some(graph) = &graph {
        let entries: std::collections::HashSet<&str> = graph
            .list_entry_points()
            .into_iter()
            .map(|n| n.id.as_str())
            .collect();
        for (&id, &index) in &file_of {
            let impact = graph.analyze_impact(id, IMPACT_DEPTH);
            for affected in impact
                .upstream
                .iter()
                .chain(impact.downstream.iter().filter(|d| d.hop_distance <= 1))
            {
                if let Some(&other) = file_of.get(&affected.node_id) {
                    if other != index && affected.hop_distance <= 1 {
                        neighbours[index].insert(other);
                        neighbours[other].insert(index);
                    }
                }
            }
            for up in &impact.upstream {
                if file_of.contains_key(&up.node_id) {
                    continue;
                }
                callers.insert(up.node_info.id.clone());
                reached_files.insert(up.node_info.file.replace('\\', "/"));
                reached_names
                    .entry(up.node_info.file.replace('\\', "/"))
                    .or_default()
                    .push(up.node_info.name.clone());
                if entries.contains(up.node_info.id.as_str()) {
                    entry_points.insert(up.node_info.name.clone());
                }
            }
            if let Some(node) = graph.get(id) {
                if entries.contains(node.id.as_str()) {
                    entry_points.insert(node.name.clone());
                }
            }
        }
    }

    // Scope: compare the request with each changed file.
    let names_per_file: Vec<Vec<String>> = changes
        .iter()
        .map(|c| {
            functions
                .get(&c.path)
                .map(|f| f.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default()
        })
        .collect();
    let neighbour_lists: Vec<Vec<usize>> = neighbours
        .iter()
        .map(|set| set.iter().copied().collect())
        .collect();
    let facts: Vec<scope::FileFacts> = changes
        .iter()
        .enumerate()
        .map(|(index, change)| scope::FileFacts {
            path: &change.path,
            functions: &names_per_file[index],
            neighbours: &neighbour_lists[index],
            supporting: supporting_file(&change.path)
                || crate::commands::is_test_file(&change.path),
        })
        .collect();
    let judged = scope::classify(&turn.prompt, &facts);

    let files: Vec<ChangedFile> = changes
        .iter()
        .enumerate()
        .map(|(index, change)| ChangedFile {
            path: change.path.clone(),
            old_path: change.old_path.clone(),
            change: change.kind,
            additions: change.additions,
            deletions: change.deletions,
            functions: names_per_file[index].clone(),
            areas: app::areas(&change.path, &names_per_file[index]),
            in_scope: match &judged {
                scope::Scope::Known(flags) => Some(flags[index]),
                scope::Scope::Unknown => None,
            },
        })
        .collect();

    // Pages, API routes and areas: the changed files and everything they reach.
    let mut routes: BTreeSet<Surface> = BTreeSet::new();
    let mut areas: BTreeSet<Area> = BTreeSet::new();
    for file in &files {
        if let Some((kind, route)) = app::route_for(&file.path) {
            routes.insert(Surface { kind, route });
        }
        areas.extend(file.areas.iter().copied());
    }
    for file in &reached_files {
        let relative = relative_to(root, file);
        let route = app::route_for(&relative);
        let page = matches!(route, Some((RouteKind::Page, _)));
        if let Some((kind, route)) = route {
            routes.insert(Surface { kind, route });
        }
        // A page that only renders the changed code is reported as that page.
        // Its name alone (/signup, /pricing) doesn't mean sign in or payments
        // logic changed, and saying so would bury the warnings that matter.
        if !page {
            areas.extend(app::areas(
                &relative,
                reached_names.get(file).map(Vec::as_slice).unwrap_or(&[]),
            ));
        }
    }

    if !unparsed.is_empty() {
        let shown: Vec<&str> = unparsed.iter().take(3).map(String::as_str).collect();
        limits.push(format!(
            "No functions found in the changed lines of {}{}, so what uses {} isn't shown.",
            shown.join(", "),
            if unparsed.len() > 3 {
                " and others"
            } else {
                ""
            },
            if unparsed.len() == 1 { "it" } else { "them" }
        ));
    }
    if matches!(judged, scope::Scope::Unknown) {
        limits.push("Your request didn't name anything we could match to files, so nothing is flagged as outside it.".into());
    }

    let affects = Affects {
        routes: routes.into_iter().collect(),
        areas: areas.into_iter().collect(),
        entry_points: entry_points.into_iter().collect(),
        callers: callers.len(),
    };
    let tests = render::tests(&files, &affects);
    let finished_at = now_rfc3339();
    let id = format!(
        "{}-{}",
        finished_at.replace([':', '-'], "").trim_end_matches('Z'),
        &now_tree[..8.min(now_tree.len())]
    );
    Ok(Some(Receipt {
        version: 1,
        id,
        agent: turn.agent.clone(),
        prompt: turn.prompt.clone(),
        started_at: turn.started_at.clone(),
        finished_at,
        files,
        scope_known: matches!(judged, scope::Scope::Known(_)),
        affects,
        tests,
        limits,
        before: turn.tree.clone(),
        after: now_tree,
    }))
}

fn relative_to(root: &Path, file: &str) -> String {
    let normalized = file.replace('\\', "/");
    let prefix = format!(
        "{}/",
        root.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
    );
    normalized
        .strip_prefix(&prefix)
        .map(str::to_string)
        .unwrap_or(normalized)
}

fn load_all(root: &Path) -> Vec<Receipt> {
    let Ok(entries) = std::fs::read_dir(receipts_dir(root)) else {
        return Vec::new();
    };
    let mut receipts: Vec<Receipt> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok())
        .collect();
    receipts.sort_by(|a, b| b.id.cmp(&a.id));
    receipts
}

/// Recent receipts, newest first.
pub fn list(path: &Path, limit: usize, json: bool) -> Result<()> {
    let root = crate::commands::resolve_project_path(path)?;
    let receipts: Vec<Receipt> = load_all(&root).into_iter().take(limit).collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&receipts)?);
        return Ok(());
    }
    if receipts.is_empty() {
        println!("No receipts yet. Run `arbor hook claude` and ask your agent for a change.");
        return Ok(());
    }
    for receipt in &receipts {
        println!("{}", render::line(receipt));
    }
    Ok(())
}

/// One receipt in full (the latest when no id is given).
pub fn show(id: Option<&str>, path: &Path, json: bool) -> Result<()> {
    let root = crate::commands::resolve_project_path(path)?;
    let receipts = load_all(&root);
    let receipt = match id {
        Some(id) => receipts.iter().find(|r| r.id.starts_with(id)),
        None => receipts.first(),
    }
    .ok_or("No matching receipt. `arbor receipt list` shows what is saved.")?;
    if json {
        println!("{}", serde_json::to_string_pretty(receipt)?);
    } else {
        println!("{}", render::text(receipt));
    }
    Ok(())
}

/// Which of a turn's files to undo: the named ones, the ones outside the
/// request, or all of them.
fn undo_targets<'a>(
    receipt: &'a Receipt,
    files: &[String],
    unasked: bool,
) -> Result<Vec<&'a ChangedFile>> {
    let wanted: Vec<String> = files
        .iter()
        .map(|file| file.replace('\\', "/").trim_start_matches("./").to_string())
        .collect();
    if let Some(unknown) = wanted
        .iter()
        .find(|name| !receipt.files.iter().any(|file| &file.path == *name))
    {
        return Err(format!("{unknown} wasn't changed in this turn.").into());
    }
    Ok(receipt
        .files
        .iter()
        .filter(|file| {
            if !wanted.is_empty() {
                wanted.contains(&file.path)
            } else if unasked {
                file.in_scope == Some(false)
            } else {
                true
            }
        })
        .collect())
}

/// Put files back the way they were before a turn. A file that changed again
/// after the turn is refused unless `force` is set, so undo never throws away
/// later work; when anything is refused, nothing is undone.
pub fn undo(id: &str, files: &[String], unasked: bool, force: bool, path: &Path) -> Result<()> {
    let root = crate::commands::resolve_project_path(path)?;
    let receipts = load_all(&root);
    let receipt = receipts
        .iter()
        .find(|r| r.id.starts_with(id))
        .ok_or("No matching receipt. `arbor receipt list` shows what is saved.")?;
    if receipt.before.is_empty() || receipt.after.is_empty() {
        return Err("This receipt was saved without snapshots, so it can't be undone.".into());
    }
    if !snapshot::exists(&root, &receipt.before) || !snapshot::exists(&root, &receipt.after) {
        return Err("Git no longer has this turn's snapshots (unused ones are cleaned up after about two weeks), so it can't be undone.".into());
    }
    let targets = undo_targets(receipt, files, unasked)?;
    if targets.is_empty() {
        println!("Nothing to undo: no file in this turn was flagged as outside your request.");
        return Ok(());
    }
    if !force {
        let mut moved = Vec::new();
        for file in &targets {
            for path in std::iter::once(&file.path).chain(file.old_path.as_ref()) {
                if !snapshot::unchanged_since(&root, &receipt.after, path)? {
                    moved.push(path.as_str());
                }
            }
        }
        if !moved.is_empty() {
            return Err(format!(
                "{} changed after this turn, so undoing would lose that work. Nothing was undone; add --force to undo anyway.",
                moved.join(", ")
            )
            .into());
        }
    }
    for file in &targets {
        snapshot::restore(&root, &receipt.before, &file.path, file.old_path.as_deref())?;
        match (file.change, &file.old_path) {
            (ChangeKind::Added, _) => println!("Removed {} (new in this turn)", file.path),
            (ChangeKind::Deleted, _) => println!("Restored {}", file.path),
            (ChangeKind::Renamed, Some(old)) => println!("Moved {} back to {old}", file.path),
            _ => println!("Put back {}", file.path),
        }
    }
    Ok(())
}

/// UTC time as RFC 3339, without a date-time dependency.
fn now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_rfc3339(seconds)
}

fn format_rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1_790_640_000), "2026-09-29T00:00:00Z");
        assert_eq!(format_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn session_ids_cannot_escape_the_receipts_folder() {
        let path = turn_path(Path::new("/p"), "../../etc/passwd");
        assert!(path.ends_with("______etc_passwd.json"), "{path:?}");
    }

    fn changed(path: &str, in_scope: Option<bool>) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            old_path: None,
            change: ChangeKind::Modified,
            additions: 1,
            deletions: 1,
            functions: vec![],
            areas: vec![],
            in_scope,
        }
    }

    #[test]
    fn undo_picks_named_unasked_or_all_files() {
        let receipt = Receipt {
            version: 1,
            id: "20260929T101203-abcdef12".into(),
            agent: "claude-code".into(),
            prompt: "make the pricing button green".into(),
            started_at: String::new(),
            finished_at: String::new(),
            files: vec![
                changed("components/PricingButton.tsx", Some(true)),
                changed("lib/auth/session.ts", Some(false)),
            ],
            scope_known: true,
            affects: Affects {
                routes: vec![],
                areas: vec![],
                entry_points: vec![],
                callers: 0,
            },
            tests: vec![],
            limits: vec![],
            before: "a".into(),
            after: "b".into(),
        };
        let paths = |files: Vec<&ChangedFile>| -> Vec<String> {
            files.into_iter().map(|f| f.path.clone()).collect()
        };
        assert_eq!(
            paths(undo_targets(&receipt, &[], true).unwrap()),
            ["lib/auth/session.ts"]
        );
        assert_eq!(undo_targets(&receipt, &[], false).unwrap().len(), 2);
        assert_eq!(
            paths(undo_targets(&receipt, &[".\\lib\\auth\\session.ts".into()], false).unwrap()),
            ["lib/auth/session.ts"]
        );
        assert!(undo_targets(&receipt, &["README.md".into()], false).is_err());
    }

    #[test]
    fn lockfiles_and_manifests_support_a_change() {
        assert!(supporting_file("web/package-lock.json"));
        assert!(supporting_file("Cargo.toml"));
        assert!(!supporting_file("src/lib.rs"));
    }
}
