//! Mechanical fixes: the model swaps [`super::check`] marks as needing no
//! judgement, turned into edits.
//!
//! [`plan_swaps`] reads one file's findings and returns byte-range edits;
//! [`apply_edits`] makes them. A finding becomes edits only when the check
//! made it a plain [`FixKind::Swap`]: a confirmed value at a direct call, an
//! exact match, a listed replacement that still works, nothing to review and
//! no choice for the repository to make. The edits are:
//!
//! - the model string at the call, rewritten in place with the same prefix
//!   and quotes (`model='o1'` becomes `model='gpt-5.6-sol'`);
//! - each argument rename that goes with it (`max_tokens=` becomes
//!   `max_completion_tokens=`), when the argument is a keyword argument
//!   written at the call.
//!
//! Everything else is left as it is and listed in [`SwapPlan::skipped`] with
//! a [`SkipReason`]: a call that needs a decision (the reasoning effort,
//! Claude's thinking) or a person, a call where any other model value needs
//! one, a model defined outside the call (a constant or a default other
//! code may read too), a swap that adds an argument, and a call whose
//! source does not read the way the usage scan recorded it. Nothing here bumps an SDK, reads files, runs code or reaches
//! the network.
//!
//! The edits are byte ranges in the text the usage scan read, so scan the
//! exact text being edited. Python only, like [`crate::usages`].

use super::{CallKind, ChangeKind, Confidence, Finding, FixKind, MatchKind, RoutingClass};
use crate::usages::{CapturedValue, FileUsages, SdkCall, Span, ValueOrigin};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The usage scan's calls by their 1-based line and column.
type CallIndex<'u> = HashMap<(u32, u32), Vec<&'u SdkCall>>;

/// Where a finding's call is: line, column and chain.
type CallAt<'f> = (u32, u32, Option<&'f str>);

/// The longest replacement model ID that is written into code.
const MAX_MODEL_ID_BYTES: usize = 200;

/// One replacement of a range of the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edit {
    /// 0-based UTF-8 byte offsets, end exclusive.
    pub start_byte: u32,
    pub end_byte: u32,
    /// The 1-based line the range starts on.
    pub line: u32,
    /// The text the range holds now.
    pub old: String,
    /// The text it gets.
    pub new: String,
}

/// An argument renamed together with the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArgumentRename {
    pub from: String,
    pub to: String,
}

/// A call whose model the plan swaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedSwap {
    /// Where the call is, as in the finding.
    pub line: u32,
    pub column: u32,
    pub chain: Option<String>,
    /// The argument holding the model (`model`, `model_name`).
    pub key: String,
    /// The model as written.
    pub from: String,
    /// The replacement written in its place.
    pub to: String,
    pub renames: Vec<ArgumentRename>,
}

/// Why a finding gets no edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The check did not make it a plain swap: the call needs a decision
    /// first, a manual check, or no change ([`SkippedSwap::fix`] says
    /// which).
    NotPlainSwap,
    /// The finding is a plain swap, but another finding at the same call is
    /// not swapped (a model nested in the call's arguments that needs a
    /// manual check, say), so nothing at the call is edited.
    CallNeedsReview,
    /// The model is not written at the call: it comes from a constant or a
    /// default, which other code (or another module) may read too.
    ValueNotAtCall,
    /// The swap adds an argument; only renames are made.
    AddsArgument,
    /// An argument to rename is not a keyword argument written at the call,
    /// is written more than once, or the call already passes its new name.
    ArgumentNotRenamable,
    /// The model is not one plain string literal at the recorded range
    /// (implicit concatenation, escapes, a `b` prefix).
    LiteralForm,
    /// Only Python files are edited.
    UnsupportedLanguage,
    /// The parser recovered from syntax errors in this file.
    SyntaxErrors,
    /// The usage scan or the source does not hold the call and value the
    /// finding names: the file changed since the check, or the scan and the
    /// findings come from different text.
    SourceMismatch,
    /// The replacement is not a plain model ID.
    UnsafeReplacement,
    /// Another swap in the plan edits the same text differently.
    Overlap,
}

