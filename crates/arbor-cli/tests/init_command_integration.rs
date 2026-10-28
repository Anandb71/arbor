use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run_arbor(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("failed to run arbor")
}

#[test]
fn init_uses_nearest_workspace_root() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".git")).expect("create home .git");

    let project = home.join("project");
    fs::create_dir_all(&project).expect("create project dir");
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .expect("write Cargo.toml");

    let output = run_arbor(&project, &["init", "."]);
    assert!(
        output.status.success(),
        "arbor init failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        project.join(".arbor").exists(),
        "expected .arbor to be created in the project directory"
    );
    assert!(
        !home.join(".arbor").exists(),
        "expected .arbor to NOT be created in the parent directory"
    );
}

fn run_arbor_with_home(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ARBOR_AUTO_INDEX")
        .output()
        .expect("failed to run arbor")
}

/// #273: a home folder with `~/.arbor` (the global config, or the binary
/// `install.ps1` puts in `~/.arbor/bin`) was the project for every unmarked
/// folder under it, so `arbor index .` indexed the whole home folder.
#[test]
fn a_folder_under_home_never_resolves_to_home_because_of_its_arbor_dir() {
    let temp = tempfile::tempdir().expect("create temp dir");
    // The temp folder is a project of its own, so no run here can walk up
    // past it to the real home folder.
    fs::create_dir(temp.path().join(".arbor")).expect("create temp .arbor");
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".arbor").join("bin")).expect("create home .arbor/bin");
    fs::write(home.join(".arbor").join("config.json"), "{}").expect("write config");
    let demo = home.join("projects").join("demo");
    fs::create_dir_all(&demo).expect("create demo");
    fs::write(demo.join("app.py"), "def main():\n    pass\n").expect("write app.py");

    let output = run_arbor_with_home(&demo, &home, &["index", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "arbor index failed:\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(demo.join(".arbor").join("graph.json").is_file(), "{stdout}");
    assert!(!home.join(".arbor").join("graph.json").exists(), "{stdout}");
    assert!(!home.join(".arbor").join("cache").exists(), "{stdout}");

    // Run in the home folder itself, Arbor still uses it.
    let output = run_arbor_with_home(&home, &home, &["init", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("Already initialized at"), "{stdout}");
    assert!(!stdout.contains("demo"), "{stdout}");

    // A folder under home named from home is its own project, also when it
    // does not exist yet.
    let output = run_arbor_with_home(&home, &home, &["init", "new-app"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(!stdout.contains("Already initialized"), "{stdout}");
    assert!(home.join("new-app").join(".arbor").is_dir(), "{stdout}");
}
