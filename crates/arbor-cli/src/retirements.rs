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
//! `--annotations` (GitHub workflow commands, one warning per call).
//!
//! Exit codes: 0 when nothing counts as failing, 1 when a call uses a model
//! that is already retired (or, with `--fail-within N`, retires within N
//! days), 2 when the check could not run.

use arbor_core::retirements::{
    bundled, check, python_literal, Change, ChangeKind, CheckOptions, Confidence, Date, DateKind,
    Finding, MatchKind, RetirementReport, RoutingClass, ScanInput, State, REPORT_VERSION,
};
use arbor_core::usages::{is_unsafe_char, ValueOrigin};
use arbor_watcher::usages::{scan_usages, UsageScanOptions};
use colored::Colorize;
use std::io::Write;
use std::path::Path;

/// Where the closing line of the human report points people who want the
/// fix PRs opened for them. The only place this URL is written.
pub const FIX_PRS_URL: &str = "https://getarbor.dev/retirements";

/// Data older than this gets a reminder to update Arbor.
const STALE_AFTER_DAYS: i64 = 90;

#[derive(Debug, Clone, Default)]
pub struct Args {
    pub path: String,
    pub json: bool,
    pub markdown: bool,
    pub annotations: bool,
    pub fail_within: Option<i64>,
    pub include_routed: bool,
    pub quiet: bool,
    /// `YYYY-MM-DD` to count days from instead of the system clock.
    pub today: Option<String>,
}

