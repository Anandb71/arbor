//! `arbor retirements`: retired and retiring models found from a fixed date,
//! routed calls kept apart, environment fallbacks flagged, the argument
//! changes and per-repo choices that go with each replacement, response
//! objects skipped, `--json` that is always valid JSON, and the exit codes
//! CI relies on (0 clean, 1 failing, 2 error).

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// The day the OpenAI data was checked; every test counts from it.
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

/// Response and type objects built in code that calls no API (the review's
/// two-function file): nothing is sent, so nothing is reported.
const OBJECTS_ONLY: &str = r#"from openai.types.chat import ChatCompletion
from anthropic.types import Message


def fake_completion():
    return ChatCompletion(id="c1", model="gpt-3.5-turbo-0613", object="chat.completion", created=0, choices=[])


def fake_message():
    return Message(id="m1", model="claude-3-5-sonnet-20241022", role="assistant", content=[], type="message", usage=None)
"#;

/// A real call next to a fixture, plus a validated object and a tool config.
const CALL_AND_FIXTURES: &str = r#"from openai import OpenAI
from openai.types.chat import ChatCompletion
from openai.types.responses.tool_param import ImageGeneration

client = OpenAI()


def call_api():
    return client.chat.completions.create(model="gpt-3.5-turbo-0613", messages=[])


def make_fixture():
    return ChatCompletion(id="1", model="gpt-3.5-turbo-0613", object="chat.completion", created=0, choices=[])


def from_cache(raw):
    return ChatCompletion.model_validate({**raw, "model": "gpt-3.5-turbo-0613"})


def image_tool():
    return ImageGeneration(type="image_generation", model="dall-e-2")


def from_partial(raw):
    return ChatCompletion.model_validate({"model": "gpt-3.5-turbo-0613", **raw})
"#;

/// Clients given a base_url: the provider's host, an environment fallback
/// to it, an environment-only value and another host.
const BASE_URLS: &str = r#"import os
from openai import OpenAI

official = OpenAI(base_url="https://api.openai.com/v1")
fallback = OpenAI(base_url=os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1"))
deployed = OpenAI(base_url=os.environ.get("OPENAI_BASE_URL"))
proxy = OpenAI(base_url="http://localhost:4000")


def one():
    return official.chat.completions.create(model="gpt-4-0314", messages=[])


def two():
    return fallback.chat.completions.create(model="gpt-4-0314", messages=[])


def three():
    return deployed.chat.completions.create(model="gpt-4-0314", messages=[])


def four():
    return proxy.chat.completions.create(model="gpt-4-0314", messages=[])
"#;

/// A replacement that retires too, and a fine-tune whose replacement cannot
/// be fine-tuned.
const CHAINS: &str = r#"from openai import OpenAI

client = OpenAI()


def vision():
    return client.chat.completions.create(model="gpt-4-vision-preview", messages=[], max_tokens=100)


def tuned():
    return client.chat.completions.create(model="ft:gpt-4o-mini-2024-07-18:acme::abc123", messages=[])
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

fn run_json(dir: &Path, extra: &[&str]) -> (Option<i32>, Value) {
    let mut args = vec!["retirements", ".", "--json", "--today", TODAY];
    args.extend_from_slice(extra);
    let output = arbor(dir, &args);
    (output.status.code(), json(&output))
}

#[test]
fn json_lists_retired_and_retiring_calls_from_a_fixed_date() {
    let dir = project(APP);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, Some(1), "a retired model fails");

    assert_eq!(report["version"], 1);
    assert_eq!(report["today"], TODAY);
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["summary"]["files_scanned"], 1);
    assert_eq!(report["summary"]["affected_calls"], 4);
    assert_eq!(report["summary"]["retired"], 1);
    assert_eq!(report["summary"]["retiring"], 3);
    assert_eq!(report["summary"]["failing"], 1);
    assert_eq!(report["summary"]["next_shutdown"]["date"], "2027-01-14");
    assert_eq!(report["summary"]["next_shutdown"]["days_left"], 88);

    let claude = finding(&report, "claude-3-5-sonnet-20241022");
    assert_eq!(claude["status"], "retired");
    assert_eq!(claude["provider"], "anthropic");
    assert_eq!(claude["date"], "2025-10-28");
    assert_eq!(claude["replacement"], "claude-sonnet-5-5");
    assert_eq!(claude["in_window"], true);
    assert_eq!(claude["fails"], true);
    assert_eq!(claude["line"], 34);

    let gpt4o = finding(&report, "gpt-4o");
    assert_eq!(gpt4o["status"], "retiring");
    assert_eq!(gpt4o["days_left"], 88);
    assert_eq!(gpt4o["replacement"], "gpt-6-luna");
    assert_eq!(gpt4o["live_replacement"], "gpt-6-luna");
    assert_eq!(gpt4o["routing_class"], "direct");
    assert_eq!(gpt4o["confidence"], "confirmed");
    assert_eq!(gpt4o["call_kind"], "request");
    assert_eq!(gpt4o["value_from"], "MODEL");
    assert_eq!(gpt4o["value_line"], 9);
    assert_eq!(gpt4o["fix"], "decide");
    assert!(gpt4o["source_url"]
        .as_str()
        .unwrap()
        .starts_with("https://developers.openai.com/"));

    // Provenance travels with every report: one date per provider, and the
    // oldest as the data date.
    assert_eq!(report["data"]["as_of"], "2026-10-16");
    let providers = report["data"]["providers"].as_array().unwrap();
    assert_eq!(providers[0]["provider"], "OpenAI");
    assert_eq!(providers[0]["as_of"], "2026-10-18");
    assert_eq!(providers[1]["provider"], "Anthropic");
    assert_eq!(providers[1]["as_of"], "2026-10-16");
    assert!(report["data"]["recipes"].as_array().unwrap().len() >= 6);
}

