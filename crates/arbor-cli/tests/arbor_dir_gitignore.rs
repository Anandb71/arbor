//! `.arbor/` holds a machine-specific graph and, with receipts, the text of
//! the user's prompts. Nothing Arbor writes there may show up in `git status`.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_git(repo: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_arbor(repo: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run arbor");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for arbor")
}

fn assert_ok(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("create temp dir");
    let repo = temp.path();
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "arbor-tests@example.com"]);
    run_git(repo, &["config", "user.name", "Arbor Tests"]);
    fs::create_dir_all(repo.join("src")).expect("create src dir");
    fs::write(repo.join("src/app.py"), "def main():\n    return 1\n").expect("write source");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
    temp
}

/// Everything git would pick up with `git add -A`.
fn untracked(repo: &Path) -> String {
    run_git(repo, &["status", "--porcelain", "--untracked-files=all"])
}

#[test]
fn indexing_leaves_git_status_clean() {
    let temp = init_repo();
    let repo = temp.path();

    assert_ok(&run_arbor(repo, &["index", "."], ""), "arbor index");

    assert!(
        repo.join(".arbor/graph.json").exists(),
        "the graph was written"
    );
    assert_eq!(untracked(repo), "", "indexing must not add untracked files");
}

#[test]
fn a_receipt_turn_keeps_the_prompt_out_of_git_status() {
    let temp = init_repo();
    let repo = temp.path();

    // No index first: the receipt creates `.arbor/` on its own.
    let hook_input = r#"{"session_id":"s1","prompt":"my secret plan for the auth rewrite"}"#;
    assert_ok(
        &run_arbor(repo, &["receipt", "begin", "."], hook_input),
        "arbor receipt begin",
    );

    let turn = fs::read_to_string(repo.join(".arbor/receipts/turns/s1.json"))
        .expect("the turn file was written");
    assert!(
        turn.contains("secret plan"),
        "the turn file holds the prompt"
    );
    assert_eq!(
        untracked(repo),
        "",
        "the saved prompt must not show up as an untracked file"
    );
}
