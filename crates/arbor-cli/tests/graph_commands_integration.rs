use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run_arbor(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run arbor")
}

fn run_arbor_stdout(dir: &Path, args: &[&str]) -> String {
    let output = run_arbor(dir, args);
    assert!(
        output.status.success(),
        "arbor {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn query_multi_term_or_search() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["query", "helper|multiply", "."]);
    assert!(
        stdout.contains("helper") && stdout.contains("multiply"),
        "expected both 'helper' and 'multiply' in results, got: {stdout}"
    );
}

#[test]
fn query_multi_term_deduplicates() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // "helper|helper" should not produce duplicates
    let stdout = run_arbor_stdout(dir, &["query", "helper|helper", "."]);
    let count = stdout.matches("helper").count();
    // name appears once in the "Found N matches" line and once in the result
    assert!(
        count <= 3,
        "expected no duplicate results, got {count} occurrences of 'helper': {stdout}"
    );
}

#[test]
fn query_exclude_test_filters_test_files() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // Add a test file
    let test_dir = dir.join("tests");
    std::fs::create_dir_all(&test_dir).expect("create tests dir");
    std::fs::write(test_dir.join("test_helper.rs"), "fn test_helper_fn() { }\n")
        .expect("write test file");

    // Re-index
    let output = run_arbor(dir, &["index", "."]);
    assert!(output.status.success());

    let stdout = run_arbor_stdout(dir, &["query", "helper", ".", "--exclude-test"]);
    assert!(
        !stdout.contains("test_helper_fn"),
        "expected test file to be excluded, got: {stdout}"
    );
    assert!(
        stdout.contains("helper"),
        "expected production 'helper' to remain, got: {stdout}"
    );
}

fn setup_rust_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("create temp dir");
    let dir = temp.path();

    fs::create_dir_all(dir.join("src")).expect("create src dir");
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .expect("write Cargo.toml");

    fs::write(
        dir.join("src").join("main.rs"),
        r#"fn helper() -> i32 { 42 }
fn compute(x: i32) -> i32 { helper() + x }
fn main() { let r = compute(1); println!("{}", r); }
"#,
    )
    .expect("write main.rs");

    fs::write(
        dir.join("src").join("lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 { a + b }
pub fn multiply(a: i32, b: i32) -> i32 { a * b }
pub fn combined(a: i32, b: i32) -> i32 { add(a, b) + multiply(a, b) }
"#,
    )
    .expect("write lib.rs");

    let output = run_arbor(dir, &["setup", "."]);
    assert!(
        output.status.success(),
        "arbor setup failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    temp
}

#[test]
fn callers_finds_direct_callers() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["callers", "helper", "."]);
    assert!(
        stdout.contains("compute"),
        "expected 'compute' to call 'helper', got: {stdout}"
    );
}

#[test]
fn callers_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["callers", "helper", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["symbol"], "helper");
    assert!(!json["callers"].as_array().unwrap().is_empty());
    assert!(json["callers"][0]["name"].as_str().is_some());
}

#[test]
fn callers_not_found_symbol() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let output = run_arbor(dir, &["callers", "nonexistent_xyz", "."]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "expected 'not found' error, got: {stderr}"
    );
}

#[test]
fn callees_finds_direct_callees() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["callees", "compute", "."]);
    assert!(
        stdout.contains("helper"),
        "expected 'compute' to call 'helper', got: {stdout}"
    );
}

#[test]
fn callees_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["callees", "combined", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["symbol"], "combined");
    let callees = json["callees"].as_array().unwrap();
    let names: Vec<&str> = callees.iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(
        names.contains(&"add"),
        "expected 'add' in callees, got: {names:?}"
    );
    assert!(
        names.contains(&"multiply"),
        "expected 'multiply' in callees, got: {names:?}"
    );
}

#[test]
fn entry_points_finds_main() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["entry-points", "."]);
    assert!(
        stdout.contains("main"),
        "expected 'main' as entry point, got: {stdout}"
    );
}