impl SkipReason {
    /// The snake_case name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::NotPlainSwap => "not_plain_swap",
            SkipReason::CallNeedsReview => "call_needs_review",
            SkipReason::ValueNotAtCall => "value_not_at_call",
            SkipReason::AddsArgument => "adds_argument",
            SkipReason::ArgumentNotRenamable => "argument_not_renamable",
            SkipReason::LiteralForm => "literal_form",
            SkipReason::UnsupportedLanguage => "unsupported_language",
            SkipReason::SyntaxErrors => "syntax_errors",
            SkipReason::SourceMismatch => "source_mismatch",
            SkipReason::UnsafeReplacement => "unsafe_replacement",
            SkipReason::Overlap => "overlap",
        }
    }
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A finding the plan leaves as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedSwap {
    pub line: u32,
    pub column: u32,
    pub chain: Option<String>,
    pub key: String,
    /// The model as written.
    pub model: String,
    /// What the check said fixing it takes.
    pub fix: FixKind,
    pub reason: SkipReason,
}

/// The edits for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SwapPlan {
    pub file: String,
    /// In source order, never overlapping. None adds or removes a line.
    pub edits: Vec<Edit>,
    /// The findings the edits fix, in the order given.
    pub swaps: Vec<PlannedSwap>,
    /// Every other finding for this file, in the order given.
    pub skipped: Vec<SkippedSwap>,
}

/// Plans the plain swaps in one file. `usages` is the usage scan of
/// `source`; `findings` may hold other files' findings, which are ignored.
/// A finding's edits go in together or not at all, and a call is edited
/// only when every finding at it is swapped.
pub fn plan_swaps(source: &str, usages: &FileUsages, findings: &[Finding]) -> SwapPlan {
    let mut plan = SwapPlan {
        file: usages.file.clone(),
        ..SwapPlan::default()
    };
    let calls = calls_by_position(usages);
    // Parsed for the first swap; `Some(None)` if the parse failed.
    let mut tree = None;
    let findings: Vec<&Finding> = findings.iter().filter(|f| f.file == usages.file).collect();
    let planned: Vec<_> = findings
        .iter()
        .map(|f| swap_edits(source, usages, &calls, f, &mut tree))
        .collect();
    // Calls with a finding that is not swapped (one that needs no change
    // holds nothing back).
    let held: HashSet<CallAt<'_>> = findings
        .iter()
        .zip(&planned)
        .filter(|(f, p)| p.is_err() && f.fix != FixKind::None)
        .map(|(f, _)| call_at(f))
        .collect();

    // Accepted edits by start; they never overlap.
    let mut accepted: BTreeMap<u32, Edit> = BTreeMap::new();
    for (finding, planned) in findings.into_iter().zip(planned) {
        let (edits, swap) = match planned {
            Ok(_) if held.contains(&call_at(finding)) => {
                plan.skipped
                    .push(skipped(finding, SkipReason::CallNeedsReview));
                continue;
            }
            Ok(planned) => planned,
            Err(reason) => {
                plan.skipped.push(skipped(finding, reason));
                continue;
            }
        };
        let mut new_edits: Vec<Edit> = Vec::new();
        let mut clash = false;
        for edit in edits {
            if accepted.get(&edit.start_byte) == Some(&edit) || new_edits.contains(&edit) {
                continue;
            }
            if clashes(&accepted, &edit) || new_edits.iter().any(|e| overlap(e, &edit)) {
                clash = true;
                break;
            }
            new_edits.push(edit);
        }
        if clash {
            plan.skipped.push(skipped(finding, SkipReason::Overlap));
            continue;
        }
        for edit in new_edits {
            accepted.insert(edit.start_byte, edit);
        }
        plan.swaps.push(swap);
    }
    plan.edits = accepted.into_values().collect();
    plan
}

/// `source` with `edits` made. Fails, and makes none, unless the edits are
/// in source order, do not overlap and each range still holds its `old`
/// text.
pub fn apply_edits(source: &str, edits: &[Edit]) -> Result<String, String> {
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for edit in edits {
        let (start, end) = (edit.start_byte as usize, edit.end_byte as usize);
        if start < at || end < start {
            return Err(format!(
                "line {}: edits overlap or are out of order",
                edit.line
            ));
        }
        if source.get(start..end) != Some(edit.old.as_str()) {
            return Err(format!(
                "line {}: the source no longer reads as the plan expects",
                edit.line
            ));
        }
        out.push_str(&source[at..start]);
        out.push_str(&edit.new);
        at = end;
    }
    out.push_str(&source[at..]);
    Ok(out)
}

