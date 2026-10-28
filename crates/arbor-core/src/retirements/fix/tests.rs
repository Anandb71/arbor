use super::super::{bundled, check, CheckOptions, Date, ScanInput};
use super::*;

/// The day the OpenAI data was read: o1, o3-mini and o4-mini retire on
/// 23 Oct 2026, gpt-4o on 14 Jan 2027.
const TODAY: &str = "2026-10-18";

const HEADER: &str = "from openai import OpenAI
from anthropic import Anthropic

client = OpenAI()
claude = Anthropic()
";

fn scan(source: &str) -> FileUsages {
    let tree = parse_python(source).unwrap();
    crate::usages::python::extract(&tree, source, "app.py")
}

/// Every finding for the scan, routed and unconfirmed calls included.
fn findings(usages: &FileUsages) -> Vec<Finding> {
    let input = ScanInput {
        calls: &usages.calls,
        gaps: &usages.gaps,
        clients: &usages.clients,
    };
    let options = CheckOptions {
        today: Date::parse(TODAY).unwrap(),
        fail_within: None,
        include_routed: true,
        count_unconfirmed: true,
    };
    let report = check(input, bundled().unwrap(), &options);
    report.findings.into_iter().chain(report.routed).collect()
}

/// The plan for `source`, and `source` with it applied.
fn fix(source: &str) -> (SwapPlan, String) {
    let usages = scan(source);
    let plan = plan_swaps(source, &usages, &findings(&usages));
    let fixed = apply_edits(source, &plan.edits).unwrap();
    (plan, fixed)
}

/// `HEADER` and one function per call, each returning `call`.
fn project(calls: &[&str]) -> String {
    let mut source = HEADER.to_string();
    for (i, call) in calls.iter().enumerate() {
        source.push_str(&format!("\n\ndef f{i}(q):\n    return {call}\n"));
    }
    source
}

fn reasons(plan: &SwapPlan) -> Vec<(&str, FixKind, SkipReason)> {
    plan.skipped
        .iter()
        .map(|s| (s.model.as_str(), s.fix, s.reason))
        .collect()
}

#[test]
fn a_plain_swap_rewrites_the_model_and_renames_max_tokens() {
    let source = project(&[
        r#"client.chat.completions.create(model="o3-mini", max_tokens=100, messages=[])"#,
    ]);
    let (plan, fixed) = fix(&source);
    assert_eq!(
        fixed,
        source.replace(
            r#"model="o3-mini", max_tokens=100"#,
            r#"model="gpt-5.6-sol", max_completion_tokens=100"#
        )
    );
    assert_eq!(plan.swaps.len(), 1, "{plan:#?}");
    let swap = &plan.swaps[0];
    assert_eq!(
        (swap.from.as_str(), swap.to.as_str()),
        ("o3-mini", "gpt-5.6-sol")
    );
    assert_eq!(
        swap.renames,
        vec![ArgumentRename {
            from: "max_tokens".into(),
            to: "max_completion_tokens".into()
        }]
    );
    assert!(plan.skipped.is_empty(), "{plan:#?}");
    assert_eq!(plan.edits.len(), 2);
    assert!(plan.edits[0].start_byte < plan.edits[1].start_byte);
    // The fixed call no longer uses a listed model.
    assert!(findings(&scan(&fixed)).is_empty());
}

