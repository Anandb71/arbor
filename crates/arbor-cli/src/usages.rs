//! `arbor usages`: where a project calls the OpenAI, Anthropic and Gemini
//! SDKs, how each client is routed, and which model IDs reach the calls.
//!
//! The scan itself lives in [`arbor_watcher::usages`]; this module only
//! prints it. `--json` prints the full [`UsageReport`] and nothing else on
//! stdout; when the scan cannot run (a missing or empty path, an unknown
//! `--sdk`, or an argument the parser rejects), it prints
//! `{"version", "error"}` instead. Human output escapes
//! control characters, so a scanned file cannot drive the terminal.

use arbor_core::usages::{is_unsafe_char, CapturedValue, GapKind, Sdk, SdkCall, ValueOrigin};
use arbor_watcher::usages::{scan_usages, UsageReport, UsageScanOptions};
use colored::Colorize;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn run(path: &str, sdk: Option<&str>, json: bool) -> Result<()> {
    let sdk = match sdk {
        None => None,
        Some(name) => match Sdk::parse(name) {
            Some(sdk) => Some(sdk),
            None => {
                return fail(
                    json,
                    format!("unknown sdk '{name}': use openai, anthropic, google (or gemini), aws (or bedrock) or litellm"),
                )
            }
        },
    };
    if path.trim().is_empty() {
        return fail(json, "the path to scan is empty".to_string());
    }
    let path = Path::new(path);
    let options = UsageScanOptions {
        sdk,
        // Paths print relative to where the command was run.
        display_root: Some(path.display().to_string()),
        ..UsageScanOptions::default()
    };
    match scan_usages(path, &options) {
        Ok(report) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_report(&report);
            }
            Ok(())
        }
        Err(error) => fail(json, format!("cannot scan {}: {}", path.display(), error)),
    }
}

/// Reports `message` as the command's error; with `--json`, also as a JSON
/// object on stdout so a caller parsing stdout always gets JSON.
fn fail(json: bool, message: String) -> Result<()> {
    if json {
        print_json_error(&message);
    }
    Err(clean(&message).into())
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
        "version": arbor_watcher::usages::USAGE_REPORT_VERSION,
        "error": first.strip_prefix("error: ").unwrap_or(first),
    });
    if let Ok(text) = serde_json::to_string_pretty(&error) {
        println!("{text}");
    }
}

/// Whether the command line (without the program name) runs `usages` with
/// `--json`: global flags may come first, and `--` ends the options.
pub fn wants_json(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    let mut args = args.into_iter();
    let command = args.find(|arg| arg == "--" || !arg.to_string_lossy().starts_with('-'));
    command.is_some_and(|c| c == "usages")
        && args
            .take_while(|arg| arg != "--")
            .any(|arg| arg == "--json")
}

