//! What a usage-scan gap means for a retirement finding.
//!
//! [`crate::usages`] confirms a value or a route only when no gap sits at its
//! call. Every gap at a call therefore keeps the finding there from reading
//! as confirmed:
//!
//! - an `unresolved_model` gap for the value's key, or one that names no key
//!   (`truncated`), makes the value confirm-first;
//! - any other gap at the call (`ambiguous_client`, and the kinds that never
//!   sit on a resolved call) makes the routing unknown.
//!
//! The effect is decided by the gap's [`GapKind`] alone, never by its reason,
//! so a reason the usage scan adds later is handled the safe way before
//! anyone names it here. [`GAP_REASONS`] only names each reason for people;
//! a test fails when the usage scan can emit a reason it does not list.
//!
//! An `unresolved_model` gap with no callee stands for part of a file the
//! scan did not analyze (scopes nested too deep, too many records): it is a
//! region, counted on its own and never matched to a call.

use crate::usages::{GapKind, UsageGap};

/// What a gap at a call leaves unconfirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    /// The value of the gap's key (of every key, when the gap names none)
    /// may differ from the one captured: confirm first.
    Value,
    /// Where the call goes: its routing is unknown.
    Routing,
}

/// The effect of a gap of `kind` on the call it sits on. Exhaustive on
/// purpose: a new kind does not compile until it is decided here.
pub(super) fn effect(kind: GapKind) -> Effect {
    match kind {
        GapKind::UnresolvedModel => Effect::Value,
        GapKind::AmbiguousClient
        | GapKind::UnknownClientFactory
        | GapKind::DynamicDispatch
        | GapKind::VendoredPath => Effect::Routing,
    }
}

/// Whether `gap` leaves the value captured for `key` unconfirmed.
pub(super) fn holds_value(gap: &UsageGap, key: &str) -> bool {
    effect(gap.kind) == Effect::Value && gap.key.as_deref().is_none_or(|k| k == key)
}

/// Whether `gap` leaves the routing of its call unconfirmed.
pub(super) fn holds_routing(gap: &UsageGap) -> bool {
    effect(gap.kind) == Effect::Routing
}

/// Whether `gap` stands for part of a file the scan did not analyze rather
/// than for one call.
pub(super) fn is_region(gap: &UsageGap) -> bool {
    gap.kind == GapKind::UnresolvedModel && gap.chain.is_none()
}