#[test]
fn the_reasoning_effort_is_a_decision_and_max_tokens_a_change() {
    let dir = project(APP);
    let (_, report) = run_json(dir.path(), &[]);

    let gpt4o = finding(&report, "gpt-4o");
    let changes = gpt4o["changes"].as_array().unwrap();
    assert_eq!(
        changes.len(),
        1,
        "only the rename is automatic: {changes:#?}"
    );
    assert_eq!(changes[0]["kind"], "rename");
    assert_eq!(changes[0]["param"], "max_tokens");
    assert_eq!(changes[0]["to"], "max_completion_tokens");
    assert_eq!(changes[0]["verified"], "live");

    let decision = &gpt4o["decisions"][0];
    assert_eq!(decision["choice"], "G6-REASONING-DEFAULT");
    assert_eq!(decision["because"][0], "temperature");
    let options = decision["options"].as_array().unwrap();
    assert_eq!(options[0]["id"], "reasoning_none");
    assert_eq!(options[0]["add"][0]["name"], "reasoning_effort");
    assert_eq!(options[0]["add"][0]["value"], "none");
    assert_eq!(options[0]["keep"][0], "temperature");
    assert_eq!(options[1]["id"], "model_default");
    assert_eq!(options[1]["remove"][0], "temperature");
    assert!(decision["notes"][0]
        .as_str()
        .unwrap()
        .contains("max_completion_tokens also bounds its reasoning tokens"));

    let tools = finding(&report, "gpt-4.1");
    assert!(tools["changes"].as_array().unwrap().is_empty());
    assert_eq!(tools["decisions"][0]["options"][0]["keep"][0], "tools");
    assert_eq!(tools["decisions"][0]["options"][1]["remove"][0], "tools");

    // stop is rejected on gpt-6-luna, and the value is a fallback: a person decides.
    let mini = finding(&report, "gpt-4o-mini");
    assert_eq!(mini["fix"], "manual");
    assert!(mini["review"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["message"].as_str().unwrap().contains("stop")));
}

/// Two calls that retire the same day but move to different models.
const TWO_TARGETS: &str = r#"
from openai import OpenAI

client = OpenAI()


def sol(fns):
    return client.chat.completions.create(model="gpt-4", messages=[], functions=fns)


def terra(fns):
    return client.chat.completions.create(model="gpt-3.5-turbo", messages=[], functions=fns)
"#;