#[test]
fn anthropic_keeps_max_tokens_and_quotes_and_prefixes_are_kept() {
    let source = project(&[
        r#"claude.messages.create(model='claude-3-haiku-20240307', max_tokens=10, messages=[])"#,
        r#"client.chat.completions.create(model=r"o4-mini", messages=[])"#,
        r#"client.chat.completions.create(model="""o1""", messages=[])"#,
    ]);
    let (plan, fixed) = fix(&source);
    let expected = source
        .replace(
            "model='claude-3-haiku-20240307', max_tokens=10",
            "model='claude-haiku-4-5-20251001', max_tokens=10",
        )
        .replace(r#"model=r"o4-mini""#, r#"model=r"gpt-5.6-terra""#)
        .replace(r#"model="""o1""""#, r#"model="""gpt-5.6-sol""""#);
    assert_eq!(fixed, expected, "{plan:#?}");
    assert_eq!(plan.swaps.len(), 3);
    assert!(plan.swaps.iter().all(|s| s.renames.is_empty()));
    assert!(plan.skipped.is_empty(), "{plan:#?}");
}

#[test]
fn decisions_constants_and_added_arguments_are_left_alone() {
    let source = project(&[
        r#"client.chat.completions.create(model="gpt-4o", messages=[], max_tokens=5)"#,
        r#"client.chat.completions.create(model=MODEL, messages=[])"#,
        r#"client.responses.create(model="o1-pro", input=q)"#,
    ])
    .replace(
        "claude = Anthropic()\n",
        "claude = Anthropic()\nMODEL = \"o1\"\n",
    );
    let (plan, fixed) = fix(&source);
    assert_eq!(fixed, source);
    assert!(plan.edits.is_empty() && plan.swaps.is_empty());
    let mut got = reasons(&plan);
    got.sort_by_key(|(model, _, _)| *model);
    assert_eq!(
        got,
        vec![
            ("gpt-4o", FixKind::Decide, SkipReason::NotPlainSwap),
            ("o1", FixKind::Swap, SkipReason::ValueNotAtCall),
            ("o1-pro", FixKind::Swap, SkipReason::AddsArgument),
        ]
    );
}

#[test]
fn unconfirmed_and_routed_calls_are_left_alone() {
    let source = r#"import os
from openai import AzureOpenAI, OpenAI

client = OpenAI()
azure = AzureOpenAI(azure_endpoint="https://x.openai.azure.com", api_version="2024-02-01")


def a(q):
    return client.chat.completions.create(model=os.getenv("M", "o1"), messages=[])


def b(q):
    return azure.chat.completions.create(model="o1", messages=[])
"#;
    let (plan, fixed) = fix(source);
    assert_eq!(fixed, source);
    assert_eq!(
        reasons(&plan),
        vec![
            ("o1", FixKind::Manual, SkipReason::NotPlainSwap),
            ("o1", FixKind::Manual, SkipReason::NotPlainSwap),
        ]
    );
}

#[test]
fn a_call_with_another_model_value_that_needs_a_person_is_left_alone() {
    // `extra_body` may override the top-level model, so swapping only the
    // top-level one would change nothing at run time.
    let source = project(&[
        r#"client.chat.completions.create(model="o3-mini", max_tokens=1, messages=[], extra_body={"model": "o3-mini"})"#,
        r#"client.chat.completions.create(model="o4-mini", messages=[])"#,
    ]);
    let (plan, fixed) = fix(&source);
    assert_eq!(
        fixed,
        source.replace(r#"model="o4-mini""#, r#"model="gpt-5.6-terra""#),
        "{plan:#?}"
    );
    let mut got = reasons(&plan);
    got.sort_by_key(|(_, _, reason)| *reason);
    assert_eq!(
        got,
        vec![
            ("o3-mini", FixKind::Manual, SkipReason::NotPlainSwap),
            ("o3-mini", FixKind::Swap, SkipReason::CallNeedsReview),
        ]
    );
}

#[test]
fn a_call_that_already_passes_the_new_name_is_left_alone() {
    let source = project(&[
        r#"client.chat.completions.create(model="o1", max_tokens=5, max_completion_tokens=5, messages=[])"#,
    ]);
    let (plan, fixed) = fix(&source);
    assert_eq!(fixed, source);
    assert_eq!(
        reasons(&plan),
        vec![("o1", FixKind::Swap, SkipReason::ArgumentNotRenamable)]
    );
}

#[test]
fn a_model_written_as_concatenated_literals_is_left_alone() {
    let source = project(&[r#"client.chat.completions.create(model="o" "1", messages=[])"#]);
    let (plan, fixed) = fix(&source);
    assert_eq!(fixed, source);
    assert_eq!(
        reasons(&plan),
        vec![("o1", FixKind::Swap, SkipReason::LiteralForm)]
    );
}

#[test]
fn byte_offsets_hold_after_non_ascii_text_and_crlf_line_ends() {
    let source = project(&[
        r#"client.chat.completions.create(model="o4-mini", messages=[{"content": "héllo ✓"}], max_tokens=9)"#,
    ])
    .replace("client = OpenAI()\n", "client = OpenAI()  # résumé — naïve\n")
    .replace('\n', "\r\n");
    let (plan, fixed) = fix(&source);
    assert_eq!(plan.swaps.len(), 1, "{plan:#?}");
    assert_eq!(
        fixed,
        source
            .replace(r#""o4-mini""#, r#""gpt-5.6-terra""#)
            .replace("max_tokens=9", "max_completion_tokens=9")
    );
}

#[test]
fn a_gemini_call_is_swapped() {
    let source = r#"from google import genai


def ask(prompt):
    client = genai.Client()
    return client.models.generate_content(model="gemini-2.0-flash", contents=prompt)
"#;
    let (plan, fixed) = fix(source);
    assert_eq!(
        fixed,
        source.replace(r#""gemini-2.0-flash""#, r#""gemini-3.6-flash""#),
        "{plan:#?}"
    );
}

#[test]
fn a_scan_of_other_text_changes_nothing() {
    let scanned =
        project(&[r#"client.chat.completions.create(model="o1", max_tokens=5, messages=[])"#]);
    let usages = scan(&scanned);
    let found = findings(&usages);
    // A line added above the call since the scan.
    let edited = scanned.replace("client = OpenAI()\n", "client = OpenAI()\nTIMEOUT = 30\n");
    let plan = plan_swaps(&edited, &usages, &found);
    assert!(plan.edits.is_empty(), "{plan:#?}");
    assert_eq!(
        reasons(&plan),
        vec![("o1", FixKind::Swap, SkipReason::SourceMismatch)]
    );
}

#[test]
fn only_python_files_without_syntax_errors_are_edited() {
    let source = project(&[r#"client.chat.completions.create(model="o1", messages=[])"#]);
    let mut usages = scan(&source);
    let found = findings(&usages);
    usages.language = "typescript".into();
    let plan = plan_swaps(&source, &usages, &found);
    assert_eq!(
        reasons(&plan),
        vec![("o1", FixKind::Swap, SkipReason::UnsupportedLanguage)]
    );

    let broken = format!("{source}\n\ndef broken(:\n    pass\n");
    let usages = scan(&broken);
    assert!(usages.syntax_errors);
    let plan = plan_swaps(&broken, &usages, &findings(&usages));
    assert!(plan.edits.is_empty());
    assert!(!plan.skipped.is_empty());
    assert!(
        plan.skipped
            .iter()
            .all(|s| s.reason == SkipReason::SyntaxErrors || s.fix != FixKind::Swap),
        "{plan:#?}"
    );
}

#[test]
fn findings_for_other_files_are_ignored() {
    let source = project(&[r#"client.chat.completions.create(model="o1", messages=[])"#]);
    let usages = scan(&source);
    let mut found = findings(&usages);
    for finding in &mut found {
        finding.file = "other.py".into();
    }
    assert_eq!(
        plan_swaps(&source, &usages, &found),
        SwapPlan {
            file: "app.py".into(),
            ..SwapPlan::default()
        }
    );
}

#[test]
fn two_swaps_of_one_literal_clash_and_the_second_is_skipped() {
    let source =
        project(&[r#"client.chat.completions.create(model="o1", max_tokens=5, messages=[])"#]);
    let usages = scan(&source);
    let mut found = findings(&usages);
    assert_eq!(found.len(), 1);
    // The same finding twice makes its edits once.
    found.push(found[0].clone());
    let mut other = found[0].clone();
    other.replacement = Some("gpt-5.6-terra".into());
    other.live_replacement = other.replacement.clone();
    found.push(other);
    let plan = plan_swaps(&source, &usages, &found);
    assert_eq!(plan.swaps.len(), 2, "{plan:#?}");
    assert_eq!(plan.edits.len(), 2);
    assert_eq!(
        reasons(&plan),
        vec![("o1", FixKind::Swap, SkipReason::Overlap)]
    );
    assert!(apply_edits(&source, &plan.edits)
        .unwrap()
        .contains(r#"model="gpt-5.6-sol", max_completion_tokens=5"#));
}

#[test]
fn an_unsafe_replacement_is_never_written() {
    let source = project(&[r#"client.chat.completions.create(model="o1", messages=[])"#]);
    let usages = scan(&source);
    for bad in [
        "gpt\"; import os; \"",
        "gpt\\n",
        "",
        " gpt",
        "gpt{x}",
        "gpt'",
    ] {
        let mut found = findings(&usages);
        found[0].replacement = Some(bad.into());
        found[0].live_replacement = Some(bad.into());
        let plan = plan_swaps(&source, &usages, &found);
        assert!(plan.edits.is_empty(), "{bad:?}");
        assert_eq!(
            reasons(&plan),
            vec![("o1", FixKind::Swap, SkipReason::UnsafeReplacement)]
        );
    }
}

#[test]
fn edits_apply_only_to_the_text_they_were_planned_for() {
    let edit = |start: u32, end: u32, old: &str| Edit {
        start_byte: start,
        end_byte: end,
        line: 1,
        old: old.into(),
        new: "X".into(),
    };
    let source = "abcdef";
    assert_eq!(apply_edits(source, &[]).unwrap(), source);
    assert_eq!(
        apply_edits(source, &[edit(1, 2, "b"), edit(4, 6, "ef")]).unwrap(),
        "aXcdX"
    );
    // Changed text, overlap, wrong order, out of range, not a char boundary.
    assert!(apply_edits(source, &[edit(1, 2, "z")]).is_err());
    assert!(apply_edits(source, &[edit(1, 3, "bc"), edit(2, 4, "cd")]).is_err());
    assert!(apply_edits(source, &[edit(4, 5, "e"), edit(1, 2, "b")]).is_err());
    assert!(apply_edits(source, &[edit(5, 9, "f")]).is_err());
    assert!(apply_edits("é", &[edit(1, 2, "")]).is_err());
}

#[test]
fn literals_and_names() {
    assert_eq!(split_literal("\"o1\""), Some(("", "\"", "o1")));
    assert_eq!(split_literal("'o1'"), Some(("", "'", "o1")));
    assert_eq!(split_literal("Rf'o1'"), Some(("Rf", "'", "o1")));
    assert_eq!(split_literal("'''o1'''"), Some(("", "'''", "o1")));
    assert_eq!(split_literal("rb'o1'"), None);
    assert_eq!(split_literal("b\"o1\""), None);
    assert_eq!(split_literal("\"o\" \"1\""), None);
    assert_eq!(split_literal("\"o\\x31\""), None);
    assert_eq!(split_literal("\"\"\""), None);
    assert_eq!(split_literal("o1"), None);
    assert!(is_plain_model_id("claude-haiku-4-5-20251001"));
    assert!(is_plain_model_id("models/gemini-3.6-flash"));
    assert!(!is_plain_model_id("gpt 5"));
    assert!(!is_plain_model_id(&"a".repeat(MAX_MODEL_ID_BYTES + 1)));
    assert!(is_identifier("max_completion_tokens"));
    assert!(!is_identifier("max-tokens"));
    assert!(!is_identifier("2x"));
}
