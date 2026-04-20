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