/// Every reason the usage scan gives a gap, by kind, with what it means for
/// the call as a clause people read ("the model is read from the
/// environment or a config lookup").
pub const GAP_REASONS: &[(GapKind, &str, &str)] = &[
    // The model value.
    (
        GapKind::UnresolvedModel,
        "env_var",
        "the model is read from the environment or a config lookup",
    ),
    (GapKind::UnresolvedModel, "f_string", "the model is built with an f-string"),
    (GapKind::UnresolvedModel, "function_call", "the model is returned by a function call"),
    (GapKind::UnresolvedModel, "attribute", "the model is an attribute of another object"),
    (
        GapKind::UnresolvedModel,
        "subscript",
        "the model is looked up with a key known only at run time",
    ),
    (GapKind::UnresolvedModel, "conditional", "the model is chosen at run time"),
    (GapKind::UnresolvedModel, "parameter", "the model is a parameter with no readable default"),
    (GapKind::UnresolvedModel, "reassigned", "the model is assigned more than once"),
    (GapKind::UnresolvedModel, "imported_name", "the model is imported from another module"),
    (GapKind::UnresolvedModel, "unbound", "the model is not defined in this file"),
    (GapKind::UnresolvedModel, "non_literal", "the model is not a plain string literal"),
    (GapKind::UnresolvedModel, "credential", "the value is credential-shaped, so it is never read"),
    (GapKind::UnresolvedModel, "splat", "a `**` expansion may replace the model"),
    (
        GapKind::UnresolvedModel,
        "model_list",
        "LiteLLM is given a `model_list`, which may map the model to another deployment",
    ),
    (
        GapKind::UnresolvedModel,
        "parameter_default",
        "the model is the default of a parameter other modules may pass",
    ),
    (
        GapKind::UnresolvedModel,
        "parameter_overridden",
        "the model is the default of a parameter that callers in this file set to other values",
    ),
    (
        GapKind::UnresolvedModel,
        "unseen_callers",
        "the model is the default of a parameter of a decorated, passed or stored function, so callers this file does not show may pass another",
    ),
    (
        GapKind::UnresolvedModel,
        "subclass_override",
        "the model is a class attribute that a subclass in another module may override",
    ),
    (
        GapKind::UnresolvedModel,
        "dynamic_feature",
        "this file may change the model at run time (exec, globals(), setattr...)",
    ),
    (
        GapKind::UnresolvedModel,
        "missing_model",
        "no model is passed, so the SDK's or the server's default applies",
    ),
    (
        GapKind::UnresolvedModel,
        "request_body",
        "the model is in an HTTP request body the scan cannot read as JSON",
    ),
    (
        GapKind::UnresolvedModel,
        "local_module",
        "the repository has its own module named like the SDK, which the import may load instead",
    ),
    (
        GapKind::UnresolvedModel,
        "truncated",
        "the call passes more model values than the scan reads, so not all are listed",
    ),
    // The client the call goes through.
    (GapKind::AmbiguousClient, "multiple_assignments", "the client is assigned more than one way"),
    (
        GapKind::AmbiguousClient,
        "in_file_callers",
        "the client is a parameter other modules may pass",
    ),
    (
        GapKind::AmbiguousClient,
        "parameter_default",
        "the client is a parameter default other modules may override",
    ),
    (
        GapKind::AmbiguousClient,
        "unseen_callers",
        "the client is a parameter default that callers this file does not show may override",
    ),
    (
        GapKind::AmbiguousClient,
        "base_url_assigned",
        "the client is given a base_url after it is built",
    ),
    (GapKind::AmbiguousClient, "endpoint_env", "this file sets the SDK's endpoint variable"),
    (GapKind::AmbiguousClient, "base_url_env", "the base_url is read from the environment"),
    (GapKind::AmbiguousClient, "base_url_unresolved", "the base_url is not a value Arbor can read"),
    (
        GapKind::AmbiguousClient,
        "endpoint_option",
        "the client has an option that may send it to Vertex AI or another endpoint",
    ),
    (
        GapKind::AmbiguousClient,
        "module_config",
        "another module may route the module-level API to Azure",
    ),
    (
        GapKind::AmbiguousClient,
        "langchain_module_config",
        "this file configures the module-level OpenAI API, which a deprecated LangChain chat model uses on the pre-1.0 SDK",
    ),
    (
        GapKind::AmbiguousClient,
        "given_client",
        "the requests go through an SDK client or model provider the chat model, agent or model object is given",
    ),
    (
        GapKind::AmbiguousClient,
        "local_module",
        "the repository has its own module named like the SDK, which the import may load instead",
    ),
    (
        GapKind::AmbiguousClient,
        "options_route",
        "the agent definition runs where the options it is given to do, which are not built around it",
    ),
    (
        GapKind::AmbiguousClient,
        "model_route",
        "the Agents SDK picks where requests go from the model, which is not confirmed or names a provider Arbor does not follow",
    ),
    (
        GapKind::AmbiguousClient,
        "dynamic_feature",
        "this file may rebind the client or the SDK's configuration at run time",
    ),
    (
        GapKind::UnknownClientFactory,
        "factory_call",
        "the client comes from a function Arbor does not follow",
    ),
    (GapKind::UnknownClientFactory, "parameter", "the client is passed in as a parameter"),
    (GapKind::UnknownClientFactory, "imported_name", "the client is imported from another module"),
    (
        GapKind::UnknownClientFactory,
        "unassigned_attribute",
        "the client is an attribute the class never assigns",
    ),
    (
        GapKind::UnknownClientFactory,
        "sdk_call_result",
        "the client is returned by another SDK call",
    ),
    (GapKind::UnknownClientFactory, "unbound", "the client is not assigned in this file"),
    (GapKind::UnknownClientFactory, "dynamic_feature", "the client may be rebound at run time"),
    (GapKind::UnknownClientFactory, "unresolved_receiver", "the client could not be resolved"),
    // Calls the scan does not follow.
    (GapKind::DynamicDispatch, "getattr", "the SDK method is reached through getattr"),
    (GapKind::DynamicDispatch, "method_reference", "the SDK method is stored or passed as a value"),
    (
        GapKind::VendoredPath,
        "vendored",
        "the file is in a vendored or virtual-environment directory",
    ),
];

/// What a gap's reason means for the call, or `None` for a reason this
/// version does not name (its effect still follows its kind).
pub fn gap_text(kind: GapKind, reason: &str) -> Option<&'static str> {
    GAP_REASONS
        .iter()
        .find(|(k, r, _)| *k == kind && *r == reason)
        .map(|(_, _, text)| *text)
}

