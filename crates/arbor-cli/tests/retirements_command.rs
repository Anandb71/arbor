//! `arbor retirements`: retired and retiring models found from a fixed date,
//! routed calls kept apart, environment fallbacks flagged, the argument
//! changes that go with each replacement, `--json` that is always valid
//! JSON, and the exit codes CI relies on (0 clean, 1 failing, 2 error).

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// The day the bundled data was checked; every test counts from it.
const TODAY: &str = "2026-10-18";

const APP: &str = r#"import os
from openai import AzureOpenAI, OpenAI
from anthropic import Anthropic

client = OpenAI()
azure = AzureOpenAI(azure_endpoint="https://example.openai.azure.com", api_version="2024-02-01")
router = OpenAI(base_url="https://openrouter.ai/api/v1")
claude = Anthropic()
MODEL = "gpt-4o"


def answer(question):
    return client.chat.completions.create(
        model=MODEL,
        messages=[{"role": "user", "content": question}],
        max_tokens=300,
        temperature=0.2,
    )


def summarize(text):
    return client.chat.completions.create(
        model=os.getenv("SUMMARY_MODEL", "gpt-4o-mini"),
        messages=[{"role": "user", "content": text}],
        stop=["\n\n"],
    )


def classify(text, tools):
    return client.chat.completions.create(model="gpt-4.1", messages=[], tools=tools)


def legacy(text):
    return claude.messages.create(model="claude-3-5-sonnet-20241022", max_tokens=512, messages=[])


def via_azure(text):
    return azure.chat.completions.create(model="gpt-4o", messages=[])


def via_router(text):
    return router.chat.completions.create(model="anthropic/claude-3-opus-20240229", messages=[])


def unknown(text):
    return client.chat.completions.create(model=os.environ["MODEL"], messages=[])
"#;

/// Only models that retire later: nothing fails by default.
const RETIRING_ONLY: &str = r#"from openai import OpenAI

client = OpenAI()


def ask(prompt):
    return client.chat.completions.create(model="gpt-4o", messages=[])
"#;

const CURRENT: &str = r#"from openai import OpenAI

client = OpenAI()


def ask(prompt):
    return client.chat.completions.create(model="gpt-6-luna", messages=[])
"#;

fn arbor(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env_remove("CI")
        .output()
        .expect("spawn arbor")
}

fn arbor_ci(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("CI", "true")
        .output()
        .expect("spawn arbor")
}

fn project(source: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("app.py"), source).unwrap();
    dir
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("stdout is one JSON object")
}

fn finding<'a>(report: &'a Value, model: &str) -> &'a Value {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["model"] == model)
        .unwrap_or_else(|| panic!("no finding for {model}: {report:#}"))
}

#[test]
fn json_lists_retired_and_retiring_calls_from_a_fixed_date() {
    let dir = project(APP);
    let output = arbor(
        dir.path(),
        &["retirements", ".", "--json", "--today", TODAY],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a retired model fails: {output:?}"
    );
    let report = json(&output);

    assert_eq!(report["version"], 1);
    assert_eq!(report["today"], TODAY);
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["summary"]["files_scanned"], 1);
    assert_eq!(report["summary"]["affected_calls"], 4);
    assert_eq!(report["summary"]["retired"], 1);
    assert_eq!(report["summary"]["retiring"], 3);
    assert_eq!(report["summary"]["next_shutdown"]["date"], "2027-01-14");
    assert_eq!(report["summary"]["next_shutdown"]["days_left"], 88);

    let claude = finding(&report, "claude-3-5-sonnet-20241022");
    assert_eq!(claude["status"], "retired");
    assert_eq!(claude["provider"], "anthropic");
    assert_eq!(claude["date"], "2025-10-28");
    assert_eq!(claude["replacement"], "claude-sonnet-5-5");
    assert_eq!(claude["in_window"], true);
    assert_eq!(claude["line"], 34);

    let gpt4o = finding(&report, "gpt-4o");
    assert_eq!(gpt4o["status"], "retiring");
    assert_eq!(gpt4o["days_left"], 88);
    assert_eq!(gpt4o["replacement"], "gpt-6-luna");
    assert_eq!(gpt4o["routing_class"], "direct");
    assert_eq!(gpt4o["confidence"], "confirmed");
    assert_eq!(gpt4o["value_from"], "MODEL");
    assert_eq!(gpt4o["value_line"], 9);
    assert_eq!(gpt4o["fix"], "swap");
    assert!(gpt4o["source_url"]
        .as_str()
        .unwrap()
        .starts_with("https://developers.openai.com/"));

    // Provenance travels with every report.
    assert_eq!(report["data"]["as_of"], "2026-10-18");
    assert!(report["data"]["recipes"].as_array().unwrap().len() >= 6);
}

