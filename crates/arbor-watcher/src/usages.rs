//! Repository-wide SDK usage scan, behind `arbor usages` and the MCP tool
//! `find_api_usages`.
//!
//! Walks the tree the way the indexer does (`.gitignore`, `.arborignore`,
//! `.arbor/config.json` ignores and the built-in excludes), runs the per-file
//! extractor from [`arbor_core::usages`] on every file that has one, and
//! aggregates the result. It reads source text only; it never runs code and
//! never touches the graph cache.
//!
//! Vendored and virtual-environment directories (`site-packages`, `venv`,
//! `.venv`, `node_modules`, any directory holding a `pyvenv.cfg`...) are not
//! project code. They are walked only to count what is skipped: their usages
//! are excluded from the report and summarised as `vendored_path` gaps.
//! Other hidden directories (`.github`, `.circleci`...) are counted the same
//! way, without a gap; version-control, Arbor and tool-cache directories are
//! not walked at all.
//!
//! Extensionless files are scanned when their first line is a Python
//! shebang (`#!/usr/bin/env python3`).
//!
//! A directory scan makes a second pass for calls through a wrapper in
//! another file: a module-level function or class whose parameter the
//! extractor confirms is the model of a request in its own file
//! ([`ModelParam`]). Each project file whose import statements name one (an
//! extensionless script included) is read again with its imports resolved
//! to the scanned files (see [`ModuleIndex::resolve`]), and a call that
//! passes the parameter is recorded in that file.

use crate::indexer::IgnoreMatcher;
use arbor_core::languages::get_parser;
use arbor_core::usages::{
    is_python_shebang, is_usage_extension, ClientConstruction, FileUsages, GapKind, ModelParam,
    Sdk, SdkCall, SdkImport, UsageGap, WrapperLookup, USAGE_LANGUAGES,
};
use arbor_core::{detect_language, parse_source_usages, parse_source_usages_with};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Version of the [`UsageReport`] JSON shape.
pub const USAGE_REPORT_VERSION: u32 = 1;

/// Directory names whose contents are third-party copies, not project code.
pub const VENDORED_DIRS: &[&str] = &[
    "site-packages",
    "dist-packages",
    "node_modules",
    "venv",
    ".venv",
    "virtualenv",
    "__pypackages__",
    ".tox",
    ".nox",
    "vendor",
    "_vendor",
    "vendored",
];

/// Hidden directories that are never walked: version control, Arbor's own
/// cache and tool caches. Any directory ending in `_cache` is skipped too.
const UNWALKED_HIDDEN_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".arbor",
    ".idea",
    ".cache",
    ".ipynb_checkpoints",
];

/// Files larger than this are skipped and listed (generated or data files).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// What to scan.
#[derive(Debug, Clone)]
pub struct UsageScanOptions {
    /// Keep only this provider's records.
    pub sdk: Option<Sdk>,
    pub follow_symlinks: bool,
    pub max_file_bytes: u64,
    /// How the caller names the scan root (`src`, `app/llm.py`). Reported
    /// paths start with it, so they stay relative to wherever the caller
    /// stands. `None`, `""` and `"."` report paths relative to the root.
    pub display_root: Option<String>,
}

impl Default for UsageScanOptions {
    fn default() -> Self {
        Self {
            sdk: None,
            follow_symlinks: false,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            display_root: None,
        }
    }
}

/// Counts for a scan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Project files with a usage extractor that were analyzed.
    pub files_scanned: usize,
    /// Of those, files with at least one SDK call.
    pub files_with_calls: usize,
    pub calls: usize,
    /// Of those, calls that send a request ([`SdkCall::request`]).
    pub requests: usize,
    pub clients: usize,
    pub imports: usize,
    pub gaps: usize,
    /// Calls with at least one captured model value.
    pub calls_with_model: usize,
    /// Calls per provider.
    pub by_sdk: BTreeMap<String, usize>,
    /// Calls per routing.
    pub by_routing: BTreeMap<String, usize>,
    /// Gaps per kind.
    pub gaps_by_kind: BTreeMap<String, usize>,
}

/// One skipped directory: vendored, a virtual environment, or hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRoot {
    /// Path relative to the scan root, forward slashes, trailing `/`.
    pub path: String,
    /// Files with a usage extractor under it.
    pub files: usize,
    /// Of those, files with an SDK import or call.
    pub files_with_usages: usize,
    /// SDK calls in those files (excluded from `usages`).
    pub calls: usize,
}

/// Everything excluded from the report, so nothing is dropped silently.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub vendored_files: usize,
    pub vendored_files_with_usages: usize,
    pub vendored_calls: usize,
    pub vendored_roots: Vec<SkippedRoot>,
    /// Files under hidden directories (`.github/scripts/llm.py`), counted
    /// like vendored ones and excluded from the report.
    #[serde(default)]
    pub hidden_files: usize,
    #[serde(default)]
    pub hidden_files_with_usages: usize,
    #[serde(default)]
    pub hidden_calls: usize,
    #[serde(default)]
    pub hidden_roots: Vec<SkippedRoot>,
    /// Files over the size limit, relative paths.
    pub too_large: Vec<String>,
    /// Files that could not be read as UTF-8 or parsed, with the reason.
    pub unreadable: Vec<String>,
    /// Project files whose syntax errors the parser recovered from. Their
    /// usages are reported, but calls inside a broken region may be missing.
    #[serde(default)]
    pub syntax_errors: Vec<String>,
}