#[test]
fn entry_points_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["entry-points", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let eps = json["entry_points"].as_array().unwrap();
    let names: Vec<&str> = eps.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(
        names.contains(&"main"),
        "expected 'main' in entry_points, got: {names:?}"
    );
}

#[test]
fn file_graph_shows_symbols_in_file() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["file-graph", "src/lib.rs", "."]);
    assert!(
        stdout.contains("add"),
        "expected 'add' in file graph, got: {stdout}"
    );
    assert!(
        stdout.contains("multiply"),
        "expected 'multiply' in file graph, got: {stdout}"
    );
    assert!(
        stdout.contains("combined"),
        "expected 'combined' in file graph, got: {stdout}"
    );
}

#[test]
fn file_graph_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["file-graph", "src/lib.rs", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(json["file"].as_str().unwrap().contains("lib.rs"));
    assert!(json["nodes"].as_array().unwrap().len() >= 3);
    assert!(json["edges"].as_array().is_some());
}

#[test]
fn file_graph_not_found() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let output = run_arbor(dir, &["file-graph", "src/nonexistent.rs", "."]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No symbols found"),
        "expected 'No symbols found' error, got: {stderr}"
    );
}

#[test]
fn inspect_shows_symbol_detail() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["inspect", "main", "."]);
    assert!(
        stdout.contains("Name"),
        "expected Name field, got: {stdout}"
    );
    assert!(
        stdout.contains("Kind"),
        "expected Kind field, got: {stdout}"
    );
    assert!(
        stdout.contains("main"),
        "expected 'main' in output, got: {stdout}"
    );
}

#[test]
fn inspect_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["inspect", "helper", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["name"], "helper");
    assert!(json["kind"].as_str().is_some());
    assert!(json["file"].as_str().is_some());
    assert!(json["line_start"].as_u64().is_some());
    assert!(json["centrality"].as_f64().is_some());
    assert!(json["role"].as_str().is_some());
    assert!(json["caller_count"].as_u64().is_some());
    assert!(json["callee_count"].as_u64().is_some());
    assert!(json["is_entry_point"].is_boolean());
}

#[test]
fn inspect_not_found_symbol() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let output = run_arbor(dir, &["inspect", "nonexistent_xyz", "."]);
    assert!(!output.status.success());
}

#[test]
fn path_finds_route_between_symbols() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["path", "main", "helper", "."]);
    assert!(
        stdout.contains("main") && stdout.contains("helper"),
        "expected path from main to helper, got: {stdout}"
    );
}

#[test]
fn path_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["path", "combined", "add", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["start"], "combined");
    assert_eq!(json["end"], "add");
    assert!(json["path"].as_array().unwrap().len() >= 2);
    assert!(json["hops"].as_u64().unwrap() >= 1);
}

#[test]
fn path_no_route() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // helper doesn't call main, so no path in that direction
    let stdout = run_arbor_stdout(dir, &["path", "helper", "main", "."]);
    assert!(
        stdout.contains("No path found"),
        "expected 'No path found', got: {stdout}"
    );
}

#[test]
fn path_no_route_json() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["path", "helper", "main", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(json["path"].is_null());
    assert!(json["message"].as_str().unwrap().contains("No path found"));
}

#[test]
fn map_default_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["map", "."]);
    assert!(
        stdout.contains("arbor map"),
        "expected header with 'arbor map', got: {stdout}"
    );
    assert!(
        stdout.contains("main") || stdout.contains("helper") || stdout.contains("compute"),
        "expected at least one symbol in output, got: {stdout}"
    );
}

#[test]
fn map_respects_token_budget() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // With a tiny budget (64 tokens = 256 chars), output should be small
    let stdout = run_arbor_stdout(dir, &["map", ".", "--tokens", "64"]);
    // 64 tokens ≈ 256 chars. Output should be short.
    assert!(
        stdout.len() < 800,
        "expected output under 800 chars with 64 token budget, got {} chars",
        stdout.len()
    );
}