/// `text` with control and bidirectional-override characters escaped.
fn clean(text: &str) -> String {
    if !text.chars().any(is_unsafe_char) {
        return text.to_string();
    }
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

fn print_report(report: &UsageReport) {
    let filter = report
        .sdk
        .map(|sdk| format!(" ({sdk} only)"))
        .unwrap_or_default();
    println!(
        "{} {}{}",
        "SDK usages in".bold(),
        clean(&report.root).bold(),
        filter
    );
    println!(
        "{}",
        format!(
            "{} file(s) scanned ({}); values are shown only for model keys and never for credentials",
            report.summary.files_scanned,
            report.languages.join(", ")
        )
        .dimmed()
    );

    if report.usages.is_empty() {
        println!("\nNo OpenAI, Anthropic, Gemini, Bedrock or LiteLLM calls found.");
    }
    for sdk in Sdk::ALL {
        let calls: Vec<&SdkCall> = report.usages.iter().filter(|c| c.sdk == sdk).collect();
        if calls.is_empty() {
            continue;
        }
        // Requests, and any call carrying a model value, are listed; other
        // SDK calls (helpers, types) are only counted here.
        let (listed, other): (Vec<&SdkCall>, Vec<&SdkCall>) = calls
            .into_iter()
            .partition(|c| c.request || !c.values.is_empty());
        let other_note = if other.is_empty() {
            String::new()
        } else {
            format!(
                ", {} other SDK call{} (helpers, types; see --json)",
                other.len(),
                plural(other.len())
            )
        };
        println!(
            "\n{} ({} listed{})",
            sdk.as_str().bold(),
            listed.len(),
            other_note
        );
        for call in listed {
            print_call(call);
        }
    }

    if !report.clients.is_empty() {
        println!("\n{} ({})", "Clients".bold(), report.clients.len());
        for client in &report.clients {
            let binding = client
                .binding
                .as_deref()
                .map(|b| format!("{} = ", clean(b)))
                .unwrap_or_default();
            let kwargs = if client.kwargs.is_empty() {
                String::new()
            } else {
                format!("  [{}]", clean(&client.kwargs.join(", ")))
            };
            println!(
                "  {}  {}{}  {}{}",
                format!("{}:{}", clean(&client.file), client.span.line).dimmed(),
                binding,
                client.constructor.cyan(),
                client.routing.as_str().yellow(),
                kwargs.dimmed()
            );
        }
    }

    if !report.gaps.is_empty() {
        println!("\n{} ({})", "Gaps".bold(), report.gaps.len());
        for gap in &report.gaps {
            let location = match gap.span {
                Some(span) => format!("{}:{}", clean(&gap.file), span.line),
                None => clean(&gap.file),
            };
            let kind = match gap.kind {
                GapKind::VendoredPath => gap.kind.as_str().dimmed(),
                _ => gap.kind.as_str().red(),
            };
            println!(
                "  {:<24} {}  {}",
                kind,
                location.dimmed(),
                clean(&gap.detail)
            );
        }
    }

    let skipped = &report.skipped;
    if skipped.vendored_files > 0
        || skipped.hidden_files > 0
        || !skipped.too_large.is_empty()
        || !skipped.unreadable.is_empty()
    {
        println!(
            "\n{} {} vendored file(s) ({} with SDK usage, {} call(s)), {} in hidden directories ({} with SDK usage, {} call(s)), {} too large, {} unreadable",
            "Skipped:".bold(),
            skipped.vendored_files,
            skipped.vendored_files_with_usages,
            skipped.vendored_calls,
            skipped.hidden_files,
            skipped.hidden_files_with_usages,
            skipped.hidden_calls,
            skipped.too_large.len(),
            skipped.unreadable.len()
        );
        for root in skipped.hidden_roots.iter().filter(|r| r.calls > 0) {
            println!(
                "  {}",
                format!(
                    "{}: {} SDK call(s) in {} file(s), not scanned as project code",
                    clean(&root.path),
                    root.calls,
                    root.files_with_usages
                )
                .dimmed()
            );
        }
    }
    if !skipped.syntax_errors.is_empty() {
        let files: Vec<String> = skipped.syntax_errors.iter().map(|f| clean(f)).collect();
        println!(
            "{} {} file(s) have syntax errors; calls in the broken part may be missing: {}",
            "Partial:".bold(),
            files.len(),
            files.join(", ")
        );
    }

    let by_sdk: Vec<String> = report
        .summary
        .by_sdk
        .iter()
        .map(|(sdk, count)| format!("{sdk} {count}"))
        .collect();
    let breakdown = if by_sdk.is_empty() {
        String::new()
    } else {
        format!(" ({})", by_sdk.join(", "))
    };
    println!(
        "\n{} {} call(s) ({} request(s)) in {} file(s){}, {} with a model value; {} client(s); {} gap(s)",
        "Summary:".bold(),
        report.summary.calls,
        report.summary.requests,
        report.summary.files_with_calls,
        breakdown,
        report.summary.calls_with_model,
        report.summary.clients,
        report.summary.gaps
    );
}

fn print_call(call: &SdkCall) {
    let values: Vec<String> = call.values.iter().map(describe_value).collect();
    let values = if values.is_empty() {
        String::new()
    } else {
        format!("  {}", values.join(" "))
    };
    let symbol = call
        .symbol
        .as_deref()
        .map(|s| format!("  in {}", clean(s)))
        .unwrap_or_default();
    // A call through a wrapper in another file: where its requests are.
    let through = call
        .wrapper
        .as_ref()
        .map(|w| {
            let lines: Vec<String> = w.requests.iter().map(|r| r.line.to_string()).collect();
            format!("  via {}:{}", clean(&w.file), lines.join(","))
        })
        .unwrap_or_default();
    println!(
        "  {}  {}  {}{}{}{}",
        format!(
            "{}:{}:{}",
            clean(&call.file),
            call.span.line,
            call.span.column
        )
        .dimmed(),
        clean(&call.chain).cyan(),
        call.routing.as_str().yellow(),
        values.green(),
        symbol.dimmed(),
        through.dimmed()
    );
}

fn describe_value(value: &CapturedValue) -> String {
    let line = value.span.line;
    let source = match (value.origin, value.name.as_deref()) {
        (ValueOrigin::EnvDefault, Some(name)) => {
            format!(" (env/config fallback via {name}, line {line})")
        }
        (ValueOrigin::EnvDefault, None) => format!(" (env/config fallback, line {line})"),
        (ValueOrigin::Literal, _) | (_, None) => String::new(),
        (ValueOrigin::ParameterDefault, Some(name)) => {
            format!(" (default of {name}, line {line})")
        }
        (_, Some(name)) => format!(" (via {name}, line {line})"),
    };
    clean(&format!("{}=\"{}\"{}", value.key, value.value, source))
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}