/// The result of [`scan_usages`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReport {
    pub version: u32,
    /// The scanned path as the caller named it.
    pub root: String,
    /// The provider filter, if any.
    pub sdk: Option<Sdk>,
    /// Languages that have a usage extractor in this build.
    pub languages: Vec<String>,
    pub summary: UsageSummary,
    /// SDK calls in project code, by file then position.
    pub usages: Vec<SdkCall>,
    pub clients: Vec<ClientConstruction>,
    pub imports: Vec<SdkImport>,
    pub gaps: Vec<UsageGap>,
    pub skipped: Skipped,
}

/// One candidate file and, when skipped, the directory it is counted under.
struct Candidate {
    path: PathBuf,
    relative: String,
    vendored_root: Option<String>,
    hidden_root: Option<String>,
    /// No extension: scanned only if its first line is a Python shebang.
    shebang: bool,
}

enum Outcome {
    Parsed(FileUsages),
    TooLarge,
    Unreadable(String),
    /// An extensionless file that is not a Python script.
    NotSource,
}

/// Scans `root` (a directory or a single file) for tracked SDK usages.
pub fn scan_usages(root: &Path, options: &UsageScanOptions) -> std::io::Result<UsageReport> {
    let metadata = std::fs::metadata(root)?;
    let prefix = display_prefix(options.display_root.as_deref());
    let candidates = if metadata.is_file() {
        single_file(root, prefix.as_deref())
    } else {
        walk(root, options.follow_symlinks, prefix.as_deref())
    };

    let mut outcomes: Vec<(Candidate, Outcome)> = candidates
        .into_par_iter()
        .map(|candidate| {
            let outcome = analyze(&candidate, options);
            (candidate, outcome)
        })
        .collect();
    if !metadata.is_file() {
        second_pass(root, &mut outcomes, options);
    }

    let mut report = UsageReport {
        version: USAGE_REPORT_VERSION,
        root: prefix.unwrap_or_else(|| root.display().to_string()),
        sdk: options.sdk,
        languages: USAGE_LANGUAGES.iter().map(|l| l.to_string()).collect(),
        summary: UsageSummary::default(),
        usages: Vec::new(),
        clients: Vec::new(),
        imports: Vec::new(),
        gaps: Vec::new(),
        skipped: Skipped::default(),
    };
    let mut vendored: BTreeMap<String, SkippedRoot> = BTreeMap::new();
    let mut hidden: BTreeMap<String, SkippedRoot> = BTreeMap::new();

    for (candidate, outcome) in outcomes {
        let mut found = match outcome {
            Outcome::Parsed(found) => found,
            Outcome::NotSource => continue,
            Outcome::TooLarge => {
                report.skipped.too_large.push(candidate.relative);
                continue;
            }
            Outcome::Unreadable(reason) => {
                report
                    .skipped
                    .unreadable
                    .push(format!("{}: {}", candidate.relative, reason));
                continue;
            }
        };
        if let Some(sdk) = options.sdk {
            found.retain_sdk(sdk);
        }

        let skipped_under = match (candidate.vendored_root, candidate.hidden_root) {
            (Some(root_path), _) => Some((&mut vendored, root_path)),
            (None, Some(root_path)) => Some((&mut hidden, root_path)),
            (None, None) => None,
        };
        if let Some((roots, root_path)) = skipped_under {
            let entry = roots
                .entry(root_path.clone())
                .or_insert_with(|| SkippedRoot {
                    path: root_path,
                    files: 0,
                    files_with_usages: 0,
                    calls: 0,
                });
            entry.files += 1;
            if !found.imports.is_empty() || !found.calls.is_empty() {
                entry.files_with_usages += 1;
                entry.calls += found.calls.len();
            }
            continue;
        }

        if found.syntax_errors {
            report.skipped.syntax_errors.push(candidate.relative);
        }
        report.summary.files_scanned += 1;
        if !found.calls.is_empty() {
            report.summary.files_with_calls += 1;
        }
        report.usages.extend(found.calls);
        report.clients.extend(found.clients);
        report.imports.extend(found.imports);
        report.gaps.extend(found.gaps);
    }

    for root_entry in vendored.into_values() {
        report.skipped.vendored_files += root_entry.files;
        report.skipped.vendored_files_with_usages += root_entry.files_with_usages;
        report.skipped.vendored_calls += root_entry.calls;
        if root_entry.files_with_usages > 0 {
            report.gaps.push(UsageGap {
                kind: GapKind::VendoredPath,
                sdk: None,
                file: root_entry.path.clone(),
                span: None,
                symbol: None,
                chain: None,
                key: None,
                reason: "vendored".to_string(),
                detail: format!(
                    "{} SDK call(s) in {} file(s) under `{}` are excluded: it is a vendored or virtual-environment directory",
                    root_entry.calls, root_entry.files_with_usages, root_entry.path
                ),
                values: Vec::new(),
                values_complete: false,
                args: None,
            });
        }
        report.skipped.vendored_roots.push(root_entry);
    }
    for root_entry in hidden.into_values() {
        report.skipped.hidden_files += root_entry.files;
        report.skipped.hidden_files_with_usages += root_entry.files_with_usages;
        report.skipped.hidden_calls += root_entry.calls;
        report.skipped.hidden_roots.push(root_entry);
    }

    summarize(&mut report);
    Ok(report)
}