/// [`gap_text`] for a value gap (`unresolved_model`).
pub fn value_gap_text(reason: &str) -> Option<&'static str> {
    gap_text(GapKind::UnresolvedModel, reason)
}

/// [`gap_text`] for a routing gap: the first kind that names `reason`,
/// `ambiguous_client` first.
pub fn routing_gap_text(reason: &str) -> Option<&'static str> {
    [
        GapKind::AmbiguousClient,
        GapKind::UnknownClientFactory,
        GapKind::DynamicDispatch,
        GapKind::VendoredPath,
    ]
    .into_iter()
    .find_map(|kind| gap_text(kind, reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every reason literal the usage scan can put on a gap, read from its
    /// source: `reason: "..."`, `Unresolved::new("...")`, `Self::new("...")`,
    /// `Err("...")`, and the lone literals of a `let reason = ...` or
    /// `let (.., reason) = match ...` statement.
    fn emitted_reasons(source: &str) -> BTreeSet<String> {
        let is_reason = |text: &str| {
            !text.is_empty()
                && text
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        let lone = |line: &str| -> Option<String> {
            let trimmed = line.trim().trim_end_matches(',');
            let trimmed = trimmed
                .strip_prefix('{')
                .and_then(|t| t.strip_suffix('}'))
                .map(str::trim)
                .unwrap_or(trimmed);
            let inner = trimmed.strip_prefix('"')?.strip_suffix('"')?;
            is_reason(inner).then(|| inner.to_string())
        };
        let mut found = BTreeSet::new();
        for opener in ["reason: \"", "Unresolved::new(\"", "Self::new(\"", "Err(\""] {
            for (at, _) in source.match_indices(opener) {
                let rest = &source[at + opener.len()..];
                let Some(end) = rest.find('"') else { continue };
                if is_reason(&rest[..end]) {
                    found.insert(rest[..end].to_string());
                }
            }
        }
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let opens = line.contains("let reason = ")
                || (line.contains("let (") && line.contains("reason) = match"))
                || line.contains("let (reason, ");
            if !opens {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            for next in &lines[index + 1..] {
                let next_indent = next.len() - next.trim_start().len();
                if next_indent == indent && next.trim_start().starts_with("};") {
                    break;
                }
                if let Some(reason) = lone(next) {
                    found.insert(reason);
                }
            }
        }
        found
    }

    #[test]
    fn every_reason_the_usage_scan_emits_is_named() {
        // `vendored` is the scan's own (arbor-watcher) and is listed by hand.
        let emitted = emitted_reasons(include_str!("../usages/python.rs"));
        // The scan itself still finds the reasons it is known to emit, so a
        // change to how the usage scan writes them cannot hide new ones.
        for known in [
            "truncated",
            "dynamic_feature",
            "subclass_override",
            "module_config",
            "endpoint_option",
            "endpoint_env",
            "base_url_env",
            "splat",
            "unseen_callers",
            "missing_model",
            "factory_call",
            "env_var",
            "getattr",
        ] {
            assert!(
                emitted.contains(known),
                "the reason scan no longer finds `{known}`: update emitted_reasons"
            );
        }
        let named: BTreeSet<&str> = GAP_REASONS.iter().map(|(_, r, _)| *r).collect();
        let unnamed: Vec<&String> = emitted
            .iter()
            .filter(|r| !named.contains(r.as_str()))
            .collect();
        assert!(
            unnamed.is_empty(),
            "the usage scan emits gap reasons GAP_REASONS does not name: {unnamed:?}. \
             Decide what each means for a retirement finding and add it."
        );
    }

    #[test]
    fn every_kind_has_an_effect_and_every_entry_is_unique() {
        let mut seen = BTreeSet::new();
        for (kind, reason, text) in GAP_REASONS {
            assert!(
                seen.insert((kind.as_str(), *reason)),
                "{kind}/{reason} listed twice"
            );
            assert!(!text.is_empty() && !text.ends_with('.'));
            // A value gap names the value; every other gap names the route,
            // and is printed inside parentheses.
            match effect(*kind) {
                Effect::Value => assert_eq!(*kind, GapKind::UnresolvedModel),
                Effect::Routing => {
                    assert_ne!(*kind, GapKind::UnresolvedModel);
                    assert!(!text.contains(['(', ')']), "{reason}: {text}");
                }
            }
        }
        assert_eq!(
            routing_gap_text("parameter_default"),
            gap_text(GapKind::AmbiguousClient, "parameter_default")
        );
        assert_eq!(value_gap_text("nonexistent"), None);
    }
}