#[test]
fn map_excludes_test_files() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // Add a test file
    let test_dir = dir.join("tests");
    std::fs::create_dir_all(&test_dir).expect("create tests dir");
    std::fs::write(test_dir.join("test_helper.rs"), "fn test_helper_fn() { }\n")
        .expect("write test file");

    let output = run_arbor(dir, &["index", "."]);
    assert!(output.status.success());

    let stdout = run_arbor_stdout(dir, &["map", ".", "--exclude-test"]);
    assert!(
        !stdout.contains("test_helper_fn"),
        "expected test file to be excluded, got: {stdout}"
    );
}

#[test]
fn map_json_output() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["map", ".", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["schema"], "arbor.map.v1");
    assert!(json["symbols_shown"].as_u64().unwrap() > 0);
    assert!(json["symbols_total"].as_u64().unwrap() > 0);
    assert!(json["files_shown"].as_u64().unwrap() > 0);
    assert!(!json["entries"].as_array().unwrap().is_empty());

    let first_file = &json["entries"][0];
    assert!(first_file["file"].as_str().is_some());
    assert!(first_file["file_short"].as_str().is_some());
    assert!(!first_file["symbols"].as_array().unwrap().is_empty());

    let first_sym = &first_file["symbols"][0];
    assert!(first_sym["name"].as_str().is_some());
    assert!(first_sym["kind"].as_str().is_some());
    assert!(first_sym["line"].as_u64().is_some());
    assert!(first_sym["centrality"].is_number());
    assert!(first_sym["callers"].is_number());
    assert!(first_sym["is_entry_point"].is_boolean());
    assert!(first_sym["signature_short"].as_str().is_some());
}

#[test]
fn map_includes_entry_points() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["map", ".", "--tokens", "4096"]);
    // main() should appear with ★ marker
    assert!(
        stdout.contains("main"),
        "expected 'main' entry point in output, got: {stdout}"
    );
    assert!(
        stdout.contains("★"),
        "expected ★ entry point marker, got: {stdout}"
    );
}

#[test]
fn map_verbose_shows_full_paths() {
    let temp = setup_rust_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["map", ".", "--verbose"]);
    // Verbose should show paths without ... compression
    // Our test paths are short (src/main.rs) so won't compress anyway,
    // but at least verify it doesn't crash
    assert!(
        stdout.contains("main.rs") || stdout.contains("lib.rs"),
        "expected file paths in verbose output, got: {stdout}"
    );
}

#[test]
fn map_focus_glob_boosts_matching_files() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // With a tight budget, --focus on lib.rs should prioritize lib symbols
    let stdout = run_arbor_stdout(dir, &["map", ".", "--tokens", "128", "--focus", "*/lib*"]);
    assert!(
        stdout.contains("add") || stdout.contains("multiply") || stdout.contains("combined"),
        "expected lib.rs symbols boosted by --focus, got: {stdout}"
    );
}

#[test]
fn map_focus_changed_does_not_crash() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // Initialize git so --focus-changed can run
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output();
    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init", "--allow-empty"])
        .current_dir(dir)
        .output();

    // Should work even with no changes (empty boost set)
    let stdout = run_arbor_stdout(dir, &["map", ".", "--focus-changed"]);
    assert!(
        stdout.contains("arbor map"),
        "expected valid output with --focus-changed, got: {stdout}"
    );
}

#[test]
fn map_centrality_persists_across_calls() {
    let temp = setup_rust_project();
    let dir = temp.path();

    // First call computes centrality and saves it
    let output1 = run_arbor(dir, &["map", "."]);
    assert!(output1.status.success());
    let stderr1 = String::from_utf8_lossy(&output1.stderr);
    assert!(
        stderr1.contains("Computing centrality"),
        "expected first call to compute centrality, got stderr: {stderr1}"
    );

    // Second call should NOT recompute (loaded from cache)
    let output2 = run_arbor(dir, &["map", "."]);
    assert!(output2.status.success());
    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(
        !stderr2.contains("Computing centrality"),
        "expected second call to skip centrality computation, got stderr: {stderr2}"
    );
}
