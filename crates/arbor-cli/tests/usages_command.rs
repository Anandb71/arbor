//! `arbor usages`: human output, `--json` that is always valid JSON, the
//! `--sdk` filter, and secrets that never reach either output.

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const APP: &str = r#"import os
from openai import AzureOpenAI, OpenAI
from anthropic import Anthropic

MODEL = "gpt-3.5-turbo"
client = OpenAI(api_key="sk-test-NOT-A-REAL-KEY-9999")
azure = AzureOpenAI(azure_endpoint="https://example.openai.azure.com", api_version="2024-02-01")
claude = Anthropic()


def ask(prompt):
    return client.chat.completions.create(model=MODEL, messages=[{"role": "user", "content": prompt}])


def ask_azure(prompt):
    return azure.chat.completions.create(model=os.environ["DEPLOYMENT"], messages=[])


def ask_claude(prompt):
    return claude.messages.create(model="claude-3-5-sonnet-20241022", max_tokens=10, messages=[])
"#;

fn arbor(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .expect("spawn arbor")
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("app.py"), APP).unwrap();
    fs::create_dir_all(dir.path().join("venv/lib/site-packages/openai")).unwrap();
    fs::write(
        dir.path().join("venv/lib/site-packages/openai/copy.py"),
        "from openai import OpenAI\nOpenAI().chat.completions.create(model=\"gpt-4\", messages=[])\n",
    )
    .unwrap();
    dir
}

#[test]
fn json_report_lists_usages_clients_gaps_and_skipped() {
    let dir = project();
    let output = arbor(dir.path(), &["usages", ".", "--json"]);
    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).expect("stdout is JSON");

    assert_eq!(report["version"], 1);
    let usages = report["usages"].as_array().unwrap();
    assert_eq!(usages.len(), 3);
    let first = &usages[0];
    assert_eq!(first["file"], "app.py");
    assert_eq!(first["chain"], "client.chat.completions.create");
    assert_eq!(first["routing"], "openai_direct");
    assert_eq!(first["symbol"], "ask");
    assert_eq!(first["span"]["line"], 12);
    assert_eq!(first["values"][0]["value"], "gpt-3.5-turbo");
    assert_eq!(first["values"][0]["origin"], "module_constant");
    assert_eq!(usages[1]["routing"], "azure_openai");
    assert_eq!(usages[2]["sdk"], "anthropic");

    assert_eq!(report["clients"].as_array().unwrap().len(), 3);
    let gap_kinds: Vec<&str> = report["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["kind"].as_str().unwrap())
        .collect();
    assert_eq!(gap_kinds, vec!["unresolved_model", "vendored_path"]);
    assert_eq!(report["skipped"]["vendored_files"], 1);
    assert_eq!(report["skipped"]["vendored_calls"], 1);
    assert_eq!(report["summary"]["calls"], 3);
    assert_eq!(report["summary"]["by_sdk"]["openai"], 2);

    assert!(!stdout.contains("sk-test-NOT-A-REAL-KEY"));
    assert!(!stdout.contains("DEPLOYMENT"));
    assert!(!stdout.contains("example.openai.azure.com"));
}

#[test]
fn sdk_filter_keeps_one_provider() {
    let dir = project();
    let output = arbor(dir.path(), &["usages", "--sdk", "anthropic", ".", "--json"]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["sdk"], "anthropic");
    let usages = report["usages"].as_array().unwrap();
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0]["chain"], "claude.messages.create");
    assert!(report["clients"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["sdk"] == "anthropic"));

    let rejected = arbor(dir.path(), &["usages", "--sdk", "cohere", "."]);
    assert!(!rejected.status.success());

    // `gemini` is an alias for `google`, as in the MCP tool.
    let gemini = arbor(dir.path(), &["usages", "--sdk", "gemini", ".", "--json"]);
    assert!(gemini.status.success());
    let report: Value = serde_json::from_slice(&gemini.stdout).unwrap();
    assert_eq!(report["sdk"], "google");
}

#[test]
fn json_errors_are_json_on_stdout() {
    let dir = project();
    for args in [
        vec!["usages", "--sdk", "cohere", ".", "--json"],
        vec!["usages", "", "--json"],
        vec!["usages", "does-not-exist", "--json"],
    ] {
        let output = arbor(dir.path(), &args);
        assert!(!output.status.success(), "{args:?}");
        let error: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: stdout is not JSON: {e}"));
        assert_eq!(error["version"], 1, "{args:?}");
        assert!(error["error"].as_str().is_some_and(|e| !e.is_empty()));
    }
}

