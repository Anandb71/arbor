use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("failed to run git");

    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("failed to run git");

    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_arbor(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(repo)
        // These tests run against fresh temp repos with no .arbor/; opt them
        // into auto-indexing (off by default to avoid mutating other projects).
        .env("ARBOR_AUTO_INDEX", "1")
        .output()
        .expect("failed to run arbor")
}

fn init_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("create temp dir");
    let repo = temp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "arbor-tests@example.com"]);
    run_git(repo, &["config", "user.name", "Arbor Tests"]);

    fs::create_dir_all(repo.join("src")).expect("create src dir");
    temp
}

#[test]
fn diff_reports_renamed_path_not_old_path() {
    let temp = init_repo();
    let repo = temp.path();

    fs::write(
        repo.join("src").join("lib.rs"),
        "fn helper() {}\nfn main_fn() { helper(); }\n",
    )
    .expect("write file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);

    run_git(repo, &["mv", "src/lib.rs", "src/core.rs"]);
    fs::write(
        repo.join("src").join("core.rs"),
        "fn helper() {}\nfn main_fn() {    helper(); }\n",
    )
    .expect("rewrite file");

    let output = run_arbor(repo, &["diff", "--json", "."]);
    assert!(
        output.status.success(),
        "arbor diff failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    let changed_files = json["changed_files"]
        .as_array()
        .expect("changed_files array");
    let changed_values: Vec<String> = changed_files
        .iter()
        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();

    assert!(changed_values.iter().any(|f| f == "src/core.rs"));
    assert!(!changed_values.iter().any(|f| f == "src/lib.rs"));
}

#[test]
fn diff_ignores_whitespace_only_changes() {
    let temp = init_repo();
    let repo = temp.path();

    fs::write(
        repo.join("src").join("whitespace.rs"),
        "fn render(){\n    println!(\"ok\");\n}\n",
    )
    .expect("write file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);

    fs::write(
        repo.join("src").join("whitespace.rs"),
        "fn render() {\n        println!(\"ok\");\n}\n",
    )
    .expect("rewrite file");

    let output = run_arbor(repo, &["diff", "."]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No modified files"),
        "expected whitespace-only change to be ignored, got: {stdout}"
    );
}

#[test]
fn diff_ignores_generated_only_changes() {
    let temp = init_repo();
    let repo = temp.path();

    fs::write(
        repo.join("src").join("main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .expect("write file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);

    fs::write(
        repo.join("src").join("models.g.dart"),
        "// generated file\n",
    )
    .expect("write generated file");

    let output = run_arbor(repo, &["diff", "."]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No modified files"),
        "expected generated-only change to be ignored, got: {stdout}"
    );
}

#[test]
fn diff_uses_env_commit_range_when_provided() {
    let temp = init_repo();
    let repo = temp.path();

    fs::write(
        repo.join("src").join("range.rs"),
        "fn alpha() {}\nfn beta() { alpha(); }\n",
    )
    .expect("write file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "base"]);

    let base_sha = run_git_stdout(repo, &["rev-parse", "HEAD"]);

    fs::write(
        repo.join("src").join("range.rs"),
        "fn alpha() {}\nfn beta() { alpha(); }\nfn gamma() { beta(); }\n",
    )
    .expect("rewrite file");
    fs::write(repo.join("src").join("extra.rs"), "fn extra() {}\n").expect("write extra file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "head"]);

    let head_sha = run_git_stdout(repo, &["rev-parse", "HEAD"]);

    let output = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["diff", "--json", "."])
        .current_dir(repo)
        .env("ARBOR_DIFF_BASE", &base_sha)
        .env("ARBOR_DIFF_HEAD", &head_sha)
        .env("ARBOR_AUTO_INDEX", "1")
        .output()
        .expect("failed to run arbor with env range");

    assert!(
        output.status.success(),
        "arbor diff with env range failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    let changed_files = json["changed_files"]
        .as_array()
        .expect("changed_files array");
    let changed_values: Vec<String> = changed_files
        .iter()
        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();

    assert!(changed_values.iter().any(|f| f == "src/range.rs"));
    assert!(changed_values.iter().any(|f| f == "src/extra.rs"));
}

#[test]
fn diff_markdown_and_summary_use_env_commit_range() {
    let temp = init_repo();
    let repo = temp.path();

    fs::write(
        repo.join("src").join("range.rs"),
        "fn alpha() {}\nfn beta() { alpha(); }\n",
    )
    .expect("write file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "base"]);

    let base_sha = run_git_stdout(repo, &["rev-parse", "HEAD"]);

    fs::write(
        repo.join("src").join("range.rs"),
        "fn alpha() {}\nfn beta() { alpha(); }\nfn gamma() { beta(); }\n",
    )
    .expect("rewrite file");

    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "head"]);

    let head_sha = run_git_stdout(repo, &["rev-parse", "HEAD"]);

    // Test arbor diff --markdown
    let diff_output = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["diff", "--markdown", "."])
        .current_dir(repo)
        .env("ARBOR_DIFF_BASE", &base_sha)
        .env("ARBOR_DIFF_HEAD", &head_sha)
        .env("ARBOR_AUTO_INDEX", "1")
        .output()
        .expect("failed to run arbor diff --markdown with env range");

    assert!(
        diff_output.status.success(),
        "arbor diff --markdown with env range failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&diff_output.stdout),
        String::from_utf8_lossy(&diff_output.stderr)
    );

    let diff_stdout = String::from_utf8_lossy(&diff_output.stdout);
    assert!(
        diff_stdout.contains("## 🌳 Arbor Impact Report"),
        "expected markdown report header, got: {diff_stdout}"
    );
    assert!(
        diff_stdout.contains("`src/range.rs`"),
        "expected file src/range.rs in markdown report, got: {diff_stdout}"
    );

    // Test arbor summary
    let summary_output = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["summary", "."])
        .current_dir(repo)
        .env("ARBOR_DIFF_BASE", &base_sha)
        .env("ARBOR_DIFF_HEAD", &head_sha)
        .env("ARBOR_AUTO_INDEX", "1")
        .output()
        .expect("failed to run arbor summary with env range");

    assert!(
        summary_output.status.success(),
        "arbor summary with env range failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&summary_output.stdout),
        String::from_utf8_lossy(&summary_output.stderr)
    );

    let summary_stdout = String::from_utf8_lossy(&summary_output.stdout);
    assert!(
        summary_stdout.contains("## 🌳 Arbor PR Auto-Description"),
        "expected auto-description header, got: {summary_stdout}"
    );
    assert!(
        summary_stdout.contains("`src/range.rs`"),
        "expected file src/range.rs in summary, got: {summary_stdout}"
    );
}

fn diff_json(repo: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["diff", "--json"];
    args.extend_from_slice(extra);
    args.push(".");
    let output = run_arbor(repo, &args);
    assert!(
        output.status.success(),
        "arbor diff failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid json output")
}

const BASE_SOURCE: &str = "\
fn helper() -> u32 {
    1
}

fn caller_a() -> u32 {
    helper()
}

fn caller_b() -> u32 {
    helper() + 1
}
";

fn committed_repo() -> tempfile::TempDir {
    let temp = init_repo();
    let repo = temp.path();
    fs::write(repo.join("src").join("lib.rs"), BASE_SOURCE).expect("write file");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "base"]);
    temp
}

