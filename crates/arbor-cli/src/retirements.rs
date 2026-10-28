//! `arbor retirements`: which calls in a project use AI models their
//! provider has retired or is about to retire, what replaces each, and which
//! other arguments have to change with it.
//!
//! The scan is [`arbor_watcher::usages::scan_usages`] (Python today); the
//! matching is [`arbor_core::retirements::check`] against data bundled at
//! build time. Everything runs locally: no code, path or result leaves the
//! machine, and nothing is sent anywhere.
//!
//! Output modes: people (default), `--json` (always one valid JSON object on
//! stdout, errors included), `--markdown` (a GitHub job summary) and
//! `--annotations` (GitHub workflow commands, one warning per call up to
//! GitHub's per-step limit). `--markdown-out <file>` also writes the
//! Markdown report from the same scan, so CI scans once.
//!
//! Exit codes: 0 when nothing counts as failing, 1 when a confirmed call
//! uses a model that is already retired (or, with `--fail-within N`, retires
//! within N days), 2 when the check could not run. Unconfirmed calls count
//! only with `--count-unconfirmed`, routed ones only with
//! `--include-routed`.

use arbor_core::retirements::{
    bundled, check, python_literal, routing_gap_text, value_gap_text, BaseUrl, CallKind, Change,
    ChangeKind, CheckOptions, Confidence, Date, DateKind, Decision, Finding, MatchKind,
    OptionStatus, RetirementReport, RoutingClass, ScanInput, State, FINE_TUNING, REPORT_VERSION,
};
use arbor_core::usages::{is_unsafe_char, ValueOrigin};
use arbor_watcher::usages::{scan_usages, UsageScanOptions};
use colored::Colorize;
use std::io::Write;
use std::path::Path;

/// Where the closing line of the human report points people who want the
/// fix PRs opened for them. The only place this URL is written. The
/// `utm_source` tag lets the page credit a fix-PR signup to the CLI; the
/// link carries nothing about the scanned code or the run.
pub const FIX_PRS_URL: &str = "https://getarbor.dev/retirements?utm_source=arbor-cli";

/// Data older than this gets a reminder to update Arbor.
const STALE_AFTER_DAYS: i64 = 90;

/// GitHub shows at most this many warning annotations per step.
pub const GITHUB_WARNING_LIMIT: usize = 10;

#[derive(Debug, Clone, Default)]
pub struct Args {
    pub path: String,
    pub json: bool,
    pub markdown: bool,
    pub annotations: bool,
    /// Also write the Markdown report to this file.
    pub markdown_out: Option<String>,
    /// The most warning annotations to print; a notice says how many more.
    pub max_annotations: Option<usize>,
    pub fail_within: Option<i64>,
    pub include_routed: bool,
    pub count_unconfirmed: bool,
    pub quiet: bool,
    /// `YYYY-MM-DD` to count days from instead of the system clock.
    pub today: Option<String>,
}

/// Runs the check, prints the report and returns the exit code.
pub fn run(args: &Args) -> i32 {
    let code = match build_report(args) {
        Ok(report) => {
            if let Some(path) = &args.markdown_out {
                if let Err(error) = std::fs::write(path, markdown(&report)) {
                    return fail(args, &format!("cannot write {path}: {error}"));
                }
            }
            if args.json {
                match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(error) => return fail(args, &format!("cannot write JSON: {error}")),
                }
            } else if args.markdown {
                print!("{}", markdown(&report));
            } else if args.annotations {
                let limit = args.max_annotations.unwrap_or(GITHUB_WARNING_LIMIT);
                print!("{}", annotations(&report, limit));
            } else {
                print_human(&report, args.quiet, show_offer(args.quiet));
            }
            report.exit_code
        }
        Err(message) => fail(args, &message),
    };
    let _ = std::io::stdout().flush();
    code
}

fn fail(args: &Args, message: &str) -> i32 {
    if args.json {
        print_json_error(message);
    }
    eprintln!("{} {}", "error:".red().bold(), clean(message));
    2
}

/// Prints `{"version", "error"}` on stdout. A multi-line message (an
/// argument error with its usage line) keeps its first line.
pub fn print_json_error(message: &str) {
    let first = message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(message);
    let error = serde_json::json!({
        "version": REPORT_VERSION,
        "error": first.strip_prefix("error: ").unwrap_or(first),
    });
    if let Ok(text) = serde_json::to_string_pretty(&error) {
        println!("{text}");
    }
}

/// Whether the command line (without the program name) runs `retirements`
/// with `--json`: global flags may come first, and `--` ends the options.
pub fn wants_json(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    let mut args = args.into_iter();
    let command = args.find(|arg| arg == "--" || !arg.to_string_lossy().starts_with('-'));
    command.is_some_and(|c| c == "retirements")
        && args
            .take_while(|arg| arg != "--")
            .any(|arg| arg == "--json")
}

