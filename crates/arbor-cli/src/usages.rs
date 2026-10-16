//! `arbor usages`: where a project calls the OpenAI, Anthropic and Gemini
//! SDKs, how each client is routed, and which model IDs reach the calls.
//!
//! The scan itself lives in [`arbor_watcher::usages`]; this module only
//! prints it. `--json` prints the full [`UsageReport`] and nothing else on
//! stdout, including when the scan fails.

use arbor_core::usages::{CapturedValue, GapKind, Sdk, SdkCall, ValueOrigin};
use arbor_watcher::usages::{scan_usages, UsageReport, UsageScanOptions};
use colored::Colorize;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The `--sdk` filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SdkFilter {
    Openai,
    Anthropic,
    Google,
}

impl From<SdkFilter> for Sdk {
    fn from(filter: SdkFilter) -> Self {
        match filter {
            SdkFilter::Openai => Sdk::Openai,
            SdkFilter::Anthropic => Sdk::Anthropic,
            SdkFilter::Google => Sdk::Google,
        }
    }
}

pub fn run(path: &Path, sdk: Option<SdkFilter>, json: bool) -> Result<()> {
    let options = UsageScanOptions {
        sdk: sdk.map(Sdk::from),
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
        Err(error) => {
            let message = format!("cannot scan {}: {}", path.display(), error);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "version": arbor_watcher::usages::USAGE_REPORT_VERSION,
                        "error": message,
                    }))?
                );
            }
            Err(message.into())
        }
    }
}

fn print_report(report: &UsageReport) {
    let filter = report
        .sdk
        .map(|sdk| format!(" ({sdk} only)"))
        .unwrap_or_default();
    println!(
        "{} {}{}",
        "SDK usages in".bold(),
        report.root.bold(),
        filter
    );
    println!(
        "{}",
        format!(
            "{} Python file(s) scanned; values are shown only for model keys",
            report.summary.files_scanned
        )
        .dimmed()
    );

    if report.usages.is_empty() {
        println!("\nNo OpenAI, Anthropic or Gemini SDK calls found.");
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
                .map(|b| format!("{b} = "))
                .unwrap_or_default();
            let kwargs = if client.kwargs.is_empty() {
                String::new()
            } else {
                format!("  [{}]", client.kwargs.join(", "))
            };
            println!(
                "  {}  {}{}  {}{}",
                format!("{}:{}", client.file, client.span.line).dimmed(),
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
                Some(span) => format!("{}:{}", gap.file, span.line),
                None => gap.file.clone(),
            };
            let kind = match gap.kind {
                GapKind::VendoredPath => gap.kind.as_str().dimmed(),
                _ => gap.kind.as_str().red(),
            };
            println!("  {:<24} {}  {}", kind, location.dimmed(), gap.detail);
        }
    }

    let skipped = &report.skipped;
    if skipped.vendored_files > 0 || !skipped.too_large.is_empty() || !skipped.unreadable.is_empty()
    {
        println!(
            "\n{} {} vendored file(s) ({} with SDK usage, {} call(s)), {} too large, {} unreadable",
            "Skipped:".bold(),
            skipped.vendored_files,
            skipped.vendored_files_with_usages,
            skipped.vendored_calls,
            skipped.too_large.len(),
            skipped.unreadable.len()
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
        .map(|s| format!("  in {s}"))
        .unwrap_or_default();
    println!(
        "  {}  {}  {}{}{}",
        format!("{}:{}:{}", call.file, call.span.line, call.span.column).dimmed(),
        call.chain.cyan(),
        call.routing.as_str().yellow(),
        values.green(),
        symbol.dimmed()
    );
}

fn describe_value(value: &CapturedValue) -> String {
    let source = match (value.origin, value.name.as_deref()) {
        (ValueOrigin::Literal, _) | (_, None) => String::new(),
        (ValueOrigin::ParameterDefault, Some(name)) => {
            format!(" (default of {name}, line {})", value.span.line)
        }
        (_, Some(name)) => format!(" (via {name}, line {})", value.span.line),
    };
    format!("{}=\"{}\"{}", value.key, value.value, source)
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}