#[test]
fn co_changes_follow_each_calls_arguments() {
    let dir = project(APP);
    let report = json(&arbor(
        dir.path(),
        &["retirements", ".", "--json", "--today", TODAY],
    ));

    let gpt4o = finding(&report, "gpt-4o");
    let changes = gpt4o["changes"].as_array().unwrap();
    assert_eq!(changes[0]["kind"], "rename");
    assert_eq!(changes[0]["param"], "max_tokens");
    assert_eq!(changes[0]["to"], "max_completion_tokens");
    assert_eq!(changes[0]["verified"], "live");
    assert_eq!(changes[1]["kind"], "add");
    assert_eq!(changes[1]["param"], "reasoning_effort");
    assert_eq!(changes[1]["value"], "none");
    let because: Vec<&str> = changes[1]["because"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    assert!(because.contains(&"temperature") && because.contains(&"max_tokens"));

    let tools = finding(&report, "gpt-4.1");
    assert_eq!(tools["changes"][0]["param"], "reasoning_effort");
    assert_eq!(tools["changes"][0]["because"][0], "tools");

    // stop is rejected on gpt-6-luna: a person decides.
    let mini = finding(&report, "gpt-4o-mini");
    assert_eq!(mini["fix"], "manual");
    assert!(mini["review"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["message"].as_str().unwrap().contains("stop")));
}

#[test]
fn environment_fallbacks_are_flagged_confirm_first() {
    let dir = project(APP);
    let report = json(&arbor(
        dir.path(),
        &["retirements", ".", "--json", "--today", TODAY],
    ));
    let mini = finding(&report, "gpt-4o-mini");
    assert_eq!(mini["confidence"], "confirm_first");
    assert_eq!(mini["origin"], "env_default");
    // The call that reads os.environ["MODEL"] has no value to match.
    assert_eq!(report["summary"]["unreadable_calls"], 1);
    assert_eq!(report["unreadable"][0]["reason"], "env_var");
}

#[test]
fn routed_calls_are_reported_separately() {
    let dir = project(APP);
    let report = json(&arbor(
        dir.path(),
        &["retirements", ".", "--json", "--today", TODAY],
    ));
    assert_eq!(report["summary"]["routed_calls"], 2);
    assert_eq!(report["routed"].as_array().unwrap().len(), 0);
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["routing_class"] != "routed"));

    let output = arbor(
        dir.path(),
        &[
            "retirements",
            ".",
            "--json",
            "--include-routed",
            "--today",
            TODAY,
        ],
    );
    let report = json(&output);
    let routed = report["routed"].as_array().unwrap();
    assert_eq!(routed.len(), 2);
    let azure = routed
        .iter()
        .find(|f| f["routing"] == "azure_openai")
        .unwrap();
    assert_eq!(azure["model"], "gpt-4o");
    let proxy = routed
        .iter()
        .find(|f| f["routing"] == "custom_base_url")
        .unwrap();
    assert_eq!(proxy["matched"], "claude-3-opus-20240229");
    assert_eq!(proxy["match_kind"], "router_prefix");
    assert_eq!(proxy["status"], "retired");
}

#[test]
fn exit_codes_follow_the_window() {
    let dir = project(RETIRING_ONLY);
    let run = |extra: &[&str]| {
        let mut args = vec!["retirements", ".", "--quiet", "--today", TODAY];
        args.extend_from_slice(extra);
        arbor(dir.path(), &args).status.code()
    };
    assert_eq!(
        run(&[]),
        Some(0),
        "retiring in 88 days does not fail by default"
    );
    assert_eq!(run(&["--fail-within", "87"]), Some(0));
    assert_eq!(run(&["--fail-within", "88"]), Some(1));
    assert_eq!(run(&["--fail-within", "120"]), Some(1));

    // After the date the same call fails with no flag.
    let after = arbor(
        dir.path(),
        &["retirements", ".", "--quiet", "--today", "2027-01-15"],
    );
    assert_eq!(after.status.code(), Some(1));
    assert!(stdout(&after).contains("OpenAI shut down on 14 Jan 2027"));

    let clean = project(CURRENT);
    let output = arbor(clean.path(), &["retirements", ".", "--today", TODAY]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("No calls to retired or retiring models in 1 Python file."));
}

