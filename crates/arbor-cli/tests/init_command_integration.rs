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