/// The edits for one finding, or why it gets none.
fn swap_edits(
    source: &str,
    usages: &FileUsages,
    calls: &CallIndex<'_>,
    finding: &Finding,
    tree: &mut Option<Option<tree_sitter::Tree>>,
) -> Result<(Vec<Edit>, PlannedSwap), SkipReason> {
    if !plain_swap(finding) {
        return Err(SkipReason::NotPlainSwap);
    }
    if usages.language != "python" {
        return Err(SkipReason::UnsupportedLanguage);
    }
    if usages.syntax_errors {
        return Err(SkipReason::SyntaxErrors);
    }
    if finding.origin != Some(ValueOrigin::Literal) {
        return Err(SkipReason::ValueNotAtCall);
    }
    let mut renames = Vec::new();
    for change in &finding.changes {
        match (change.kind, &change.to) {
            (ChangeKind::Rename, Some(to)) => renames.push(ArgumentRename {
                from: change.param.clone(),
                to: to.clone(),
            }),
            (ChangeKind::Rename, None) => return Err(SkipReason::ArgumentNotRenamable),
            (ChangeKind::Add, _) => return Err(SkipReason::AddsArgument),
        }
    }
    let to = finding
        .live_replacement
        .as_deref()
        .filter(|m| is_plain_model_id(m))
        .ok_or(SkipReason::UnsafeReplacement)?;
    let (call, value) = locate(calls, finding).ok_or(SkipReason::SourceMismatch)?;
    let tree = tree
        .get_or_insert_with(|| parse_python(source))
        .as_ref()
        .ok_or(SkipReason::SourceMismatch)?;
    let call_node = node_at(tree, &call.span, &["call"]).ok_or(SkipReason::SourceMismatch)?;
    node_at(tree, &value.span, &["string", "concatenated_string"])
        .ok_or(SkipReason::SourceMismatch)?;

    let mut edits = vec![literal_edit(source, value, to)?];
    edits.extend(rename_edits(source, call_node, &renames)?);
    let swap = PlannedSwap {
        line: finding.line,
        column: finding.column,
        chain: finding.chain.clone(),
        key: finding.key.clone(),
        from: finding.model.clone(),
        to: to.to_string(),
        renames,
    };
    Ok((edits, swap))
}

/// What the check calls a plain swap, restated so a finding built some
/// other way cannot slip through.
fn plain_swap(finding: &Finding) -> bool {
    finding.fix == FixKind::Swap
        && finding.confidence == Confidence::Confirmed
        && finding.routing_class == RoutingClass::Direct
        && finding.match_kind == MatchKind::Exact
        && finding.call_kind != CallKind::RequestParam
        && !finding.candidate
        && !finding.splat
        && finding.review.is_empty()
        && finding.decisions.is_empty()
        && finding.replacement.is_some()
        && finding.live_replacement == finding.replacement
}

/// The call and the literal the finding names: the call at the finding's
/// position with its chain, holding a literal for the finding's key with
/// the finding's value on the finding's value line, inside the call. `None`
/// unless exactly one matches.
fn locate<'u>(
    calls: &CallIndex<'u>,
    finding: &Finding,
) -> Option<(&'u SdkCall, &'u CapturedValue)> {
    let mut found = None;
    for &call in calls.get(&(finding.line, finding.column))? {
        if finding.chain.as_deref() != Some(call.chain.as_str()) {
            continue;
        }
        for value in &call.values {
            if value.key == finding.key
                && value.value == finding.model
                && value.origin == ValueOrigin::Literal
                && value.span.line == finding.value_line
                && call.span.start_byte <= value.span.start_byte
                && value.span.end_byte <= call.span.end_byte
            {
                if found.is_some() {
                    return None;
                }
                found = Some((call, value));
            }
        }
    }
    found
}

/// The edit that writes `to` in place of the literal holding `value`.
fn literal_edit(source: &str, value: &CapturedValue, to: &str) -> Result<Edit, SkipReason> {
    let (start, end) = (value.span.start_byte, value.span.end_byte);
    let text = source
        .get(start as usize..end as usize)
        .ok_or(SkipReason::SourceMismatch)?;
    let (prefix, quote, body) = split_literal(text).ok_or(SkipReason::LiteralForm)?;
    if body != value.value {
        return Err(SkipReason::LiteralForm);
    }
    Ok(Edit {
        start_byte: start,
        end_byte: end,
        line: value.span.line,
        old: text.to_string(),
        new: format!("{prefix}{quote}{to}{quote}"),
    })
}

