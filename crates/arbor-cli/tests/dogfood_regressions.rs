//! Regressions from dogfooding v3.0.0 on a Rust + TypeScript codebase (#225).

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {program}: {e}"))
}

fn arbor(dir: &Path, args: &[&str]) -> Output {
    run(dir, env!("CARGO_BIN_EXE_arbor"), args)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = run(dir, "git", args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let output = arbor(dir, args);
    assert!(
        output.status.success(),
        "arbor {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid json")
}

fn names(list: &Value) -> Vec<String> {
    let mut names: Vec<String> = list
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|v| v["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

fn write(dir: &Path, path: &str, content: &str) {
    let full = dir.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

#[test]
fn a_branch_switch_never_answers_from_the_other_branch() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "arbor-tests@example.com"]);
    git(dir, &["config", "user.name", "Arbor Tests"]);
    write(dir, "src/lib.rs", "pub fn shared() -> u32 {\n    1\n}\n");
    write(dir, ".gitignore", ".arbor/\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "base"]);
    let base_branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert!(arbor(dir, &["index", "."]).status.success());

    git(dir, &["checkout", "-q", "-b", "feature"]);
    write(
        dir,
        "src/billing.rs",
        "pub fn order_history() -> u32 {\n    crate::shared()\n}\n",
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "feature"]);

    let on_feature = json(dir, &["callers", "shared", ".", "--json"]);
    assert_eq!(names(&on_feature["callers"]), vec!["order_history"]);

    // Back on the base branch the function doesn't exist, even though no
    // remaining file is newer than the graph: only HEAD moved.
    git(dir, &["checkout", "-q", &base_branch]);
    let gone = arbor(dir, &["callers", "order_history", "."]);
    assert!(
        !gone.status.success(),
        "answered from the feature branch's graph: {}",
        String::from_utf8_lossy(&gone.stdout)
    );

    git(dir, &["checkout", "-q", "feature"]);
    let back = json(dir, &["callers", "shared", ".", "--json"]);
    assert_eq!(names(&back["callers"]), vec!["order_history"]);
}

#[test]
fn an_upgrade_rebuilds_a_graph_the_old_version_saved() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let lib = "pub fn helper() -> u32 {\n    1\n}\n\npub fn other() -> u32 {\n    2\n}\n";
    write(dir, "src/lib.rs", lib);
    write(
        dir,
        "src/run.rs",
        "pub fn run() -> u32 {\n    crate::helper()\n}\n",
    );
    assert!(arbor(dir, &["index", "."]).status.success());
    let arbor_dir = dir.join(".arbor");
    let old_bin = fs::read(arbor_dir.join("graph.bin")).unwrap();
    let old_json = fs::read(arbor_dir.join("graph.json")).ok();

    // What the current extractor sees: `run` calls `other` now.
    write(
        dir,
        "src/run.rs",
        "pub fn run() -> u32 {\n    crate::other()\n}\n",
    );
    assert!(arbor(dir, &["index", "."]).status.success());

    // Put back the graph an older release built, newer than every source
    // file, the way it sits on disk right after upgrading.
    fs::write(arbor_dir.join("graph.bin"), old_bin).unwrap();
    if let Some(json) = old_json {
        fs::write(arbor_dir.join("graph.json"), json).unwrap();
    }
    fs::write(arbor_dir.join("graph.builder"), "arbor-3.0.0").unwrap();

    let callers = json(dir, &["callers", "other", ".", "--json"]);
    assert_eq!(
        names(&callers["callers"]),
        vec!["run"],
        "answered from the old version's graph: {callers}"
    );
}

fn mixed_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "src/jobs.rs", "pub fn enqueue() -> u32 {\n    1\n}\n");
    write(
        dir,
        "src/dispatch.rs",
        "pub fn dispatch() -> u32 {\n    crate::jobs::enqueue()\n}\n",
    );
    write(
        dir,
        "src/checks.rs",
        "fn verify() -> bool {\n    true\n}\n\nfn verify_is_true() {\n    assert!(verify());\n}\n",
    );
    write(
        dir,
        "web/toast.ts",
        "export function enqueue(): number {\n  return 1;\n}\n\nexport function start(): number {\n  return enqueue();\n}\n",
    );
    assert!(arbor(dir, &["index", "."]).status.success());
    temp
}

#[test]
fn rust_path_and_macro_calls_are_callers() {
    let temp = mixed_project();
    let dir = temp.path();

    let jobs = json(dir, &["callers", "jobs::enqueue", ".", "--json"]);
    assert_eq!(names(&jobs["callers"]), vec!["dispatch"]);

    let verify = json(dir, &["callers", "verify", ".", "--json"]);
    assert_eq!(names(&verify["callers"]), vec!["verify_is_true"]);
}

#[test]
fn same_named_symbols_in_two_languages_are_kept_apart() {
    let temp = mixed_project();
    let dir = temp.path();

    let both = json(dir, &["callers", "enqueue", ".", "--json"]);
    let matches = both["matches"]
        .as_array()
        .expect("one entry per definition");
    assert_eq!(matches.len(), 2, "{both}");
    for m in matches {
        let callers = names(&m["callers"]);
        match m["definition"]["language"].as_str() {
            Some("Rust") => assert_eq!(callers, vec!["dispatch"], "{both}"),
            Some("TypeScript") => assert_eq!(callers, vec!["start"], "{both}"),
            other => panic!("unexpected language {other:?}"),
        }
    }

    let text = arbor(dir, &["callers", "enqueue", "."]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("matches 2 definitions"), "{stdout}");
    assert!(
        stdout.contains("Rust function") && stdout.contains("TypeScript function"),
        "{stdout}"
    );
}

#[test]
fn an_empty_answer_says_what_it_cannot_see() {
    let temp = mixed_project();
    let dir = temp.path();

    let none = json(dir, &["callers", "dispatch", ".", "--json"]);
    assert!(names(&none["callers"]).is_empty());
    assert!(
        none["note"]
            .as_str()
            .is_some_and(|n| n.contains("Not proof")),
        "{none}"
    );
}