#[test]
fn a_shared_decision_names_every_replacement_and_scopes_its_evidence() {
    let dir = project(TWO_TARGETS);
    let (_, report) = run_json(dir.path(), &[]);
    for (model, target) in [("gpt-4", "gpt-5.6-sol"), ("gpt-3.5-turbo", "gpt-5.6-terra")] {
        let call = finding(&report, model);
        assert_eq!(call["fix"], "decide", "{model}");
        let decision = &call["decisions"][0];
        assert_eq!(decision["choice"], "OAI-REASONING-DEFAULT");
        // Legacy functions were called only on gpt-5.6-terra: keeping them is
        // not verified for any target, and the reason says where they were checked.
        let keep = &decision["options"][0];
        assert_eq!(keep["keep"][0], "functions");
        assert!(keep["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["rule"] == "OAI-LEGACY-FUNCTIONS" && e["status"] == "unverified"));
        let reasons = decision["reasons"].as_array().unwrap();
        assert!(
            reasons[0].as_str().unwrap().starts_with(target),
            "{reasons:?}"
        );
        assert!(reasons.iter().any(|r| r
            .as_str()
            .unwrap()
            .contains("checked only on gpt-5.6-terra")));
    }

    // One decision block for the day, naming both replacement models.
    let text = stdout(&arbor(dir.path(), &["retirements", ".", "--today", TODAY]));
    let header = text
        .lines()
        .find(|l| l.contains("decide once for this repo"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(header.contains("gpt-5.6-sol runs at medium"), "{header}");
    assert!(header.contains("gpt-5.6-terra runs at medium"), "{header}");
    assert_eq!(
        text.matches("decide once for this repo").count(),
        1,
        "{text}"
    );
    let markdown = stdout(&arbor(
        dir.path(),
        &["retirements", ".", "--markdown", "--today", TODAY],
    ));
    assert!(
        markdown.contains("gpt-5.6-terra runs at medium"),
        "{markdown}"
    );
}

#[test]
fn environment_fallbacks_are_flagged_confirm_first() {
    let dir = project(APP);
    let (_, report) = run_json(dir.path(), &[]);
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
    let (_, report) = run_json(dir.path(), &[]);
    assert_eq!(report["summary"]["routed_calls"], 2);
    assert_eq!(report["routed"].as_array().unwrap().len(), 0);
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["routing_class"] != "routed"));

    let (_, report) = run_json(dir.path(), &["--include-routed"]);
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
    assert_eq!(proxy["base_url"], "other_host");
    assert_eq!(proxy["status"], "retired");
}

#[test]
fn a_base_url_on_the_provider_host_is_a_direct_call() {
    let dir = project(BASE_URLS);
    let (code, report) = run_json(dir.path(), &[]);
    let at = |line: u64| {
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["line"] == line)
            .cloned()
    };
    let official = at(11).expect("the provider's host is direct");
    assert_eq!(official["routing_class"], "direct");
    assert_eq!(official["base_url"], "provider_host");
    assert_eq!(official["fails"], true);
    let fallback = at(15).expect("an environment fallback to the provider is direct");
    assert_eq!(fallback["base_url"], "env_provider_default");
    assert_eq!(fallback["routing_class"], "direct");
    // Set only by the environment: listed with routing unknown, not counted.
    let deployed = at(19).expect("an environment-only base_url is listed");
    assert_eq!(deployed["routing_class"], "unknown");
    assert_eq!(deployed["base_url"], "env_only");
    assert_eq!(deployed["fails"], false);
    // Another host stays routed.
    assert!(at(23).is_none());
    assert_eq!(report["summary"]["routed_calls"], 1);
    assert_eq!(code, Some(1));
    assert_eq!(report["summary"]["failing"], 2);
    assert_eq!(report["summary"]["uncounted"], 1);

    let (_, report) = run_json(dir.path(), &["--count-unconfirmed"]);
    assert_eq!(report["summary"]["failing"], 3);
    assert_eq!(report["options"]["count_unconfirmed"], true);
}

#[test]
fn unconfirmed_values_fail_only_when_counted() {
    let dir = project(
        r#"import os
from openai import OpenAI

client = OpenAI()


def ask():
    return client.chat.completions.create(model=os.getenv("MODEL", "gpt-4-0314"), messages=[])
"#,
    );
    let output = arbor(dir.path(), &["retirements", ".", "--today", TODAY]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a fallback does not fail by default"
    );
    let text = stdout(&output);
    assert!(
        text.contains("Not counted toward the exit code: 1 call to a model that is already retired, because the value, the client or the routing is unconfirmed. Add --count-unconfirmed to count it."),
        "{text}"
    );
    let output = arbor(
        dir.path(),
        &["retirements", ".", "--today", TODAY, "--count-unconfirmed"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("Failing (exit 1): 1 call uses a model that is already retired.")
    );
}

#[test]
fn response_and_type_objects_are_not_reported() {
    let dir = project(OBJECTS_ONLY);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, Some(0), "{report:#}");
    assert!(report["findings"].as_array().unwrap().is_empty());
    assert_eq!(report["summary"]["response_objects"], 2);
    let text = stdout(&arbor(dir.path(), &["retirements", ".", "--today", TODAY]));
    assert!(
        text.contains("Skipped: 2 SDK response or type objects built in code"),
        "{text}"
    );
    assert!(!text.contains("thinking"), "no advice for objects: {text}");

    let dir = project(CALL_AND_FIXTURES);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, Some(1));
    let findings = report["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2, "{findings:#?}");
    let call = findings.iter().find(|f| f["line"] == 9).unwrap();
    assert_eq!(call["call_kind"], "request");
    let tool = findings.iter().find(|f| f["model"] == "dall-e-2").unwrap();
    assert_eq!(tool["call_kind"], "request_param");
    assert_eq!(tool["fix"], "manual");
    // ChatCompletion(...) and from_cache's .model_validate carry a model and
    // are skipped. In from_partial the later `**raw` may replace the model,
    // so the usage scan reads no value there: nothing to skip or report.
    assert_eq!(report["summary"]["response_objects"], 2);
    assert_eq!(report["summary"]["unreadable_calls"], 0);
}

#[test]
fn chained_replacements_and_fine_tunes_read_plainly() {
    let dir = project(CHAINS);
    let (_, report) = run_json(dir.path(), &[]);
    let vision = finding(&report, "gpt-4-vision-preview");
    assert_eq!(vision["replacement"], "gpt-4o");
    assert_eq!(vision["live_replacement"], "gpt-6-luna");
    assert_eq!(vision["replacement_chain"][0]["model"], "gpt-4o");
    assert_eq!(vision["replacement_chain"][0]["status"], "retiring");
    assert_eq!(vision["replacement_chain"][1]["model"], "gpt-6-luna");
    assert_eq!(vision["replacement_chain"][1]["status"], Value::Null);
    assert_eq!(vision["fix"], "manual");

    let tuned = finding(&report, "ft:gpt-4o-mini-2024-07-18:acme::abc123");
    assert_eq!(tuned["replacement"], Value::Null);
    assert_eq!(
        tuned["replacement_text"],
        "no fine-tunable replacement is listed"
    );

    let text = stdout(&arbor(dir.path(), &["retirements", ".", "--today", TODAY]));
    assert!(
        text.contains("gpt-4-vision-preview → gpt-6-luna (via gpt-4o)"),
        "{text}"
    );
    assert!(
        text.contains(
            "(fine-tune of gpt-4o-mini-2024-07-18) → no fine-tunable replacement is listed"
        ),
        "{text}"
    );
    assert!(!text.contains("retrain on gpt-6-luna"), "{text}");
    let markdown = stdout(&arbor(
        dir.path(),
        &["retirements", ".", "--markdown", "--today", TODAY],
    ));
    assert!(markdown.contains("`gpt-6-luna (via gpt-4o)`"), "{markdown}");
    assert!(
        markdown.contains("no fine-tunable replacement is listed"),
        "{markdown}"
    );
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

    // On the shutdown day (UTC) the call counts as retired.
    let day = arbor(
        dir.path(),
        &["retirements", ".", "--quiet", "--today", "2027-01-14"],
    );
    assert_eq!(day.status.code(), Some(1));
    assert!(stdout(&day).contains("OpenAI shuts down today (14 Jan 2027)"));
    let after = arbor(
        dir.path(),
        &["retirements", ".", "--quiet", "--today", "2027-01-15"],
    );
    assert_eq!(after.status.code(), Some(1));
    assert!(stdout(&after).contains("OpenAI shut down on 14 Jan 2027"));
    let window = arbor(
        dir.path(),
        &[
            "retirements",
            ".",
            "--quiet",
            "--today",
            TODAY,
            "--fail-within",
            "88",
        ],
    );
    assert!(stdout(&window).contains(
        "Failing (exit 1): 1 call uses a model that is retired or retiring within 88 days."
    ));

    let clean = project(CURRENT);
    let output = arbor(clean.path(), &["retirements", ".", "--today", TODAY]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains(
        "No direct openai/anthropic/google SDK calls to retired or retiring models in 1 Python file. Calls through LangChain, LiteLLM or other wrappers are not checked yet."
    ));
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
    assert!(text.contains("also change: max_tokens → max_completion_tokens\n"));
    assert!(text.contains(
        "decide: keep reasoning off (add reasoning_effort=\"none\"; keep temperature) or let the model reason (remove temperature)"
    ));
    assert!(text.contains("decide once for this repo (Arbor does not choose)"));
    assert!(
        !text.contains("also change: add reasoning_effort"),
        "{text}"
    );
    assert!(text.contains("confirm the deployed value"));
    assert!(text.contains("Add --include-routed to list them."));
    assert!(text.contains(
        "OpenAI as of 18 Oct 2026; Anthropic, Google and Google Cloud as of 16 Oct 2026"
    ));
    assert!(text.contains("Days are counted in UTC."));
    assert!(text.contains("Runs locally: no code leaves this machine."));
    // No line ends in a space.
    assert!(text.lines().all(|l| !l.ends_with(' ')), "{text}");
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
    assert!(text.contains("| Call | Model | Replacement | Also change | Decide | Check by hand |"));
    assert!(
        text.contains("| `app.py:13` | `gpt-4o (MODEL, line 9)` | `gpt-6-luna` |"),
        "{text}"
    );

    // One scan for both: annotations on stdout, Markdown in a file.
    let summary = dir.path().join("summary.md");
    let output = arbor(
        dir.path(),
        &[
            "retirements",
            ".",
            "--annotations",
            "--markdown-out",
            summary.to_str().unwrap(),
            "--today",
            TODAY,
        ],
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
    assert!(text.contains("Decide (per repo): keep reasoning off"));
    let written = fs::read_to_string(&summary).unwrap();
    assert!(written.starts_with("## Model retirements"));
}

#[test]
fn annotations_stay_within_githubs_per_step_limit() {
    let mut source = String::from("from openai import OpenAI\n\nclient = OpenAI()\n\n");
    for i in 0..12 {
        source.push_str(&format!(
            "\ndef ask{i}():\n    return client.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        ));
    }
    let dir = project(&source);
    let text = stdout(&arbor(
        dir.path(),
        &["retirements", ".", "--annotations", "--today", TODAY],
    ));
    let warnings = text.lines().filter(|l| l.starts_with("::warning ")).count();
    assert_eq!(warnings, 10, "{text}");
    let notices: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("::notice "))
        .collect();
    assert_eq!(notices.len(), 1, "{text}");
    assert!(notices[0].contains("2 more calls to retired or retiring models are not annotated"));

    let text = stdout(&arbor(
        dir.path(),
        &[
            "retirements",
            ".",
            "--annotations",
            "--max-annotations",
            "50",
            "--today",
            TODAY,
        ],
    ));
    assert_eq!(text.lines().count(), 12);
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


def merged(overrides):
    params = {"model": "gpt-4.1", **overrides}
    return client.chat.completions.create(**params, messages=[])
"#;

#[test]
fn values_a_call_may_take_are_listed_to_confirm() {
    let dir = project(LOOKUPS);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, Some(0), "{report:#}");

    let mini = finding(&report, "gpt-4o-mini");
    assert_eq!(mini["candidate"], true);
    assert_eq!(mini["confidence"], "confirm_first");
    assert_eq!(mini["fix"], "manual");
    assert_eq!(mini["line"], 8);
    assert_eq!(mini["value_line"], 4);

    // What an in-file caller passes for the parameter.
    let o3 = finding(&report, "o3-mini");
    assert_eq!(o3["candidate"], true);
    assert_eq!(o3["date"], "2026-10-23");
    assert_eq!(o3["days_left"], 5);
    assert_eq!(o3["value_line"], 16);

    // A dict entry that a later `**` expansion may replace is one of the
    // values the call may get, not a confirmed one.
    let merged = finding(&report, "gpt-4.1");
    assert_eq!(merged["candidate"], true);
    assert_eq!(merged["confidence"], "confirm_first");
    assert_eq!(merged["fix"], "manual");
    assert_eq!(merged["line"], 25);
    assert_eq!(merged["value_line"], 24);

    assert_eq!(report["summary"]["affected_calls"], 3);
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
