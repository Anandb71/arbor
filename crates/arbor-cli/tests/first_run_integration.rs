//! First run on a machine or folder that is missing a prerequisite: no git
//! on PATH, no repository, a repository git refuses, or no write access.
//! Each case must say what is wrong and what to do, and leave nothing behind.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn arbor(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arbor"));
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("ARBOR_AUTO_INDEX");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("failed to run arbor")
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn outside_a_repository_diff_says_so_and_creates_nothing() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("a.ts"),
        "export function a() { return 1; }\n",
    )
    .unwrap();

    // Auto-index on: the git check must still come before .arbor/ is created.
    let output = arbor(temp.path(), &["diff", "."], &[("ARBOR_AUTO_INDEX", "1")]);

    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(
        stderr.contains("arbor diff needs a git repository"),
        "{stderr}"
    );
    assert!(stderr.contains("git init"), "{stderr}");
    assert!(!temp.path().join(".arbor").exists());
}

#[test]
fn without_git_on_path_commands_say_git_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    let empty_path = tempfile::tempdir().unwrap();
    let empty_path = empty_path.path().to_str().unwrap();

    let output = arbor(temp.path(), &["check", "."], &[("PATH", empty_path)]);

    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(
        stderr.contains("arbor check needs git, but no `git` executable was found on PATH"),
        "{stderr}"
    );
    assert!(stderr.contains("https://git-scm.com/downloads"), "{stderr}");
}

#[test]
fn a_repository_git_refuses_is_explained_in_git_s_words() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q", "--bare", "repo.git"]);
    let bare = temp.path().join("repo.git");

    let output = arbor(
        &bare,
        &["diff", "."],
        &[
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "safe.bareRepository"),
            ("GIT_CONFIG_VALUE_0", "explicit"),
        ],
    );

    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(
        stderr.contains("arbor diff could not use the git repository at"),
        "{stderr}"
    );
    assert!(stderr.contains("safe.bareRepository"), "{stderr}");
}

#[test]
fn receipt_begin_outside_a_repository_explains_on_stderr_only() {
    let temp = tempfile::tempdir().unwrap();

    let output = arbor(temp.path(), &["receipt", "begin", "."], &[]);

    // A hook must never block the agent, and its stdout would reach the
    // agent's context.
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = stderr(&output);
    assert!(
        stderr.contains("recording a receipt needs a git repository"),
        "{stderr}"
    );
    assert!(!temp.path().join(".arbor").exists());
}

#[test]
fn hook_install_outside_a_repository_warns_that_receipts_will_not_record() {
    let temp = tempfile::tempdir().unwrap();

    let output = arbor(temp.path(), &["hook", "claude", "."], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let stderr = stderr(&output);
    assert!(
        stderr.contains("is not inside a git repository, so receipts will record nothing there"),
        "{stderr}"
    );
    assert!(temp.path().join("CLAUDE.md").is_file());
}

#[cfg(unix)]
#[test]
fn a_read_only_project_names_the_path_and_the_fix() {
    use std::os::unix::fs::PermissionsExt;

    // Root ignores directory permissions, so there is nothing to observe.
    let uid = Command::new("id").arg("-u").output().unwrap();
    if String::from_utf8_lossy(&uid.stdout).trim() == "0" {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    fs::write(
        temp.path().join("a.ts"),
        "export function a() { return 1; }\n",
    )
    .unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let output = arbor(temp.path(), &["index", "."], &[]);
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();

    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains("Arbor could not write"), "{stderr}");
    assert!(stderr.contains(".arbor"), "{stderr}");
    assert!(
        stderr.contains("Check that your user can write there"),
        "{stderr}"
    );
}