fn build_report(args: &Args) -> Result<RetirementReport, String> {
    if args.path.trim().is_empty() {
        return Err("the path to scan is empty".to_string());
    }
    let today = match &args.today {
        Some(text) => Date::parse(text)
            .ok_or_else(|| format!("--today must be a date like 2026-10-18, not '{text}'"))?,
        None => Date::today_utc(),
    };
    let data = bundled()?;
    let path = Path::new(&args.path);
    let scan = scan_usages(
        path,
        &UsageScanOptions {
            display_root: Some(args.path.clone()),
            ..UsageScanOptions::default()
        },
    )
    .map_err(|error| format!("cannot scan {}: {}", path.display(), error))?;
    let options = CheckOptions {
        today,
        fail_within: args.fail_within,
        include_routed: args.include_routed,
        count_unconfirmed: args.count_unconfirmed,
    };
    let input = ScanInput {
        calls: &scan.usages,
        gaps: &scan.gaps,
        clients: &scan.clients,
    };
    let mut report = check(input, data, &options);
    report.root = scan.root;
    report.summary.files_scanned = scan.summary.files_scanned;
    report.summary.syntax_error_files = scan.skipped.syntax_errors.len();
    report.summary.skipped_files = scan.skipped.too_large.len() + scan.skipped.unreadable.len();
    report.summary.hidden_calls = scan.skipped.hidden_calls;
    Ok(report)
}

/// The closing offer is for people at a terminal: never in JSON, Markdown
/// or annotations, and not with `--quiet` or in CI.
fn show_offer(quiet: bool) -> bool {
    !quiet && !in_ci()
}

fn in_ci() -> bool {
    std::env::var("CI")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && value != "false" && value != "0"
        })
        .unwrap_or(false)
}

/// `text` with control and bidirectional-override characters escaped, so a
/// scanned file cannot drive the terminal.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if is_unsafe_char(c) {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Wording shared by every output
// ---------------------------------------------------------------------------

fn calls(n: usize) -> String {
    if n == 1 {
        "1 call".to_string()
    } else {
        format!("{n} calls")
    }
}

/// `uses` for one, `use` for several.
fn verb(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

fn days(n: i64) -> String {
    match n {
        0 => "today".to_string(),
        1 => "1 day".to_string(),
        n => format!("{n} days"),
    }
}

/// Findings that read under one headline: same provider, state and date.
fn same_group(a: &Finding, b: &Finding) -> bool {
    a.provider == b.provider && a.status == b.status && a.date == b.date
}

fn groups(findings: &[Finding]) -> Vec<&[Finding]> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=findings.len() {
        if i == findings.len() || !same_group(&findings[start], &findings[i]) {
            if i > start {
                out.push(&findings[start..i]);
            }
            start = i;
        }
    }
    out
}

/// Where the model stands, without a subject: `OpenAI shuts down on
/// 14 Jan 2027 (88 days)`.
fn status_phrase(finding: &Finding, today: Date) -> String {
    let provider = finding.provider.provider();
    match (finding.status, finding.date) {
        (State::Retired, Some(d)) if d == today => {
            format!("{provider} shuts down today ({})", d.human())
        }
        (State::Retired, Some(d)) if d > today => {
            format!(
                "{provider} has already shut down (listed for {})",
                d.human()
            )
        }
        (State::Retired, Some(d)) => format!("{provider} shut down on {}", d.human()),
        (State::Retired, None) => format!("{provider} has shut down"),
        (State::Retiring, d) => {
            let d = d.unwrap_or(today);
            let left = days(today.days_until(d));
            match finding.date_kind {
                DateKind::Hard => format!("{provider} shuts down on {} ({left})", d.human()),
                DateKind::Earliest => {
                    format!("{provider} shuts down on or after {} ({left})", d.human())
                }
            }
        }
        (State::Unconfirmed, Some(d)) if d > today => format!(
            "may stop working on {} ({}; not confirmed by {provider})",
            d.human(),
            days(today.days_until(d))
        ),
        // From the date itself on, as for a replacement in a chain.
        (State::Unconfirmed, Some(d)) if d == today => format!(
            "may already be retired ({provider}'s date is today, {}; not confirmed)",
            d.human()
        ),
        (State::Unconfirmed, Some(d)) => format!(
            "may already be retired ({provider}'s date was {}; not confirmed)",
            d.human()
        ),
        (State::Unconfirmed, None) => format!("{provider} may have retired (not confirmed)"),
        (State::Deprecated, _) => format!("{provider} has deprecated, with no shutdown date yet"),
        (State::AutoRouted, _) => {
            format!("{provider} now routes to a newer model (calls still work)")
        }
    }
}

