//! SDK-usage evaluation: runs the real `arbor usages . --json` on every
//! fixture the corpus manifest marks `"suite": "usages"` and diffs the report
//! against hand-audited truth in `eval/expected/<name>.json`.
//!
//! Records are compared as sets of canonical keys, so recall (missed) and
//! precision (unexpected) failures are reported separately:
//!
//! - `usage <file:line> <chain> <routing>`: an SDK call site
//! - `client <file:line> <constructor> <binding|-> <routing>`: a construction
//! - `value <file:line> <chain> <key>=<value>`: a captured model value
//! - `gap <file:line|dir/> <kind>/<reason>`: a typed gap
//!
//! As in the graph corpus, `known_missing` / `known_extra` record today's
//! deltas: a regression adding to either fails, and so does a fix that leaves
//! a recorded entry behind. Every string in `forbidden_in_output` (secrets,
//! non-allowlisted argument values, vendored-only model IDs) must be absent
//! from the raw JSON.

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
struct ExpectedUsage {
    at: String,
    chain: String,
    routing: String,
    #[serde(default)]
    values: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ExpectedClient {
    at: String,
    constructor: String,
    #[serde(default)]
    binding: Option<String>,
    routing: String,
}

#[derive(Deserialize)]
struct ExpectedGap {
    at: String,
    kind: String,
    reason: String,
}

#[derive(Deserialize, Default)]
struct ExpectedSkipped {
    #[serde(default)]
    vendored_files: usize,
    #[serde(default)]
    vendored_calls: usize,
    #[serde(default)]
    hidden_files: usize,
    #[serde(default)]
    hidden_calls: usize,
}

#[derive(Deserialize)]
struct Expected {
    #[serde(default)]
    usages_expected: Vec<ExpectedUsage>,
    #[serde(default)]
    clients_expected: Vec<ExpectedClient>,
    #[serde(default)]
    gaps_expected: Vec<ExpectedGap>,
    #[serde(default)]
    skipped_expected: ExpectedSkipped,
    #[serde(default)]
    forbidden_in_output: Vec<String>,
    #[serde(default)]
    known_missing: Vec<String>,
    #[serde(default)]
    known_extra: Vec<String>,
}

/// The four kinds of record, as canonical keys.
#[derive(Default)]
struct Keys {
    sites: BTreeSet<String>,
    values: BTreeSet<String>,
    gaps: BTreeSet<String>,
}

impl Keys {
    fn all(&self) -> BTreeSet<String> {
        self.sites
            .iter()
            .chain(&self.values)
            .chain(&self.gaps)
            .cloned()
            .collect()
    }
}

fn expected_keys(expected: &Expected) -> Keys {
    let mut keys = Keys::default();
    for usage in &expected.usages_expected {
        keys.sites.insert(format!(
            "usage {} {} {}",
            usage.at, usage.chain, usage.routing
        ));
        for (key, value) in &usage.values {
            keys.values
                .insert(format!("value {} {} {key}={value}", usage.at, usage.chain));
        }
    }
    for client in &expected.clients_expected {
        keys.sites.insert(format!(
            "client {} {} {} {}",
            client.at,
            client.constructor,
            client.binding.as_deref().unwrap_or("-"),
            client.routing
        ));
    }
    for gap in &expected.gaps_expected {
        keys.gaps
            .insert(format!("gap {} {}/{}", gap.at, gap.kind, gap.reason));
    }
    keys
}

fn str_of<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("-")
}

fn at(record: &Value) -> String {
    match record["span"]["line"].as_u64() {
        Some(line) => format!("{}:{line}", str_of(record, "file")),
        None => str_of(record, "file").to_string(),
    }
}

fn found_keys(report: &Value) -> Keys {
    let mut keys = Keys::default();
    for call in report["usages"].as_array().expect("usages array") {
        let chain = str_of(call, "chain");
        keys.sites.insert(format!(
            "usage {} {chain} {}",
            at(call),
            str_of(call, "routing")
        ));
        for value in call["values"].as_array().expect("values array") {
            keys.values.insert(format!(
                "value {} {chain} {}={}",
                at(call),
                str_of(value, "key"),
                str_of(value, "value")
            ));
        }
    }
    for client in report["clients"].as_array().expect("clients array") {
        keys.sites.insert(format!(
            "client {} {} {} {}",
            at(client),
            str_of(client, "constructor"),
            str_of(client, "binding"),
            str_of(client, "routing")
        ));
    }
    for gap in report["gaps"].as_array().expect("gaps array") {
        keys.gaps.insert(format!(
            "gap {} {}/{}",
            at(gap),
            str_of(gap, "kind"),
            str_of(gap, "reason")
        ));
    }
    keys
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

/// Run `arbor usages . --json` on a temporary copy (so the repository's own
/// ignore files do not apply) and return the raw stdout.
fn scan(source: &Path, name: &str) -> String {
    let tmp = std::env::temp_dir().join(format!("arbor-eval-usages-{name}"));
    let _ = fs::remove_dir_all(&tmp);
    copy_dir(source, &tmp);
    let output = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["usages", ".", "--json"])
        .current_dir(&tmp)
        .output()
        .expect("spawn arbor usages");
    let _ = fs::remove_dir_all(&tmp);
    assert!(
        output.status.success(),
        "arbor usages failed on {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 stdout")
}