/// A plain Python string literal split into its prefix, its quotes and the
/// text between them. `None` for anything else: a `b` prefix, implicit
/// concatenation, an escape or a quote inside.
fn split_literal(text: &str) -> Option<(&str, &str, &str)> {
    let open = text.find(['"', '\''])?;
    let (prefix, rest) = text.split_at(open);
    if prefix.len() > 2
        || !prefix
            .chars()
            .all(|c| matches!(c, 'r' | 'R' | 'u' | 'U' | 'f' | 'F'))
    {
        return None;
    }
    let quote = ["\"\"\"", "'''", "\"", "'"]
        .into_iter()
        .find(|q| rest.starts_with(q))?;
    if rest.len() < 2 * quote.len() {
        return None;
    }
    let body = rest.strip_prefix(quote)?.strip_suffix(quote)?;
    if body.contains(['"', '\'', '\\', '\n', '\r']) {
        return None;
    }
    Some((prefix, quote, body))
}

/// The node of one of `kinds` that spans exactly `span`.
fn node_at<'t>(
    tree: &'t tree_sitter::Tree,
    span: &Span,
    kinds: &[&str],
) -> Option<tree_sitter::Node<'t>> {
    let (start, end) = (span.start_byte as usize, span.end_byte as usize);
    tree.root_node()
        .descendant_for_byte_range(start, end)
        .filter(|n| kinds.contains(&n.kind()) && n.start_byte() == start && n.end_byte() == end)
}

/// The edits that rename keyword arguments of the call at `call`.
fn rename_edits(
    source: &str,
    call: tree_sitter::Node<'_>,
    renames: &[ArgumentRename],
) -> Result<Vec<Edit>, SkipReason> {
    if renames.is_empty() {
        return Ok(Vec::new());
    }
    let list = call
        .child_by_field_name("arguments")
        .filter(|n| n.kind() == "argument_list")
        .ok_or(SkipReason::ArgumentNotRenamable)?;
    let mut cursor = list.walk();
    let names: Vec<tree_sitter::Node<'_>> = list
        .named_children(&mut cursor)
        .filter(|arg| arg.kind() == "keyword_argument")
        .filter_map(|arg| arg.child_by_field_name("name"))
        .collect();
    let text = |n: &tree_sitter::Node<'_>| source.get(n.byte_range()).unwrap_or("");

    let mut edits = Vec::new();
    for rename in renames {
        if !is_identifier(&rename.to) || names.iter().any(|n| text(n) == rename.to) {
            return Err(SkipReason::ArgumentNotRenamable);
        }
        let mut matching = names.iter().filter(|n| text(n) == rename.from);
        let (Some(name), None) = (matching.next(), matching.next()) else {
            return Err(SkipReason::ArgumentNotRenamable);
        };
        edits.push(Edit {
            start_byte: name.start_byte() as u32,
            end_byte: name.end_byte() as u32,
            line: name.start_position().row as u32 + 1,
            old: rename.from.clone(),
            new: rename.to.clone(),
        });
    }
    Ok(edits)
}

fn parse_python(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_python::language()).ok()?;
    parser.parse(source, None)
}

/// A model ID that can be written between quotes unchanged in any language:
/// ASCII letters and digits, then `.`, `_`, `-`, `:`, `/` or `@`.
fn is_plain_model_id(model: &str) -> bool {
    model.len() <= MAX_MODEL_ID_BYTES
        && model.starts_with(|c: char| c.is_ascii_alphanumeric())
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@'))
}

fn is_identifier(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn calls_by_position(usages: &FileUsages) -> CallIndex<'_> {
    let mut index: CallIndex<'_> = HashMap::new();
    for call in &usages.calls {
        index
            .entry((call.span.line, call.span.column))
            .or_default()
            .push(call);
    }
    index
}

fn call_at(finding: &Finding) -> CallAt<'_> {
    (finding.line, finding.column, finding.chain.as_deref())
}

fn overlap(a: &Edit, b: &Edit) -> bool {
    a.start_byte < b.end_byte && b.start_byte < a.end_byte
}

/// Whether `edit` overlaps an accepted edit. Accepted edits never overlap,
/// so the last one starting before `edit` ends reaches furthest.
fn clashes(accepted: &BTreeMap<u32, Edit>, edit: &Edit) -> bool {
    accepted
        .range(..edit.end_byte)
        .next_back()
        .is_some_and(|(_, e)| overlap(e, edit))
}

fn skipped(finding: &Finding, reason: SkipReason) -> SkippedSwap {
    SkippedSwap {
        line: finding.line,
        column: finding.column,
        chain: finding.chain.clone(),
        key: finding.key.clone(),
        model: finding.model.clone(),
        fix: finding.fix,
        reason,
    }
}

#[cfg(test)]
mod tests;