#[test]
fn human_output_escapes_control_characters() {
    let dir = tempfile::tempdir().unwrap();
    // A file name with an escape sequence: the extractor never captures such
    // a value, and every printed path or name is escaped.
    let name = "evil\u{1b}[2J.py";
    if fs::write(dir.path().join(name), "from openai import OpenAI\nOpenAI().chat.completions.create(model=\"gpt-4\", messages=[])\n").is_err() {
        // Some file systems reject control characters in names.
        return;
    }
    let output = arbor(dir.path(), &["usages"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains('\u{1b}'), "{stdout:?}");
    assert!(stdout.contains("\\u{1b}"), "{stdout}");
}

#[test]
fn human_output_names_sites_routing_and_gaps() {
    let dir = project();
    let output = arbor(dir.path(), &["usages"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("app.py:12:12  client.chat.completions.create  openai_direct"));
    assert!(stdout.contains("model=\"gpt-3.5-turbo\" (via MODEL, line 5)"));
    assert!(stdout.contains("azure_openai"));
    assert!(stdout.contains("unresolved_model"));
    assert!(stdout.contains("vendored_path"));
    assert!(stdout.contains("Skipped: 1 vendored file(s)"));
    assert!(stdout.contains("never for credentials"));
    assert!(stdout.contains("openai (2 listed)"));
    assert!(stdout.contains("3 call(s) (3 request(s))"));
    assert!(!stdout.contains("sk-test-NOT-A-REAL-KEY"));
    assert!(serde_json::from_str::<Value>(&stdout).is_err());
}

/// The five leaks from the review of #276, plus model-shaped values behind
/// secret-named bindings. None of the `LEAK` markers may be printed.
const LEAKS: &str = r#"import os
from openai import OpenAI

client = OpenAI()
OPENAI_API_KEY = "sk-proj-LEAKONE0000000000000000"
API_TOKEN = "gpt-4o-LEAKTWO"
PARAMS = {"model": API_TOKEN}


def from_default(token="sk-LEAKTHREE000000000000000"):
    return client.chat.completions.create(model=token, messages=[])


class Holder:
    def __init__(self):
        self.secret = "sk-LEAKFOUR0000000000000000"

    def go(self):
        return client.chat.completions.create(model=self.secret, messages=[])


client.chat.completions.create(model=OPENAI_API_KEY, messages=[])
client.chat.completions.create(model="sk-ant-api03-LEAKFIVE000000000", messages=[])
client.chat.completions.create(model="gpt-4", messages=[], extra_body={"sk-proj-LEAKSIX000000000": {"model": "gpt-4"}})
client.chat.completions.create(model=API_TOKEN, messages=[])
client.chat.completions.create(**PARAMS, messages=[])
client.chat.completions.create(model=os.getenv("OPENAI_API_KEY", "gpt-4o-LEAKSEVEN"), messages=[])
"#;

#[test]
fn credentials_never_reach_human_or_json_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("leaks.py"), LEAKS).unwrap();
    for args in [vec!["usages", ".", "--json"], vec!["usages", "."]] {
        let output = arbor(dir.path(), &args);
        assert!(output.status.success(), "{args:?}");
        for stream in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(stream);
            assert!(!text.contains("LEAK"), "{args:?} printed a secret: {text}");
            assert!(
                !text.contains("sk-"),
                "{args:?} printed a key prefix: {text}"
            );
        }
    }
    let output = arbor(dir.path(), &["usages", ".", "--json"]);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let credential_gaps = report["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["reason"] == "credential")
        .count();
    // Every refused model is a typed gap; only the model key of the call
    // with the credential-shaped dict key is captured.
    assert_eq!(credential_gaps, 7, "{}", report["gaps"]);
    let captured: Vec<&Value> = report["usages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|u| u["values"].as_array().unwrap())
        .collect();
    assert_eq!(captured.len(), 1, "{captured:?}");
    assert_eq!(captured[0]["key"], "model");
    assert_eq!(captured[0]["value"], "gpt-4");
}

#[test]
fn json_is_valid_when_nothing_is_found_or_the_path_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let empty = arbor(dir.path(), &["usages", ".", "--json"]);
    assert!(empty.status.success());
    let report: Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(report["usages"].as_array().unwrap().len(), 0);
    assert_eq!(report["gaps"].as_array().unwrap().len(), 0);

    let missing = arbor(dir.path(), &["usages", "does-not-exist", "--json"]);
    assert!(!missing.status.success());
    let error: Value = serde_json::from_slice(&missing.stdout).expect("error is JSON");
    assert!(error["error"].as_str().unwrap().contains("does-not-exist"));
}

#[test]
fn a_single_file_can_be_scanned() {
    let dir = project();
    let output = arbor(dir.path(), &["usages", "app.py", "--json"]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["usages"].as_array().unwrap().len(), 3);
    assert_eq!(report["usages"][0]["file"], "app.py");
}