/// The display prefix, normalised: forward slashes, no trailing slash, and
/// `None` for the root itself.
fn display_prefix(display_root: Option<&str>) -> Option<String> {
    let prefix = display_root?.replace('\\', "/");
    let prefix = prefix.trim_end_matches('/');
    let prefix = prefix.strip_prefix("./").unwrap_or(prefix);
    if prefix.is_empty() || prefix == "." {
        None
    } else {
        Some(prefix.to_string())
    }
}

fn under(prefix: Option<&str>, relative: &str) -> String {
    match prefix {
        Some(prefix) => format!("{prefix}/{relative}"),
        None => relative.to_string(),
    }
}

fn single_file(path: &Path, prefix: Option<&str>) -> Vec<Candidate> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let shebang = extension.is_empty();
    if !shebang && !is_usage_extension(extension) {
        return Vec::new();
    }
    let relative = match prefix {
        Some(prefix) => prefix.to_string(),
        None => path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
    };
    vec![Candidate {
        path: path.to_path_buf(),
        relative,
        vendored_root: None,
        hidden_root: None,
        shebang,
    }]
}

fn walk(root: &Path, follow_symlinks: bool, prefix: Option<&str>) -> Vec<Candidate> {
    let matcher = IgnoreMatcher::new(root);
    let walker = WalkBuilder::new(root)
        // Hidden entries are filtered below, so `.venv` and `.tox` can be
        // walked (and counted) while `.git` and `.arbor` are not.
        .hidden(false)
        .git_ignore(true)
        .ignore(true)
        .require_git(false)
        .git_global(true)
        .git_exclude(true)
        .add_custom_ignore_filename(".arborignore")
        .follow_links(follow_symlinks)
        .filter_entry(|entry| {
            if entry.depth() == 0 || !entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !UNWALKED_HIDDEN_DIRS.contains(&name.as_ref()) && !name.ends_with("_cache")
        })
        .build();

    let mut venv_dirs: HashMap<PathBuf, bool> = HashMap::new();
    let mut candidates = Vec::new();
    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let shebang = extension.is_empty();
        if !shebang && !is_usage_extension(extension) {
            continue;
        }
        let relative_path = path.strip_prefix(root).unwrap_or(path);
        let vendored_root =
            vendored_root(root, relative_path, &mut venv_dirs).map(|dir| under(prefix, &dir));
        // Built-in excludes (`build/`, `dist/`...) and configured ignores
        // apply to project code. Vendored directories are matched first
        // because the built-in excludes would hide them entirely.
        if vendored_root.is_none() && matcher.is_ignored(path, false) {
            continue;
        }
        let hidden_root = if vendored_root.is_none() {
            hidden_root(relative_path).map(|dir| under(prefix, &dir))
        } else {
            None
        };
        candidates.push(Candidate {
            path: path.to_path_buf(),
            relative: under(prefix, &slash_path(relative_path)),
            vendored_root,
            hidden_root,
            shebang,
        });
    }
    candidates.sort_by(|a, b| a.relative.cmp(&b.relative));
    candidates
}

/// The vendored directory containing `relative` (a path under `root`), as a
/// relative path with a trailing `/`.
fn vendored_root(
    root: &Path,
    relative: &Path,
    venv_dirs: &mut HashMap<PathBuf, bool>,
) -> Option<String> {
    let parent = relative.parent()?;
    let mut prefix = PathBuf::new();
    for component in parent.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        prefix.push(name);
        let name = name.to_string_lossy();
        let is_vendored = VENDORED_DIRS
            .iter()
            .any(|v| v.eq_ignore_ascii_case(name.as_ref()))
            || *venv_dirs
                .entry(prefix.clone())
                .or_insert_with(|| root.join(&prefix).join("pyvenv.cfg").is_file());
        if is_vendored {
            return Some(format!("{}/", slash_path(&prefix)));
        }
    }
    None
}

/// The outermost hidden directory containing `relative` (or the file itself
/// when it is hidden), as a relative path with a trailing `/` for a
/// directory.
fn hidden_root(relative: &Path) -> Option<String> {
    let mut prefix = PathBuf::new();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            continue;
        };
        prefix.push(name);
        if name.to_string_lossy().starts_with('.') {
            let is_dir = components.peek().is_some();
            let path = slash_path(&prefix);
            return Some(if is_dir { format!("{path}/") } else { path });
        }
    }
    None
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Why a candidate's text is not analyzed.
enum Unread {
    NotSource,
    TooLarge,
    Unreadable(String),
}

/// The text of a candidate.
fn read_source(candidate: &Candidate, options: &UsageScanOptions) -> Result<String, Unread> {
    if candidate.shebang && !has_python_shebang(&candidate.path) {
        return Err(Unread::NotSource);
    }
    let size = std::fs::metadata(&candidate.path)
        .map(|m| m.len())
        .unwrap_or(0);
    if size > options.max_file_bytes {
        return Err(Unread::TooLarge);
    }
    let bytes = std::fs::read(&candidate.path).map_err(|e| Unread::Unreadable(e.to_string()))?;
    String::from_utf8(bytes).map_err(|_| Unread::Unreadable("not valid UTF-8".to_string()))
}