#[test]
fn errors_exit_two_with_json_on_stdout() {
    let dir = project(APP);
    let output = arbor(dir.path(), &["retirements", "missing-dir", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let error = json(&output);
    assert_eq!(error["version"], 1);
    assert!(error["error"].as_str().unwrap().contains("missing-dir"));

    let output = arbor(
        dir.path(),
        &["retirements", ".", "--json", "--today", "18/10/2026"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(json(&output)["error"].as_str().unwrap().contains("--today"));

    // Conflicting output modes are a usage error.
    let output = arbor(dir.path(), &["retirements", ".", "--json", "--markdown"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn human_report_reads_plainly_and_offers_fix_prs_once() {
    let dir = project(APP);
    let output = arbor(dir.path(), &["retirements", ".", "--today", TODAY]);
    let text = stdout(&output);
    assert!(
        text.contains("3 calls use models OpenAI shuts down on 14 Jan 2027 (88 days)"),
        "{text}"
    );
    assert!(text.contains("1 call uses a model Anthropic shut down on 28 Oct 2025"));
    assert!(text.contains("gpt-4o (MODEL, line 9) → gpt-6-luna"));
    assert!(text.contains("max_tokens → max_completion_tokens"));
    assert!(text.contains("add reasoning_effort=\"none\""));
    assert!(text.contains("confirm the deployed value"));
    assert!(text.contains("Add --include-routed to list them."));
    assert!(text.contains("Runs locally: no code leaves this machine."));
    assert_eq!(text.matches("https://getarbor.dev/retirements").count(), 1);

    // Never in CI, with --quiet, or in machine-readable output.
    for output in [
        arbor_ci(dir.path(), &["retirements", ".", "--today", TODAY]),
        arbor(
            dir.path(),
            &["retirements", ".", "--quiet", "--today", TODAY],
        ),
        arbor(
            dir.path(),
            &["retirements", ".", "--json", "--today", TODAY],
        ),
        arbor(
            dir.path(),
            &["retirements", ".", "--markdown", "--today", TODAY],
        ),
        arbor(
            dir.path(),
            &["retirements", ".", "--annotations", "--today", TODAY],
        ),
    ] {
        assert!(!stdout(&output).contains("getarbor.dev"), "{output:?}");
    }
}

#[test]
fn markdown_and_annotations_for_github() {
    let dir = project(APP);
    let output = arbor(
        dir.path(),
        &["retirements", ".", "--markdown", "--today", TODAY],
    );
    let text = stdout(&output);
    assert!(text.starts_with("## Model retirements"));
    assert!(text.contains("| Call | Model | Replacement | Also change | Check by hand |"));
    assert!(
        text.contains("| `app.py:13` | `gpt-4o (MODEL, line 9)` | `gpt-6-luna` |"),
        "{text}"
    );

    let output = arbor(
        dir.path(),
        &["retirements", ".", "--annotations", "--today", TODAY],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "the exit code does not depend on the format"
    );
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines
        .iter()
        .all(|l| l.starts_with("::warning file=app.py,line=")));
    assert!(text.contains("title=gpt-4o%3A OpenAI shuts down on 14 Jan 2027 (88 days)::"));
}

const LOOKUPS: &str = r#"from openai import OpenAI

client = OpenAI()
MODELS = {"fast": "gpt-4o-mini", "smart": "gpt-6-luna"}


def ask(prompt, tier):
    return client.chat.completions.create(model=MODELS[tier], messages=[])


def pick(prompt, model="gpt-6-luna"):
    return client.chat.completions.create(model=model, messages=[], temperature=0)


def caller():
    return pick("x", model="o3-mini")


def nothing():
    return client.chat.completions.create(messages=[])
"#;

#[test]
fn values_a_call_may_take_are_listed_to_confirm() {
    let dir = project(LOOKUPS);
    let output = arbor(
        dir.path(),
        &["retirements", ".", "--json", "--today", TODAY],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report = json(&output);

    let mini = finding(&report, "gpt-4o-mini");
    assert_eq!(mini["candidate"], true);
    assert_eq!(mini["confidence"], "confirm_first");
    assert_eq!(mini["line"], 8);
    assert_eq!(mini["value_line"], 4);

    // What an in-file caller passes for the parameter.
    let o3 = finding(&report, "o3-mini");
    assert_eq!(o3["candidate"], true);
    assert_eq!(o3["date"], "2026-10-23");
    assert_eq!(o3["days_left"], 5);
    assert_eq!(o3["value_line"], 16);

    assert_eq!(report["summary"]["affected_calls"], 2);
    assert_eq!(report["summary"]["no_model_calls"], 1);
    assert_eq!(report["summary"]["unreadable_calls"], 0);

    let text = stdout(&arbor(dir.path(), &["retirements", ".", "--today", TODAY]));
    assert!(
        text.contains("one of the values this call may get at run time"),
        "{text}"
    );
    assert!(text.contains("1 call passes no model"), "{text}");
}

#[test]
fn argument_errors_are_json_with_json() {
    let dir = project(CURRENT);
    let output = arbor(
        dir.path(),
        &["retirements", ".", "--json", "--fail-within", "soon"],
    );
    assert_eq!(output.status.code(), Some(2));
    let error = json(&output);
    assert_eq!(error["version"], 1);
    assert!(error["error"].as_str().unwrap().contains("--fail-within"));

    // Without --json the parser's usual message is enough.
    let output = arbor(dir.path(), &["retirements", ".", "--fail-within", "soon"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
}
