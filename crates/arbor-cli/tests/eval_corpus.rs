//! Evaluation corpus: indexes every fixture under eval/corpus with the real
//! `arbor` binary and diffs the produced graph against hand-audited truth.
//!
//! Expected and found relationships are compared as sets so recall (missed)
//! and precision (incorrect) failures are reported separately. `known_missing`
//! and `known_extra` record today's deltas: a regression adding to either set
//! fails, and so does a fix that removes a recorded gap without updating the
//! expectation file.
//!
//! A fixture that contains `before/` and `after/` subdirectories is a
//! controlled change: each state is indexed independently and compared
//! against `expected/<name>.json`'s `states` map, so renames, deletions and
//! removed exports are recorded per revision instead of flattened into one.

use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives under crates/")
        .to_path_buf()
}

#[derive(Deserialize)]
struct Expected {
    #[serde(default)]
    symbols: Vec<String>,
    #[serde(default)]
    symbols_forbidden: Vec<String>,
    #[serde(default)]
    edges_expected: Vec<[String; 2]>,
    #[serde(default)]
    edges_forbidden: Vec<[String; 2]>,
    #[serde(default)]
    known_missing: Vec<[String; 2]>,
    #[serde(default)]
    known_extra: Vec<[String; 2]>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExpectedFile {
    States { states: BTreeMap<String, Expected> },
    Single(Expected),
}

fn rel(file: &str, root: &Path) -> String {
    let path = Path::new(file);
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Index one directory copy and return its symbol set and call-edge set as
/// `file::name` pairs.
fn index_dir(source: &Path, tag: &str) -> (BTreeSet<String>, BTreeSet<(String, String)>) {
    let tmp = std::env::temp_dir().join(format!("arbor-eval-{tag}"));
    let _ = fs::remove_dir_all(&tmp);
    copy_dir(source, &tmp);

    let status = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["index", ".", "--no-cache"])
        .current_dir(&tmp)
        .output()
        .expect("spawn arbor index");
    assert!(
        status.status.success(),
        "arbor index failed on {tag}: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let snapshot: Value = serde_json::from_str(
        &fs::read_to_string(tmp.join(".arbor/graph.json")).expect("graph.json written"),
    )
    .unwrap();
    let nodes = snapshot["graph"]["nodes"].as_array().expect("graph.nodes");
    let ids: Vec<String> = nodes
        .iter()
        .map(|n| {
            format!(
                "{}::{}",
                rel(n["file"].as_str().unwrap(), &tmp),
                n["name"].as_str().unwrap()
            )
        })
        .collect();

    let mut symbols = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for (i, node) in nodes.iter().enumerate() {
        if node["kind"] == "function" {
            symbols.insert(ids[i].clone());
        }
    }
    for edge in snapshot["graph"]["edges"].as_array().expect("graph.edges") {
        // Only call relationships are asserted; other edge kinds are not part
        // of the recorded truth.
        if edge[2]["kind"].as_str() != Some("calls") {
            continue;
        }
        if let (Some(s), Some(t)) = (edge[0].as_u64(), edge[1].as_u64()) {
            edges.insert((ids[s as usize].clone(), ids[t as usize].clone()));
        }
    }
    (symbols, edges)
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

fn pair(edge: &[String; 2]) -> (String, String) {
    (edge[0].clone(), edge[1].clone())
}

/// Diff one indexed state against its recorded truth, appending failures.
fn compare_state(
    label: &str,
    expected: &Expected,
    symbols: &BTreeSet<String>,
    edges: &BTreeSet<(String, String)>,
    failures: &mut Vec<String>,
) {
    for sym in &expected.symbols {
        if !symbols.contains(sym) {
            failures.push(format!("{label}: expected symbol missing: {sym}"));
        }
    }
    for sym in &expected.symbols_forbidden {
        if symbols.contains(sym) {
            failures.push(format!("{label}: forbidden symbol present: {sym}"));
        }
    }

    let expected_set: BTreeSet<(String, String)> =
        expected.edges_expected.iter().map(pair).collect();
    let forbidden: BTreeSet<(String, String)> = expected.edges_forbidden.iter().map(pair).collect();
    let known_missing: BTreeSet<(String, String)> =
        expected.known_missing.iter().map(pair).collect();
    let known_extra: BTreeSet<(String, String)> = expected.known_extra.iter().map(pair).collect();

    let missed: Vec<_> = expected_set.difference(edges).collect();
    let incorrect: Vec<_> = edges.difference(&expected_set).collect();
    for edge in &missed {
        if !known_missing.contains(*edge) {
            failures.push(format!(
                "{label}: missed relationship (recall): {} -> {}",
                edge.0, edge.1
            ));
        }
    }
    for edge in &incorrect {
        if forbidden.contains(*edge) {
            failures.push(format!(
                "{label}: forbidden edge present (precision): {} -> {}",
                edge.0, edge.1
            ));
        } else if !known_extra.contains(*edge) {
            failures.push(format!(
                "{label}: unexpected edge (precision): {} -> {}",
                edge.0, edge.1
            ));
        }
    }
    // Self-healing: a recorded gap that the engine now gets right must be
    // removed from known_missing; a recorded false positive that stops being
    // produced must be removed from known_extra.
    for edge in &known_missing {
        if edges.contains(edge) {
            failures.push(format!(
                "{label}: known-miss resolved — drop it from expected: {} -> {}",
                edge.0, edge.1
            ));
        }
    }
    for edge in &known_extra {
        if !edges.contains(edge) {
            failures.push(format!(
                "{label}: known-extra no longer produced — drop it from expected: {} -> {}",
                edge.0, edge.1
            ));
        }
    }

    println!(
        "eval {label}: symbols={} edges={} missed={} incorrect={} (recorded: {} missing, {} extra)",
        symbols.len(),
        edges.len(),
        missed.len(),
        incorrect.len(),
        known_missing.len(),
        known_extra.len(),
    );
}

#[test]
fn corpus_graphs_match_recorded_truth() {
    let eval_dir = repo_root().join("eval");
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(eval_dir.join("manifest.json")).expect("eval/manifest.json"),
    )
    .unwrap();
    let fixtures: Vec<String> = manifest["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["name"].as_str().map(str::to_string))
        .collect();
    assert!(!fixtures.is_empty(), "corpus manifest lists no fixtures");

    let mut failures = Vec::new();
    for name in &fixtures {
        let expected_file: ExpectedFile = serde_json::from_str(
            &fs::read_to_string(eval_dir.join("expected").join(format!("{name}.json")))
                .expect("expected file"),
        )
        .unwrap();
        let source = eval_dir.join("corpus").join(name);

        match expected_file {
            ExpectedFile::States { states } => {
                for state in ["before", "after"] {
                    let dir = source.join(state);
                    assert!(
                        dir.is_dir(),
                        "{name}: expected/ records a {state} state but corpus/{name}/{state} is missing"
                    );
                    let expected = states.get(state).unwrap_or_else(|| {
                        panic!("{name}: corpus/{name}/{state} exists but expected/{name}.json has no states.{state}")
                    });
                    let (symbols, edges) = index_dir(&dir, &format!("{name}-{state}"));
                    compare_state(
                        &format!("{name}:{state}"),
                        expected,
                        &symbols,
                        &edges,
                        &mut failures,
                    );
                }
            }
            ExpectedFile::Single(expected) => {
                assert!(
                    !source.join("before").is_dir() && !source.join("after").is_dir(),
                    "{name}: corpus/{name} has before/after states but expected/{name}.json is single-state"
                );
                let (symbols, edges) = index_dir(&source, name);
                compare_state(name, &expected, &symbols, &edges, &mut failures);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "corpus evaluation failed:\n{}",
        failures.join("\n")
    );
}