fn analyze(candidate: &Candidate, options: &UsageScanOptions) -> Outcome {
    let source = match read_source(candidate, options) {
        Ok(source) => source,
        Err(Unread::NotSource) => return Outcome::NotSource,
        Err(Unread::TooLarge) => return Outcome::TooLarge,
        Err(Unread::Unreadable(reason)) => return Outcome::Unreadable(reason),
    };
    let parser = if candidate.shebang {
        get_parser("py")
    } else {
        detect_language(&candidate.path)
    };
    let Some(parser) = parser else {
        return Outcome::Parsed(FileUsages::new(&candidate.relative, "unknown"));
    };
    match parse_source_usages(&source, &candidate.relative, parser.as_ref()) {
        Ok(Some(found)) => Outcome::Parsed(found),
        Ok(None) => Outcome::Parsed(FileUsages::new(&candidate.relative, "unknown")),
        Err(error) => Outcome::Unreadable(error.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Second pass: calls through a wrapper in another file
// ---------------------------------------------------------------------------

/// One project Python module, for resolving imports.
struct Module {
    /// Its path as the report names it.
    display: String,
    /// What its module-level functions and classes take as a request's model.
    params: Vec<ModelParam>,
}

/// The project's Python modules (`.py` files outside vendored and hidden
/// directories), by their path under the scan root (`pkg/llm.py`,
/// `pkg/__init__.py`).
struct ModuleIndex {
    modules: HashMap<String, Module>,
}

impl ModuleIndex {
    /// The module an import statement in the file at `from` names (both
    /// paths under the root). A relative import (`.llm`, `..pkg.llm`)
    /// starts from the directory of `from`, one level up for each dot past
    /// the first. An absolute one (`pkg.llm`) is the one module at that path
    /// under the directory of `from` or any directory above it, up to the
    /// root: Python finds it there when that directory is on its path, and
    /// when two of them could be meant the scan resolves neither. `pkg/llm`
    /// is `pkg/llm.py` or `pkg/llm/__init__.py`, not both.
    fn resolve(&self, from: &str, module: &str) -> Option<&str> {
        let level = module.bytes().take_while(|b| *b == b'.').count();
        let dotted = &module[level..];
        let parts: Vec<&str> = if dotted.is_empty() {
            Vec::new()
        } else {
            dotted.split('.').collect()
        };
        if parts.iter().any(|p| !is_identifier(p)) || (level == 0 && parts.is_empty()) {
            return None;
        }
        let mut dir: Vec<&str> = from.split('/').collect();
        dir.pop();
        let found = if level > 0 {
            let up = level - 1;
            if up > dir.len() {
                return None;
            }
            self.at(&dir[..dir.len() - up], &parts)
        } else {
            let mut found = None;
            for depth in 0..=dir.len() {
                if let Some(file) = self.at(&dir[..depth], &parts) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(file);
                }
            }
            found
        };
        found.filter(|file| *file != from)
    }

    /// The module `parts` names inside directory `dir`.
    fn at(&self, dir: &[&str], parts: &[&str]) -> Option<&str> {
        let stem = dir
            .iter()
            .chain(parts)
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        let package = if stem.is_empty() {
            "__init__.py".to_string()
        } else {
            format!("{stem}/__init__.py")
        };
        let module = (!parts.is_empty()).then(|| format!("{stem}.py"));
        let package = self.modules.get_key_value(&package);
        let module = module.and_then(|m| self.modules.get_key_value(&m));
        match (package, module) {
            (Some((path, _)), None) | (None, Some((path, _))) => Some(path.as_str()),
            _ => None,
        }
    }
}

/// A Python identifier, as one segment of a module path.
fn is_identifier(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// What the second pass tells the extractor about the imports of the file
/// at `from`.
struct ImportsOf<'a> {
    index: &'a ModuleIndex,
    from: &'a str,
}

impl WrapperLookup for ImportsOf<'_> {
    fn model_params(&self, module: &str, name: &str) -> Option<(String, Vec<ModelParam>)> {
        let file = self.index.resolve(self.from, module)?;
        let found = self.index.modules.get(file)?;
        let params: Vec<ModelParam> = found
            .params
            .iter()
            .filter(|p| p.function == name)
            .cloned()
            .collect();
        (!params.is_empty()).then(|| (found.display.clone(), params))
    }
}

/// A candidate's path under the scan root, with forward slashes.
fn under_root(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root).ok().map(slash_path)
}