fn ratio(hit: usize, total: usize) -> f64 {
    if total == 0 {
        1.0
    } else {
        hit as f64 / total as f64
    }
}

fn evaluate(name: &str, expected: &Expected, raw: &str, failures: &mut Vec<String>) {
    let report: Value = match serde_json::from_str(raw) {
        Ok(report) => report,
        Err(error) => {
            failures.push(format!("{name}: output is not valid JSON: {error}"));
            return;
        }
    };

    for forbidden in &expected.forbidden_in_output {
        if raw.contains(forbidden.as_str()) {
            failures.push(format!("{name}: forbidden string in output: {forbidden}"));
        }
    }

    let skipped = &report["skipped"];
    for (field, want) in [
        ("vendored_files", expected.skipped_expected.vendored_files),
        ("vendored_calls", expected.skipped_expected.vendored_calls),
        ("hidden_files", expected.skipped_expected.hidden_files),
        ("hidden_calls", expected.skipped_expected.hidden_calls),
    ] {
        let got = skipped[field].as_u64().unwrap_or(u64::MAX) as usize;
        if got != want {
            failures.push(format!("{name}: skipped.{field} is {got}, expected {want}"));
        }
    }

    let want = expected_keys(expected);
    let got = found_keys(&report);
    let want_all = want.all();
    let got_all = got.all();
    let known_missing: BTreeSet<String> = expected.known_missing.iter().cloned().collect();
    let known_extra: BTreeSet<String> = expected.known_extra.iter().cloned().collect();

    for key in want_all.difference(&got_all) {
        if !known_missing.contains(key) {
            failures.push(format!("{name}: missed (recall): {key}"));
        }
    }
    for key in got_all.difference(&want_all) {
        if !known_extra.contains(key) {
            failures.push(format!("{name}: unexpected (precision): {key}"));
        }
    }
    for key in &known_missing {
        if got_all.contains(key) {
            failures.push(format!(
                "{name}: known-miss resolved, drop it from expected: {key}"
            ));
        } else if !want_all.contains(key) {
            failures.push(format!(
                "{name}: known_missing entry is not in the expected records: {key}"
            ));
        }
    }
    for key in &known_extra {
        if !got_all.contains(key) {
            failures.push(format!(
                "{name}: known-extra no longer produced, drop it from expected: {key}"
            ));
        }
    }

    let site_hits = want.sites.intersection(&got.sites).count();
    let value_hits = want.values.intersection(&got.values).count();
    let gap_hits = want.gaps.intersection(&got.gaps).count();
    println!(
        "eval {name}: sites precision={:.3} recall={:.3} ({site_hits}/{} found, {} expected); \
         values precision={:.3} recall={:.3} ({value_hits}/{} found, {} expected); \
         gaps {gap_hits}/{} expected found, {} reported; skipped vendored files={}",
        ratio(site_hits, got.sites.len()),
        ratio(site_hits, want.sites.len()),
        got.sites.len(),
        want.sites.len(),
        ratio(value_hits, got.values.len()),
        ratio(value_hits, want.values.len()),
        got.values.len(),
        want.values.len(),
        want.gaps.len(),
        got.gaps.len(),
        skipped["vendored_files"],
    );
}

#[test]
fn usage_fixtures_match_recorded_truth() {
    let eval_dir = repo_root().join("eval");
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(eval_dir.join("manifest.json")).expect("eval/manifest.json"),
    )
    .unwrap();
    let fixtures: Vec<String> = manifest["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["suite"].as_str() == Some("usages"))
        .filter_map(|f| f["name"].as_str().map(str::to_string))
        .collect();
    assert!(
        !fixtures.is_empty(),
        "the corpus manifest lists no usage fixtures"
    );

    let mut failures = Vec::new();
    for name in &fixtures {
        let expected: Expected = serde_json::from_str(
            &fs::read_to_string(eval_dir.join("expected").join(format!("{name}.json")))
                .expect("expected file"),
        )
        .unwrap_or_else(|e| panic!("{name}: bad expected file: {e}"));
        let raw = scan(&eval_dir.join("corpus").join(name), name);
        evaluate(name, &expected, &raw, &mut failures);
    }
    assert!(
        failures.is_empty(),
        "usage evaluation failed:\n{}",
        failures.join("\n")
    );
}