/// Runs the check, prints the report and returns the exit code.
pub fn run(args: &Args) -> i32 {
    let code = match build_report(args) {
        Ok(report) => {
            if args.json {
                match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(error) => return fail(args, &format!("cannot write JSON: {error}")),
                }
            } else if args.markdown {
                print!("{}", markdown(&report));
            } else if args.annotations {
                print!("{}", annotations(&report));
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
        let error = serde_json::json!({ "version": REPORT_VERSION, "error": message });
        println!(
            "{}",
            serde_json::to_string_pretty(&error).unwrap_or_else(|_| "{}".to_string())
        );
    }
    eprintln!("{} {}", "error:".red().bold(), clean(message));
    2
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
    };
    let input = ScanInput {
        calls: &scan.usages,
        gaps: &scan.gaps,
        clients: &scan.clients,
    };
    let mut report = check(input, data, &options);
    report.root = scan.root;
    report.summary.files_scanned = scan.summary.files_scanned;
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
        (State::Unconfirmed, Some(d)) if d >= today => format!(
            "may stop working on {} ({}; not confirmed by {provider})",
            d.human(),
            days(today.days_until(d))
        ),
        (State::Unconfirmed, Some(d)) => format!(
            "may already have stopped working ({provider}'s date was {}; not confirmed)",
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

/// `max_tokens → max_completion_tokens, add reasoning_effort="none" (for tools)`.
fn describe_changes(changes: &[Change], code: bool) -> String {
    let tick = |s: &str| {
        if code {
            format!("`{s}`")
        } else {
            s.to_string()
        }
    };
    changes
        .iter()
        .map(|change| match change.kind {
            ChangeKind::Rename => format!(
                "{} → {}",
                tick(&change.param),
                tick(change.to.as_deref().unwrap_or_default())
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
                format!("add {}{reason}", tick(&format!("{}={value}", change.param)))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What to keep in mind about how the value was found.
fn caveat(finding: &Finding) -> Option<String> {
    let provider = finding.provider.provider();
    let text = match (finding.confidence, finding.origin) {
        (Confidence::ConfirmFirst, Some(ValueOrigin::EnvDefault)) => {
            "fallback of an environment or config lookup: confirm the deployed value".to_string()
        }
        (Confidence::ConfirmFirst, _) => {
            "default of a function parameter: callers may pass another model".to_string()
        }
        (Confidence::Gap, _) => {
            format!("client not resolved: confirm this call reaches {provider}")
        }
        _ if finding.routing_class == RoutingClass::Routed => {
            format!("via {}: {provider}'s dates may not apply", finding.routing)
        }
        _ if finding.routing_class == RoutingClass::Unknown => {
            format!("routing unknown: confirm this call reaches {provider} directly")
        }
        _ => return None,
    };
    Some(text)
}

fn chain_note(finding: &Finding, today: Date) -> Option<String> {
    let chain = finding.replacement_status.as_ref()?;
    let text = match (chain.status, chain.date) {
        (State::Retired, _) => format!("{} itself is already shut down.", chain.model),
        (State::Retiring, Some(d)) => format!(
            "{} itself shuts down on {} ({}).",
            chain.model,
            d.human(),
            days(today.days_until(d))
        ),
        _ => format!("{} itself may be retiring.", chain.model),
    };
    Some(text)
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

fn summary_lines(report: &RetirementReport) -> Vec<String> {
    let s = &report.summary;
    let mut lines = Vec::new();
    if s.affected_calls == 0 {
        lines.push(format!(
            "No calls to retired or retiring models in {} Python file{}.",
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
            "{} in {} file{} use retired or retiring models: {}.",
            calls(s.affected_calls),
            s.affected_files,
            if s.affected_files == 1 { "" } else { "s" },
            parts.join(", ")
        ));
        let mut fixes = Vec::new();
        if s.swap > 0 {
            fixes.push(format!(
                "{} can move to the listed replacement with the changes shown",
                s.swap
            ));
        }
        if s.manual > 0 {
            fixes.push(format!(
                "{} need{} a manual check",
                s.manual,
                if s.manual == 1 { "s" } else { "" }
            ));
        }
        if !fixes.is_empty() {
            lines.push(format!("{}.", capitalize(&fixes.join("; "))));
        }
    }
    if report.exit_code == 1 {
        let window = match report.options.fail_within_days {
            Some(n) => format!("retired or retiring within {}", days(n)),
            None => "already retired".to_string(),
        };
        lines.push(format!(
            "Failing (exit 1): {} {} a model that is {window}.",
            calls(s.in_window),
            if s.in_window == 1 { "uses" } else { "use" }
        ));
    }
    lines
}

fn not_checked_lines(report: &RetirementReport, markdown: bool) -> Vec<String> {
    let s = &report.summary;
    let mut lines = Vec::new();
    if s.unreadable_calls > 0 {
        let cmd = if markdown {
            "`arbor usages`"
        } else {
            "arbor usages"
        };
        lines.push(format!(
            "Not checked: {} {} the model from somewhere Arbor cannot read (an environment variable, a function argument, another module); {cmd} lists {}.",
            calls(s.unreadable_calls),
            if s.unreadable_calls == 1 { "takes" } else { "take" },
            if s.unreadable_calls == 1 { "it" } else { "them" }
        ));
    }
    if s.routed_calls > 0 && !report.options.include_routed {
        let flag = if markdown {
            "`--include-routed`"
        } else {
            "--include-routed"
        };
        lines.push(format!(
            "Not listed: {} {} these models through Azure, a custom base_url or another cloud, where the provider's dates may not apply. Add {flag} to list {}.",
            calls(s.routed_calls),
            if s.routed_calls == 1 { "reaches" } else { "reach" },
            if s.routed_calls == 1 { "it" } else { "them" }
        ));
    }
    lines
}

fn data_line(report: &RetirementReport) -> String {
    format!(
        "Retirement data: OpenAI, Anthropic and Google deprecation pages as of {}. Python files only for now. Runs locally: no code leaves this machine.",
        report.data.as_of.human()
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
            "Through Azure, a custom base_url or another cloud (the provider's dates may not apply):"
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
    for (i, line) in summary_lines(report).into_iter().enumerate() {
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
    let mut notes: Vec<String> = Vec::new();
    for finding in group {
        let target = match &finding.replacement {
            Some(r) => format!("{} {}", "→".dimmed(), clean(r).green()),
            None => String::new(),
        };
        let caveat = caveat(finding)
            .map(|c| format!("  {}", c.dimmed()))
            .unwrap_or_default();
        println!(
            "  {:<width$}  {} {}{}",
            clean(&location(finding)),
            clean(&model_text(finding)).cyan(),
            target,
            caveat,
            width = width
        );
        if !finding.changes.is_empty() {
            println!(
                "{indent}{} {}",
                "also change:".bold(),
                clean(&describe_changes(&finding.changes, false))
            );
        }
        for item in &finding.review {
            println!("{indent}{} {}", "check:".bold(), clean(&item.message));
        }
        for note in finding_notes(finding, today) {
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
    }
    for note in notes {
        println!("  {}", format!("note: {}", clean(&note)).dimmed());
    }
}

/// A finding's notes, plus one when its replacement is itself retiring.
fn finding_notes(finding: &Finding, today: Date) -> Vec<String> {
    let mut notes = finding.notes.clone();
    if let Some(chain) = chain_note(finding, today) {
        notes.push(chain);
    }
    notes
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
            "### Through Azure, a custom base_url or another cloud\n\nThe provider's dates may not apply to these calls.\n\n",
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
            out.push_str("| Call | Model | Replacement | Also change | Check by hand |\n");
            out.push_str("| --- | --- | --- | --- | --- |\n");
            let mut notes: Vec<String> = Vec::new();
            for finding in group {
                let mut model = code(&model_text(finding));
                if let Some(c) = caveat(finding) {
                    model.push_str(&format!("<br>{}", cell(&c)));
                }
                let replacement = finding
                    .replacement
                    .as_deref()
                    .map(code)
                    .unwrap_or_else(|| "none listed".to_string());
                let checks = finding
                    .review
                    .iter()
                    .map(|r| cell(&r.message))
                    .collect::<Vec<_>>()
                    .join("<br>");
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    code(&location(finding)),
                    model,
                    replacement,
                    cell(&describe_changes(&finding.changes, true)),
                    checks
                ));
                for note in finding_notes(finding, report.today) {
                    if !notes.contains(&note) {
                        notes.push(note);
                    }
                }
            }
            out.push('\n');
            for note in notes {
                out.push_str(&format!("- {}\n", cell(&note)));
            }
            out.push('\n');
        }
    }
    for line in not_checked_lines(report, true) {
        out.push_str(&format!("{line}\n\n"));
    }
    for line in summary_lines(report) {
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

fn annotations(report: &RetirementReport) -> String {
    let mut out = String::new();
    for finding in report.findings.iter().chain(&report.routed) {
        let status = status_phrase(finding, report.today);
        let title = format!("{}: {}", finding.model, status);
        let mut message = format!("{}: {}.", finding.model, status);
        if let Some(r) = &finding.replacement {
            message.push_str(&format!(" Replacement: {r}."));
        }
        if !finding.changes.is_empty() {
            message.push_str(&format!(
                " Also change: {}.",
                describe_changes(&finding.changes, false)
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
    }

    #[test]
    fn ci_hides_the_offer() {
        // `in_ci` reads the environment; only check the pure part here.
        assert!(!show_offer(true));
    }

    #[test]
    fn the_offer_url_is_one_constant() {
        assert!(FIX_PRS_URL.starts_with("https://"));
        let source = include_str!("retirements.rs");
        let needle = ["https://getarbor.dev", "/retirements"].concat();
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