/// The second pass. When some project file lists model parameters
/// ([`FileUsages::model_params`]), every other project file with an import
/// statement that names one of those functions or classes (see
/// [`imports_one`]) is read and analyzed again with its imports resolved,
/// and its records replace the first pass's. An extensionless script may
/// call a wrapper but is not a module others import.
fn second_pass(root: &Path, outcomes: &mut [(Candidate, Outcome)], options: &UsageScanOptions) {
    let project = |c: &Candidate| c.vendored_root.is_none() && c.hidden_root.is_none();
    let mut modules = HashMap::new();
    for (candidate, outcome) in outcomes.iter_mut() {
        if !project(candidate) || candidate.shebang {
            continue;
        }
        let Some(path) = under_root(root, &candidate.path) else {
            continue;
        };
        let params = match outcome {
            Outcome::Parsed(found) => std::mem::take(&mut found.model_params),
            _ => Vec::new(),
        };
        let display = candidate.relative.clone();
        modules.insert(path, Module { display, params });
    }
    let names: HashSet<String> = modules
        .values()
        .flat_map(|m| m.params.iter().map(|p| p.function.clone()))
        .collect();
    if names.is_empty() {
        return;
    }
    let index = ModuleIndex { modules };
    let redone: Vec<Option<FileUsages>> = outcomes
        .par_iter()
        .map(|(candidate, outcome)| {
            if !project(candidate) || !matches!(outcome, Outcome::Parsed(_)) {
                return None;
            }
            let from = under_root(root, &candidate.path)?;
            let source = read_source(candidate, options).ok()?;
            if !imports_one(&source, &names) {
                return None;
            }
            let parser = if candidate.shebang {
                get_parser("py")
            } else {
                detect_language(&candidate.path)
            }?;
            let lookup = ImportsOf {
                index: &index,
                from: &from,
            };
            parse_source_usages_with(&source, &candidate.relative, parser.as_ref(), &lookup)
                .ok()
                .flatten()
        })
        .collect();
    for ((_, outcome), found) in outcomes.iter_mut().zip(redone) {
        if let Some(found) = found {
            *outcome = Outcome::Parsed(found);
        }
    }
}

/// Whether `source` may import one of `names`: some word of an import
/// statement's text after `import` is one, read to the end of its line,
/// through `\` continuations and a parenthesized list. A call through a
/// wrapper is followed only through `from module import name`, so a file
/// whose import statements name none is not read again. Text in strings and
/// comments counts too, which only reads a file that need not be.
fn imports_one(source: &str, names: &HashSet<String>) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(found) = source[from..].find("import") {
        let at = from + found;
        from = at + "import".len();
        let before = source[..at].chars().next_back();
        let after = source[from..].chars().next();
        if before.is_some_and(word) || after.is_some_and(word) {
            continue;
        }
        let mut depth = 0usize;
        let mut end = source.len();
        let mut last = ' ';
        for (i, c) in source[from..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                '\n' | ';' if depth == 0 && last != '\\' => {
                    end = from + i;
                    break;
                }
                _ => {}
            }
            if c != '\r' {
                last = c;
            }
        }
        let statement = &source[from..end];
        if statement
            .split(|c: char| !word(c))
            .any(|name| names.contains(name))
        {
            return true;
        }
    }
    false
}

/// Whether an extensionless file starts with a Python shebang. Reads at most
/// 256 bytes, so binaries and data files cost almost nothing.
fn has_python_shebang(path: &Path) -> bool {
    let mut head = [0u8; 256];
    let read = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .unwrap_or(0);
    is_python_shebang(&head[..read])
}