/// `3 calls use models OpenAI shuts down on 14 Jan 2027 (88 days)`.
fn headline(group: &[Finding], today: Date) -> String {
    let first = &group[0];
    let subject = if group.len() == 1 {
        "1 call uses a model".to_string()
    } else {
        format!("{} calls use models", group.len())
    };
    let link = if first.status == State::Unconfirmed && first.date.is_some() {
        " that"
    } else {
        ""
    };
    format!("{subject}{link} {}", status_phrase(first, today))
}

fn tick(text: &str, code: bool) -> String {
    if code {
        format!("`{text}`")
    } else {
        text.to_string()
    }
}

/// `max_tokens → max_completion_tokens, add thinking={"type": "between_tools"} (for messages)`.
fn describe_changes(changes: &[Change], code: bool) -> String {
    changes
        .iter()
        .map(|change| match change.kind {
            ChangeKind::Rename => format!(
                "{} → {}",
                tick(&change.param, code),
                tick(change.to.as_deref().unwrap_or_default(), code)
            ),
            ChangeKind::Add => {
                let value = change
                    .value
                    .as_ref()
                    .map(python_literal)
                    .unwrap_or_default();
                let reason = if change.because.is_empty() {
                    String::new()
                } else {
                    format!(" (for {})", change.because.join(", "))
                };
                format!(
                    "add {}{reason}",
                    tick(&format!("{}={value}", change.param), code)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `keep reasoning off (add reasoning_effort="none"; keep tools) or let the
/// model reason (remove tools)`. A parameter an option removes and adds again
/// reads as set: `set thinking={"type": "between_tools"}`.
fn describe_decision(decision: &Decision, code: bool) -> String {
    decision
        .options
        .iter()
        .map(|option| {
            let mut edits = Vec::new();
            for add in &option.add {
                let verb = if option.remove.contains(&add.name) {
                    "set"
                } else {
                    "add"
                };
                edits.push(format!(
                    "{verb} {}",
                    tick(
                        &format!("{}={}", add.name, python_literal(&add.value)),
                        code
                    )
                ));
            }
            if !option.keep.is_empty() {
                edits.push(format!("keep {}", option.keep.join(", ")));
            }
            let removed: Vec<&str> = option
                .remove
                .iter()
                .filter(|name| !option.add.iter().any(|a| &a.name == *name))
                .map(String::as_str)
                .collect();
            if !removed.is_empty() {
                edits.push(format!("remove {}", removed.join(", ")));
            }
            if edits.is_empty() {
                edits.push("no change".to_string());
            }
            format!("{} ({})", option.label, edits.join("; "))
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

/// What to keep in mind about how the value was found and where the call
/// goes: a clause for each that is not confirmed.
fn caveat(finding: &Finding) -> Option<String> {
    let parts: Vec<String> = [value_caveat(finding), routing_caveat(finding)]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn value_caveat(finding: &Finding) -> Option<String> {
    let provider = finding.provider.provider();
    let text = match (finding.confidence, finding.origin) {
        _ if finding.candidate => {
            "one of the values this call may get at run time: confirm which one it uses".to_string()
        }
        (Confidence::Gap, _) => match finding.gap_reason.as_deref().and_then(routing_gap_text) {
            Some(why) => {
                format!("client not resolved ({why}): confirm this call reaches {provider}")
            }
            None => format!("client not resolved: confirm this call reaches {provider}"),
        },
        (Confidence::ConfirmFirst, Some(ValueOrigin::EnvDefault)) => {
            "fallback of an environment or config lookup: confirm the deployed value".to_string()
        }
        (Confidence::ConfirmFirst, Some(ValueOrigin::ParameterDefault)) => {
            "default of a function parameter: callers may pass another model".to_string()
        }
        (Confidence::ConfirmFirst, _) => {
            match finding.gap_reason.as_deref().and_then(value_gap_text) {
                Some(why) => format!("{why}: confirm the model this call uses"),
                None => "not confirmed: confirm the model this call uses".to_string(),
            }
        }
        (Confidence::Confirmed, _) => return None,
    };
    Some(text)
}

fn routing_caveat(finding: &Finding) -> Option<String> {
    let provider = finding.provider.provider();
    let text = match finding.routing_class {
        RoutingClass::Direct => return None,
        // The value's clause already says the client is not resolved.
        RoutingClass::Unknown if finding.confidence == Confidence::Gap => return None,
        RoutingClass::Routed => {
            format!("via {}: {provider}'s dates may not apply", finding.routing)
        }
        RoutingClass::Unknown => {
            let why = match finding.base_url {
                Some(BaseUrl::EnvProviderDefault) => Some(format!(
                    "the base_url is read from the environment and falls back to {provider}'s host"
                )),
                Some(BaseUrl::EnvOnly) => {
                    Some("the base_url is read from the environment with no fallback".to_string())
                }
                Some(BaseUrl::OtherHost) if finding.routing_gap.is_some() => Some(
                    "the base_url is read from the environment and falls back to another host"
                        .to_string(),
                ),
                _ => match finding.routing_gap.as_deref() {
                    Some(reason) => Some(routing_gap_text(reason).unwrap_or(reason).to_string()),
                    None if finding.call_kind == CallKind::RequestParam => {
                        Some("the request that sends this object decides where it goes".to_string())
                    }
                    None => None,
                },
            };
            let subject = if finding.call_kind == CallKind::RequestParam {
                "that request"
            } else {
                "this call"
            };
            match why {
                Some(why) => format!(
                    "routing not confirmed ({why}): confirm {subject} reaches {provider} directly"
                ),
                None => format!("routing unknown: confirm {subject} reaches {provider} directly"),
            }
        }
    };
    Some(text)
}

/// Where to move: `gpt-6-luna`, `gpt-6-luna (via gpt-4o)`, or why there is
/// no single model.
fn target_text(finding: &Finding) -> (String, bool) {
    let chained = finding
        .replacement_chain
        .first()
        .is_some_and(|step| step.status.is_some());
    // A fine-tune, or a fine-tuning job: only a fine-tunable base helps.
    if finding.match_kind == MatchKind::FineTuned || finding.endpoint == Some(FINE_TUNING) {
        let text = finding
            .replacement_text
            .clone()
            .unwrap_or_else(|| "no fine-tunable replacement is listed".to_string());
        return (text, false);
    }
    match (&finding.replacement, &finding.live_replacement) {
        (Some(_), Some(live)) if !chained => (live.clone(), true),
        (Some(_), Some(live)) => {
            let via: Vec<&str> = finding
                .replacement_chain
                .iter()
                .filter(|s| s.status.is_some() && &s.model != live)
                .map(|s| s.model.as_str())
                .collect();
            (format!("{live} (via {})", via.join(", ")), true)
        }
        (Some(listed), None) => {
            let state = finding.replacement_chain.first().and_then(|s| s.status);
            let how = match state {
                Some(State::Retired) => "is shut down too",
                Some(State::Unconfirmed) => "may already be retired",
                _ => "retires too",
            };
            (
                format!("no current replacement (the listed {listed} {how})"),
                false,
            )
        }
        (None, _) if finding.replacement_text.is_some() => {
            ("several listed (see check)".to_string(), false)
        }
        (None, _) => ("none listed".to_string(), false),
    }
}

fn model_text(finding: &Finding) -> String {
    let mut text = finding.model.clone();
    if finding.match_kind == MatchKind::FineTuned {
        text = format!("{} (fine-tune of {})", finding.model, finding.matched);
    }
    if finding.key != "model" {
        text.push_str(&format!(" [{}]", finding.key));
    }
    // The literal to edit sits elsewhere: a constant, a default, a fallback.
    if finding.value_line != finding.line {
        match &finding.value_from {
            Some(name) => text.push_str(&format!(" ({name}, line {})", finding.value_line)),
            None => text.push_str(&format!(" (line {})", finding.value_line)),
        }
    }
    text
}

/// `already retired`, `retired or retiring within 90 days`.
fn window_phrase(fail_within: Option<i64>) -> String {
    match fail_within {
        None | Some(0) => "already retired".to_string(),
        Some(n) => format!("retired or retiring within {}", days(n)),
    }
}

fn summary_lines(report: &RetirementReport, markdown: bool) -> Vec<String> {
    let s = &report.summary;
    let mut lines = Vec::new();
    if s.affected_calls == 0 {
        lines.push(format!(
            "No direct openai/anthropic/google SDK calls to retired or retiring models in {} Python file{} (LangChain's ChatOpenAI and ChatAnthropic, raw HTTP posts to their APIs and the project's own wrappers of them included). Calls through LiteLLM count as routed; Bedrock's own model schedule and other wrapper libraries are not checked yet.",
            s.files_scanned,
            if s.files_scanned == 1 { "" } else { "s" }
        ));
    } else {
        let mut parts = Vec::new();
        if s.retired > 0 {
            parts.push(format!("{} already retired", s.retired));
        }
        if s.retiring > 0 {
            let next = s
                .next_shutdown
                .as_ref()
                .map(|n| format!(" (next: {}, {})", n.date.human(), days(n.days_left)))
                .unwrap_or_default();
            parts.push(format!("{} retiring{next}", s.retiring));
        }
        if s.other > 0 {
            parts.push(format!("{} to keep an eye on", s.other));
        }
        lines.push(format!(
            "{} in {} file{} {} retired or retiring models: {}.",
            calls(s.affected_calls),
            s.affected_files,
            if s.affected_files == 1 { "" } else { "s" },
            verb(s.affected_calls, "uses", "use"),
            parts.join(", ")
        ));
        let mut fixes = Vec::new();
        if s.swap > 0 {
            fixes.push(format!(
                "{} can move to the listed replacement with the changes shown",
                s.swap
            ));
        }
        if s.decide > 0 {
            fixes.push(format!(
                "{} can move once this repo makes the choice shown under decide",
                s.decide
            ));
        }
        if s.manual > 0 {
            fixes.push(format!(
                "{} {} a manual check",
                s.manual,
                verb(s.manual, "needs", "need")
            ));
        }
        if !fixes.is_empty() {
            lines.push(format!("{}.", capitalize(&fixes.join("; "))));
        }
    }
    if report.exit_code == 1 {
        lines.push(format!(
            "Failing (exit 1): {} {} a model that is {}.",
            calls(s.failing),
            verb(s.failing, "uses", "use"),
            window_phrase(report.options.fail_within_days)
        ));
    }
    if s.uncounted > 0 {
        let flag = tick("--count-unconfirmed", markdown);
        lines.push(format!(
            "Not counted toward the exit code: {} to a model that is {}, because the value, the client or the routing is unconfirmed. Add {flag} to count {}.",
            calls(s.uncounted),
            window_phrase(report.options.fail_within_days),
            verb(s.uncounted, "it", "them")
        ));
    }
    lines
}

fn not_checked_lines(report: &RetirementReport, markdown: bool) -> Vec<String> {
    let s = &report.summary;
    let mut lines = Vec::new();
    if s.unreadable_calls > 0 {
        lines.push(format!(
            "Not checked: {} {} the model from somewhere Arbor cannot read (an environment variable, a function argument, another module); {} lists {}.",
            calls(s.unreadable_calls),
            verb(s.unreadable_calls, "takes", "take"),
            tick("arbor usages", markdown),
            verb(s.unreadable_calls, "it", "them")
        ));
    }
    if s.no_model_calls > 0 {
        lines.push(format!(
            "Not checked: {} {} no model, so the SDK's or the server's default applies.",
            calls(s.no_model_calls),
            verb(s.no_model_calls, "passes", "pass")
        ));
    }
    if s.routed_calls > 0 && !report.options.include_routed {
        lines.push(format!(
            "Not listed: {} {} these models through Azure, another host in base_url, another cloud or LiteLLM, where the provider's dates may not apply. Add {} to list {}.",
            calls(s.routed_calls),
            verb(s.routed_calls, "reaches", "reach"),
            tick("--include-routed", markdown),
            verb(s.routed_calls, "it", "them")
        ));
    }
    if s.not_analyzed > 0 {
        lines.push(format!(
            "Not checked: {} {} Arbor did not analyze (scopes nested more than 64 deep, or more than it reads in one file); {} lists {}.",
            s.not_analyzed,
            verb(s.not_analyzed, "part of a file", "parts of files"),
            tick("arbor usages", markdown),
            verb(s.not_analyzed, "it", "them")
        ));
    }
    if s.unresolved_clients > 0 {
        lines.push(format!(
            "Not checked: {} through {} Arbor could not resolve (built by a factory or passed in), whose model it could not read either; {} lists {}.",
            calls(s.unresolved_clients),
            verb(s.unresolved_clients, "a client", "clients"),
            tick("arbor usages", markdown),
            verb(s.unresolved_clients, "it", "them")
        ));
    }
    if s.untracked_methods > 0 {
        lines.push(format!(
            "Not checked: calls through {} SDK {} reached with {} or stored or passed as a value.",
            s.untracked_methods,
            verb(s.untracked_methods, "method", "methods"),
            tick("getattr", markdown)
        ));
    }
    if s.syntax_error_files > 0 {
        lines.push(format!(
            "Partly checked: {} {} syntax errors, so calls inside a broken region may be missing.",
            s.syntax_error_files,
            verb(s.syntax_error_files, "file has", "files have")
        ));
    }
    if s.skipped_files > 0 {
        lines.push(format!(
            "Not checked: {} {} over the size limit or not readable as UTF-8.",
            s.skipped_files,
            verb(s.skipped_files, "file", "files")
        ));
    }
    if s.hidden_calls > 0 {
        lines.push(format!(
            "Not checked: {} in hidden directories (such as {}).",
            calls(s.hidden_calls),
            tick(".github/", markdown)
        ));
    }
    if s.response_objects > 0 {
        lines.push(format!(
            "Skipped: {} SDK response or type object{} built in code, such as {} or {}: {} no request.",
            s.response_objects,
            if s.response_objects == 1 { "" } else { "s" },
            tick("ChatCompletion(...)", markdown),
            tick(".model_validate(...)", markdown),
            verb(s.response_objects, "it sends", "they send")
        ));
    }
    lines
}

/// `OpenAI's pages read 18 Oct 2026; Anthropic's, Google's and Google
/// Cloud's 16 Oct 2026`.
fn provider_dates(report: &RetirementReport) -> String {
    let mut by_date: Vec<(Date, Vec<&str>)> = Vec::new();
    for provider in &report.data.providers {
        match by_date.iter_mut().find(|(d, _)| *d == provider.as_of) {
            Some((_, names)) => names.push(&provider.provider),
            None => by_date.push((provider.as_of, vec![&provider.provider])),
        }
    }
    by_date
        .iter()
        .map(|(date, names)| format!("{} as of {}", join_and(names), date.human()))
        .collect::<Vec<_>>()
        .join("; ")
}

fn join_and(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn data_line(report: &RetirementReport) -> String {
    format!(
        "Retirement data from the providers' deprecation pages: {}. Days are counted in UTC. Python only for now: direct SDK calls, raw HTTP posts to OpenAI's and Anthropic's APIs, the project's own wrappers of them, LangChain chat models and LiteLLM (routed). Runs locally: no code leaves this machine.",
        provider_dates(report)
    )
}

fn stale_line(report: &RetirementReport) -> Option<String> {
    let age = report.data.as_of.days_until(report.today);
    (age > STALE_AFTER_DAYS).then(|| {
        format!("This retirement data is {age} days old; update Arbor to see newer announcements.")
    })
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The choices a group of calls shares, merged by choice: each distinct
/// reason once (one per replacement model), then each option with every
/// effect the group's calls bring up.
struct DecisionBlock {
    reasons: Vec<String>,
    /// `(label, [(effect, unverified)])`.
    options: Vec<(String, Vec<(String, bool)>)>,
}

fn decision_blocks(group: &[Finding]) -> Vec<DecisionBlock> {
    let mut blocks: Vec<(String, DecisionBlock)> = Vec::new();
    for decision in group.iter().flat_map(|f| &f.decisions) {
        let index = match blocks
            .iter()
            .position(|(choice, _)| choice == &decision.choice)
        {
            Some(index) => index,
            None => {
                blocks.push((
                    decision.choice.clone(),
                    DecisionBlock {
                        reasons: Vec::new(),
                        options: decision
                            .options
                            .iter()
                            .map(|o| (o.label.clone(), Vec::new()))
                            .collect(),
                    },
                ));
                blocks.len() - 1
            }
        };
        let block = &mut blocks[index].1;
        if let Some(reason) = decision.reasons.first() {
            if !block.reasons.contains(reason) {
                block.reasons.push(reason.clone());
            }
        }
        for option in &decision.options {
            let Some((_, effects)) = block.options.iter_mut().find(|(l, _)| l == &option.label)
            else {
                continue;
            };
            for effect in &option.effects {
                if !effects.iter().any(|(text, _)| text == &effect.text) {
                    effects.push((
                        effect.text.clone(),
                        effect.status == OptionStatus::Unverified,
                    ));
                }
            }
        }
    }
    blocks.into_iter().map(|(_, block)| block).collect()
}

/// A finding's notes, its decisions' notes, deduplicated across a group.
fn group_notes(group: &[Finding]) -> Vec<String> {
    let mut notes: Vec<String> = Vec::new();
    for finding in group {
        let decision_notes = finding.decisions.iter().flat_map(|d| d.notes.iter());
        for note in decision_notes.chain(&finding.notes) {
            if !notes.contains(note) {
                notes.push(note.clone());
            }
        }
    }
    notes
}

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

fn print_human(report: &RetirementReport, quiet: bool, offer: bool) {
    println!(
        "{} {} {}",
        "Model retirements in".bold(),
        clean(&report.root).bold(),
        format!(
            "({} Python file{} checked)",
            report.summary.files_scanned,
            if report.summary.files_scanned == 1 {
                ""
            } else {
                "s"
            }
        )
        .dimmed()
    );
    if let Some(line) = stale_line(report) {
        println!("{}", line.yellow());
    }

    for group in groups(&report.findings) {
        print_group(group, report.today);
    }
    if !report.routed.is_empty() {
        println!(
            "\n{}",
            "Through Azure, another host in base_url or another cloud (the provider's dates may not apply):"
                .bold()
        );
        for group in groups(&report.routed) {
            print_group(group, report.today);
        }
    }

    println!();
    for line in not_checked_lines(report, false) {
        println!("{}", line.dimmed());
    }
    for (i, line) in summary_lines(report, false).into_iter().enumerate() {
        if i == 0 {
            println!("{} {}", "Summary:".bold(), line);
        } else {
            println!("{line}");
        }
    }
    if !quiet {
        println!("{}", data_line(report).dimmed());
    }
    if offer && (report.summary.affected_calls > 0 || !report.routed.is_empty()) {
        println!("\nWant Arbor to open these fixes as pull requests for you? {FIX_PRS_URL}");
    }
}

fn print_group(group: &[Finding], today: Date) {
    let first = &group[0];
    let title = headline(group, today);
    let title = match first.status {
        State::Retired => title.red().bold(),
        State::Retiring => title.yellow().bold(),
        _ => title.bold(),
    };
    println!("\n{title}");
    let width = group
        .iter()
        .map(|f| location(f).chars().count())
        .max()
        .unwrap_or(0)
        .min(48);
    let indent = " ".repeat(width + 4);
    for finding in group {
        let (target, is_model) = target_text(finding);
        let target = if is_model {
            clean(&target).green().to_string()
        } else {
            clean(&target)
        };
        let mut line = format!(
            "  {:<width$}  {} {} {}",
            clean(&location(finding)),
            clean(&model_text(finding)).cyan(),
            "→".dimmed(),
            target,
            width = width
        );
        if let Some(caveat) = caveat(finding) {
            line.push_str(&format!("  {}", clean(&caveat).dimmed()));
        }
        println!("{line}");
        if !finding.changes.is_empty() {
            println!(
                "{indent}{} {}",
                "also change:".bold(),
                clean(&describe_changes(&finding.changes, false))
            );
        }
        for decision in &finding.decisions {
            println!(
                "{indent}{} {}",
                "decide:".bold(),
                clean(&describe_decision(decision, false))
            );
        }
        for item in &finding.review {
            println!("{indent}{} {}", "check:".bold(), clean(&item.message));
        }
    }
    for block in decision_blocks(group) {
        println!(
            "  {} {}",
            "decide once for this repo (Arbor does not choose):".bold(),
            clean(&block.reasons.join(" "))
        );
        for (label, effects) in &block.options {
            for (index, (effect, unverified)) in effects.iter().enumerate() {
                let lead = if index == 0 {
                    format!("{label}:")
                } else {
                    " ".repeat(label.chars().count() + 1)
                };
                let mark = if *unverified { " (not verified)" } else { "" };
                println!("    {} {}{mark}", lead.bold(), clean(effect));
            }
        }
    }
    for note in group_notes(group) {
        println!("  {}", format!("note: {}", clean(&note)).dimmed());
    }
}

fn location(finding: &Finding) -> String {
    format!("{}:{}", finding.file, finding.line)
}

// ---------------------------------------------------------------------------
// Markdown (GitHub job summary)
// ---------------------------------------------------------------------------

/// A table cell: one line, no unescaped pipes, no HTML.
fn cell(text: &str) -> String {
    clean(text)
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(['\r', '\n'], " ")
}

fn code(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!("`{}`", cell(text).replace('`', "'"))
    }
}

fn markdown(report: &RetirementReport) -> String {
    let mut out = String::from("## Model retirements\n\n");
    if let Some(line) = stale_line(report) {
        out.push_str(&format!("> {line}\n\n"));
    }
    let sections: [(&str, &[Finding]); 2] = [
        ("", &report.findings),
        (
            "### Through Azure, another host in base_url or another cloud\n\nThe provider's dates may not apply to these calls.\n\n",
            &report.routed,
        ),
    ];
    for (title, findings) in sections {
        if findings.is_empty() {
            continue;
        }
        out.push_str(title);
        for group in groups(findings) {
            out.push_str(&format!("**{}**\n\n", cell(&headline(group, report.today))));
            out.push_str("| Call | Model | Replacement | Also change | Decide | Check by hand |\n");
            out.push_str("| --- | --- | --- | --- | --- | --- |\n");
            for finding in group {
                let mut model = code(&model_text(finding));
                if let Some(c) = caveat(finding) {
                    model.push_str(&format!("<br>{}", cell(&c)));
                }
                let (target, is_model) = target_text(finding);
                let replacement = if is_model {
                    code(&target)
                } else {
                    cell(&target)
                };
                let decide = finding
                    .decisions
                    .iter()
                    .map(|d| cell(&describe_decision(d, true)))
                    .collect::<Vec<_>>()
                    .join("<br>");
                let checks = finding
                    .review
                    .iter()
                    .map(|r| cell(&r.message))
                    .collect::<Vec<_>>()
                    .join("<br>");
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    code(&location(finding)),
                    model,
                    replacement,
                    cell(&describe_changes(&finding.changes, true)),
                    decide,
                    checks
                ));
            }
            out.push('\n');
            for block in decision_blocks(group) {
                out.push_str(&format!(
                    "Decide once for this repo (Arbor does not choose): {}\n\n",
                    cell(&block.reasons.join(" "))
                ));
                for (label, effects) in &block.options {
                    for (effect, unverified) in effects {
                        let mark = if *unverified { " (not verified)" } else { "" };
                        out.push_str(&format!("- **{}**: {}{mark}\n", cell(label), cell(effect)));
                    }
                }
                out.push('\n');
            }
            let notes = group_notes(group);
            for note in &notes {
                out.push_str(&format!("- {}\n", cell(note)));
            }
            if !notes.is_empty() {
                out.push('\n');
            }
        }
    }
    for line in not_checked_lines(report, true) {
        out.push_str(&format!("{line}\n\n"));
    }
    for line in summary_lines(report, true) {
        out.push_str(&format!("**{}**\n\n", cell(&line)));
    }
    out.push_str(&format!("<sub>{}</sub>\n", cell(&data_line(report))));
    out
}

// ---------------------------------------------------------------------------
// GitHub annotations
// ---------------------------------------------------------------------------

/// Workflow-command data: `%`, CR and LF escaped.
fn escape_data(text: &str) -> String {
    clean(text)
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Workflow-command property: also `:` and `,`.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

/// One warning per call, up to `limit` (GitHub drops warnings past its
/// per-step limit); then one notice for the rest. The calls that fail the
/// check come first, so a failing step always annotates why; then the rest,
/// most urgent first.
fn annotations(report: &RetirementReport, limit: usize) -> String {
    let mut out = String::new();
    let mut all: Vec<&Finding> = report.findings.iter().chain(&report.routed).collect();
    // Stable: each part keeps the report's order.
    all.sort_by_key(|finding| !finding.fails);
    for finding in all.iter().take(limit) {
        let status = status_phrase(finding, report.today);
        let title = format!("{}: {}", finding.model, status);
        let mut message = format!("{}: {}.", finding.model, status);
        let (target, _) = target_text(finding);
        message.push_str(&format!(" Replacement: {target}."));
        if !finding.changes.is_empty() {
            message.push_str(&format!(
                " Also change: {}.",
                describe_changes(&finding.changes, false)
            ));
        }
        for decision in &finding.decisions {
            message.push_str(&format!(
                " Decide (per repo): {}.",
                describe_decision(decision, false)
            ));
        }
        for item in &finding.review {
            message.push_str(&format!(" Check: {}", item.message));
        }
        if let Some(c) = caveat(finding) {
            message.push_str(&format!(" ({c})"));
        }
        out.push_str(&format!(
            "::warning file={},line={},endLine={},title={}::{}\n",
            escape_property(&finding.file),
            finding.line,
            finding.end_line.max(finding.line),
            escape_property(&title),
            escape_data(&message)
        ));
    }
    if all.len() > limit {
        let more = all.len() - limit;
        out.push_str(&format!(
            "::notice title={}::{}\n",
            escape_property("Arbor retirements"),
            escape_data(&format!(
                "{} more {} to retired or retiring models {} not annotated (at most {limit} warnings per step); the full report lists every call.",
                more,
                verb(more, "call", "calls"),
                verb(more, "is", "are")
            ))
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_commands_are_escaped() {
        assert_eq!(escape_data("50% done"), "50%25 done");
        assert_eq!(escape_property("a:b,c"), "a%3Ab%2Cc");
        // A line break (from a scanned file name, say) cannot end the command.
        let escaped = escape_data("x\r\ny");
        assert!(
            !escaped.contains('\n') && !escaped.contains('\r'),
            "{escaped}"
        );
    }

    #[test]
    fn table_cells_cannot_break_the_table() {
        assert_eq!(cell("a|b<c>"), "a\\|b&lt;c&gt;");
        assert!(!cell("a\nb").contains('\n'));
    }

    #[test]
    fn wording() {
        assert_eq!(calls(1), "1 call");
        assert_eq!(calls(3), "3 calls");
        assert_eq!(days(0), "today");
        assert_eq!(days(1), "1 day");
        assert_eq!(days(88), "88 days");
        assert_eq!(verb(1, "uses", "use"), "uses");
        assert_eq!(verb(2, "uses", "use"), "use");
        assert_eq!(window_phrase(None), "already retired");
        assert_eq!(window_phrase(Some(0)), "already retired");
        assert_eq!(window_phrase(Some(1)), "retired or retiring within 1 day");
        assert_eq!(join_and(&["A", "B", "C"]), "A, B and C");
        assert_eq!(join_and(&["A"]), "A");
    }

    #[test]
    fn ci_hides_the_offer() {
        // `in_ci` reads the environment; only check the pure part here.
        assert!(!show_offer(true));
    }

    #[test]
    fn the_offer_url_is_one_constant() {
        let needle = ["https://getarbor.dev", "/retirements"].concat();
        // The page credits a fix-PR signup to the CLI by this tag.
        assert_eq!(FIX_PRS_URL, format!("{needle}?utm_source=arbor-cli"));
        let source = include_str!("retirements.rs");
        assert_eq!(
            source.matches(&needle).count(),
            1,
            "the URL is written once"
        );
    }

    #[test]
    fn no_findings_make_no_groups() {
        assert!(groups(&[]).is_empty());
    }
}