#[test]
fn an_additive_change_is_new_code_without_blast_radius() {
    let temp = committed_repo();
    let repo = temp.path();
    fs::write(
        repo.join("src").join("lib.rs"),
        format!("{BASE_SOURCE}\nfn brand_new() -> u32 {{\n    7\n}}\n"),
    )
    .expect("append a function");

    let json = diff_json(repo, &[]);
    assert_eq!(json["modified_symbols"], 0, "{json}");
    assert_eq!(json["added_symbols"], 1, "{json}");
    assert_eq!(json["new_symbols"][0], "brand_new", "{json}");
    // Untouched neighbours in the same file don't bring their callers in.
    assert_eq!(json["impact"]["blast_radius_nodes"], 0, "{json}");
}

#[test]
fn editing_a_function_counts_its_callers_only() {
    let temp = committed_repo();
    let repo = temp.path();
    fs::write(
        repo.join("src").join("lib.rs"),
        BASE_SOURCE.replacen("    1\n", "    2\n", 1),
    )
    .expect("edit helper");

    let json = diff_json(repo, &[]);
    assert_eq!(json["modified_symbols"], 1, "{json}");
    assert_eq!(json["added_symbols"], 0, "{json}");
    assert_eq!(json["impact"]["direct_callers"], 2, "{json}");
}

#[test]
fn base_compares_committed_branch_work_against_the_merge_base() {
    let temp = committed_repo();
    let repo = temp.path();
    run_git(repo, &["branch", "trunk"]);
    run_git(repo, &["checkout", "-b", "feature"]);
    fs::write(
        repo.join("src").join("extra.rs"),
        "fn extra() -> u32 {\n    3\n}\n",
    )
    .expect("write extra");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "feature work"]);

    // Everything is committed, so there's nothing against HEAD...
    let working = run_arbor(repo, &["diff", "."]);
    assert!(String::from_utf8_lossy(&working.stdout).contains("No modified files"));

    // ...but the branch still changed something since it left trunk.
    let json = diff_json(repo, &["--base", "trunk"]);
    assert_eq!(json["changed_files"][0], "src/extra.rs", "{json}");
    assert_eq!(json["added_symbols"], 1, "{json}");
    assert!(
        json["compared"]
            .as_str()
            .is_some_and(|c| c.contains("trunk")),
        "{json}"
    );

    let unknown = run_arbor(repo, &["diff", "--base", "no-such-branch", "."]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("no-such-branch"));
}

#[test]
fn staged_only_counts_what_is_staged() {
    let temp = committed_repo();
    let repo = temp.path();
    fs::write(
        repo.join("src").join("staged.rs"),
        "fn staged() -> u32 {\n    4\n}\n",
    )
    .expect("write staged");
    run_git(repo, &["add", "src/staged.rs"]);
    fs::write(
        repo.join("src").join("lib.rs"),
        BASE_SOURCE.replacen("    1\n", "    9\n", 1),
    )
    .expect("unstaged edit");

    let json = diff_json(repo, &["--staged"]);
    let files: Vec<&str> = json["changed_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(files, vec!["src/staged.rs"], "{json}");
}

#[test]
fn tests_calling_the_change_are_listed_not_counted_as_impact() {
    let temp = committed_repo();
    let repo = temp.path();
    fs::create_dir_all(repo.join("tests")).expect("tests dir");
    fs::write(
        repo.join("tests").join("helper_tests.rs"),
        "fn helper_works() {\n    assert_eq!(helper(), 1);\n}\n",
    )
    .expect("write test");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "test"]);
    fs::write(
        repo.join("src").join("lib.rs"),
        BASE_SOURCE.replacen("    1\n", "    2\n", 1),
    )
    .expect("edit helper");

    let json = diff_json(repo, &[]);
    assert_eq!(json["impact"]["direct_callers"], 2, "{json}");
    assert_eq!(json["impact"]["tests_exercising"], 1, "{json}");
    // A test has no callers of its own, but it isn't an API entrypoint.
    assert_eq!(json["impact"]["api_entrypoints_affected"], 2, "{json}");
}