fn summarize(report: &mut UsageReport) {
    report.gaps.sort_by(|a, b| {
        (a.file.as_str(), a.span.map(|s| s.start_byte), a.kind).cmp(&(
            b.file.as_str(),
            b.span.map(|s| s.start_byte),
            b.kind,
        ))
    });
    let summary = &mut report.summary;
    summary.calls = report.usages.len();
    summary.requests = report.usages.iter().filter(|c| c.request).count();
    summary.clients = report.clients.len();
    summary.imports = report.imports.len();
    summary.gaps = report.gaps.len();
    summary.calls_with_model = report
        .usages
        .iter()
        .filter(|c| !c.values.is_empty())
        .count();
    for call in &report.usages {
        *summary
            .by_sdk
            .entry(call.sdk.as_str().to_string())
            .or_default() += 1;
        *summary
            .by_routing
            .entry(call.routing.as_str().to_string())
            .or_default() += 1;
    }
    for gap in &report.gaps {
        *summary
            .gaps_by_kind
            .entry(gap.kind.as_str().to_string())
            .or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbor_core::usages::Routing;
    use std::fs;
    use tempfile::tempdir;

    const CHAT: &str = "from openai import OpenAI\nclient = OpenAI()\nclient.chat.completions.create(model=\"gpt-3.5-turbo\", messages=[])\n";

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn project_usages_are_reported_with_relative_paths() {
        let dir = tempdir().unwrap();
        write(dir.path(), "app/chat.py", CHAT);
        write(dir.path(), "app/util.py", "def f():\n    return 1\n");
        write(dir.path(), "README.md", "client.chat.completions.create");

        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(report.summary.files_scanned, 2);
        assert_eq!(report.summary.files_with_calls, 1);
        assert_eq!(report.usages.len(), 1);
        assert_eq!(report.usages[0].file, "app/chat.py");
        assert_eq!(report.usages[0].routing, Routing::OpenaiDirect);
        assert_eq!(report.summary.by_sdk.get("openai"), Some(&1));
        assert_eq!(report.summary.by_routing.get("openai_direct"), Some(&1));
        assert_eq!(report.summary.calls_with_model, 1);
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn vendored_directories_are_excluded_and_counted() {
        let dir = tempdir().unwrap();
        write(dir.path(), "app.py", CHAT);
        write(
            dir.path(),
            "venv/lib/python3.11/site-packages/openai/x.py",
            CHAT,
        );
        write(dir.path(), ".venv/lib/site-packages/thing.py", "x = 1\n");
        write(dir.path(), "third_party/node_modules/pkg/tool.py", CHAT);
        // A virtual environment with an unusual name is found by pyvenv.cfg.
        write(dir.path(), "myenv/pyvenv.cfg", "home = /usr/bin\n");
        write(dir.path(), "myenv/lib/llm.py", CHAT);

        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(report.usages.len(), 1, "only project code is reported");
        assert_eq!(report.usages[0].file, "app.py");
        assert_eq!(report.summary.files_scanned, 1);

        let skipped = &report.skipped;
        assert_eq!(skipped.vendored_files, 4);
        assert_eq!(skipped.vendored_files_with_usages, 3);
        assert_eq!(skipped.vendored_calls, 3);
        let roots: Vec<&str> = skipped
            .vendored_roots
            .iter()
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(
            roots,
            vec![".venv/", "myenv/", "third_party/node_modules/", "venv/"]
        );
        let gaps: Vec<&str> = report
            .gaps
            .iter()
            .filter(|g| g.kind == GapKind::VendoredPath)
            .map(|g| g.file.as_str())
            .collect();
        assert_eq!(gaps, vec!["myenv/", "third_party/node_modules/", "venv/"]);
    }

    #[test]
    fn gitignored_and_hidden_paths_are_not_part_of_the_project() {
        let dir = tempdir().unwrap();
        write(dir.path(), ".gitignore", "venv/\nignored/\n");
        write(dir.path(), "venv/lib/site-packages/openai/x.py", CHAT);
        write(dir.path(), "ignored/llm.py", CHAT);
        write(dir.path(), ".github/scripts/llm.py", CHAT);
        write(dir.path(), ".github/scripts/util.py", "x = 1\n");
        write(dir.path(), ".git/hooks/pre-commit.py", CHAT);
        write(dir.path(), ".mypy_cache/x.py", CHAT);
        write(dir.path(), "build/lib/app.py", CHAT);
        write(dir.path(), "app.py", CHAT);

        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(report.usages.len(), 1);
        assert_eq!(report.skipped.vendored_files, 0);
        // Hidden directories are counted, not dropped silently; version
        // control and tool caches are not walked.
        let skipped = &report.skipped;
        assert_eq!(skipped.hidden_files, 2);
        assert_eq!(skipped.hidden_files_with_usages, 1);
        assert_eq!(skipped.hidden_calls, 1);
        let roots: Vec<&str> = skipped
            .hidden_roots
            .iter()
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(roots, vec![".github/"]);
    }

    #[test]
    fn python_shebang_scripts_are_scanned() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "bin/summarize",
            &format!("#!/usr/bin/env python3\n{CHAT}"),
        );
        write(dir.path(), "bin/deploy", "#!/bin/sh\necho openai\n");
        write(dir.path(), "Makefile", "all:\n\techo openai\n");
        fs::write(dir.path().join("blob"), [0u8, 159, 146, 150, 255]).unwrap();

        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(report.summary.files_scanned, 1);
        assert_eq!(report.usages.len(), 1);
        assert_eq!(report.usages[0].file, "bin/summarize");
        assert!(report.skipped.unreadable.is_empty());

        let single = scan_usages(
            &dir.path().join("bin/summarize"),
            &UsageScanOptions::default(),
        )
        .unwrap();
        assert_eq!(single.usages.len(), 1);
        let not_python =
            scan_usages(&dir.path().join("bin/deploy"), &UsageScanOptions::default()).unwrap();
        assert_eq!(not_python.summary.files_scanned, 0);
    }

    #[test]
    fn files_with_syntax_errors_are_listed() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "broken.py",
            "from openai import OpenAI\nclient = OpenAI()\ndef broken(:\n    pass\n",
        );
        write(dir.path(), "fine.py", CHAT);
        // A file that mentions no SDK is not even parsed.
        write(dir.path(), "other.py", "def broken(:\n");
        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(report.skipped.syntax_errors, vec!["broken.py"]);
        assert_eq!(report.summary.files_scanned, 3);
    }

    #[test]
    fn sdk_filter_and_single_file_scan() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "mixed.py",
            "from openai import OpenAI\nfrom anthropic import Anthropic\nOpenAI().chat.completions.create(model=\"gpt-4\", messages=[])\nAnthropic().messages.create(model=\"claude-2.1\", max_tokens=1, messages=[])\n",
        );
        let options = UsageScanOptions {
            sdk: Some(Sdk::Anthropic),
            ..UsageScanOptions::default()
        };
        let report = scan_usages(dir.path(), &options).unwrap();
        assert_eq!(report.sdk, Some(Sdk::Anthropic));
        assert_eq!(report.usages.len(), 1);
        assert_eq!(report.usages[0].sdk, Sdk::Anthropic);
        assert!(report.imports.iter().all(|i| i.sdk == Sdk::Anthropic));

        let single =
            scan_usages(&dir.path().join("mixed.py"), &UsageScanOptions::default()).unwrap();
        assert_eq!(single.usages.len(), 2);
        assert_eq!(single.usages[0].file, "mixed.py");
    }

    #[test]
    fn display_root_prefixes_every_reported_path() {
        let dir = tempdir().unwrap();
        write(dir.path(), "src/app.py", CHAT);
        write(dir.path(), "src/venv/lib/x.py", CHAT);
        let options = UsageScanOptions {
            display_root: Some("src\\".to_string()),
            ..UsageScanOptions::default()
        };
        let report = scan_usages(&dir.path().join("src"), &options).unwrap();
        assert_eq!(report.root, "src");
        assert_eq!(report.usages[0].file, "src/app.py");
        assert_eq!(report.clients[0].file, "src/app.py");
        assert_eq!(report.skipped.vendored_roots[0].path, "src/venv/");
        assert_eq!(report.gaps[0].file, "src/venv/");

        for root in [None, Some(""), Some("."), Some("./")] {
            assert_eq!(display_prefix(root), None, "{root:?}");
        }
        assert_eq!(display_prefix(Some("./a/b/")), Some("a/b".to_string()));
    }

    #[test]
    fn oversized_and_non_utf8_files_are_listed() {
        let dir = tempdir().unwrap();
        write(dir.path(), "big.py", &"# openai\n".repeat(64));
        fs::write(dir.path().join("latin1.py"), b"x = '\xe9'\nimport openai\n").unwrap();
        let options = UsageScanOptions {
            max_file_bytes: 100,
            ..UsageScanOptions::default()
        };
        let report = scan_usages(dir.path(), &options).unwrap();
        assert_eq!(report.skipped.too_large, vec!["big.py"]);
        assert_eq!(report.skipped.unreadable.len(), 1);
        assert!(report.skipped.unreadable[0].starts_with("latin1.py"));
    }

    #[test]
    fn missing_path_is_an_error() {
        let dir = tempdir().unwrap();
        assert!(scan_usages(&dir.path().join("nope"), &UsageScanOptions::default()).is_err());
    }

    #[test]
    fn report_json_round_trips() {
        let dir = tempdir().unwrap();
        write(dir.path(), "app.py", CHAT);
        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        let json = serde_json::to_string(&report).unwrap();
        let back: UsageReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
        assert_eq!(back.version, USAGE_REPORT_VERSION);
    }

    const SAMPLER: &str = "from openai import OpenAI\n\nclass Sampler:\n    def __init__(self, model=\"o1-mini\"):\n        self.client = OpenAI()\n        self.model = model\n\n    def __call__(self, messages):\n        return self.client.chat.completions.create(model=self.model, messages=messages)\n";

    /// The wrapper calls a scan records, as (file, line, function, model).
    fn wrapper_calls(report: &UsageReport) -> Vec<(String, u32, String, Option<String>)> {
        report
            .usages
            .iter()
            .filter_map(|c| {
                let wrapper = c.wrapper.as_ref()?;
                let model = c.values.first().map(|v| v.value.clone());
                Some((c.file.clone(), c.span.line, wrapper.function.clone(), model))
            })
            .collect()
    }

    #[test]
    fn a_call_to_a_wrapper_in_another_file_is_recorded_where_it_is() {
        let dir = tempdir().unwrap();
        write(dir.path(), "evals/sampler/o_sampler.py", SAMPLER);
        // An extensionless script calls it too.
        write(
            dir.path(),
            "bin/bench",
            "#!/usr/bin/env python3\nfrom evals.sampler.o_sampler import Sampler\nSampler(model=\"o1-mini\")\n",
        );
        // A relative import from the package, an absolute one from a
        // script in a subdirectory, and a file that names no SDK at all.
        write(
            dir.path(),
            "evals/run.py",
            "from .sampler.o_sampler import Sampler\nMODELS = {\"o1\": Sampler(model=\"o1\")}\n",
        );
        write(
            dir.path(),
            "scripts/bench/main.py",
            "from evals.sampler.o_sampler import Sampler\n\nSampler(model=\"o3-mini\")\nSampler()\n",
        );
        write(
            dir.path(),
            "evals/other.py",
            "from .sampler.o_sampler import Sampler as S\nfrom .missing import Sampler\n",
        );

        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(
            wrapper_calls(&report),
            [
                (
                    "bin/bench".to_string(),
                    3,
                    "Sampler".to_string(),
                    Some("o1-mini".to_string())
                ),
                (
                    "evals/run.py".to_string(),
                    2,
                    "Sampler".to_string(),
                    Some("o1".to_string())
                ),
                (
                    "scripts/bench/main.py".to_string(),
                    3,
                    "Sampler".to_string(),
                    Some("o3-mini".to_string())
                ),
            ]
        );
        let call = report
            .usages
            .iter()
            .find(|c| c.file == "evals/run.py")
            .unwrap();
        assert_eq!(call.routing, Routing::OpenaiDirect);
        assert!(!call.request);
        let wrapper = call.wrapper.as_ref().unwrap();
        assert_eq!(wrapper.file, "evals/sampler/o_sampler.py");
        assert_eq!(wrapper.requests[0].line, 9);
        // The wrapper's own request and default are reported as before.
        let request = report
            .usages
            .iter()
            .find(|c| c.file == "evals/sampler/o_sampler.py")
            .unwrap();
        assert!(request.request);
        assert_eq!(request.values[0].value, "o1-mini");
        assert_eq!(report.summary.requests, 1);
        assert_eq!(report.summary.calls, 4);
        assert_eq!(report.summary.files_with_calls, 4);
        assert_eq!(report.summary.calls_with_model, 4);

        // Paths in records follow the display root, resolution does not.
        let options = UsageScanOptions {
            display_root: Some("repo".to_string()),
            ..UsageScanOptions::default()
        };
        let report = scan_usages(dir.path(), &options).unwrap();
        let call = report
            .usages
            .iter()
            .find(|c| c.file == "repo/evals/run.py")
            .unwrap();
        assert_eq!(
            call.wrapper.as_ref().unwrap().file,
            "repo/evals/sampler/o_sampler.py"
        );

        // A provider filter keeps the calls of that provider.
        let options = UsageScanOptions {
            sdk: Some(Sdk::Anthropic),
            ..UsageScanOptions::default()
        };
        let report = scan_usages(dir.path(), &options).unwrap();
        assert!(report.usages.is_empty());
    }

    #[test]
    fn an_import_that_could_name_two_modules_is_not_followed() {
        let dir = tempdir().unwrap();
        write(dir.path(), "llm/sampler.py", SAMPLER);
        write(dir.path(), "app/llm/sampler.py", SAMPLER);
        write(
            dir.path(),
            "app/main.py",
            "from llm.sampler import Sampler\nSampler(model=\"o1\")\n",
        );
        // A module and a package of the same name.
        write(dir.path(), "dup/sampler.py", SAMPLER);
        write(dir.path(), "dup/sampler/__init__.py", SAMPLER);
        write(
            dir.path(),
            "dup/main.py",
            "from .sampler import Sampler\nSampler(model=\"o1\")\n",
        );
        // Above the root, and a vendored copy.
        write(
            dir.path(),
            "top.py",
            "from ..sampler import Sampler\nSampler(model=\"o1\")\n",
        );
        write(dir.path(), "venv/lib/vendored.py", SAMPLER);
        write(
            dir.path(),
            "venv_user.py",
            "from venv.lib.vendored import Sampler\nSampler(model=\"o1\")\n",
        );
        let report = scan_usages(dir.path(), &UsageScanOptions::default()).unwrap();
        assert_eq!(wrapper_calls(&report), []);

        // A single-file scan has no other files.
        let report = scan_usages(
            &dir.path().join("app/main.py"),
            &UsageScanOptions::default(),
        )
        .unwrap();
        assert!(report.usages.is_empty());
    }

    #[test]
    fn only_a_file_whose_imports_name_a_wrapper_is_read_again() {
        let names: HashSet<String> = ["Sampler", "ask"].iter().map(|n| n.to_string()).collect();
        for source in [
            "from .sampler import Sampler\n",
            "from .sampler import Other as O, ask\n",
            "from .sampler import (\n    Other,\n    Sampler,\n)\n",
            "from .sampler import Other, \\\n    Sampler\n",
            "import os; from .sampler import ask\n",
        ] {
            assert!(imports_one(source, &names), "{source}");
        }
        for source in [
            "Sampler(model=\"o1\")\nask(\"hi\")\n",
            "from .sampler import Other\nSampler = Other\n",
            "from .sampler import Other\nask = 1\n",
            "reimport(Sampler)\n",
            "from .sampler import Samplers\n",
        ] {
            assert!(!imports_one(source, &names), "{source}");
        }
    }

    #[test]
    fn modules_resolve_like_python_imports() {
        let mut modules = HashMap::new();
        for path in [
            "pkg/__init__.py",
            "pkg/llm.py",
            "pkg/sub/__init__.py",
            "pkg/sub/client.py",
            "src/lib/core.py",
            "app.py",
        ] {
            let module = Module {
                display: path.to_string(),
                params: Vec::new(),
            };
            modules.insert(path.to_string(), module);
        }
        let index = ModuleIndex { modules };
        let cases = [
            ("pkg/sub/client.py", ".", Some("pkg/sub/__init__.py")),
            ("pkg/sub/client.py", "..", Some("pkg/__init__.py")),
            ("pkg/sub/client.py", "..llm", Some("pkg/llm.py")),
            ("pkg/sub/client.py", "...", None),
            ("pkg/sub/client.py", "....x", None),
            ("pkg/llm.py", ".sub.client", Some("pkg/sub/client.py")),
            ("pkg/llm.py", ".sub", Some("pkg/sub/__init__.py")),
            ("pkg/llm.py", "pkg.sub.client", Some("pkg/sub/client.py")),
            ("pkg/llm.py", "pkg", Some("pkg/__init__.py")),
            ("pkg/llm.py", ".llm", None),
            ("app.py", "pkg.llm", Some("pkg/llm.py")),
            ("app.py", "lib.core", None),
            ("src/lib/core.py", "lib.core", None),
            ("src/main.py", "lib.core", Some("src/lib/core.py")),
            ("app.py", "pkg.sub-client", None),
            ("app.py", "", None),
        ];
        for (from, module, expected) in cases {
            assert_eq!(index.resolve(from, module), expected, "{from}: {module}");
        }
    }
}
