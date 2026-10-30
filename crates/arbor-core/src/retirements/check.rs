//! Matching SDK calls against the retirement data.
//!
//! For every model value [`crate::usages`] captured at an SDK call, [`check`]
//! decides which provider schedule applies from the client's routing, looks
//! the value up (exactly; never a substring or a case-folded match), and
//! applies the parameter rules for the replacement to the call's
//! keyword-argument names. The result says, per call site:
//!
//! - when the model stops working (already retired, or N days left);
//! - what the provider lists as the replacement and, when that model is
//!   itself retired or retiring, the first replacement down the chain that
//!   still works (`gpt-6-luna` via `gpt-4o`);
//! - which other arguments must change with it, which choices the
//!   repository has to make first (the replacement's reasoning effort), and
//!   what needs a person;
//! - how sure the match is: a literal or constant ([`Confidence::Confirmed`]),
//!   an environment or parameter default the deployed value may override
//!   ([`Confidence::ConfirmFirst`]), or a call whose client Arbor could not
//!   resolve ([`Confidence::Gap`]);
//! - whether the call reaches the provider directly. A provider's dates do
//!   not necessarily apply through Azure, a proxy or another cloud, so those
//!   calls are kept apart in [`RetirementReport::routed`].
//!
//! A value is confirmed only when the usage scan put no gap at its call for
//! it, and a route only when no client gap sits at the call: any
//! `unresolved_model` gap for the value's key (or for no key) makes it
//! confirm-first, and any other gap at the call makes the routing unknown
//! (see `gaps.rs`). Parts of files the scan did not analyze, SDK methods it
//! does not follow and files it skipped are counted in the [`Summary`], so a
//! clean report never claims more than was read.
//!
//! Only SDK calls that send a request, bind a model for later requests
//! (`genai.GenerativeModel(...)`) or set a model in a request parameter
//! object (a tool config) are checked. Response and type objects built in
//! code (`ChatCompletion(...)`, `Message(...)`, `.model_validate(...)`) send
//! nothing; they are counted in [`Summary::response_objects`] and skipped.

use super::base_url::{base_url_of, BaseUrl};
use super::gaps;
use super::{
    Action, DateKind, EntryStatus, OptionStatus, ParamRule, RecipeInfo, RetirementData,
    Retirements, RuleEffect, Table, Verified,
};
use super::{Date, ModelEntry};
use crate::usages::{
    model_family, CapturedValue, ClientConstruction, GapKind, Routing, Sdk, SdkCall, UsageGap,
    ValueOrigin, VertexAi,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Version of the [`RetirementReport`] JSON shape.
pub const REPORT_VERSION: u32 = 1;

/// What to check against.
#[derive(Debug, Clone, Copy)]
pub struct CheckOptions {
    /// The day days are counted from.
    pub today: Date,
    /// Count calls whose model retires within this many days (and every
    /// retired one) as failing. `None`: only already-retired models fail.
    pub fail_within: Option<i64>,
    /// List calls that go through Azure, a proxy or another cloud, and let
    /// them count toward the exit code.
    pub include_routed: bool,
    /// Also count toward the exit code the calls that are not confirmed:
    /// a confirm-first value (an environment fallback, a parameter default,
    /// one of several possible values), an unresolved client, or a client
    /// whose routing is unknown. By default only confirmed values at direct
    /// calls fail the check.
    pub count_unconfirmed: bool,
}

/// The usage scan to check.
#[derive(Debug, Clone, Copy)]
pub struct ScanInput<'a> {
    pub calls: &'a [SdkCall],
    pub gaps: &'a [UsageGap],
    pub clients: &'a [ClientConstruction],
}

/// Where a model stands today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Shut down: calls fail (or the provider says they will). A hard
    /// shutdown date counts as retired from that day on, in UTC.
    Retired,
    /// Shuts down on `date`, `days_left` (at least 1) from today.
    Retiring,
    /// Probably shut down, not confirmed for this exact ID; or an "earliest"
    /// date that has been reached.
    Unconfirmed,
    /// Deprecated, no shutdown date yet.
    Deprecated,
    /// Requests are routed to a newer model; nothing fails.
    AutoRouted,
}

/// How the value in the code matched the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// The whole string is a listed ID.
    Exact,
    /// After a `models/` or `publishers/google/models/` prefix (Gemini).
    ResourcePrefix,
    /// After a router prefix (`openai/`, `anthropic/`, `gemini/`).
    RouterPrefix,
    /// A fine-tuned model (`ft:<base>:...`) whose base is listed.
    FineTuned,
}

/// Whether the call reaches the provider's own API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingClass {
    /// A direct client (`OpenAI()`, `Anthropic()`, `genai.Client()`), or one
    /// whose `base_url` is the provider's own host ([`BaseUrl`]).
    Direct,
    /// Azure, another host in `base_url`, Bedrock or Vertex for Claude: the
    /// provider's dates may not apply.
    Routed,
    /// Arbor could not tell (a client from a factory, `**kwargs` passed to
    /// the constructor, a client assigned two ways, a `base_url` read from
    /// the environment or from code Arbor cannot read).
    Unknown,
}

/// How sure the match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// A literal or a same-file constant.
    Confirmed,
    /// An environment/config fallback, a parameter default, or one of the
    /// values a lookup, a branch or an in-file caller may give: the deployed
    /// or passed value may differ, so confirm before acting.
    ConfirmFirst,
    /// An SDK-shaped call whose client Arbor could not resolve (a typed
    /// `unknown_client_factory` gap).
    Gap,
}

/// What fixing the call takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixKind {
    /// Swap the model for the listed replacement and make the listed
    /// argument changes; nothing needs judgement.
    Swap,
    /// Like `Swap`, once the repository has made the choices in `decisions`
    /// (Arbor never makes them).
    Decide,
    /// Something needs a person: see `review`, the confidence, the routing
    /// or the replacement chain.
    Manual,
    /// Nothing fails (the provider routes the ID to a newer model).
    None,
}

/// What the call is, as far as retirements go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    /// Sends a request (`client.chat.completions.create`).
    Request,
    /// Builds a model object later requests use
    /// (`genai.GenerativeModel("gemini-pro")`), or hands the model to
    /// another SDK helper.
    ModelBinding,
    /// A request-side parameter object, such as a tool config
    /// (`tool_param.ImageGeneration(model=...)`): its model is sent with a
    /// request built elsewhere.
    RequestParam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Rename,
    Add,
}

/// An argument change that goes with the model swap.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// The argument to add, or to rename from.
    pub param: String,
    /// For a rename: the new name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// For an add: the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// The call's arguments that make it necessary.
    pub because: Vec<String>,
    pub rules: Vec<String>,
    /// For this call: `live` only when the live check called this
    /// replacement on this API.
    pub verified: Verified,
}

/// Something about the call a person has to look at.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewItem {
    /// The parameter rule, or `None` for a property of the model itself.
    pub rule: Option<String>,
    pub message: String,
    pub because: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<Verified>,
}

/// One model in a replacement chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainStep {
    pub model: String,
    /// Where it stands today; `None` when the data lists no retirement for
    /// it, so it is the replacement to move to.
    pub status: Option<State>,
    pub date: Option<Date>,
    pub date_kind: Option<DateKind>,
    /// What the provider lists for it when it lists several models or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_text: Option<String>,
}

/// A parameter and its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParamValue {
    pub name: String,
    pub value: serde_json::Value,
}

/// What an option does for this call, per rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptionEffect {
    pub rule: String,
    pub text: String,
    pub status: OptionStatus,
}

/// One way to settle a [`Decision`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionOption {
    /// `reasoning_none`, `model_default`.
    pub id: String,
    /// `keep reasoning off`, `let the model reason`.
    pub label: String,
    /// Parameters to add.
    pub add: Vec<ParamValue>,
    /// The call's parameters to remove.
    pub remove: Vec<String>,
    /// The call's parameters to keep.
    pub keep: Vec<String>,
    /// The call's parameters a check line flags (they fail or change on the
    /// replacement) and this option leaves as they are: someone has to look
    /// at them whichever way the choice goes, so the option is never "no
    /// change".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub check: Vec<String>,
    /// The call's parameters this option leaves as they are, whose support
    /// on the replacement no check verified for this API (a check line says
    /// so): Arbor cannot say whether they work.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unverified: Vec<String>,
    pub effects: Vec<OptionEffect>,
}

/// A choice the repository makes once (the recipe's `report_choice`). Arbor
/// shows every option and applies none.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    /// The choice (`G6-REASONING-DEFAULT`): the same id across calls.
    pub choice: String,
    /// Why this call needs it, one sentence per rule.
    pub reasons: Vec<String>,
    /// The call's arguments that bring it up (`tools`, `temperature`).
    pub because: Vec<String>,
    pub rules: Vec<String>,
    pub options: Vec<DecisionOption>,
    /// How the call's automatic changes depend on the choice.
    pub notes: Vec<String>,
    /// Why Arbor could not read every argument the call sends. The options
    /// then list changes only for the arguments it read: an option that
    /// adds nothing (letting the model reason or think) may still need
    /// arguments removed that Arbor could not see, so it is never "no
    /// change". `None` (omitted from JSON) when every argument was read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unchecked_args: Option<UncheckedArgs>,
    /// With `unchecked_args` `splat`: the `**` expressions Arbor could not
    /// read, by name as written (`PARAMS`, `kwargs`). Empty (omitted from
    /// JSON) when they have no name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unread_spreads: Vec<String>,
}

/// Why Arbor could not read every argument a call sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UncheckedArgs {
    /// Arguments are spread into the call with `**`.
    Splat,
    /// The call passes `extra_body`, whose entries the SDK sends as request
    /// arguments.
    ExtraBody,
    /// The usage scan did not record the call's arguments (a call whose
    /// client it could not resolve, from a scan that does not record them).
    NotRecorded,
}

impl UncheckedArgs {
    /// What a person has to check, for the report. `spreads` names the `**`
    /// expressions Arbor could not read (`PARAMS`), for `Splat`. The
    /// TypeScript scan records no spreads: in its files `Splat` means some
    /// property of the options object is not written at the call.
    pub fn message(self, spreads: &[String], syntax: Syntax) -> String {
        match self {
            UncheckedArgs::Splat if syntax == Syntax::JavaScript => {
                "Some properties of the options object are not written at the call (a spread, a computed key or an object passed by name), so Arbor could not check them all."
                    .to_string()
            }
            UncheckedArgs::Splat if spreads.is_empty() => {
                "Some arguments are spread into the call with `**`, so Arbor could not check them all."
                    .to_string()
            }
            UncheckedArgs::Splat => format!(
                "Some arguments come from {}, so Arbor could not check them all.",
                spread_list(spreads, true)
            ),
            UncheckedArgs::ExtraBody => {
                "The entries of `extra_body` are sent as request arguments, and Arbor could not check them."
                    .to_string()
            }
            UncheckedArgs::NotRecorded => {
                "Arbor did not record this call's arguments, so it could not check them."
                    .to_string()
            }
        }
    }
}

/// `**PARAMS and **overrides` (each in backticks with `code`).
pub fn spread_list(names: &[String], code: bool) -> String {
    let names: Vec<String> = names
        .iter()
        .map(|n| {
            if code {
                format!("`**{n}`")
            } else {
                format!("**{n}")
            }
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One affected call site.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    /// The callee as written (`client.chat.completions.create`).
    pub chain: Option<String>,
    pub call_kind: CallKind,
    pub provider: Table,
    /// The value as written.
    pub model: String,
    /// The listed ID it matched.
    pub matched: String,
    pub match_kind: MatchKind,
    /// The argument holding the value (`model`, `engine`, `fallbacks.model`).
    pub key: String,
    pub status: State,
    /// What the data says, before today's date is applied.
    pub listed_status: EntryStatus,
    pub date: Option<Date>,
    pub date_kind: DateKind,
    /// Days from today to `date`; zero or negative once it has come.
    pub days_left: Option<i64>,
    /// The replacement the provider lists for this model.
    pub replacement: Option<String>,
    /// The provider's replacement text when it lists several models, none,
    /// or (for a fine-tune) says why none fits.
    pub replacement_text: Option<String>,
    /// The first replacement that still works: `replacement`, or, when that
    /// is itself retired or retiring, the one listed for it in turn.
    pub live_replacement: Option<String>,
    /// Every model from `replacement` to `live_replacement` (or to the dead
    /// end), with where each stands.
    pub replacement_chain: Vec<ChainStep>,
    pub fix: FixKind,
    pub changes: Vec<Change>,
    /// Choices the repository makes before the swap; never applied by Arbor.
    pub decisions: Vec<Decision>,
    pub review: Vec<ReviewItem>,
    pub notes: Vec<String>,
    /// The client's routing (`openai_direct`, `azure_openai`...), or
    /// `unresolved_client` for a gap.
    pub routing: String,
    pub routing_class: RoutingClass,
    /// For a client given a `base_url`: how its value reads.
    pub base_url: Option<BaseUrl>,
    /// `chat.completions`, `responses`, `messages`...
    pub endpoint: Option<&'static str>,
    pub confidence: Confidence,
    pub origin: Option<ValueOrigin>,
    /// The constant, parameter or environment lookup the value came from.
    pub value_from: Option<String>,
    /// Line of the string literal itself.
    pub value_line: u32,
    /// Why the value is not confirmed: the reason of the usage scan's gap
    /// for this key at this call (`subclass_override`, `dynamic_feature`,
    /// `truncated`, `parameter_overridden`...), or, for a client Arbor could
    /// not resolve, why (`factory_call`).
    pub gap_reason: Option<String>,
    /// Why the routing is not confirmed: the reason of the usage scan's
    /// client gap at this call (`dynamic_feature`, `module_config`,
    /// `endpoint_env`, `base_url_env`...). The routing class is then
    /// `unknown`.
    pub routing_gap: Option<String>,
    /// The value is one of several the call may get at run time (a lookup's
    /// entries, a branch, what an in-file caller passes), not the call's own.
    pub candidate: bool,
    /// Arguments were spread into the call with `**`.
    pub splat: bool,
    /// The model is retired, or retires within `--fail-within` days.
    pub in_window: bool,
    /// Counts toward the exit code: in the window, and confirmed and direct
    /// unless the options count more ([`CheckOptions::count_unconfirmed`],
    /// [`CheckOptions::include_routed`]).
    pub fails: bool,
    pub recipe: String,
    pub recipe_version: String,
    pub verified: Verified,
    /// The provider's deprecations page.
    pub source_url: Option<String>,
}

/// A request whose model Arbor could not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unreadable {
    pub file: String,
    pub line: u32,
    pub chain: Option<String>,
    pub key: Option<String>,
    pub sdk: Option<Sdk>,
    /// `env_var`, `parameter`, `imported_name`...
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeRef {
    pub id: String,
    pub version: String,
    pub provider: String,
    /// The day its provider pages were read.
    pub retrieved_on: String,
}

/// When one provider's pages were read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderDate {
    pub provider: String,
    /// The oldest read of that provider's pages.
    pub as_of: Date,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataInfo {
    /// The oldest date any provider page was read.
    pub as_of: Date,
    /// Per provider, in the order the report lists them.
    pub providers: Vec<ProviderDate>,
    pub source: String,
    pub recipes: Vec<RecipeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportOptions {
    pub fail_within_days: Option<i64>,
    pub include_routed: bool,
    pub count_unconfirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextShutdown {
    pub date: Date,
    pub days_left: i64,
    pub provider: Table,
    pub calls: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Files analyzed (filled in by the caller).
    pub files_scanned: usize,
    /// Calls in `findings`.
    pub affected_calls: usize,
    pub affected_files: usize,
    pub retired: usize,
    pub retiring: usize,
    /// Unconfirmed, deprecated without a date, or auto-routed.
    pub other: usize,
    /// Listed calls whose model is retired or inside the window.
    pub in_window: usize,
    /// Of those, the calls that count toward the exit code.
    pub failing: usize,
    /// Of those, the calls that do not count: unconfirmed values, unresolved
    /// clients or unknown routing (see [`CheckOptions::count_unconfirmed`]).
    pub uncounted: usize,
    pub swap: usize,
    pub decide: usize,
    pub manual: usize,
    pub by_provider: BTreeMap<String, usize>,
    /// The soonest date among retiring calls.
    pub next_shutdown: Option<NextShutdown>,
    /// Calls through Azure, a proxy or another cloud that use a listed
    /// model (listed in `routed` only with `include_routed`).
    pub routed_calls: usize,
    /// Requests whose model could not be read.
    pub unreadable_calls: usize,
    /// Requests that pass no model, so the SDK's or server's default applies.
    pub no_model_calls: usize,
    /// SDK response or type objects built in code with a model value
    /// (`ChatCompletion(...)`, `.model_validate(...)`): no request, skipped.
    pub response_objects: usize,
    /// Parts of files the usage scan did not analyze (scopes nested too
    /// deep, more records or imports than it reads): calls there are not
    /// checked.
    pub not_analyzed: usize,
    /// SDK methods reached through `getattr` or stored or passed as values:
    /// calls through them are not checked.
    pub untracked_methods: usize,
    /// SDK-shaped calls through a client the usage scan could not resolve
    /// (built by a factory, passed in), whose model it could not read either
    /// (a parameter, `**params`, none passed): no finding or other count
    /// shows them, so they are counted here.
    pub unresolved_clients: usize,
    /// Files whose syntax errors the parser recovered from: calls inside a
    /// broken region may be missing (filled in by the caller).
    pub syntax_error_files: usize,
    /// Files over the size limit, or not readable as UTF-8 (filled in by the
    /// caller).
    pub skipped_files: usize,
    /// SDK calls in hidden directories (`.github/`), which the scan excludes
    /// (filled in by the caller).
    pub hidden_calls: usize,
}

/// The result of [`check`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetirementReport {
    pub version: u32,
    pub today: Date,
    /// The scanned path as the caller named it (filled in by the caller).
    pub root: String,
    pub languages: Vec<String>,
    pub data: DataInfo,
    pub options: ReportOptions,
    pub summary: Summary,
    /// Direct calls and calls whose routing is unknown.
    pub findings: Vec<Finding>,
    /// Calls through Azure, a proxy or another cloud; empty unless
    /// `include_routed`.
    pub routed: Vec<Finding>,
    pub unreadable: Vec<Unreadable>,
    /// 1 when a call counts toward failure, else 0.
    pub exit_code: i32,
}

/// Checks a usage scan against the retirement data.
pub fn check(input: ScanInput<'_>, data: &Retirements, options: &CheckOptions) -> RetirementReport {
    let mut direct = Vec::new();
    let mut routed = Vec::new();
    let mut response_objects = 0;

    // The gaps at each call, and each call by its position.
    let mut gaps_at: HashMap<(&str, u32), Vec<&UsageGap>> = HashMap::new();
    for gap in input.gaps.iter().filter(|g| !gaps::is_region(g)) {
        if let Some(span) = gap.span {
            gaps_at
                .entry((gap.file.as_str(), span.start_byte))
                .or_default()
                .push(gap);
        }
    }
    let calls_at: HashMap<(&str, u32), &SdkCall> = input
        .calls
        .iter()
        .map(|c| ((c.file.as_str(), c.span.start_byte), c))
        .collect();
    let at_call = |call: &SdkCall| -> Vec<&UsageGap> {
        gaps_at
            .get(&(call.file.as_str(), call.span.start_byte))
            .cloned()
            .unwrap_or_default()
    };

    for call in input.calls {
        let Some(kind) = call_kind(call) else {
            response_objects += usize::from(!call.values.is_empty());
            continue;
        };
        let here = at_call(call);
        let class = call_routing(call, kind, input.clients, &here);
        for value in &call.values {
            // A gap for this key (or for none) leaves the value unconfirmed,
            // whatever its origin.
            let value_gap = here.iter().find(|g| gaps::holds_value(g, &value.key));
            let confidence = match (confidence_of(value.origin), value_gap) {
                (Confidence::Confirmed, Some(_)) => Confidence::ConfirmFirst,
                (confidence, _) => confidence,
            };
            let Some(mut finding) = finding_for(
                data,
                options,
                Site::Call(call, kind),
                &class,
                value,
                confidence,
            ) else {
                continue;
            };
            finding.gap_reason = value_gap.map(|g| g.reason.clone());
            if finding.routing_class == RoutingClass::Routed {
                routed.push(finding);
            } else {
                direct.push(finding);
            }
        }
    }
    for gap in input.gaps.iter().filter(|g| !g.values.is_empty()) {
        // An unresolved model with listed values: the call is known, its
        // model is one of these (a lookup's entries, a branch, what in-file
        // callers pass). Any other gap with values is an SDK-shaped call
        // whose client was not resolved.
        let call = (gap.kind == GapKind::UnresolvedModel)
            .then(|| {
                gap.span
                    .and_then(|s| calls_at.get(&(gap.file.as_str(), s.start_byte)))
                    .copied()
            })
            .flatten();
        let (site, class, confidence) = match call {
            Some(call) => {
                let Some(kind) = call_kind(call) else {
                    continue;
                };
                (
                    Site::Call(call, kind),
                    call_routing(call, kind, input.clients, &at_call(call)),
                    Confidence::ConfirmFirst,
                )
            }
            None => (
                Site::Gap(gap),
                CallRouting {
                    class: RoutingClass::Unknown,
                    routing: "unresolved_client".to_string(),
                    sdk: gap.sdk,
                    schedule: Schedule::Guess,
                    base_url: None,
                    gap: None,
                },
                if gap.kind == GapKind::UnresolvedModel {
                    Confidence::ConfirmFirst
                } else {
                    Confidence::Gap
                },
            ),
        };
        for value in &gap.values {
            let Some(mut finding) = finding_for(data, options, site, &class, value, confidence)
            else {
                continue;
            };
            finding.gap_reason = Some(gap.reason.clone());
            finding.candidate = gap.kind == GapKind::UnresolvedModel;
            if finding.routing_class == RoutingClass::Routed {
                routed.push(finding);
            } else {
                direct.push(finding);
            }
        }
    }

    // A value can reach the report twice (a parameter default that a caller
    // also passes): keep the first, which is the call's own.
    dedupe_findings(&mut direct);
    dedupe_findings(&mut routed);
    sort_findings(&mut direct);
    sort_findings(&mut routed);
    let routed_calls = routed.len();
    if !options.include_routed {
        routed.clear();
    }
    let (unreadable, no_model_calls) = unreadable(input);

    let mut summary = summarize(&direct, &routed, routed_calls, unreadable.len());
    summary.no_model_calls = no_model_calls;
    summary.response_objects = response_objects;
    summary.not_analyzed = input.gaps.iter().filter(|g| gaps::is_region(g)).count();
    summary.untracked_methods = input
        .gaps
        .iter()
        .filter(|g| g.kind == GapKind::DynamicDispatch)
        .count();
    summary.unresolved_clients = unresolved_clients(input);
    let exit_code = i32::from(summary.failing > 0);

    RetirementReport {
        version: REPORT_VERSION,
        today: options.today,
        root: String::new(),
        languages: crate::usages::USAGE_LANGUAGES
            .iter()
            .map(|l| l.to_string())
            .collect(),
        data: data_info(&data.data, data.as_of),
        options: ReportOptions {
            fail_within_days: options.fail_within,
            include_routed: options.include_routed,
            count_unconfirmed: options.count_unconfirmed,
        },
        summary,
        findings: direct,
        routed,
        unreadable,
        exit_code,
    }
}

fn data_info(data: &RetirementData, as_of: Date) -> DataInfo {
    let mut providers: Vec<(Table, ProviderDate)> = Vec::new();
    for recipe in &data.recipes {
        let Some(date) = Date::parse(&recipe.retrieved_on) else {
            continue;
        };
        match providers
            .iter_mut()
            .find(|(_, p)| p.provider == recipe.provider)
        {
            Some((_, existing)) => existing.as_of = existing.as_of.min(date),
            None => providers.push((
                recipe.table,
                ProviderDate {
                    provider: recipe.provider.clone(),
                    as_of: date,
                },
            )),
        }
    }
    providers.sort_by_key(|(table, _)| *table);
    DataInfo {
        as_of,
        providers: providers.into_iter().map(|(_, p)| p).collect(),
        source: data.source.clone(),
        recipes: data
            .recipes
            .iter()
            .map(|r| RecipeRef {
                id: r.id.clone(),
                version: r.version.clone(),
                provider: r.provider.clone(),
                retrieved_on: r.retrieved_on.clone(),
            })
            .collect(),
    }
}

fn confidence_of(origin: ValueOrigin) -> Confidence {
    match origin {
        ValueOrigin::Literal
        | ValueOrigin::ModuleConstant
        | ValueOrigin::ClassConstant
        | ValueOrigin::LocalConstant => Confidence::Confirmed,
        ValueOrigin::EnvDefault | ValueOrigin::ParameterDefault => Confidence::ConfirmFirst,
    }
}

// ---------------------------------------------------------------------------
// What a call is
// ---------------------------------------------------------------------------

/// Pydantic constructors on SDK types: they build an object from data and
/// send nothing (`ChatCompletion.model_validate(...)`).
const OBJECT_BUILDERS: &[&str] = &[
    "model_validate",
    "model_validate_json",
    "model_validate_strings",
    "model_construct",
    "construct",
    "parse_obj",
    "parse_raw",
    "parse_file",
    "from_orm",
    "model_copy",
];

/// Request-side type names: parameter TypedDicts, configs and request
/// models (`ChatCompletionToolParam`, `GenerateContentConfig`, and
/// `RealtimeSessionCreateRequest`, which openai-agents-python builds with the
/// session's model and sends).
const REQUEST_TYPE_SUFFIXES: &[&str] = &["Param", "Params", "Parameters", "Config", "Request"];

/// The kind of call, or `None` for a response or type object built in code,
/// which sends no request. [`SdkCall::member`] keeps the module path of an
/// imported type (`types.chat.ChatCompletion`), which tells them apart.
fn call_kind(call: &SdkCall) -> Option<CallKind> {
    if call.request {
        return Some(CallKind::Request);
    }
    let segments: Vec<&str> = call.member.split('.').collect();
    let last = segments.last().copied().unwrap_or_default();
    if OBJECT_BUILDERS.contains(&last) {
        return None;
    }
    if segments.first() == Some(&"types") {
        // `openai.types.responses.tool_param.ImageGeneration`: a request
        // parameter module; `openai.types.chat.ChatCompletion`: a response.
        let request_module = segments[..segments.len() - 1]
            .iter()
            .any(|s| s.ends_with("_param") || s.ends_with("_params"));
        let request_type = REQUEST_TYPE_SUFFIXES.iter().any(|s| last.ends_with(s));
        return (request_module || request_type).then_some(CallKind::RequestParam);
    }
    Some(CallKind::ModelBinding)
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

struct CallRouting {
    class: RoutingClass,
    routing: String,
    sdk: Option<Sdk>,
    /// Which provider schedule applies.
    schedule: Schedule,
    base_url: Option<BaseUrl>,
    /// The client gap at the call that leaves its routing unknown.
    gap: Option<String>,
}

/// Which provider schedule a call's model is looked up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schedule {
    /// The client's provider: a direct client follows its schedule only.
    One(Table),
    /// A Gemini client whose `vertexai=` is decided at run time: Vertex AI
    /// first, then the Gemini API. Which one applies is not confirmed.
    VertexOrGeminiApi,
    /// Routed or unknown: a router prefix names the schedule, else the
    /// model ID's shape, else the SDK's own provider.
    Guess,
}

/// The routing [`crate::usages`] decided. A `base_url` is already routed by
/// its host there (the provider's own host is direct; an environment-only or
/// unresolved value is unknown), so it is followed as it is and only named.
/// Any client gap at the call (`at_call`) makes the routing unknown: the
/// scan could not confirm where the call goes. So does a request parameter
/// object: it is built from an imported type, not a client, and the request
/// that sends it decides where it goes. The provider's schedule is still the
/// one looked up.
///
/// A Gemini client follows Vertex AI's schedule only when its constructor
/// says `vertexai=True`. `vertexai=False` or `None` is the Gemini API (the
/// usage scan routes it `google_genai`, with no gap), and a value decided at
/// run time could be either, so both schedules are looked up and the finding
/// says which one it used.
fn call_routing(
    call: &SdkCall,
    kind: CallKind,
    clients: &[ClientConstruction],
    at_call: &[&UsageGap],
) -> CallRouting {
    let base_url = base_url_of(call, at_call, clients);
    let google = || match vertexai_of(call, clients) {
        Some(VertexAi::On) => Schedule::One(Table::Vertex),
        Some(VertexAi::Off) => Schedule::One(Table::GeminiApi),
        Some(VertexAi::Unknown) => Schedule::VertexOrGeminiApi,
        None => Schedule::Guess,
    };
    let (class, schedule) = match call.routing {
        Routing::OpenaiDirect => (RoutingClass::Direct, Schedule::One(Table::Openai)),
        Routing::AnthropicDirect => (RoutingClass::Direct, Schedule::One(Table::Anthropic)),
        Routing::GoogleGenai => match google() {
            Schedule::Guess => (RoutingClass::Direct, Schedule::One(Table::GeminiApi)),
            schedule => (RoutingClass::Direct, schedule),
        },
        Routing::GoogleGenerativeaiLegacy => {
            (RoutingClass::Direct, Schedule::One(Table::GeminiApi))
        }
        Routing::AzureOpenai
        | Routing::CustomBaseUrl
        | Routing::AnthropicBedrock
        | Routing::AnthropicVertex => (RoutingClass::Routed, Schedule::Guess),
        // The usage scan leaves a client built with `vertexai=True` or a
        // run-time value unrouted (an `endpoint_option` gap); the schedule
        // still follows what its constructor says.
        Routing::Unknown if call.sdk == Sdk::Google => (RoutingClass::Unknown, google()),
        Routing::Unknown => (RoutingClass::Unknown, Schedule::Guess),
    };
    let gap = at_call.iter().find(|g| gaps::holds_routing(g));
    CallRouting {
        class: if gap.is_some() || kind == CallKind::RequestParam {
            RoutingClass::Unknown
        } else {
            class
        },
        routing: call.routing.as_str().to_string(),
        sdk: Some(call.sdk),
        schedule,
        base_url,
        gap: gap.map(|g| g.reason.clone()),
    }
}

/// How the `vertexai=` argument of the `genai.Client(...)` behind `call`
/// reads, when its constructor passes one. Two constructions on the client's
/// line that disagree read as decided at run time.
fn vertexai_of(call: &SdkCall, clients: &[ClientConstruction]) -> Option<VertexAi> {
    let line = call.client_line?;
    let mut found = clients
        .iter()
        .filter(|c| c.sdk == Sdk::Google && c.file == call.file && c.span.line == line)
        .filter_map(|c| c.vertexai);
    let first = found.next()?;
    Some(if found.all(|v| v == first) {
        first
    } else {
        VertexAi::Unknown
    })
}

fn sdk_table(sdk: Sdk) -> Table {
    match sdk {
        Sdk::Openai => Table::Openai,
        Sdk::Anthropic => Table::Anthropic,
        Sdk::Google => Table::GeminiApi,
    }
}

/// Router prefixes (OpenRouter, LiteLLM) and the schedule they name.
const ROUTER_PREFIXES: &[(&str, Table)] = &[
    ("openai/", Table::Openai),
    ("anthropic/", Table::Anthropic),
    ("google/", Table::GeminiApi),
    ("gemini/", Table::GeminiApi),
    ("vertex_ai/", Table::Vertex),
];

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// A model value resolved to the data.
struct Hit<'d> {
    matched: String,
    match_kind: MatchKind,
    table: Table,
    recipe: &'d RecipeInfo,
    status: EntryStatus,
    date: Option<Date>,
    replacement: Option<String>,
    replacement_text: Option<String>,
    action: Action,
    rule_set: Option<&'d str>,
    entry: Option<&'d ModelEntry>,
    verified: Verified,
    notes: Vec<String>,
    /// For a fine-tune: what to tell people about moving it.
    fine_tune_advice: Option<String>,
    /// Looked up on Vertex AI's schedule, then the Gemini API's, because the
    /// client's `vertexai=` is decided at run time.
    schedule_unconfirmed: bool,
}

fn entry_hit<'d>(data: &'d Retirements, entry: &'d ModelEntry, kind: MatchKind) -> Option<Hit<'d>> {
    let recipe = data.recipe(&entry.recipe)?;
    Some(Hit {
        matched: entry.model.clone(),
        match_kind: kind,
        table: entry.table,
        recipe,
        status: entry.status,
        date: entry.date.as_deref().and_then(Date::parse),
        replacement: entry.replacement.clone(),
        replacement_text: entry.replacement_text.clone(),
        action: entry.action,
        rule_set: entry.rule_set.as_deref(),
        entry: Some(entry),
        verified: entry.verified,
        notes: entry.notes.clone(),
        fine_tune_advice: None,
        schedule_unconfirmed: false,
    })
}

/// Looks `value` up on one table: exactly, then without a Gemini resource
/// prefix, then as an OpenAI fine-tune.
fn lookup<'d>(data: &'d Retirements, table: Table, value: &str) -> Option<Hit<'d>> {
    if let Some(entry) = data.entry(table, value) {
        return entry_hit(data, entry, MatchKind::Exact);
    }
    if matches!(table, Table::GeminiApi | Table::Vertex) {
        for prefix in ["models/", "publishers/google/models/"] {
            if let Some(entry) = value
                .strip_prefix(prefix)
                .and_then(|id| data.entry(table, id))
            {
                let mut hit = entry_hit(data, entry, MatchKind::ResourcePrefix)?;
                hit.notes.push(format!(
                    "Matched without the `{prefix}` prefix; Google lists the bare ID."
                ));
                return Some(hit);
            }
        }
    }
    if table == Table::Openai {
        let base = value.strip_prefix("ft:")?.split(':').next()?;
        return fine_tune_hit(data, table, base);
    }
    None
}

const NONE_LISTED: &str = "no fine-tunable replacement is listed";

/// What to tell people about fine-tunes of `base`, from the data's fine-tune
/// row: `subject` is what the call is (`Fine-tuned model`, `Fine-tuning
/// job`) and `retrain` what to do on a listed base that can be fine-tuned.
/// Returns the advice and the replacement text (`retrain on X`, or that no
/// fine-tunable replacement is listed).
fn fine_tune_advice(
    fine: &super::FineTuneEntry,
    base: &str,
    subject: &str,
    retrain: &str,
) -> (String, String) {
    let note = fine.note.as_deref().unwrap_or_default();
    let advice = match (&fine.replacement_base, fine.fine_tunable) {
        (Some(listed), true) => format!(
            "{subject}: {retrain} {listed}, the base model OpenAI lists for fine-tunes of {base}."
        ),
        (Some(listed), false) => format!(
            "{subject}: OpenAI lists {listed} for fine-tunes of {base}, so {NONE_LISTED}. {note}"
        ),
        (None, _) => format!(
            "{subject}: {} for fine-tunes of {base}. {note}",
            capitalize(NONE_LISTED)
        ),
    };
    let text = fine
        .replacement_base
        .as_ref()
        .filter(|_| fine.fine_tunable)
        .map(|b| format!("retrain on {b}"))
        .unwrap_or_else(|| NONE_LISTED.to_string());
    (advice.trim_end().to_string(), text)
}

/// A fine-tune of `base`. Advice to retrain names a base model only when the
/// data lists one for these fine-tunes and it can itself be fine-tuned;
/// otherwise the report says no fine-tunable replacement is listed.
fn fine_tune_hit<'d>(data: &'d Retirements, table: Table, base: &str) -> Option<Hit<'d>> {
    if let Some(fine) = data.fine_tune(table, base) {
        let recipe = data.recipe(&fine.recipe)?;
        let (advice, text) = fine_tune_advice(fine, base, "Fine-tuned model", "retrain it on");
        return Some(Hit {
            matched: base.to_string(),
            match_kind: MatchKind::FineTuned,
            table,
            recipe,
            status: fine.status,
            date: Date::parse(&fine.date),
            replacement: None,
            replacement_text: Some(text),
            action: Action::ReportOnly,
            rule_set: None,
            entry: None,
            verified: Verified::Docs,
            notes: vec![format!(
                "OpenAI retires fine-tunes of {base} with their base model's schedule."
            )],
            fine_tune_advice: Some(advice),
            schedule_unconfirmed: false,
        });
    }
    // A base the data lists as a model but not for fine-tunes: its listed
    // replacement is a base model, which may not be fine-tunable.
    let entry = data.entry(table, base)?;
    let mut hit = entry_hit(data, entry, MatchKind::FineTuned)?;
    hit.replacement = None;
    hit.replacement_text = Some(NONE_LISTED.to_string());
    hit.action = Action::ReportOnly;
    hit.rule_set = None;
    hit.notes = vec![format!(
        "Fine-tuned models follow the schedule of their base model, {base}."
    )];
    hit.fine_tune_advice = Some(format!(
        "Fine-tuned model: OpenAI lists no replacement for fine-tunes of {base}, so {NONE_LISTED}."
    ));
    Some(hit)
}

/// `hit` for a fine-tuning job (`client.fine_tuning.jobs.create(model=...)`):
/// the model is the base the job trains on, so the advice comes from the
/// data's fine-tune rows, never from the base's listed replacement, which
/// may not support fine-tuning. The job keeps its base model's status and
/// date, takes no argument rules and always needs a person.
fn fine_tuning_job_hit<'d>(data: &'d Retirements, mut hit: Hit<'d>) -> Hit<'d> {
    if hit.match_kind == MatchKind::FineTuned {
        // An `ft:` model as the job's base: the fine-tune advice already
        // says what to do with it.
        return hit;
    }
    let base = hit.matched.clone();
    let provider = hit.table.provider();
    let (advice, text) = match data.fine_tune(hit.table, &base) {
        Some(fine) => fine_tune_advice(fine, &base, "Fine-tuning job", "start new jobs on"),
        None => {
            let listed = match &hit.replacement {
                Some(listed) => format!(
                    "{provider} lists {listed} as the replacement for {base}, not as a base for fine-tuning"
                ),
                None => format!("{provider} lists no replacement for {base}"),
            };
            (
                format!("Fine-tuning job: {listed}, so {NONE_LISTED}."),
                NONE_LISTED.to_string(),
            )
        }
    };
    hit.replacement = None;
    hit.replacement_text = Some(text);
    hit.action = Action::ReportOnly;
    hit.rule_set = None;
    hit.fine_tune_advice = Some(advice);
    hit
}

/// Resolves a value for a call, deciding which schedule applies.
fn resolve<'d>(
    data: &'d Retirements,
    routing: &CallRouting,
    value: &str,
) -> Option<(Hit<'d>, RoutingClass)> {
    match routing.schedule {
        // A direct client follows its provider's schedule only.
        Schedule::One(table) => return lookup(data, table, value).map(|hit| (hit, routing.class)),
        Schedule::VertexOrGeminiApi => {
            let mut hit = lookup(data, Table::Vertex, value)
                .or_else(|| lookup(data, Table::GeminiApi, value))?;
            hit.schedule_unconfirmed = true;
            return Some((hit, RoutingClass::Unknown));
        }
        Schedule::Guess => {}
    }
    // Routed or unknown: a router prefix names the schedule, else the model
    // ID's shape, else the SDK's own provider.
    for (prefix, table) in ROUTER_PREFIXES {
        if let Some(id) = value.strip_prefix(prefix) {
            let mut hit = lookup(data, *table, id)?;
            hit.match_kind = MatchKind::RouterPrefix;
            return Some((hit, RoutingClass::Routed));
        }
    }
    let table = model_family(value)
        .map(sdk_table)
        .or(routing.sdk.map(sdk_table))?;
    let hit = match lookup(data, table, value) {
        Some(hit) => hit,
        // A Google client whose endpoint is not known may be on Vertex AI,
        // which keeps its own schedule (`text-bison` is listed only there).
        None if table == Table::GeminiApi && routing.sdk == Some(Sdk::Google) => {
            lookup(data, Table::Vertex, value)?
        }
        None => return None,
    };
    let table = hit.table;
    // An SDK sending another provider's model is talking to a proxy.
    let crosses = routing.sdk.is_some_and(|sdk| sdk_table(sdk) != table)
        && !(routing.sdk == Some(Sdk::Google) && table == Table::Vertex);
    let class = if crosses {
        RoutingClass::Routed
    } else {
        routing.class
    };
    Some((hit, class))
}

// ---------------------------------------------------------------------------
// Replacement chains
// ---------------------------------------------------------------------------

struct Chain<'d> {
    steps: Vec<ChainStep>,
    /// The first model down the chain that still works.
    live: Option<String>,
    /// The entry whose listed replacement is `live`, when that is not the
    /// model's own listing: its rule set describes the move.
    last_hop: Option<&'d ModelEntry>,
}

impl Chain<'_> {
    /// The listed replacement is itself retired, retiring or deprecated.
    fn chained(&self) -> bool {
        self.steps.first().is_some_and(|s| s.status.is_some())
    }
}

/// Follows `first` (a listed replacement) through the data on `table` to the
/// first model that is not retired, retiring or unconfirmed. A deprecated or
/// auto-routed model with a replacement of its own is followed too; without
/// one it still works, so the chain ends there.
fn follow_chain<'d>(data: &'d Retirements, table: Table, first: &str, today: Date) -> Chain<'d> {
    let mut steps = Vec::new();
    let mut seen = HashSet::new();
    let mut current = first.to_string();
    let mut last_hop = None;
    loop {
        if !seen.insert(current.clone()) || steps.len() >= 8 {
            return Chain {
                steps,
                live: None,
                last_hop,
            };
        }
        let Some(entry) = data.entry(table, &current) else {
            steps.push(ChainStep {
                model: current.clone(),
                status: None,
                date: None,
                date_kind: None,
                replacement_text: None,
            });
            return Chain {
                steps,
                live: Some(current),
                last_hop,
            };
        };
        let date = entry.date.as_deref().and_then(Date::parse);
        let kind = data
            .recipe(&entry.recipe)
            .map(|r| r.date_kind)
            .unwrap_or(DateKind::Hard);
        let state = effective_state(entry.status, date, kind, today);
        steps.push(ChainStep {
            model: entry.model.clone(),
            status: Some(state),
            date,
            date_kind: Some(kind),
            replacement_text: entry.replacement_text.clone(),
        });
        match &entry.replacement {
            Some(next) => {
                last_hop = Some(entry);
                current = next.clone();
            }
            None => {
                let works = matches!(state, State::Deprecated | State::AutoRouted);
                return Chain {
                    steps,
                    live: works.then(|| entry.model.clone()),
                    last_hop,
                };
            }
        }
    }
}

/// `shuts down on 14 Jan 2027 (88 days)`.
fn step_phrase(step: &ChainStep, today: Date) -> String {
    match (step.status, step.date) {
        (Some(State::Retired), Some(d)) if d <= today => format!("shut down on {}", d.human()),
        (Some(State::Retired), _) => "is already shut down".to_string(),
        (Some(State::Retiring), Some(d)) => {
            let left = days_phrase(today.days_until(d));
            match step.date_kind {
                Some(DateKind::Earliest) => format!("retires on or after {} ({left})", d.human()),
                _ => format!("shuts down on {} ({left})", d.human()),
            }
        }
        (Some(State::Retiring), None) => "is retiring".to_string(),
        (Some(State::Unconfirmed), _) => "may already be retired".to_string(),
        (Some(State::Deprecated), _) => "is deprecated".to_string(),
        (Some(State::AutoRouted), _) => "is routed to a newer model".to_string(),
        (None, _) => "is current".to_string(),
    }
}

fn days_phrase(days: i64) -> String {
    match days {
        0 => "today".to_string(),
        1 => "1 day".to_string(),
        n => format!("{n} days"),
    }
}

/// Why a chained replacement needs a person. `rules_known` says whether a
/// rule set describes the last hop's move; without one the parameters the
/// live model needs are unchecked, and the message says so.
fn chain_message(provider: &str, chain: &Chain<'_>, today: Date, rules_known: bool) -> String {
    let dead: Vec<&ChainStep> = chain
        .steps
        .iter()
        .take_while(|s| s.status.is_some() && Some(&s.model) != chain.live.as_ref())
        .collect();
    let mut text = String::new();
    for (index, step) in dead.iter().enumerate() {
        if index == 0 {
            text.push_str(&format!(
                "{provider} lists {} as the replacement, and it {}",
                step.model,
                step_phrase(step, today)
            ));
        } else {
            text.push_str(&format!(
                "; {provider} lists {} for it, and it {}",
                step.model,
                step_phrase(step, today)
            ));
        }
    }
    let last = dead.last().map(|s| s.model.as_str()).unwrap_or_default();
    match &chain.live {
        Some(live) if rules_known => text.push_str(&format!(
            ". {live} is the replacement {provider} lists for {last}; the changes shown are for {live}."
        )),
        Some(live) => text.push_str(&format!(
            ". {live} is the replacement {provider} lists for {last}; the parameters {live} needs have not been checked yet."
        )),
        None => match dead.last().and_then(|s| s.replacement_text.as_deref()) {
            Some(listed) => text.push_str(&format!(
                ". For {last}, {provider} lists {listed}: pick one by hand."
            )),
            None => text.push_str(&format!(
                ". {provider} lists no replacement for {last}: pick one by hand."
            )),
        },
    }
    text
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Site<'a> {
    Call(&'a SdkCall, CallKind),
    Gap(&'a UsageGap),
}

fn finding_for(
    data: &Retirements,
    options: &CheckOptions,
    site: Site<'_>,
    routing: &CallRouting,
    value: &CapturedValue,
    confidence: Confidence,
) -> Option<Finding> {
    let (hit, class) = resolve(data, routing, &value.value)?;

    // `args`: the call's argument names and whether some are spread with
    // `**`, or `None` when the usage scan did not record them.
    let (file, span, symbol, chain, args, endpoint, call_kind) = match site {
        Site::Call(call, kind) => (
            call.file.clone(),
            Some(call.span),
            call.symbol.clone(),
            Some(call.chain.clone()),
            Some((call.kwargs.clone(), call.splat, call.spreads.clone())),
            // A parameter object's arguments are not the request's.
            (kind != CallKind::RequestParam)
                .then(|| endpoint_of(&call.chain, &call.member))
                .flatten(),
            kind,
        ),
        Site::Gap(gap) => {
            let chain = gap.chain.clone().unwrap_or_default();
            (
                gap.file.clone(),
                gap.span,
                gap.symbol.clone(),
                gap.chain.clone(),
                // A call whose client was not resolved still shows its
                // arguments: the same rules apply to them.
                gap.args
                    .as_ref()
                    .map(|a| (a.kwargs.clone(), a.splat, a.spreads.clone())),
                endpoint_of(&chain, &chain),
                CallKind::Request,
            )
        }
    };
    let recorded = args.is_some();
    let (mut kwargs, splat, spreads) = args.unwrap_or_default();
    // A same-file dict spread in with `**` sends its keys as arguments: the
    // rules check them like the call's own. A spread Arbor could not read in
    // full, or a scan that recorded none, leaves arguments unseen.
    for key in spreads.iter().flat_map(|s| &s.keys) {
        if !kwargs.contains(key) {
            kwargs.push(key.clone());
        }
    }
    let unread_spread = splat && (spreads.is_empty() || spreads.iter().any(|s| !s.complete));
    let mut unread_spreads: Vec<String> = Vec::new();
    for name in spreads
        .iter()
        .filter(|s| !s.complete)
        .filter_map(|s| s.name.clone())
    {
        push_unique(&mut unread_spreads, name);
    }
    let gap_reason = match site {
        Site::Gap(gap) => Some(gap.reason.clone()),
        Site::Call(..) => None,
    };
    let span = span.unwrap_or(value.span);
    // A fine-tuning job trains on the model: only a fine-tunable base helps.
    let hit = if endpoint == Some(FINE_TUNING) {
        fine_tuning_job_hit(data, hit)
    } else {
        hit
    };
    let today = options.today;
    let date_kind = hit.recipe.date_kind;
    let status = effective_state(hit.status, hit.date, date_kind, today);
    let days_left = hit.date.map(|d| today.days_until(d));

    // The listed replacement, followed to the first one that still works.
    let replacement_chain = hit
        .replacement
        .as_deref()
        .map(|r| follow_chain(data, hit.table, r, today));
    let chained = replacement_chain.as_ref().is_some_and(Chain::chained);
    let live = replacement_chain.as_ref().and_then(|c| c.live.clone());

    // The rules that describe the move: the model's own rule set, or, for a
    // chained replacement, the rule set of the last hop.
    let source = match (&replacement_chain, live.as_deref()) {
        (Some(c), Some(target)) if !c.chained() => hit.rule_set.map(|set| {
            (
                set,
                target,
                hit.matched.as_str(),
                hit.entry.and_then(|e| e.source_family.as_deref()),
            )
        }),
        (Some(c), Some(target)) => c.last_hop.and_then(|hop| {
            hop.rule_set.as_deref().map(|set| {
                (
                    set,
                    target,
                    hop.model.as_str(),
                    hop.source_family
                        .as_deref()
                        .or(hit.entry.and_then(|e| e.source_family.as_deref())),
                )
            })
        }),
        _ => None,
    };
    // A model nested in the call's arguments (`requests[].params.model` in a
    // Message Batch, a tool's `model` in `tools=[...]`) belongs to a request
    // or an object of its own: the call's argument names and endpoint say
    // nothing about it, so only the rules about the model itself apply.
    let nested = value.key.rsplit_once('.').map(|(container, _)| container);
    let mut outcome = Outcome::default();
    if call_kind != CallKind::RequestParam {
        if let Some((rule_set, target, source_model, source_family)) = source {
            let context = RuleContext {
                target,
                source_model,
                source_family,
                syntax: Syntax::of_file(&file),
                endpoint: if nested.is_some() { None } else { endpoint },
                routing: &routing.routing,
                kwargs: if nested.is_some() {
                    &[]
                } else {
                    kwargs.as_slice()
                },
            };
            let rules = data
                .rules(rule_set)
                .filter(|rule| nested.is_none() || about_the_model(rule));
            outcome = apply_rules(rules, &context);
        }
    }
    // Arguments the rules above could not see. `extra_body` is the call's
    // own only for the call's own model: a nested model and a parameter
    // object are checked by hand anyway (see below).
    let unchecked = if !recorded {
        Some(UncheckedArgs::NotRecorded)
    } else if unread_spread {
        Some(UncheckedArgs::Splat)
    } else if nested.is_none()
        && call_kind != CallKind::RequestParam
        && kwargs.iter().any(|k| k == "extra_body")
    {
        Some(UncheckedArgs::ExtraBody)
    } else {
        None
    };
    if unchecked != Some(UncheckedArgs::Splat) {
        unread_spreads.clear();
    }
    for decision in &mut outcome.decisions {
        decision.unchecked_args = unchecked;
        decision.unread_spreads = unread_spreads.clone();
    }

    let mut reasons = model_reasons(
        &hit,
        status,
        unchecked.map(|u| u.message(&unread_spreads, Syntax::of_file(&file))),
        source.is_some(),
        chained,
    );
    if let Some(chain) = replacement_chain.as_ref().filter(|c| c.chained()) {
        reasons.push(ReviewItem {
            rule: None,
            message: chain_message(hit.table.provider(), chain, today, source.is_some()),
            because: Vec::new(),
            verified: None,
        });
    }
    if hit.schedule_unconfirmed {
        let message = match hit.table {
            Table::Vertex => format!(
                "This client's vertexai= is decided at run time, so Arbor used Google Cloud's (Vertex AI) schedule for {}. If the client uses the Gemini API, Google's schedule applies instead, with its own date and replacement.",
                hit.matched
            ),
            _ => format!(
                "This client's vertexai= is decided at run time. Google Cloud (Vertex AI) does not list {}, so this is the Gemini API's schedule: confirm which API the client uses.",
                hit.matched
            ),
        };
        reasons.push(ReviewItem {
            rule: None,
            message,
            because: Vec::new(),
            verified: None,
        });
    }
    if let Some(container) = nested {
        let against = live
            .as_deref()
            .map(|m| format!(" against {m}"))
            .unwrap_or_default();
        reasons.push(ReviewItem {
            rule: None,
            message: format!(
                "The model is set in {container}, nested in this call's arguments: Arbor checked only the model there, not the parameters set next to it, so check those{against} by hand."
            ),
            because: Vec::new(),
            verified: None,
        });
    }
    if call_kind == CallKind::RequestParam {
        reasons.push(ReviewItem {
            rule: None,
            message: "The model is set in a request parameter object, not at the request: check the request that sends it."
                .to_string(),
            because: Vec::new(),
            verified: None,
        });
    }
    reasons.append(&mut outcome.review);
    let review = dedupe_review(reasons);
    let mut notes = hit.notes.clone();
    notes.append(&mut outcome.notes);

    let manual = !review.is_empty()
        || chained
        || live.is_none()
        || confidence != Confidence::Confirmed
        || class != RoutingClass::Direct
        || hit.action != Action::Rewrite
        || hit.match_kind != MatchKind::Exact;
    let fix = if status == State::AutoRouted {
        FixKind::None
    } else if manual {
        FixKind::Manual
    } else if !outcome.decisions.is_empty() {
        FixKind::Decide
    } else {
        FixKind::Swap
    };

    let in_window = match status {
        State::Retired => true,
        State::Retiring => match (options.fail_within, days_left) {
            (Some(window), Some(days)) => days <= window,
            _ => false,
        },
        _ => false,
    };
    let counted = (confidence == Confidence::Confirmed || options.count_unconfirmed)
        && match class {
            RoutingClass::Direct => true,
            RoutingClass::Routed => options.include_routed,
            RoutingClass::Unknown => options.count_unconfirmed,
        };

    Some(Finding {
        file,
        line: span.line,
        column: span.column,
        end_line: span.end_line,
        symbol,
        chain,
        call_kind,
        provider: hit.table,
        model: value.value.clone(),
        matched: hit.matched.clone(),
        match_kind: hit.match_kind,
        key: value.key.clone(),
        status,
        listed_status: hit.status,
        date: hit.date,
        date_kind,
        days_left,
        replacement: hit.replacement.clone(),
        replacement_text: hit.replacement_text.clone(),
        live_replacement: live,
        replacement_chain: replacement_chain.map(|c| c.steps).unwrap_or_default(),
        fix,
        changes: outcome.changes,
        decisions: outcome.decisions,
        review,
        notes: dedupe(notes),
        routing: routing.routing.clone(),
        routing_class: class,
        base_url: routing.base_url,
        endpoint,
        confidence,
        origin: (confidence != Confidence::Gap).then_some(value.origin),
        value_from: value.name.clone(),
        value_line: value.span.line,
        gap_reason,
        routing_gap: routing.gap.clone(),
        candidate: false,
        splat,
        in_window,
        fails: in_window && counted,
        recipe: hit.recipe.id.clone(),
        recipe_version: hit.recipe.version.clone(),
        verified: hit.verified,
        source_url: hit.recipe.source_urls.first().cloned(),
    })
}

/// A rule about the model itself rather than the request that sends it: a
/// check or a note with no endpoint and no argument condition. Only these
/// apply to a model nested in a call's arguments, whose own request the
/// call's argument names do not describe.
fn about_the_model(rule: &ParamRule) -> bool {
    matches!(rule.effect, RuleEffect::Review | RuleEffect::Note)
        && rule.endpoints.is_empty()
        && rule.any_kwargs.is_empty()
        && rule.all_kwargs.is_empty()
        && rule.lacks_kwargs.is_empty()
}

/// The state on `today`. A hard date counts as retired from that day on (the
/// provider may switch the model off at any time that day); a reached
/// "earliest" date is only a possibility.
fn effective_state(status: EntryStatus, date: Option<Date>, kind: DateKind, today: Date) -> State {
    match status {
        EntryStatus::ShutDown => State::Retired,
        EntryStatus::Retiring => match date {
            Some(date) if date <= today => match kind {
                DateKind::Hard => State::Retired,
                DateKind::Earliest => State::Unconfirmed,
            },
            _ => State::Retiring,
        },
        EntryStatus::Deprecated => State::Deprecated,
        EntryStatus::Unconfirmed => State::Unconfirmed,
        EntryStatus::AutoRouted => State::AutoRouted,
    }
}

/// The endpoint of a fine-tuning job (`fine_tuning.jobs.create`, and the
/// pre-1.0 `FineTuningJob.create` and `FineTune.create`).
pub const FINE_TUNING: &str = "fine_tuning";

/// An endpoint as the docs name it, for a sentence.
fn endpoint_name(endpoint: &str) -> &str {
    match endpoint {
        "chat.completions" => "Chat Completions",
        "responses" => "the Responses API",
        "completions_legacy" => "the legacy Completions API",
        "assistants" => "the Assistants API",
        other => other,
    }
}

/// Which API a call uses, from its callee.
fn endpoint_of(chain: &str, member: &str) -> Option<&'static str> {
    let text = if member.contains('.') { member } else { chain };
    let has = |needle: &str| text.contains(needle);
    // Python's snake_case and the JavaScript SDKs' camelCase names
    // (`fineTuning.jobs.create`, `models.generateContent`), and openai v3's
    // `OpenAIApi` methods (`createChatCompletion`).
    let endpoint = if has("fine_tuning.")
        || has("fineTuning.")
        || has("fine_tunes.")
        || has("FineTuningJob.")
        || has("FineTune.")
        || has("createFineTune")
    {
        FINE_TUNING
    } else if has("chat.completions") || has("ChatCompletion") {
        "chat.completions"
    } else if has("responses.") {
        "responses"
    } else if has("assistants.") || has("threads.") {
        "assistants"
    } else if has("completions.") || has("Completion.") || has("createCompletion") {
        "completions_legacy"
    } else if has("messages.") {
        "messages"
    } else if has("generate_content")
        || has("generateContent")
        || has("GenerativeModel")
        || has("chats.")
        || has("send_message")
        || has("sendMessage")
    {
        "generate_content"
    } else if has("interactions.") {
        "interactions"
    } else if has("embeddings.")
        || has("embed_content")
        || has("embedContent")
        || has("Embedding.")
        || has("createEmbedding")
    {
        "embeddings"
    } else if has("images.")
        || has("generate_images")
        || has("generateImages")
        || has("createImage")
    {
        "images"
    } else if has("audio.") || has("createTranscription") || has("createTranslation") {
        "audio"
    } else {
        return None;
    };
    Some(endpoint)
}

/// Why the model itself needs a person, before any parameter rule.
fn model_reasons(
    hit: &Hit<'_>,
    status: State,
    unchecked: Option<String>,
    rules_known: bool,
    chained: bool,
) -> Vec<ReviewItem> {
    if status == State::AutoRouted {
        return Vec::new();
    }
    let provider = hit.table.provider();
    let fine_tuned = hit.match_kind == MatchKind::FineTuned
        || hit.entry.and_then(|e| e.kind.as_deref()) == Some("fine_tune_label");
    let category = hit.entry.map(|e| e.category.as_str()).unwrap_or("text");
    let mut messages = Vec::new();
    if let Some(advice) = &hit.fine_tune_advice {
        // A fine-tuned model, or a fine-tuning job on this base.
        messages.push(advice.clone());
    } else if fine_tuned {
        messages.push(
            "Fine-tuned model: retrain it on a base model that supports fine-tuning.".to_string(),
        );
    } else if hit.replacement.is_none() {
        messages.push(match &hit.replacement_text {
            Some(text) => format!("{provider} lists more than one way forward: {text}."),
            None => format!("{provider} lists no replacement."),
        });
    } else if category == "embedding" {
        messages.push(
            "Embedding model: a new model means re-embedding every stored vector.".to_string(),
        );
    } else if category != "text" {
        messages.push(format!(
            "{} model: Arbor checks chat and text parameters only, so review this call by hand.",
            capitalize(&category.replace('_', " "))
        ));
    } else if chained {
        // The chain's own message says what to check.
    } else if !rules_known {
        messages.push(format!(
            "The parameters {} needs have not been checked yet.",
            hit.replacement.as_deref().unwrap_or("the replacement")
        ));
    } else if hit.action == Action::ReportOnly {
        messages.push(
            "Arbor does not swap this model automatically yet; review the call by hand."
                .to_string(),
        );
    }
    if let Some(unchecked) = unchecked {
        messages.push(unchecked);
    }
    messages
        .into_iter()
        .map(|message| ReviewItem {
            rule: None,
            message,
            because: Vec::new(),
            verified: None,
        })
        .collect()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Parameter rules
// ---------------------------------------------------------------------------

struct RuleContext<'a> {
    target: &'a str,
    source_model: &'a str,
    source_family: Option<&'a str>,
    endpoint: Option<&'static str>,
    routing: &'a str,
    kwargs: &'a [String],
    /// How the call's arguments are written, for values named in messages.
    syntax: Syntax,
}

impl RuleContext<'_> {
    fn has(&self, name: &str) -> bool {
        self.kwargs.iter().any(|k| k == name)
    }

    /// Whether every condition holds. `added` are parameters the swap adds:
    /// a rule about calls that lack them no longer applies.
    fn applies(&self, rule: &ParamRule, added: &[String]) -> bool {
        let listed = |list: &[String], value: Option<&str>| {
            list.is_empty() || value.is_some_and(|v| list.iter().any(|x| x == v))
        };
        listed(&rule.endpoints, self.endpoint)
            && listed(&rule.routings, Some(self.routing))
            && listed(&rule.source_families, self.source_family)
            && listed(&rule.targets, Some(self.target))
            && listed(&rule.source_models, Some(self.source_model))
            && (rule.any_kwargs.is_empty() || rule.any_kwargs.iter().any(|k| self.has(k)))
            && rule.all_kwargs.iter().all(|k| self.has(k))
            && !rule
                .lacks_kwargs
                .iter()
                .any(|k| self.has(k) || added.contains(k))
    }

    /// The call's arguments that triggered the rule.
    fn because(&self, rule: &ParamRule) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in rule.any_kwargs.iter().chain(&rule.all_kwargs) {
            if self.has(name) && !out.contains(name) {
                out.push(name.clone());
            }
        }
        if let Some(rename) = &rule.rename {
            if self.has(&rename.from) && !out.contains(&rename.from) {
                out.push(rename.from.clone());
            }
        }
        out
    }

    /// `text` for this call: `{target}` filled in, and on the Responses API
    /// its output limit, `max_output_tokens`, where recipe text written for
    /// Chat Completions names `max_completion_tokens`.
    fn message(&self, text: &str) -> String {
        let text = text.replace("{target}", self.target);
        if self.endpoint == Some("responses") {
            text.replace("max_completion_tokens", "max_output_tokens")
        } else {
            text
        }
    }

    /// Sorts argument names into the order the call passes them.
    fn order(&self, names: &mut [String]) {
        names.sort_by_key(|name| self.kwargs.iter().position(|k| k == name));
    }

    fn endpoint_matches(&self, endpoint: &str) -> bool {
        endpoint == "*" || Some(endpoint) == self.endpoint
    }

    /// For a rule a live check saw on this model but on another API, the
    /// sentence that says so: "stop and logit_bias are not supported on
    /// gpt-6-luna (400)" was seen on Chat Completions, and no live check
    /// called the Responses API. `None` when the check covers this call.
    fn unchecked_endpoint(&self, rule: &ParamRule) -> Option<String> {
        let model =
            rule.live_models.is_empty() || rule.live_models.iter().any(|m| m == self.target);
        let endpoint = self
            .endpoint
            .is_some_and(|e| rule.live_endpoints.iter().any(|x| x == e));
        if rule.verified != Verified::Live || rule.live_endpoints.is_empty() || !model || endpoint {
            return None;
        }
        let seen: Vec<&str> = rule
            .live_endpoints
            .iter()
            .map(|e| endpoint_name(e))
            .collect();
        Some(match self.endpoint {
            Some(here) => format!(
                "Seen on {} only; {} was not checked.",
                seen.join(" and "),
                endpoint_name(here)
            ),
            None => format!("Seen on {} only.", seen.join(" and ")),
        })
    }

    /// The label for this call: a live check vouches only for the models
    /// and the API it called.
    fn verified(&self, rule: &ParamRule) -> Verified {
        if rule.verified != Verified::Live {
            return rule.verified;
        }
        let model =
            rule.live_models.is_empty() || rule.live_models.iter().any(|m| m == self.target);
        let endpoint = rule.live_endpoints.is_empty()
            || self
                .endpoint
                .is_some_and(|e| rule.live_endpoints.iter().any(|x| x == e));
        if model && endpoint {
            Verified::Live
        } else {
            rule.otherwise_verified.unwrap_or(Verified::Docs)
        }
    }
}

#[derive(Default)]
struct Outcome {
    changes: Vec<Change>,
    review: Vec<ReviewItem>,
    notes: Vec<String>,
    decisions: Vec<Decision>,
}

fn apply_rules<'r>(
    rules: impl Iterator<Item = &'r ParamRule>,
    context: &RuleContext<'_>,
) -> Outcome {
    let rules: Vec<&ParamRule> = rules.collect();
    let mut out = Outcome::default();
    let changes_args =
        |rule: &&&ParamRule| matches!(rule.effect, RuleEffect::Add | RuleEffect::Rename);
    // Notes from automatic changes that depend on a choice.
    let mut choice_notes: Vec<(String, String)> = Vec::new();

    // First the argument changes, so the other rules see what the swap adds.
    for rule in rules.iter().filter(changes_args) {
        if !context.applies(rule, &[]) {
            continue;
        }
        let because = context.because(rule);
        if let Some(rename) = rule.rename.as_ref().filter(|r| context.has(&r.from)) {
            merge_change(
                &mut out.changes,
                Change {
                    kind: ChangeKind::Rename,
                    param: rename.from.clone(),
                    to: Some(rename.to.clone()),
                    value: None,
                    because: vec![rename.from.clone()],
                    rules: vec![rule.id.clone()],
                    verified: context.verified(rule),
                },
            );
        }
        apply_adds(rule, context, &because, &mut out.changes, &mut out.review);
        if let (Some(choice), Some(note)) = (&rule.see_choice, &rule.choice_note) {
            choice_notes.push((choice.clone(), context.message(note)));
        }
    }
    // Renames read first: they are edits to arguments already in the call.
    out.changes.sort_by_key(|c| c.kind != ChangeKind::Rename);
    let added: Vec<String> = out
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Add)
        .map(|c| c.param.clone())
        .collect();

    // Choices the repository makes: shown with every option, applied never.
    for rule in rules.iter().filter(|r| r.effect == RuleEffect::Choice) {
        if !context.applies(rule, &added) {
            continue;
        }
        add_choice(rule, context, &mut out);
    }

    let mut keeps = Vec::new();
    for rule in rules.iter().filter(|r| {
        !matches!(
            r.effect,
            RuleEffect::Add | RuleEffect::Rename | RuleEffect::Choice
        )
    }) {
        if !context.applies(rule, &added) {
            continue;
        }
        let because = context.because(rule);
        match rule.effect {
            RuleEffect::Review => out.review.push(ReviewItem {
                rule: Some(rule.id.clone()),
                message: match context.unchecked_endpoint(rule) {
                    Some(scope) => format!("{} {scope}", context.message(&rule.message)),
                    None => context.message(&rule.message),
                },
                because,
                verified: Some(context.verified(rule)),
            }),
            RuleEffect::Keep => keeps.push((*rule, because)),
            RuleEffect::Note => out.notes.push(context.message(&rule.message)),
            RuleEffect::Add | RuleEffect::Rename | RuleEffect::Choice => {}
        }
    }
    // Keep rules depend on the reasoning effort the call runs at.
    for (rule, because) in keeps {
        let on_target =
            rule.keep_targets.is_empty() || rule.keep_targets.iter().any(|t| t == context.target);
        let verb = if because.len() == 1 { "was" } else { "were" };
        if !on_target {
            let text = rule.else_message.as_deref().unwrap_or(&rule.message);
            out.review.push(ReviewItem {
                rule: Some(rule.id.clone()),
                message: context.message(text),
                because,
                verified: Some(context.verified(rule)),
            });
        } else if rule.keep_needs_effort_none && !effort_none(&out.changes) {
            if let Some(decision) = out.decisions.first_mut() {
                decision.notes.push(format!(
                    "{} {verb} accepted on {} at reasoning_effort \"none\" ({}); with the model reasoning, untested.",
                    because.join(", "),
                    context.target,
                    decision
                        .options
                        .iter()
                        .find(|o| o.add.iter().any(|a| a.name.contains("reasoning")))
                        .map(|o| o.label.as_str())
                        .unwrap_or("reasoning off")
                ));
                // Only an option that turns reasoning off runs the call the
                // way it was checked. On any other, no check says whether
                // the argument works, so that option is never "no change"
                // (`seed=` on gpt-6-luna with the model reasoning). Off the
                // checked endpoint, no option runs it that way.
                let checked_here = context.verified(rule) != Verified::Unverified;
                for option in decision.options.iter_mut().filter(|o| {
                    !checked_here
                        || !o
                            .add
                            .iter()
                            .any(|a| is_effort_none(&a.name, Some(&a.value)))
                }) {
                    for name in &because {
                        let settled = option.remove.contains(name)
                            || option.keep.contains(name)
                            || option.add.iter().any(|a| &a.name == name);
                        if !settled {
                            push_unique(&mut option.unverified, name.clone());
                        }
                    }
                }
            } else {
                out.review.push(ReviewItem {
                    rule: Some(rule.id.clone()),
                    message: format!(
                        "{} {verb} checked on {} only at reasoning effort \"none\"; check the effort this call runs at.",
                        because.join(", "),
                        context.target
                    ),
                    because,
                    verified: Some(context.verified(rule)),
                });
            }
        } else {
            out.notes.push(context.message(&rule.message));
        }
    }
    for (choice, note) in choice_notes {
        match out.decisions.iter_mut().find(|d| d.choice == choice) {
            Some(decision) => push_unique(&mut decision.notes, note),
            None => out.notes.push(note),
        }
    }
    flag_checked_arguments(&mut out);
    out
}

/// A check line about an argument the call sends holds whichever option is
/// chosen: `temperature` returns 400 on claude-sonnet-5-5 with or without
/// thinking, and whether gpt-6-luna takes it on the Responses API is not
/// verified at any effort. Each option that neither removes, keeps nor sets
/// that argument lists it, so no option reads "no change" next to a check
/// line that says otherwise.
fn flag_checked_arguments(out: &mut Outcome) {
    for item in out.review.iter().filter(|r| r.rule.is_some()) {
        let unverified = item.verified == Some(Verified::Unverified);
        for option in out.decisions.iter_mut().flat_map(|d| &mut d.options) {
            for name in &item.because {
                let settled = option.remove.contains(name)
                    || option.keep.contains(name)
                    || option.add.iter().any(|a| &a.name == name);
                if settled {
                    continue;
                }
                let list = if unverified {
                    &mut option.unverified
                } else {
                    &mut option.check
                };
                push_unique(list, name.clone());
            }
        }
    }
    // An argument flagged both ways reads once, as the firmer check.
    for option in out.decisions.iter_mut().flat_map(|d| &mut d.options) {
        let check = option.check.clone();
        option.unverified.retain(|name| !check.contains(name));
    }
}

/// Adds a choice rule's options to the call's decision for that choice. A
/// call that already sets a parameter an option adds has made the choice in
/// code; its value is not visible, so it is a check instead. An option that
/// removes the parameter it adds replaces the call's value (Claude's
/// `thinking={"type": "disabled"}`), so the call setting it has not chosen.
fn add_choice(rule: &ParamRule, context: &RuleContext<'_>, out: &mut Outcome) {
    let Some(choice) = rule.choice.clone() else {
        return;
    };
    let because = context.because(rule);
    let preset = rule
        .options
        .iter()
        .flat_map(|o| o.add.iter().filter(|a| !o.remove.contains(&a.name)))
        .filter(|a| context.endpoint_matches(&a.endpoint))
        .find(|a| context.has(&a.name));
    if let Some(add) = preset {
        out.review.push(ReviewItem {
            rule: Some(rule.id.clone()),
            message: format!(
                "{} This call already sets {}; check that it is {}.",
                context.message(&rule.message),
                add.name,
                source_literal(&add.value, context.syntax)
            ),
            because,
            verified: Some(context.verified(rule)),
        });
        return;
    }
    let index = match out.decisions.iter().position(|d| d.choice == choice) {
        Some(index) => index,
        None => {
            out.decisions.push(Decision {
                choice,
                reasons: Vec::new(),
                because: Vec::new(),
                rules: Vec::new(),
                options: rule
                    .options
                    .iter()
                    .map(|o| DecisionOption {
                        id: o.id.clone(),
                        label: o.label.clone(),
                        add: Vec::new(),
                        remove: Vec::new(),
                        keep: Vec::new(),
                        check: Vec::new(),
                        unverified: Vec::new(),
                        effects: Vec::new(),
                    })
                    .collect(),
                notes: Vec::new(),
                // Set by the caller, which knows what the scan read.
                unchecked_args: None,
                unread_spreads: Vec::new(),
            });
            out.decisions.len() - 1
        }
    };
    let decision = &mut out.decisions[index];
    push_unique(&mut decision.reasons, context.message(&rule.message));
    for name in because {
        push_unique(&mut decision.because, name);
    }
    push_unique(&mut decision.rules, rule.id.clone());
    for option in &rule.options {
        let Some(target) = decision.options.iter_mut().find(|o| o.id == option.id) else {
            continue;
        };
        for add in option
            .add
            .iter()
            .filter(|a| context.endpoint_matches(&a.endpoint))
        {
            if !target
                .add
                .iter()
                .any(|p| p.name == add.name && p.value == add.value)
            {
                target.add.push(ParamValue {
                    name: add.name.clone(),
                    value: add.value.clone(),
                });
            }
        }
        for name in option.remove.iter().filter(|n| context.has(n)) {
            push_unique(&mut target.remove, name.clone());
        }
        for name in option.keep.iter().filter(|n| context.has(n)) {
            push_unique(&mut target.keep, name.clone());
        }
        let text = context.message(&option.effect);
        if !target.effects.iter().any(|e| e.text == text) {
            target.effects.push(OptionEffect {
                rule: rule.id.clone(),
                text,
                status: option.status,
            });
        }
        // The call's own argument order reads best.
        context.order(&mut target.remove);
        context.order(&mut target.keep);
    }
    context.order(&mut decision.because);
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

fn apply_adds(
    rule: &ParamRule,
    context: &RuleContext<'_>,
    because: &[String],
    changes: &mut Vec<Change>,
    review: &mut Vec<ReviewItem>,
) {
    if rule.add.is_empty() {
        return;
    }
    let spec = rule
        .add
        .iter()
        .find(|a| context.endpoint_matches(&a.endpoint));
    let Some(spec) = spec else {
        let text = rule.else_message.clone().unwrap_or_else(|| {
            format!(
                "{} Arbor could not tell which API this call uses, so set it by hand.",
                rule.message
            )
        });
        review.push(ReviewItem {
            rule: Some(rule.id.clone()),
            message: context.message(&text),
            because: because.to_vec(),
            verified: Some(context.verified(rule)),
        });
        return;
    };
    if context.has(&spec.name) {
        // The value is not visible: it may already be right.
        review.push(ReviewItem {
            rule: Some(rule.id.clone()),
            message: format!(
                "This call already sets {}; on {} it must be {}.",
                spec.name,
                context.target,
                source_literal(&spec.value, context.syntax)
            ),
            because: because.to_vec(),
            verified: Some(context.verified(rule)),
        });
        return;
    }
    merge_change(
        changes,
        Change {
            kind: ChangeKind::Add,
            param: spec.name.clone(),
            to: None,
            value: Some(spec.value.clone()),
            because: because.to_vec(),
            rules: vec![rule.id.clone()],
            verified: context.verified(rule),
        },
    );
}

fn verified_rank(verified: Verified) -> u8 {
    match verified {
        Verified::Live => 2,
        Verified::Docs => 1,
        Verified::Unverified => 0,
    }
}

/// Adds `change`, or folds it into an equal change already listed.
fn merge_change(changes: &mut Vec<Change>, change: Change) {
    if let Some(existing) = changes.iter_mut().find(|c| {
        c.kind == change.kind
            && c.param == change.param
            && c.value == change.value
            && c.to == change.to
    }) {
        for name in change.because {
            if !existing.because.contains(&name) {
                existing.because.push(name);
            }
        }
        for rule in change.rules {
            if !existing.rules.contains(&rule) {
                existing.rules.push(rule);
            }
        }
        if verified_rank(change.verified) > verified_rank(existing.verified) {
            existing.verified = change.verified;
        }
        return;
    }
    changes.push(change);
}

/// Whether the changes set reasoning effort to `none`.
fn effort_none(changes: &[Change]) -> bool {
    changes
        .iter()
        .any(|c| c.kind == ChangeKind::Add && is_effort_none(&c.param, c.value.as_ref()))
}

/// Whether setting `param` to `value` turns reasoning off:
/// `reasoning_effort="none"` or `reasoning={"effort": "none"}`.
fn is_effort_none(param: &str, value: Option<&serde_json::Value>) -> bool {
    match (param, value) {
        ("reasoning_effort", Some(serde_json::Value::String(v))) => v == "none",
        ("reasoning", Some(serde_json::Value::Object(map))) => {
            map.get("effort").and_then(|v| v.as_str()) == Some("none")
        }
        _ => false,
    }
}

/// The language a call's arguments are written in, from its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// Keyword arguments: `reasoning_effort="none"`, `None`, `True`.
    Python,
    /// An options object: `reasoning_effort: "none"`, `null`, `true`.
    JavaScript,
}

impl Syntax {
    /// JavaScript for `.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts`
    /// and `.cts` files; Python for the rest.
    pub fn of_file(file: &str) -> Self {
        let extension = std::path::Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let javascript = ["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"]
            .iter()
            .any(|e| e.eq_ignore_ascii_case(extension));
        if javascript {
            Syntax::JavaScript
        } else {
            Syntax::Python
        }
    }
}

/// A JSON value as the call's language writes it: [`python_literal`] for
/// Python; for JavaScript the JSON text itself, which is a valid literal
/// there (`"none"`, `{"type": "between_tools"}`, `null`).
pub fn source_literal(value: &serde_json::Value, syntax: Syntax) -> String {
    match syntax {
        Syntax::Python => python_literal(value),
        Syntax::JavaScript => javascript_literal(value),
    }
}

/// An argument as the call's language passes it: `name=value` (a keyword
/// argument) or `name: value` (a property of the options object).
pub fn source_argument(name: &str, value: &serde_json::Value, syntax: Syntax) -> String {
    match syntax {
        Syntax::Python => format!("{name}={}", python_literal(value)),
        Syntax::JavaScript => format!("{name}: {}", javascript_literal(value)),
    }
}

/// A JSON value as a JavaScript literal, spaced like [`python_literal`].
fn javascript_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(javascript_literal)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!(
                    "{}: {}",
                    serde_json::Value::String(k.clone()),
                    javascript_literal(v)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

/// A JSON value as a Python literal (`"none"`, `{"effort": "none"}`).
pub fn python_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(true) => "True".to_string(),
        serde_json::Value::Bool(false) => "False".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(_) => value.to_string(),
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(python_literal)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!(
                    "{}: {}",
                    serde_json::Value::String(k.clone()),
                    python_literal(v)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn dedupe_review(items: Vec<ReviewItem>) -> Vec<ReviewItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.message.clone()))
        .collect()
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Unreadable models and the summary
// ---------------------------------------------------------------------------

/// Requests whose model is not a value Arbor can read (environment, a
/// parameter, another module), except where a fallback or the possible
/// values were captured; and the number of requests that pass no model.
fn unreadable(input: ScanInput<'_>) -> (Vec<Unreadable>, usize) {
    let captured: HashSet<(&str, u32, &str)> = input
        .calls
        .iter()
        .flat_map(|call| {
            call.values
                .iter()
                .map(move |v| (call.file.as_str(), call.span.start_byte, v.key.as_str()))
        })
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut no_model = 0;
    for gap in input.gaps {
        if gap.kind != GapKind::UnresolvedModel || gap.reason == "truncated" {
            continue;
        }
        if gap.reason == "missing_model" {
            no_model += 1;
            continue;
        }
        if !gap.values.is_empty() {
            continue;
        }
        let Some(span) = gap.span else { continue };
        let key = gap.key.as_deref().unwrap_or("model");
        if captured.contains(&(gap.file.as_str(), span.start_byte, key)) {
            continue;
        }
        if !seen.insert((gap.file.clone(), span.start_byte, key.to_string())) {
            continue;
        }
        out.push(Unreadable {
            file: gap.file.clone(),
            line: span.line,
            chain: gap.chain.clone(),
            key: gap.key.clone(),
            sdk: gap.sdk,
            reason: gap.reason.clone(),
        });
    }
    (out, no_model)
}

/// SDK-shaped calls the usage scan records only as a client gap: a client
/// it could not resolve, and no call record, no captured value and no model
/// gap at the same call. A finding, the unreadable list or the no-model
/// count shows every other call; these would otherwise appear on no line.
fn unresolved_clients(input: ScanInput<'_>) -> usize {
    let shown: HashSet<(&str, u32)> = input
        .calls
        .iter()
        .map(|c| (c.file.as_str(), c.span.start_byte))
        .chain(
            input
                .gaps
                .iter()
                .filter(|g| {
                    !gaps::is_region(g)
                        && (g.kind == GapKind::UnresolvedModel || !g.values.is_empty())
                })
                .filter_map(|g| g.span.map(|s| (g.file.as_str(), s.start_byte))),
        )
        .collect();
    input
        .gaps
        .iter()
        // `getattr` methods are counted on their own; vendored code is not
        // the project's.
        .filter(|g| {
            gaps::holds_routing(g)
                && !matches!(g.kind, GapKind::DynamicDispatch | GapKind::VendoredPath)
        })
        .filter_map(|g| g.span.map(|s| (g.file.as_str(), s.start_byte)))
        .filter(|at| !shown.contains(at))
        .collect::<HashSet<_>>()
        .len()
}

fn dedupe_findings(findings: &mut Vec<Finding>) {
    let mut seen = HashSet::new();
    findings.retain(|f| {
        seen.insert((
            f.file.clone(),
            f.line,
            f.column,
            f.key.clone(),
            f.model.clone(),
        ))
    });
}

fn state_rank(state: State) -> u8 {
    match state {
        State::Retired => 0,
        State::Retiring => 1,
        State::Unconfirmed => 2,
        State::Deprecated => 3,
        State::AutoRouted => 4,
    }
}

fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        state_rank(a.status)
            .cmp(&state_rank(b.status))
            .then(a.date.cmp(&b.date))
            .then(a.provider.cmp(&b.provider))
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.key.cmp(&b.key))
    });
}

fn summarize(
    findings: &[Finding],
    routed: &[Finding],
    routed_calls: usize,
    unreadable: usize,
) -> Summary {
    let mut summary = Summary {
        affected_calls: findings.len(),
        routed_calls,
        unreadable_calls: unreadable,
        ..Summary::default()
    };
    let files: BTreeSet<&str> = findings.iter().map(|f| f.file.as_str()).collect();
    summary.affected_files = files.len();
    for finding in findings {
        match finding.status {
            State::Retired => summary.retired += 1,
            State::Retiring => summary.retiring += 1,
            _ => summary.other += 1,
        }
        match finding.fix {
            FixKind::Swap => summary.swap += 1,
            FixKind::Decide => summary.decide += 1,
            FixKind::Manual => summary.manual += 1,
            FixKind::None => {}
        }
        *summary
            .by_provider
            .entry(finding.provider.as_str().to_string())
            .or_default() += 1;
    }
    for finding in findings.iter().chain(routed).filter(|f| f.in_window) {
        summary.in_window += 1;
        if finding.fails {
            summary.failing += 1;
        } else {
            summary.uncounted += 1;
        }
    }
    let next = findings
        .iter()
        .filter(|f| f.status == State::Retiring)
        .filter_map(|f| Some((f.date?, f.days_left?, f.provider)))
        .min_by_key(|(date, _, provider)| (*date, *provider));
    summary.next_shutdown = next.map(|(date, days_left, provider)| NextShutdown {
        date,
        days_left,
        provider,
        calls: findings
            .iter()
            .filter(|f| {
                f.status == State::Retiring && f.date == Some(date) && f.provider == provider
            })
            .count(),
    });
    summary
}

#[cfg(test)]
mod tests {
    use super::super::bundled;
    use super::*;
    use crate::usages::{CallArgs, Receiver, Span};

    fn span(line: u32) -> Span {
        Span {
            line,
            column: 5,
            end_line: line,
            end_column: 40,
            start_byte: line * 100,
            end_byte: line * 100 + 35,
        }
    }

    fn value(model: &str, origin: ValueOrigin, line: u32) -> CapturedValue {
        CapturedValue {
            key: "model".into(),
            value: model.into(),
            origin,
            name: None,
            positional: false,
            span: span(line),
            model_family: model_family(model),
        }
    }

    fn call(
        sdk: Sdk,
        routing: Routing,
        chain: &str,
        model: &str,
        kwargs: &[&str],
        line: u32,
    ) -> SdkCall {
        let member = chain.split_once('.').map(|(_, m)| m).unwrap_or(chain);
        let mut all: Vec<String> = vec!["model".to_string()];
        all.extend(kwargs.iter().map(|k| k.to_string()));
        SdkCall {
            sdk,
            package: sdk.as_str().into(),
            chain: chain.into(),
            member: member.into(),
            request: true,
            receiver: Receiver::Client,
            routing,
            client_line: Some(1),
            file: "app.py".into(),
            span: span(line),
            symbol: Some("ask".into()),
            kwargs: all,
            positional: 0,
            splat: false,
            spreads: Vec::new(),
            values: vec![value(model, ValueOrigin::Literal, line)],
        }
    }

    fn openai(model: &str, kwargs: &[&str]) -> SdkCall {
        call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "client.chat.completions.create",
            model,
            kwargs,
            10,
        )
    }

    /// An object built from an imported SDK type: `member` keeps the module
    /// path, as the usage scan reports it.
    fn object(sdk: Sdk, chain: &str, member: &str, model: &str, line: u32) -> SdkCall {
        let mut object = call(sdk, Routing::OpenaiDirect, chain, model, &[], line);
        object.member = member.into();
        object.request = false;
        object.receiver = Receiver::Symbol;
        object.client_line = None;
        object
    }

    fn options(today: &str) -> CheckOptions {
        CheckOptions {
            today: Date::parse(today).unwrap(),
            fail_within: None,
            include_routed: false,
            count_unconfirmed: false,
        }
    }

    fn run(calls: &[SdkCall], gaps: &[UsageGap], opts: &CheckOptions) -> RetirementReport {
        let input = ScanInput {
            calls,
            gaps,
            clients: &[],
        };
        check(input, bundled().unwrap(), opts)
    }

    fn one(calls: &[SdkCall]) -> Finding {
        let report = run(calls, &[], &options("2026-10-18"));
        assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
        report.findings[0].clone()
    }

    fn change(finding: &Finding, param: &str) -> Option<Change> {
        finding.changes.iter().find(|c| c.param == param).cloned()
    }

    fn option<'a>(finding: &'a Finding, id: &str) -> &'a DecisionOption {
        finding.decisions[0]
            .options
            .iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("no option {id}: {:#?}", finding.decisions))
    }

    #[test]
    fn retiring_model_counts_days_from_a_fixed_clock() {
        let finding = one(&[openai("gpt-4o", &["messages"])]);
        assert_eq!(finding.status, State::Retiring);
        assert_eq!(finding.date.unwrap().iso(), "2027-01-14");
        assert_eq!(finding.days_left, Some(88));
        assert_eq!(finding.replacement.as_deref(), Some("gpt-6-luna"));
        assert_eq!(finding.live_replacement.as_deref(), Some("gpt-6-luna"));
        assert_eq!(finding.routing_class, RoutingClass::Direct);
        assert_eq!(finding.confidence, Confidence::Confirmed);
        assert_eq!(finding.call_kind, CallKind::Request);
        assert_eq!(finding.endpoint, Some("chat.completions"));
        assert!(!finding.in_window, "retiring is not failing by default");
        assert!(finding
            .notes
            .iter()
            .any(|n| n.contains("most efficient model for focused, high-volume tasks")));

        // The day after the date the call is retired.
        let report = run(&[openai("gpt-4o", &[])], &[], &options("2027-01-15"));
        let later = &report.findings[0];
        assert_eq!(later.status, State::Retired);
        assert_eq!(later.days_left, Some(-1));
        assert!(later.in_window && later.fails);
        assert_eq!(report.exit_code, 1);

        // The shutdown day itself counts as retired: the provider may turn
        // the model off at any time that day (UTC).
        let report = run(&[openai("gpt-4o", &[])], &[], &options("2027-01-14"));
        assert_eq!(report.findings[0].status, State::Retired);
        assert_eq!(report.findings[0].days_left, Some(0));
        assert_eq!(report.exit_code, 1);
        let report = run(&[openai("gpt-4o", &[])], &[], &options("2027-01-13"));
        assert_eq!(report.findings[0].status, State::Retiring);
        assert_eq!(report.findings[0].days_left, Some(1));
        assert_eq!(report.exit_code, 0);
    }

    #[test]
    fn fail_within_counts_retiring_calls_inside_the_window() {
        let calls = [openai("gpt-4o", &[])];
        let mut opts = options("2026-10-18");
        assert_eq!(run(&calls, &[], &opts).exit_code, 0);
        opts.fail_within = Some(87);
        assert_eq!(run(&calls, &[], &opts).exit_code, 0);
        opts.fail_within = Some(88);
        let report = run(&calls, &[], &opts);
        assert_eq!(report.exit_code, 1);
        assert_eq!(report.summary.in_window, 1);
        assert_eq!(report.summary.failing, 1);
    }

    #[test]
    fn retired_model_fails_by_default() {
        let claude = call(
            Sdk::Anthropic,
            Routing::AnthropicDirect,
            "claude.messages.create",
            "claude-3-5-sonnet-20241022",
            &["max_tokens", "messages"],
            20,
        );
        let report = run(&[claude], &[], &options("2026-10-18"));
        let finding = &report.findings[0];
        assert_eq!(finding.status, State::Retired);
        assert_eq!(finding.provider, Table::Anthropic);
        assert_eq!(finding.date.unwrap().iso(), "2025-10-28");
        assert_eq!(finding.replacement.as_deref(), Some("claude-sonnet-5-5"));
        assert!(finding.in_window && finding.fails);
        assert_eq!(report.exit_code, 1);
        assert_eq!(report.summary.retired, 1);
        // Sonnet 5.5 thinks by default. Whether to send between_tools is the
        // repository's choice (the lead's 20 Oct decision), never a change.
        assert!(
            change(finding, "thinking").is_none(),
            "{:#?}",
            finding.changes
        );
        assert_eq!(finding.fix, FixKind::Decide);
        assert_eq!(finding.decisions.len(), 1);
        assert_eq!(finding.decisions[0].choice, "ANT-THINKING-DEFAULT");
        let between = option(finding, "thinking_between_tools");
        assert_eq!(between.label, "think only between tools");
        assert_eq!(between.add.len(), 1);
        assert_eq!(between.add[0].name, "thinking");
        assert_eq!(
            python_literal(&between.add[0].value),
            "{\"type\": \"between_tools\"}"
        );
        let default = option(finding, "model_default");
        assert!(default.add.is_empty() && default.remove.is_empty());
        assert!(default
            .effects
            .iter()
            .any(|e| e.text.contains("thinks adaptively")));
    }

    #[test]
    fn claude_thinking_is_a_decision_never_a_change() {
        let claude = |kwargs: &[&str]| {
            call(
                Sdk::Anthropic,
                Routing::AnthropicDirect,
                "claude.messages.create",
                "claude-3-7-sonnet-20250219",
                kwargs,
                20,
            )
        };
        // With tools: the same choice, and no check that hides it.
        let finding = one(&[claude(&["max_tokens", "messages", "tools"])]);
        assert!(finding.changes.is_empty(), "{:#?}", finding.changes);
        assert_eq!(finding.decisions[0].choice, "ANT-THINKING-DEFAULT");
        assert!(finding
            .review
            .iter()
            .all(|r| r.rule.as_deref() != Some("ANT-THINKING-DEFAULT")));

        // A call that sets thinking, whose value Arbor cannot read: the
        // disabled case is the same choice (send between_tools instead, or
        // drop thinking), not a check that it is already between_tools.
        let finding = one(&[claude(&["max_tokens", "messages", "thinking"])]);
        assert!(finding.changes.is_empty(), "{:#?}", finding.changes);
        assert_eq!(finding.decisions.len(), 1, "{:#?}", finding.review);
        let decision = &finding.decisions[0];
        assert_eq!(decision.choice, "ANT-THINKING-DEFAULT");
        assert_eq!(decision.rules, ["ANT-THINKING-DISABLED"]);
        assert_eq!(decision.because, ["thinking"]);
        assert!(decision.reasons[0].contains("if it is {\"type\": \"disabled\"}"));
        let between = option(&finding, "thinking_between_tools");
        assert_eq!(between.remove, ["thinking"]);
        assert_eq!(between.add[0].name, "thinking");
        assert_eq!(option(&finding, "model_default").remove, ["thinking"]);
        // The budget check still applies, so a person looks at it.
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("ANT-THINKING-BUDGET")));
        assert_eq!(finding.fix, FixKind::Manual);
    }

    #[test]
    fn the_reasoning_effort_is_a_decision_never_a_change() {
        let finding = one(&[openai(
            "gpt-4o-mini",
            &["messages", "max_tokens", "temperature", "tools"],
        )]);
        // The rename stays automatic, verified live on gpt-6-luna.
        let rename = change(&finding, "max_tokens").expect("rename");
        assert_eq!(rename.kind, ChangeKind::Rename);
        assert_eq!(rename.to.as_deref(), Some("max_completion_tokens"));
        assert_eq!(rename.verified, Verified::Live);
        // Nothing adds reasoning_effort or removes a parameter on its own.
        assert_eq!(finding.changes.len(), 1, "{:#?}", finding.changes);
        assert_eq!(finding.fix, FixKind::Decide);

        // One decision for the call, with both options.
        assert_eq!(finding.decisions.len(), 1);
        let decision = &finding.decisions[0];
        assert_eq!(decision.choice, "G6-REASONING-DEFAULT");
        for rule in ["G6-REASONING-DEFAULT", "G6-SAMPLING", "G6-TOOLS-CC"] {
            assert!(decision.rules.iter().any(|r| r == rule), "{rule}");
        }
        assert_eq!(decision.because, ["temperature", "tools"]);
        let off = option(&finding, "reasoning_none");
        assert_eq!(off.label, "keep reasoning off");
        assert_eq!(off.add.len(), 1);
        assert_eq!(off.add[0].name, "reasoning_effort");
        assert_eq!(off.add[0].value, serde_json::json!("none"));
        assert_eq!(off.keep, ["temperature", "tools"]);
        let reason = option(&finding, "model_default");
        assert!(reason.add.is_empty());
        assert_eq!(reason.remove, ["temperature", "tools"]);
        assert!(reason
            .effects
            .iter()
            .any(|e| e.text.contains("tool calls stop")));
        // max_tokens's note joins the decision: the budget covers reasoning.
        assert!(decision
            .notes
            .iter()
            .any(|n| n.contains("max_completion_tokens also bounds its reasoning")));
        // The anchor's effect for "none" is not verified to match the old model.
        assert!(off
            .effects
            .iter()
            .any(|e| e.rule == "G6-REASONING-DEFAULT" && e.status == OptionStatus::Unverified));
    }

    #[test]
    fn the_responses_api_names_its_own_output_limit() {
        // rv6 nit: "max_completion_tokens also has to cover them" was shown
        // for Responses calls, whose limit is max_output_tokens.
        let effects = |chain: &str| -> Vec<String> {
            let mut call = openai("gpt-4o", &["input"]);
            call.chain = chain.into();
            call.member = chain.split_once('.').unwrap().1.into();
            let finding = one(&[call]);
            option(&finding, "model_default")
                .effects
                .iter()
                .map(|e| e.text.clone())
                .collect()
        };
        let responses = effects("client.responses.create");
        assert!(
            responses
                .iter()
                .any(|t| t.contains("max_output_tokens also has to cover them")),
            "{responses:#?}"
        );
        assert!(responses
            .iter()
            .all(|t| !t.contains("max_completion_tokens")));
        let chat = effects("client.chat.completions.create");
        assert!(chat
            .iter()
            .any(|t| t.contains("max_completion_tokens also has to cover them")));
    }

    #[test]
    fn a_plain_call_is_a_decision_and_nothing_else() {
        let finding = one(&[openai("gpt-4o", &["messages"])]);
        assert!(finding.changes.is_empty());
        assert!(finding.review.is_empty());
        assert_eq!(finding.fix, FixKind::Decide);
        let off = option(&finding, "reasoning_none");
        assert_eq!(off.add[0].name, "reasoning_effort");
        assert!(option(&finding, "model_default").remove.is_empty());
        let report = run(&[openai("gpt-4o", &[])], &[], &options("2026-10-18"));
        assert_eq!(report.summary.decide, 1);
        assert_eq!(report.summary.swap, 0);
    }

    #[test]
    fn unsupported_parameters_need_a_person() {
        let finding = one(&[openai("gpt-4o", &["messages", "stop", "logit_bias"])]);
        assert_eq!(finding.fix, FixKind::Manual);
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-STOP-LOGIT-BIAS")
                && r.message.contains("gpt-6-luna")
                && r.verified == Some(Verified::Live)));

        // Penalties failed live on gpt-6-luna.
        let finding = one(&[openai("gpt-4o", &["presence_penalty", "seed"])]);
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-PENALTIES")));
        // seed was checked only at reasoning_effort "none": part of the decision.
        assert!(
            finding.decisions[0]
                .notes
                .iter()
                .any(|n| n
                    .starts_with("seed was accepted on gpt-6-luna at reasoning_effort \"none\""))
        );

        // A call that sets its own reasoning effort has no decision; seed is a check.
        let finding = one(&[openai("gpt-4o", &["seed", "reasoning_effort"])]);
        assert!(finding.decisions.is_empty());
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-N-SEED")));
    }

    #[test]
    fn reasoning_sources_and_endpoints_pick_the_right_rules() {
        // o3-mini is a reasoning model: sampling parameters mean manual review.
        let finding = one(&[openai("o3-mini", &["temperature"])]);
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("OAI-SAMPLING-REASONING-SOURCE")));
        assert!(finding.decisions.is_empty());
        assert!(change(&finding, "reasoning_effort").is_none());

        // The Responses API: the decision offers reasoning={"effort": "none"},
        // and sampling there is not verified.
        let responses = call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "client.responses.create",
            "gpt-3.5-turbo",
            &["input", "temperature"],
            12,
        );
        let finding = one(&[responses]);
        assert!(finding.changes.is_empty());
        let off = option(&finding, "reasoning_none");
        assert_eq!(off.add[0].name, "reasoning");
        assert_eq!(off.add[0].value, serde_json::json!({"effort": "none"}));
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("OAI-SAMPLING-RESPONSES")
                && r.verified == Some(Verified::Unverified)));

        // A reasoning_effort the call already sets cannot be read: check it.
        let finding = one(&[openai("gpt-4", &["tools", "reasoning_effort"])]);
        assert!(finding.decisions.is_empty());
        assert!(change(&finding, "reasoning_effort").is_none());
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("OAI-TOOLS-CC")
                && r.message
                    .contains("already sets reasoning_effort; check that it is \"none\"")));

        // The legacy Completions endpoint is not supported on the target.
        let legacy = call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "openai.Completion.create",
            "gpt-3.5-turbo",
            &["prompt"],
            14,
        );
        let finding = one(&[legacy]);
        assert_eq!(finding.endpoint, Some("completions_legacy"));
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("OAI-LEGACY-COMPLETIONS-ENDPOINT")));
    }

    #[test]
    fn live_labels_hold_only_for_the_models_and_api_called() {
        // OAI-TOOLS-CC was called live on gpt-5.6-terra (gpt-3.5-turbo's
        // replacement), not on gpt-5.6-sol (gpt-4's).
        let label = |model: &str| {
            let finding = one(&[openai(model, &["tools", "reasoning_effort"])]);
            finding
                .review
                .iter()
                .find(|r| r.rule.as_deref() == Some("OAI-TOOLS-CC"))
                .and_then(|r| r.verified)
                .unwrap()
        };
        assert_eq!(label("gpt-3.5-turbo"), Verified::Live);
        assert_eq!(label("gpt-4"), Verified::Docs);

        // The max_tokens rename: live on terra, documented on sol.
        let rename = |model: &str| change(&one(&[openai(model, &["max_tokens"])]), "max_tokens");
        assert_eq!(rename("gpt-3.5-turbo").unwrap().verified, Verified::Live);
        assert_eq!(
            rename("gpt-4o-2024-05-13").unwrap().verified,
            Verified::Docs
        );

        // stop on gpt-6-luna was called on Chat Completions only.
        let assistants = call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "client.beta.assistants.create",
            "gpt-4o",
            &["stop"],
            16,
        );
        let finding = one(&[assistants]);
        let stop = finding
            .review
            .iter()
            .find(|r| r.rule.as_deref() == Some("G6-STOP-LOGIT-BIAS"))
            .unwrap();
        assert_eq!(stop.verified, Some(Verified::Unverified));
    }

    #[test]
    fn response_and_type_objects_are_not_requests() {
        // The reviewer's file: two functions that build objects and call no API.
        let calls = [
            object(
                Sdk::Openai,
                "ChatCompletion",
                "types.chat.ChatCompletion",
                "gpt-3.5-turbo-0613",
                5,
            ),
            object(
                Sdk::Anthropic,
                "Message",
                "types.Message",
                "claude-3-5-sonnet-20241022",
                9,
            ),
        ];
        let report = run(&calls, &[], &options("2026-10-18"));
        assert!(report.findings.is_empty(), "{:#?}", report.findings);
        assert_eq!(report.exit_code, 0);
        assert_eq!(report.summary.response_objects, 2);

        // Pydantic constructors and module-qualified types are objects too.
        for (chain, member) in [
            (
                "ChatCompletion.model_validate",
                "types.chat.ChatCompletion.model_validate",
            ),
            (
                "ChatCompletion.construct",
                "types.chat.ChatCompletion.construct",
            ),
            (
                "openai.types.chat.ChatCompletion",
                "types.chat.ChatCompletion",
            ),
            ("Choice", "types.chat.chat_completion.Choice"),
        ] {
            let report = run(
                &[object(Sdk::Openai, chain, member, "gpt-3.5-turbo-0613", 5)],
                &[],
                &options("2026-10-18"),
            );
            assert!(report.findings.is_empty(), "{chain}");
        }

        // A tool config is a request parameter: its model is used.
        let tool = object(
            Sdk::Openai,
            "ImageGeneration",
            "types.responses.tool_param.ImageGeneration",
            "dall-e-2",
            7,
        );
        let finding = one(&[tool]);
        assert_eq!(finding.call_kind, CallKind::RequestParam);
        assert_eq!(finding.endpoint, None);
        assert_eq!(finding.fix, FixKind::Manual);
        // Built from an imported type: the request that sends it decides
        // where it goes, so its routing is not confirmed.
        assert_eq!(finding.routing_class, RoutingClass::Unknown);
        assert!(!finding.fails);
        assert!(finding
            .review
            .iter()
            .any(|r| r.message.contains("request parameter object")));

        // A request model built in code and sent (a realtime session) is
        // request-side too.
        let session = object(
            Sdk::Openai,
            "RealtimeSessionCreateRequest",
            "types.realtime.realtime_session_create_request.RealtimeSessionCreateRequest",
            "gpt-4o-realtime-preview",
            11,
        );
        let finding = one(&[session]);
        assert_eq!(finding.call_kind, CallKind::RequestParam);
        assert_eq!(finding.status, State::Retired);

        // `genai.GenerativeModel("gemini-1.5-pro")` binds the model.
        let mut binding = call(
            Sdk::Google,
            Routing::GoogleGenerativeaiLegacy,
            "genai.GenerativeModel",
            "gemini-2.0-flash",
            &[],
            8,
        );
        binding.request = false;
        binding.member = "GenerativeModel".into();
        let finding = one(&[binding]);
        assert_eq!(finding.call_kind, CallKind::ModelBinding);
        assert_eq!(finding.endpoint, Some("generate_content"));
        assert_eq!(finding.status, State::Retired);
    }

    #[test]
    fn replacement_chains_end_at_a_model_that_still_works() {
        // gpt-4-vision-preview → gpt-4o (shuts down 14 Jan 2027) → gpt-6-luna.
        let finding = one(&[openai("gpt-4-vision-preview", &["max_tokens"])]);
        assert_eq!(finding.replacement.as_deref(), Some("gpt-4o"));
        assert_eq!(finding.live_replacement.as_deref(), Some("gpt-6-luna"));
        let steps: Vec<(&str, Option<State>)> = finding
            .replacement_chain
            .iter()
            .map(|s| (s.model.as_str(), s.status))
            .collect();
        assert_eq!(
            steps,
            [("gpt-4o", Some(State::Retiring)), ("gpt-6-luna", None)]
        );
        assert_eq!(finding.fix, FixKind::Manual);
        assert!(finding.review.iter().any(|r| r.message
            == "OpenAI lists gpt-4o as the replacement, and it shuts down on 14 Jan 2027 (88 days). \
                gpt-6-luna is the replacement OpenAI lists for gpt-4o; the changes shown are for gpt-6-luna."));
        // The changes are for the model at the end of the chain.
        assert_eq!(
            change(&finding, "max_tokens").unwrap().to.as_deref(),
            Some("max_completion_tokens")
        );

        // An already shut-down replacement: text-davinci-003 → gpt-3.5-turbo-instruct.
        let finding = one(&[openai("text-davinci-003", &[])]);
        assert_eq!(
            finding.replacement.as_deref(),
            Some("gpt-3.5-turbo-instruct")
        );
        assert_eq!(finding.live_replacement.as_deref(), Some("gpt-5.6-terra"));
        assert_eq!(finding.replacement_chain[0].status, Some(State::Retired));
        // No rule set covers gpt-3.5-turbo-instruct → gpt-5.6-terra (a past
        // shutdown, report only): no changes, and the message does not imply
        // there are none to make.
        assert!(finding.changes.is_empty());
        assert!(
            finding.review.iter().any(|r| r.message
                == "OpenAI lists gpt-3.5-turbo-instruct as the replacement, and it shut down on \
                    28 Sep 2026. gpt-5.6-terra is the replacement OpenAI lists for \
                    gpt-3.5-turbo-instruct; the parameters gpt-5.6-terra needs have not been \
                    checked yet."),
            "{:#?}",
            finding.review
        );
        assert!(!finding
            .review
            .iter()
            .any(|r| r.message.contains("the changes shown are for")));

        // A dead end: Vertex text-bison → gemini-2.5-flash-lite, which retires
        // too and has several replacements.
        // (`genai.Client(vertexai=True)`, which the usage scan leaves unrouted.)
        let bison = call(
            Sdk::Google,
            Routing::Unknown,
            "client.models.generate_content",
            "text-bison",
            &["contents"],
            30,
        );
        let vertex = ClientConstruction {
            sdk: Sdk::Google,
            package: "google.genai".into(),
            constructor: "google.genai.Client".into(),
            chain: "genai.Client".into(),
            binding: Some("client".into()),
            scope: crate::usages::ClientScope::Module,
            routing: Routing::Unknown,
            is_async: false,
            kwargs: vec!["vertexai".into()],
            splat: false,
            file: "app.py".into(),
            span: span(1),
            symbol: None,
            vertexai: Some(VertexAi::On),
        };
        let input = ScanInput {
            calls: std::slice::from_ref(&bison),
            gaps: &[],
            clients: std::slice::from_ref(&vertex),
        };
        let report = check(input, bundled().unwrap(), &options("2026-10-18"));
        let finding = &report.findings[0];
        assert_eq!(
            finding.replacement.as_deref(),
            Some("gemini-2.5-flash-lite")
        );
        assert_eq!(finding.live_replacement, None);
        assert_eq!(finding.fix, FixKind::Manual);
        assert!(finding
            .review
            .iter()
            .any(|r| r.message.contains("pick one by hand")));
    }

    #[test]
    fn fine_tunes_never_get_a_base_that_cannot_be_fine_tuned() {
        // gpt-6-luna does not support fine-tuning: no retrain advice.
        let finding = one(&[openai("ft:gpt-4o-mini-2024-07-18:acme::abc123", &[])]);
        assert_eq!(finding.match_kind, MatchKind::FineTuned);
        assert_eq!(finding.matched, "gpt-4o-mini-2024-07-18");
        assert_eq!(finding.replacement, None);
        assert_eq!(
            finding.replacement_text.as_deref(),
            Some("no fine-tunable replacement is listed")
        );
        assert_eq!(finding.fix, FixKind::Manual);
        let advice = &finding.review[0].message;
        assert!(advice.starts_with("Fine-tuned model:"), "{advice}");
        assert!(advice.contains("does not support fine-tuning"), "{advice}");
        assert!(!advice.contains("retrain on gpt-6-luna"), "{advice}");

        // A listed base that cannot be fine-tuned is named, not recommended:
        // ft-o4-mini-2025-04-16 is the 23 Oct table's only fine-tune row.
        let finding = one(&[openai("ft:o4-mini-2025-04-16:acme::abc123", &[])]);
        assert_eq!(
            finding.replacement_text.as_deref(),
            Some("no fine-tunable replacement is listed")
        );
        assert!(finding.review[0]
            .message
            .contains("OpenAI lists gpt-5.6-terra for fine-tunes of o4-mini-2025-04-16"));

        // Since OpenAI removed its separate fine-tune table (26-27 Oct), the
        // page names no replacement for fine-tunes of gpt-3.5-turbo: they
        // follow their base model, and no base is named.
        let finding = one(&[openai("ft:gpt-3.5-turbo:acme::abc123", &[])]);
        assert_eq!(finding.date.unwrap().iso(), "2026-10-23");
        assert_eq!(
            finding.replacement_text.as_deref(),
            Some("no fine-tunable replacement is listed")
        );
        let advice = &finding.review[0].message;
        assert!(
            advice.contains("OpenAI lists no replacement for fine-tunes of gpt-3.5-turbo"),
            "{advice}"
        );
        assert!(!advice.contains("gpt-5.6-terra"), "{advice}");
    }

    #[test]
    fn fine_tuning_jobs_never_get_a_base_that_cannot_be_fine_tuned() {
        // rv6 should-fix 2: `fine_tuning.jobs.create(model=<retiring base>)`
        // read as a swap to gpt-6-luna, which does not support fine-tuning.
        let job = |model: &str| {
            call(
                Sdk::Openai,
                Routing::OpenaiDirect,
                "client.fine_tuning.jobs.create",
                model,
                &["training_file", "hyperparameters"],
                30,
            )
        };
        for model in ["gpt-4o-mini-2024-07-18", "gpt-4o-mini", "gpt-3.5-turbo"] {
            let finding = one(&[job(model)]);
            assert_eq!(finding.endpoint, Some(FINE_TUNING), "{model}");
            assert_eq!(finding.status, State::Retiring, "{model}");
            assert_eq!(finding.replacement, None, "{model}");
            assert_eq!(finding.live_replacement, None, "{model}");
            assert!(finding.replacement_chain.is_empty(), "{model}");
            assert_eq!(
                finding.replacement_text.as_deref(),
                Some("no fine-tunable replacement is listed"),
                "{model}"
            );
            assert_eq!(finding.fix, FixKind::Manual, "{model}");
            // No argument rule of the base's replacement applies to a job.
            assert!(
                finding.changes.is_empty() && finding.decisions.is_empty(),
                "{model}: {finding:#?}"
            );
            let advice = &finding.review[0].message;
            assert!(advice.starts_with("Fine-tuning job:"), "{advice}");
            assert!(!advice.contains("start new jobs on"), "{advice}");
        }
        // The snapshot's fine-tune row says why.
        let finding = one(&[job("gpt-4o-mini-2024-07-18")]);
        assert_eq!(finding.date.unwrap().iso(), "2027-01-14");
        assert!(
            finding.review[0]
                .message
                .contains("gpt-6-luna, the replacement OpenAI lists for these models, does not support fine-tuning"),
            "{:#?}",
            finding.review
        );
        // The alias has no fine-tune row: its listed replacement is named as
        // a replacement, not as a base to fine-tune.
        let finding = one(&[job("gpt-4o-mini")]);
        assert!(
            finding.review[0].message.contains(
                "OpenAI lists gpt-6-luna as the replacement for gpt-4o-mini, not as a base for fine-tuning"
            ),
            "{:#?}",
            finding.review
        );
        // A listed fine-tune base that cannot be fine-tuned is named, not
        // recommended.
        let finding = one(&[job("o4-mini-2025-04-16")]);
        assert!(finding.review[0]
            .message
            .contains("OpenAI lists gpt-5.6-terra for fine-tunes of o4-mini-2025-04-16"));
        // gpt-3.5-turbo has no fine-tune row since OpenAI removed that table:
        // its listed replacement is named as a replacement, not as a base.
        let finding = one(&[job("gpt-3.5-turbo")]);
        assert!(
            finding.review[0].message.contains(
                "OpenAI lists gpt-5.6-terra as the replacement for gpt-3.5-turbo, not as a base for fine-tuning"
            ),
            "{:#?}",
            finding.review
        );
        // An `ft:` model continued in a job keeps the fine-tune advice.
        let finding = one(&[job("ft:gpt-4o-mini-2024-07-18:acme::abc123")]);
        assert_eq!(finding.match_kind, MatchKind::FineTuned);
        assert!(finding.review[0].message.starts_with("Fine-tuned model:"));
        // The pre-1.0 module-level calls are fine-tuning jobs too.
        assert_eq!(
            endpoint_of("openai.FineTuningJob.create", "FineTuningJob.create"),
            Some(FINE_TUNING)
        );
        assert_eq!(
            endpoint_of("openai.FineTune.create", "FineTune.create"),
            Some(FINE_TUNING)
        );
        let report = run(&[job("gpt-4o-mini")], &[], &options("2026-10-18"));
        assert_eq!((report.summary.swap, report.summary.manual), (0, 1));
    }

    #[test]
    fn nested_models_get_only_the_rules_about_the_model() {
        // rv6 should-fix 3: a Message Batch's `requests[].params.model` got
        // the outer call's argument rules ("add thinking=..." on
        // `batches.create`, which takes no such argument), and the nested
        // request's own parameters were never looked at.
        let source = r#"from anthropic import Anthropic
from openai import OpenAI

claude = Anthropic()
client = OpenAI()


def batch():
    return claude.messages.batches.create(
        requests=[{"custom_id": "a", "params": {"model": "claude-3-5-sonnet-20241022", "max_tokens": 10, "messages": [], "temperature": 0.2}}]
    )


def plain():
    return claude.messages.create(model="claude-3-5-sonnet-20241022", max_tokens=10, messages=[], temperature=0.2)


def tool_model(tools):
    return client.chat.completions.create(model="gpt-4o", messages=[], max_tokens=10, tools=[{"type": "x", "model": "gpt-4o-mini"}])
"#;
        let report = check_source(source, &options("2026-10-20"));
        let at = |needle: &str, key: &str| -> Finding {
            let line = line_of(source, needle);
            let found: Vec<&Finding> = report
                .findings
                .iter()
                .filter(|f| f.line == line && f.key == key)
                .collect();
            assert_eq!(found.len(), 1, "{needle} {key}: {:#?}", report.findings);
            found[0].clone()
        };
        let nested = at("batches.create", "requests.params.model");
        assert!(nested.changes.is_empty(), "{:#?}", nested.changes);
        assert!(nested.decisions.is_empty(), "{:#?}", nested.decisions);
        assert_eq!(nested.fix, FixKind::Manual);
        let check = nested
            .review
            .iter()
            .find(|r| r.message.contains("requests.params"))
            .expect("a check naming the nested request");
        assert!(
            check
                .message
                .contains("check those against claude-sonnet-5-5 by hand"),
            "{}",
            check.message
        );
        // No rule about arguments or an endpoint: only the model's own.
        let data = bundled().unwrap();
        for item in &nested.review {
            if let Some(id) = &item.rule {
                let rule = data.data.rules.iter().find(|r| &r.id == id).unwrap();
                assert!(about_the_model(rule), "{id} is about the request");
            }
        }
        assert!(nested
            .review
            .iter()
            .map(|r| r.message.as_str())
            .chain(nested.notes.iter().map(String::as_str))
            .all(|m| !m.contains("temperature")));
        // The same model at the request itself still gets its rules.
        let plain = at("claude.messages.create(", "model");
        assert!(plain
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("ANT-SAMPLING")));

        // A tool's model in a Chat Completions call: the call's tools,
        // max_tokens and reasoning rules are about the call, not the tool.
        if let Some(tool) = report.findings.iter().find(|f| f.key == "tools.model") {
            assert!(
                tool.changes.is_empty() && tool.decisions.is_empty(),
                "{tool:#?}"
            );
            assert_eq!(tool.fix, FixKind::Manual);
            assert!(tool.review.iter().any(|r| r.message.contains("in tools,")));
        }
        let call = at("client.chat.completions.create(", "model");
        assert!(change(&call, "max_tokens").is_some(), "{call:#?}");
        assert_eq!(call.decisions.len(), 1);
    }

    #[test]
    fn calls_through_unresolved_clients_with_unread_models_are_counted() {
        // rv6 should-fix 4: a call through a client the usage scan could not
        // resolve, whose model it could not read either, appeared on no line
        // (openai/swarm's helper sends a shut-down default via `**params`).
        let source = r#"from openai import OpenAI

client = OpenAI()


def helper(api, model, messages):
    return api.chat.completions.create(model=model, messages=messages)


def helper_dict(api, messages, model="gpt-4-0125-preview"):
    params = {"model": model, "messages": messages}
    return api.chat.completions.create(**params)


def factory(make):
    return make().chat.completions.create(messages=[])


def factory_literal(make):
    return make().chat.completions.create(model="gpt-4-0125-preview", messages=[])


def known(model):
    return client.chat.completions.create(model=model, messages=[])
"#;
        let report = check_source(source, &options("2026-10-20"));
        // helper, helper_dict and factory: shown nowhere else.
        assert_eq!(
            report.summary.unresolved_clients, 3,
            "{:#?}",
            report.summary
        );
        // The factory call with a literal model is a finding (a gap), the
        // known client's parameter is unreadable: neither is counted again.
        let line = line_of(source, "make().chat.completions.create(model=");
        assert!(report
            .findings
            .iter()
            .any(|f| f.line == line && f.confidence == Confidence::Gap));
        assert_eq!(report.summary.unreadable_calls, 1);
        // Every SDK-shaped call in the file is on some line of the report.
        let found = scan(source);
        let shown = report.findings.len()
            + report.summary.unreadable_calls
            + report.summary.no_model_calls
            + report.summary.untracked_methods
            + report.summary.unresolved_clients;
        let sdk_calls: HashSet<u32> = found
            .calls
            .iter()
            .map(|c| c.span.start_byte)
            .chain(
                found
                    .gaps
                    .iter()
                    .filter_map(|g| g.span.map(|s| s.start_byte)),
            )
            .collect();
        assert!(shown >= sdk_calls.len(), "{shown} < {}", sdk_calls.len());
    }

    #[test]
    fn routed_calls_are_kept_apart() {
        let azure = call(
            Sdk::Openai,
            Routing::AzureOpenai,
            "azure.chat.completions.create",
            "gpt-4o",
            &[],
            30,
        );
        let proxy = call(
            Sdk::Openai,
            Routing::CustomBaseUrl,
            "router.chat.completions.create",
            "anthropic/claude-3-opus-20240229",
            &[],
            31,
        );
        let calls = [azure, proxy];
        let report = run(&calls, &[], &options("2026-10-18"));
        assert!(report.findings.is_empty());
        assert!(report.routed.is_empty());
        assert_eq!(report.summary.routed_calls, 2);
        assert_eq!(report.exit_code, 0, "routed calls do not fail by default");

        let mut opts = options("2026-10-18");
        opts.include_routed = true;
        let report = run(&calls, &[], &opts);
        assert_eq!(report.routed.len(), 2);
        let claude = report
            .routed
            .iter()
            .find(|f| f.provider == Table::Anthropic)
            .unwrap();
        assert_eq!(claude.match_kind, MatchKind::RouterPrefix);
        assert_eq!(claude.matched, "claude-3-opus-20240229");
        assert_eq!(claude.routing, "custom_base_url");
        assert_eq!(
            report.exit_code, 1,
            "with --include-routed a retired model fails"
        );
    }

    #[test]
    fn base_url_routing_follows_the_usage_scan() {
        // The usage scan routes a base_url by its host: the provider's own
        // host stays direct, an environment lookup adds a gap, another host
        // is custom_base_url. A gap leaves the routing unknown, even when the
        // lookup falls back to the provider's own host: the deployment may
        // set the variable.
        let model = "gpt-3.5-turbo-0613";
        let at = |line: u32, routing: Routing| {
            let mut c = call(
                Sdk::Openai,
                routing,
                "client.chat.completions.create",
                model,
                &[],
                line,
            );
            c.client_line = Some(line - 20);
            c
        };
        let calls = [
            at(21, Routing::OpenaiDirect),
            at(22, Routing::OpenaiDirect),
            at(23, Routing::Unknown),
            at(24, Routing::Unknown),
            at(25, Routing::CustomBaseUrl),
            at(26, Routing::OpenaiDirect),
        ];
        let client = |line: u32| ClientConstruction {
            sdk: Sdk::Openai,
            package: "openai".into(),
            constructor: "openai.OpenAI".into(),
            chain: "OpenAI".into(),
            binding: Some("client".into()),
            scope: crate::usages::ClientScope::Module,
            routing: Routing::OpenaiDirect,
            is_async: false,
            kwargs: if line == 6 {
                Vec::new()
            } else {
                vec!["base_url".into()]
            },
            splat: false,
            file: "app.py".into(),
            span: span(line),
            symbol: None,
            vertexai: None,
        };
        let clients: Vec<ClientConstruction> = (1..=6).map(client).collect();
        let gap = |line: u32, reason: &str| UsageGap {
            kind: GapKind::AmbiguousClient,
            sdk: Some(Sdk::Openai),
            file: "app.py".into(),
            span: Some(span(line)),
            symbol: None,
            chain: Some("client.chat.completions.create".into()),
            key: None,
            reason: reason.into(),
            detail: String::new(),
            values: Vec::new(),
            values_complete: false,
            args: None,
        };
        let gaps = [
            gap(22, "base_url_env"),
            gap(23, "base_url_env"),
            gap(24, "base_url_unresolved"),
        ];
        let run_with = |opts: &CheckOptions| {
            let input = ScanInput {
                calls: &calls,
                gaps: &gaps,
                clients: &clients,
            };
            check(input, bundled().unwrap(), opts)
        };
        let report = run_with(&options("2026-10-18"));
        let class = |line: u32| {
            report
                .findings
                .iter()
                .find(|f| f.line == line)
                .map(|f| (f.routing_class, f.base_url, f.fails))
        };
        assert_eq!(
            class(21),
            Some((RoutingClass::Direct, Some(BaseUrl::ProviderHost), true))
        );
        // Listed with routing unknown, and not counted by default.
        assert_eq!(
            class(22),
            Some((
                RoutingClass::Unknown,
                Some(BaseUrl::EnvProviderDefault),
                false
            ))
        );
        assert_eq!(
            class(23),
            Some((RoutingClass::Unknown, Some(BaseUrl::EnvOnly), false))
        );
        assert_eq!(
            class(24),
            Some((RoutingClass::Unknown, Some(BaseUrl::Unreadable), false))
        );
        assert_eq!(class(25), None, "another host stays routed");
        assert_eq!(class(26), Some((RoutingClass::Direct, None, true)));
        assert_eq!(report.summary.routed_calls, 1);
        assert_eq!(report.summary.uncounted, 3);
        let gap_of = |line: u32| {
            report
                .findings
                .iter()
                .find(|f| f.line == line)
                .and_then(|f| f.routing_gap.clone())
        };
        assert_eq!(gap_of(22).as_deref(), Some("base_url_env"));
        assert_eq!(gap_of(24).as_deref(), Some("base_url_unresolved"));
        assert_eq!(gap_of(21), None);

        let mut opts = options("2026-10-18");
        opts.count_unconfirmed = true;
        assert_eq!(run_with(&opts).summary.failing, 5);
        opts.include_routed = true;
        let report = run_with(&opts);
        assert_eq!(report.summary.failing, 6);
        assert_eq!(report.routed[0].base_url, Some(BaseUrl::OtherHost));
    }

    #[test]
    fn unconfirmed_values_are_listed_but_count_only_when_asked() {
        let mut env = openai("gpt-3.5-turbo-0613", &[]);
        env.values[0].origin = ValueOrigin::EnvDefault;
        env.values[0].name = Some("os.getenv(\"MODEL\")".into());
        let report = run(std::slice::from_ref(&env), &[], &options("2026-10-18"));
        let finding = &report.findings[0];
        assert_eq!(finding.confidence, Confidence::ConfirmFirst);
        assert_eq!(finding.origin, Some(ValueOrigin::EnvDefault));
        assert_eq!(
            finding.fix,
            FixKind::Manual,
            "confirm-first is never a swap"
        );
        assert!(finding.in_window && !finding.fails);
        assert_eq!(report.exit_code, 0);
        assert_eq!(report.summary.uncounted, 1);

        let mut opts = options("2026-10-18");
        opts.count_unconfirmed = true;
        assert_eq!(run(&[env], &[], &opts).exit_code, 1);

        let gap = UsageGap {
            kind: GapKind::UnknownClientFactory,
            sdk: Some(Sdk::Openai),
            file: "svc.py".into(),
            span: Some(span(40)),
            symbol: None,
            chain: Some("self.client.chat.completions.create".into()),
            key: None,
            reason: "factory_call".into(),
            detail: String::new(),
            values: vec![value("gpt-4o-2024-08-06", ValueOrigin::Literal, 40)],
            values_complete: false,
            args: None,
        };
        let report = run(&[], &[gap], &options("2026-10-18"));
        let finding = &report.findings[0];
        assert_eq!(finding.confidence, Confidence::Gap);
        assert_eq!(finding.routing_class, RoutingClass::Unknown);
        assert_eq!(finding.gap_reason.as_deref(), Some("factory_call"));
        assert_eq!(finding.origin, None);
        assert_eq!(finding.replacement.as_deref(), Some("gpt-6-luna"));
        assert_eq!(finding.fix, FixKind::Manual, "a gap is never a swap");
    }

    #[test]
    fn unreadable_models_are_counted_once_and_fallbacks_are_not() {
        let mut env = openai("gpt-4.1", &[]);
        env.values[0].origin = ValueOrigin::EnvDefault;
        let gap = |line: u32| UsageGap {
            kind: GapKind::UnresolvedModel,
            sdk: Some(Sdk::Openai),
            file: "app.py".into(),
            span: Some(span(line)),
            symbol: None,
            chain: Some("client.chat.completions.create".into()),
            key: Some("model".into()),
            reason: "env_var".into(),
            detail: String::new(),
            values: Vec::new(),
            values_complete: false,
            args: None,
        };
        // Line 10 has a captured fallback; line 50 has nothing.
        let report = run(&[env], &[gap(10), gap(50), gap(50)], &options("2026-10-18"));
        assert_eq!(report.unreadable.len(), 1);
        assert_eq!(report.unreadable[0].line, 50);
        assert_eq!(report.summary.unreadable_calls, 1);
    }

    #[test]
    fn bare_gpt_5_and_o3_shut_down_with_their_snapshots_on_11_december() {
        // OpenAI's deprecations page lists each 11 Dec row as the bare ID and
        // its snapshot (`gpt-5` | `gpt-5-2025-08-07`) since late October 2026;
        // Arbor 3.1.0 had only the snapshots, so `model="gpt-5"` and
        // `model="o3"` were not listed at all.
        let source = r#"from openai import OpenAI

client = OpenAI()


def run(question):
    messages = [{"role": "user", "content": question}]
    client.chat.completions.create(model="gpt-5-2025-08-07", messages=messages)
    client.chat.completions.create(model="gpt-5", messages=messages)
    client.chat.completions.create(model="o3", messages=messages)
    client.chat.completions.create(model="gpt-5-mini", messages=messages)
    client.chat.completions.create(model="gpt-5-nano", messages=messages)
    client.chat.completions.create(model="gpt-5-pro", messages=messages)
    client.chat.completions.create(model="o3-pro", messages=messages)
    client.chat.completions.create(model="o1-preview", messages=messages)
"#;
        let report = check_source(source, &options("2026-10-28"));
        let at = |model: &str| -> Finding {
            let line = line_of(source, &format!("model=\"{model}\""));
            let found: Vec<&Finding> = report.findings.iter().filter(|f| f.line == line).collect();
            assert_eq!(found.len(), 1, "{model}: {:#?}", report.findings);
            found[0].clone()
        };
        for (model, replacement, text) in [
            ("gpt-5-2025-08-07", "gpt-5.6-sol", None),
            ("gpt-5", "gpt-5.6-sol", None),
            ("o3", "gpt-5.6-sol", None),
            ("gpt-5-mini", "gpt-5.6-terra", None),
            ("gpt-5-nano", "gpt-5.6-luna", None),
            (
                "gpt-5-pro",
                "gpt-5.6-sol",
                Some("gpt-5.6-sol (reasoning.mode: pro)"),
            ),
            (
                "o3-pro",
                "gpt-5.6-sol",
                Some("gpt-5.6-sol (reasoning.mode: pro)"),
            ),
        ] {
            let finding = at(model);
            assert_eq!(finding.matched, model, "matched exactly, never by prefix");
            assert_eq!(finding.match_kind, MatchKind::Exact, "{model}");
            assert_eq!(finding.provider, Table::Openai, "{model}");
            assert_eq!(finding.status, State::Retiring, "{model}");
            assert_eq!(finding.date.unwrap().iso(), "2026-12-11", "{model}");
            assert_eq!(finding.days_left, Some(44), "{model}");
            assert_eq!(finding.replacement.as_deref(), Some(replacement), "{model}");
            assert_eq!(finding.replacement_text.as_deref(), text, "{model}");
            assert_eq!(finding.confidence, Confidence::Confirmed, "{model}");
            // The page is the source: these rows carry no models API note.
            assert_eq!(
                finding.source_url.as_deref(),
                Some("https://developers.openai.com/api/docs/deprecations"),
                "{model}"
            );
            assert!(finding.notes.is_empty(), "{model}: {:?}", finding.notes);
            // No recipe has rules for GPT-5.6 targets of these rows yet: the
            // date and replacement only, and a manual check, never a change.
            assert!(finding.changes.is_empty(), "{model}");
            assert!(finding.decisions.is_empty(), "{model}");
            assert_eq!(finding.fix, FixKind::Manual, "{model}");
            assert!(
                finding.review.iter().any(|r| r.message
                    == format!("The parameters {replacement} needs have not been checked yet.")),
                "{model}: {:#?}",
                finding.review
            );
        }
        // o1-preview (shut down 28 Jul 2025) lists o3, which now has a date
        // too, so the chain goes on to gpt-5.6-sol.
        let preview = at("o1-preview");
        assert_eq!(preview.status, State::Retired);
        assert_eq!(preview.replacement.as_deref(), Some("o3"));
        assert_eq!(preview.live_replacement.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(preview.replacement_chain[0].model, "o3");
        assert_eq!(preview.replacement_chain[0].status, Some(State::Retiring));
        assert_eq!(report.findings.len(), 8, "{:#?}", report.findings);
    }

    #[test]
    fn matching_is_exact() {
        for not_listed in [
            "GPT-4O",
            "gpt-4o ",
            "gpt-4o-2099-01-01",
            "my-gpt-4o",
            "gpt-6-luna",
        ] {
            let report = run(&[openai(not_listed, &[])], &[], &options("2026-10-18"));
            assert!(report.findings.is_empty(), "{not_listed}");
        }
        // A model ID from another provider at a direct OpenAI client is not
        // matched against Anthropic's schedule.
        let report = run(
            &[openai("claude-3-opus-20240229", &[])],
            &[],
            &options("2026-10-18"),
        );
        assert!(report.findings.is_empty());
    }

    #[test]
    fn resource_prefixes_are_matched_and_never_swapped() {
        let gemini = call(
            Sdk::Google,
            Routing::GoogleGenai,
            "client.models.generate_content",
            "models/gemini-2.0-flash",
            &["contents"],
            60,
        );
        let finding = one(&[gemini]);
        assert_eq!(finding.provider, Table::GeminiApi);
        assert_eq!(finding.match_kind, MatchKind::ResourcePrefix);
        assert_eq!(finding.status, State::Retired);
        assert_eq!(
            finding.fix,
            FixKind::Manual,
            "a prefixed match is never a plain swap"
        );
    }

    /// The usage scan of one Python file, `app.py`.
    fn scan(source: &str) -> crate::usages::FileUsages {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::usages::python::extract(&tree, source, "app.py")
    }

    /// The usage scan of `source`, checked.
    fn check_source(source: &str, opts: &CheckOptions) -> RetirementReport {
        let found = scan(source);
        let input = ScanInput {
            calls: &found.calls,
            gaps: &found.gaps,
            clients: &found.clients,
        };
        check(input, bundled().unwrap(), opts)
    }

    /// The usage scan of one TypeScript file, `app.ts`, checked.
    fn check_typescript(source: &str, opts: &CheckOptions) -> RetirementReport {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::language_typescript())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let found = crate::usages::typescript::extract(&tree, source, "app.ts");
        let input = ScanInput {
            calls: &found.calls,
            gaps: &found.gaps,
            clients: &found.clients,
        };
        check(input, bundled().unwrap(), opts)
    }

    #[test]
    fn typescript_calls_are_checked_like_python_ones() {
        let source = r#"import OpenAI from "openai";
import Anthropic from "@anthropic-ai/sdk";
import { GoogleGenAI } from "@google/genai";
import { Configuration, OpenAIApi } from "openai";
const openai = new OpenAI();
const proxied = new OpenAI({ baseURL: "https://api.openai.com/v1" });
const anthropic = new Anthropic();
const ai = new GoogleGenAI({ vertexai: false });
const legacy = new OpenAIApi(new Configuration({ apiKey: key }));
export async function run() {
  await openai.chat.completions.create({ model: "gpt-3.5-turbo", messages: [], max_tokens: 10 });
  await proxied.chat.completions.create({ model: "gpt-4", messages: [] });
  await anthropic.messages.create({ model: "claude-3-opus-20240229", max_tokens: 10, messages: [] });
  await ai.models.generateContent({ model: "gemini-1.5-flash", contents: "x" });
  await openai.fineTuning.jobs.create({ model: "gpt-3.5-turbo", training_file: "f" });
  await legacy.createChatCompletion({ model: "gpt-4o", messages: [], max_tokens: 5 });
  await anthropic.messages.create({ model: "claude-sonnet-4-5", max_tokens: 10, messages: [] });
}
"#;
        let report = check_typescript(source, &options("2026-10-24"));
        let at = |needle: &str| -> &Finding {
            let line = line_of(source, needle);
            let found: Vec<&Finding> = report.findings.iter().filter(|f| f.line == line).collect();
            assert_eq!(found.len(), 1, "{needle}: {:#?}", report.findings);
            found[0]
        };
        for needle in [
            "\"gpt-3.5-turbo\", messages",
            "\"gpt-4\"",
            "claude-3-opus-20240229",
            "gemini-1.5-flash",
            "training_file",
        ] {
            let finding = at(needle);
            assert_eq!(finding.status, State::Retired, "{needle}");
            assert_eq!(finding.confidence, Confidence::Confirmed, "{needle}");
            assert_eq!(finding.routing_class, RoutingClass::Direct, "{needle}");
        }
        // Endpoints read the JavaScript SDKs' camelCase members and openai
        // v3's methods.
        assert_eq!(at("gemini-1.5-flash").endpoint, Some("generate_content"));
        assert_eq!(at("training_file").endpoint, Some(FINE_TUNING));
        assert_eq!(
            at("createChatCompletion").endpoint,
            Some("chat.completions")
        );
        assert_eq!(at("createChatCompletion").status, State::Retiring);
        // `baseURL` is the JavaScript SDKs' base URL option.
        assert_eq!(at("\"gpt-4\"").base_url, Some(BaseUrl::ProviderHost));
        // The argument rules read the options object's property names.
        let swap = at("\"gpt-3.5-turbo\", messages");
        assert_eq!(swap.changes[0].param, "max_tokens");
        assert_eq!(swap.changes[0].to.as_deref(), Some("max_completion_tokens"));
        // A decision's options are written the way the call passes them.
        let thinking = at("claude-sonnet-4-5");
        let between = &thinking.decisions[0].options[0].add[0];
        assert_eq!(
            source_argument(
                &between.name,
                &between.value,
                Syntax::of_file(&thinking.file)
            ),
            "thinking: {\"type\": \"between_tools\"}"
        );
        assert_eq!(
            source_argument(&between.name, &between.value, Syntax::Python),
            "thinking={\"type\": \"between_tools\"}"
        );
    }

    /// gpt-4o's move asks whether gpt-6-luna should reason, and an option
    /// that adds nothing reads "no change" only when every argument the
    /// call sends was read. In TypeScript a spread, a computed key or an
    /// object passed by name may send any of them (#291).
    #[test]
    fn a_typescript_property_not_written_at_the_call_is_never_read_as_checked() {
        let source = r#"import OpenAI from "openai";
const openai = new OpenAI();
const BASE = { temperature: 0.2 };
export async function run(opts: any, key: string, client: any, params: any) {
  await openai.chat.completions.create({ model: "gpt-4o", messages: [] });
  await openai.chat.completions.create({ model: "gpt-4o-mini", ...opts });
  await openai.chat.completions.create({ ...opts, model: "gpt-4.1", messages: [] });
  await openai.chat.completions.create({ model: "gpt-4.1-mini", [key]: 1, messages: [] });
  await openai.chat.completions.create({ ...BASE, model: "gpt-4o-2024-08-06", messages: [] });
  await openai.chat.completions.create(...params, { model: "gpt-4o-2024-05-13" });
  await client.chat.completions.create({ model: "gpt-4o", max_tokens: 5, messages: [] });
  await client.chat.completions.create({ ...opts, model: "gpt-4o-mini", messages: [] });
}
"#;
        let report = check_typescript(source, &options("2026-10-24"));
        let at = |needle: &str| -> &Finding {
            let line = line_of(source, needle);
            let found: Vec<&Finding> = report.findings.iter().filter(|f| f.line == line).collect();
            assert_eq!(found.len(), 1, "{needle}: {:#?}", report.findings);
            found[0]
        };
        let unread = "Some properties of the options object are not written at the call (a spread, a computed key or an object passed by name), so Arbor could not check them all.";

        // Every property written at the call: read in full.
        let written = at("openai.chat.completions.create({ model: \"gpt-4o\"");
        assert!(!written.decisions.is_empty(), "{written:#?}");
        assert_eq!(written.decisions[0].unchecked_args, None);
        assert!(written.review.iter().all(|r| r.message != unread));

        for needle in [
            "\"gpt-4o-mini\", ...opts",
            "\"gpt-4.1\"",
            "[key]",
            "...BASE",
            "...params",
            "{ ...opts, model: \"gpt-4o-mini\"",
        ] {
            let finding = at(needle);
            assert!(!finding.decisions.is_empty(), "{needle}: {finding:#?}");
            for decision in &finding.decisions {
                assert_eq!(
                    decision.unchecked_args,
                    Some(UncheckedArgs::Splat),
                    "{needle}"
                );
                assert!(decision.unread_spreads.is_empty(), "{needle}");
            }
            assert!(
                finding.review.iter().any(|r| r.message == unread),
                "{needle}: {:#?}",
                finding.review
            );
            assert_eq!(finding.fix, FixKind::Manual, "{needle}");
        }

        // A call whose client is not resolved still has its arguments
        // checked: the TypeScript scan records them on the gap.
        let unresolved = at("max_tokens: 5");
        assert_eq!(unresolved.confidence, Confidence::Gap);
        assert_eq!(unresolved.decisions[0].unchecked_args, None);
        assert_eq!(unresolved.changes[0].param, "max_tokens");
        assert_eq!(
            unresolved.changes[0].to.as_deref(),
            Some("max_completion_tokens")
        );
    }

    #[test]
    fn literals_follow_the_call_language() {
        let value = serde_json::json!([true, null, 2, {"effort": "none"}]);
        assert_eq!(
            source_literal(&value, Syntax::Python),
            "[True, None, 2, {\"effort\": \"none\"}]"
        );
        assert_eq!(
            source_literal(&value, Syntax::JavaScript),
            "[true, null, 2, {\"effort\": \"none\"}]"
        );
        for (file, syntax) in [
            ("app.py", Syntax::Python),
            ("scripts/run", Syntax::Python),
            ("src/app.ts", Syntax::JavaScript),
            ("pages/chat.tsx", Syntax::JavaScript),
            ("lib/ai.MJS", Syntax::JavaScript),
            ("server.cjs", Syntax::JavaScript),
        ] {
            assert_eq!(Syntax::of_file(file), syntax, "{file}");
        }
        assert_eq!(
            endpoint_of("ai.models.embedContent", "models.embedContent"),
            Some("embeddings")
        );
        assert_eq!(
            endpoint_of("chat.sendMessage", "sendMessage"),
            Some("generate_content")
        );
        assert_eq!(
            endpoint_of("openai.createCompletion", "createCompletion"),
            Some("completions_legacy")
        );
        assert_eq!(
            endpoint_of("openai.createTranscription", "createTranscription"),
            Some("audio")
        );
    }

    /// The 1-based line of the only line holding `needle`.
    fn line_of(source: &str, needle: &str) -> u32 {
        let lines: Vec<usize> = source
            .lines()
            .enumerate()
            .filter(|(_, l)| l.contains(needle))
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(lines.len(), 1, "{needle}");
        lines[0] as u32
    }

    #[test]
    fn the_vertexai_flag_picks_the_schedule() {
        // `vertexai=False` or `None` is the Gemini API, `True` is Vertex AI,
        // and a value decided at run time could be either (rv6 should-fix 1:
        // every `vertexai=` was looked up on Vertex AI's schedule only).
        let source = r#"import os

from google import genai

api = genai.Client(vertexai=False, api_key=os.environ["GEMINI_API_KEY"])
nothing = genai.Client(vertexai=None)
cloud = genai.Client(vertexai=True, project="p", location="us-central1")
toggle = genai.Client(vertexai=os.getenv("USE_VERTEX", "false") == "true")
plain = genai.Client()


def run():
    api.models.generate_content(model="gemini-1.5-flash", contents="api-retired")
    api.models.generate_content(model="gemini-2.5-flash", contents="api-vertex-only")
    api.models.generate_content(model="gemini-2.0-flash", contents="api-both")
    nothing.models.generate_content(model="gemini-2.0-flash", contents="none-both")
    cloud.models.generate_content(model="gemini-2.0-flash", contents="cloud-both")
    cloud.models.generate_content(model="gemini-1.5-flash", contents="cloud-api-only")
    cloud.models.generate_content(model="gemini-2.5-flash", contents="cloud-vertex-only")
    toggle.models.generate_content(model="gemini-1.5-flash", contents="toggle-api-only")
    toggle.models.generate_content(model="gemini-2.0-flash", contents="toggle-both")
    plain.models.generate_content(model="gemini-2.5-flash", contents="plain-vertex-only")
"#;
        // The scan records the class of each `vertexai=`, never the value.
        let found = scan(source);
        let flags: Vec<(Option<&str>, Option<VertexAi>)> = found
            .clients
            .iter()
            .map(|c| (c.binding.as_deref(), c.vertexai))
            .collect();
        assert_eq!(
            flags,
            [
                (Some("api"), Some(VertexAi::Off)),
                (Some("nothing"), Some(VertexAi::Off)),
                (Some("cloud"), Some(VertexAi::On)),
                (Some("toggle"), Some(VertexAi::Unknown)),
                (Some("plain"), None),
            ]
        );
        let plain = serde_json::to_value(&found.clients[4]).unwrap();
        assert!(plain.get("vertexai").is_none(), "{plain}");

        let data = bundled().unwrap();
        let report = check_source(source, &options("2026-10-20"));
        let at = |needle: &str| -> Vec<&Finding> {
            let line = line_of(source, needle);
            report.findings.iter().filter(|f| f.line == line).collect()
        };
        let one = |needle: &str| -> &Finding {
            let found = at(needle);
            assert_eq!(found.len(), 1, "{needle}: {found:#?}");
            found[0]
        };
        let listed = |table: Table, model: &str| data.entry(table, model).unwrap();

        // vertexai=False: the Gemini API, which shut gemini-1.5-flash down.
        let retired = one("api-retired");
        assert_eq!(retired.provider, Table::GeminiApi);
        assert_eq!(retired.status, State::Retired);
        assert_eq!(retired.routing_class, RoutingClass::Direct);
        assert_eq!(retired.confidence, Confidence::Confirmed);
        assert!(retired.fails);
        assert_eq!(report.exit_code, 1);
        // Never Vertex AI's schedule on a Gemini API client.
        assert!(at("api-vertex-only")
            .iter()
            .all(|f| f.provider == Table::GeminiApi));
        for needle in ["api-both", "none-both"] {
            let both = one(needle);
            assert_eq!(both.provider, Table::GeminiApi, "{needle}");
            assert_eq!(
                both.replacement,
                listed(Table::GeminiApi, "gemini-2.0-flash").replacement,
                "{needle}"
            );
            assert!(both.review.iter().all(|r| !r.message.contains("run time")));
        }

        // vertexai=True: Vertex AI only, routing not confirmed (the scan's
        // `endpoint_option` gap).
        let cloud = one("cloud-both");
        assert_eq!(cloud.provider, Table::Vertex);
        assert_eq!(
            cloud.replacement,
            listed(Table::Vertex, "gemini-2.0-flash").replacement
        );
        assert_eq!(cloud.routing_class, RoutingClass::Unknown);
        assert_eq!(cloud.routing_gap.as_deref(), Some("endpoint_option"));
        assert!(!cloud.fails);
        assert!(at("cloud-api-only").is_empty(), "a Gemini API-only model");
        // Google Cloud's dates are the earliest possible.
        let earliest = one("cloud-vertex-only");
        let entry = listed(Table::Vertex, "gemini-2.5-flash");
        assert_eq!(earliest.date_kind, DateKind::Earliest);
        assert_eq!(earliest.date.map(|d| d.iso()), entry.date.clone());

        // A run-time toggle: Vertex AI first, then the Gemini API, never
        // confirmed, and the check says which schedule was used.
        let toggled = one("toggle-api-only");
        assert_eq!(toggled.provider, Table::GeminiApi);
        assert_eq!(toggled.status, State::Retired);
        assert_eq!(toggled.routing_class, RoutingClass::Unknown);
        assert!(!toggled.fails);
        assert_eq!(toggled.fix, FixKind::Manual);
        assert!(toggled
            .review
            .iter()
            .any(|r| r.message.contains("decided at run time")
                && r.message.contains("Gemini API's schedule")));
        let toggled = one("toggle-both");
        assert_eq!(toggled.provider, Table::Vertex);
        assert!(toggled
            .review
            .iter()
            .any(|r| r.message.contains("decided at run time")
                && r.message.contains("Google's schedule applies instead")));
        let mut opts = options("2026-10-20");
        opts.count_unconfirmed = true;
        let counted = check_source(source, &opts);
        let line = line_of(source, "toggle-api-only");
        assert!(counted.findings.iter().any(|f| f.line == line && f.fails));

        // No flag at all: the Gemini API.
        assert!(at("plain-vertex-only")
            .iter()
            .all(|f| f.provider == Table::GeminiApi));
    }

    #[test]
    fn an_unrouted_google_client_still_finds_vertex_models() {
        // The usage scan routes a `vertexai=True` client as unknown, with an
        // `endpoint_option` gap at each call: the Vertex schedule still
        // applies, and the routing reads as not confirmed.
        let mut gemini = call(
            Sdk::Google,
            Routing::Unknown,
            "client.models.generate_content",
            "gemini-2.5-flash",
            &["contents"],
            70,
        );
        let vertex = ClientConstruction {
            sdk: Sdk::Google,
            package: "google.genai".into(),
            constructor: "google.genai.Client".into(),
            chain: "genai.Client".into(),
            binding: Some("client".into()),
            scope: crate::usages::ClientScope::Module,
            routing: Routing::Unknown,
            is_async: false,
            kwargs: vec!["vertexai".into(), "project".into()],
            splat: false,
            file: "app.py".into(),
            span: span(1),
            symbol: None,
            vertexai: Some(VertexAi::On),
        };
        let mut option = gap_at(GapKind::AmbiguousClient, "endpoint_option", 70, None);
        option.sdk = Some(Sdk::Google);
        let report = check(
            ScanInput {
                calls: std::slice::from_ref(&gemini),
                gaps: std::slice::from_ref(&option),
                clients: std::slice::from_ref(&vertex),
            },
            bundled().unwrap(),
            &options("2026-10-18"),
        );
        let finding = &report.findings[0];
        assert_eq!(finding.provider, Table::Vertex);
        assert_eq!(finding.routing_class, RoutingClass::Unknown);
        assert_eq!(finding.routing_gap.as_deref(), Some("endpoint_option"));
        assert!(!finding.fails);

        // With no `vertexai=` in sight, a model only Vertex lists is still
        // found there.
        gemini.values[0] = value("text-bison", ValueOrigin::Literal, 70);
        let report = run(
            std::slice::from_ref(&gemini),
            std::slice::from_ref(&option),
            &options("2026-10-18"),
        );
        let finding = &report.findings[0];
        assert_eq!(finding.provider, Table::Vertex);
        assert_eq!(finding.routing_class, RoutingClass::Unknown);
    }

    #[test]
    fn summary_and_provider_dates() {
        let calls = [
            openai("gpt-4-0314", &[]),
            openai("gpt-4o", &[]),
            call(
                Sdk::Openai,
                Routing::OpenaiDirect,
                "client.chat.completions.create",
                "gpt-4o-mini",
                &[],
                11,
            ),
        ];
        let report = run(&calls, &[], &options("2026-10-18"));
        assert_eq!(report.summary.affected_calls, 3);
        assert_eq!(report.summary.retired, 1);
        assert_eq!(report.summary.retiring, 2);
        let next = report.summary.next_shutdown.as_ref().unwrap();
        assert_eq!(next.date.iso(), "2027-01-14");
        assert_eq!(next.days_left, 88);
        assert_eq!(next.calls, 2);
        assert_eq!(
            report.findings[0].status,
            State::Retired,
            "retired sorts first"
        );
        assert_eq!(report.findings[0].fix, FixKind::Manual);

        // Each provider's pages carry their own date; as_of is the oldest.
        // The 28 Oct 2026 refresh read every provider's pages that day.
        let dates: Vec<(&str, String)> = report
            .data
            .providers
            .iter()
            .map(|p| (p.provider.as_str(), p.as_of.iso()))
            .collect();
        assert_eq!(dates[0], ("OpenAI", "2026-10-28".to_string()));
        assert_eq!(dates[1], ("Anthropic", "2026-10-28".to_string()));
        let oldest = report.data.providers.iter().map(|p| p.as_of).min().unwrap();
        assert_eq!(report.data.as_of, oldest);
        assert_eq!(report.data.as_of.iso(), "2026-10-28");

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["findings"][1]["status"], "retiring");
        assert_eq!(json["findings"][1]["date"], "2027-01-14");
    }

    #[test]
    fn candidate_values_of_a_known_call_are_confirm_first() {
        // `model=MODELS[tier]`: the call is known, its model is one of the
        // dict's entries, listed on the unresolved_model gap.
        let mut lookup = openai("gpt-4o", &["temperature"]);
        lookup.values.clear();
        let gap = UsageGap {
            kind: GapKind::UnresolvedModel,
            sdk: Some(Sdk::Openai),
            file: "app.py".into(),
            span: Some(span(10)),
            symbol: Some("ask".into()),
            chain: Some("client.chat.completions.create".into()),
            key: Some("model".into()),
            reason: "dict_lookup".into(),
            detail: String::new(),
            values: vec![
                value("gpt-4o-mini", ValueOrigin::Literal, 3),
                value("gpt-6-luna", ValueOrigin::Literal, 3),
            ],
            values_complete: true,
            args: None,
        };
        let missing = UsageGap {
            kind: GapKind::UnresolvedModel,
            span: Some(span(20)),
            key: Some("model".into()),
            reason: "missing_model".into(),
            values: Vec::new(),
            ..gap.clone()
        };
        let report = run(&[lookup], &[gap, missing], &options("2026-10-18"));
        assert_eq!(report.findings.len(), 1, "only the retiring entry matches");
        let finding = &report.findings[0];
        assert_eq!(finding.model, "gpt-4o-mini");
        assert!(finding.candidate);
        assert_eq!(finding.confidence, Confidence::ConfirmFirst);
        assert_eq!(
            finding.routing_class,
            RoutingClass::Direct,
            "the call's own routing"
        );
        assert_eq!(finding.gap_reason.as_deref(), Some("dict_lookup"));
        // The call's arguments still drive the decision.
        assert!(finding.decisions[0]
            .because
            .iter()
            .any(|b| b == "temperature"));
        // Neither gap is an unreadable model: one lists its values, the
        // other passes no model at all.
        assert_eq!(report.summary.unreadable_calls, 0);
        assert_eq!(report.summary.no_model_calls, 1);
    }

    /// A gap of `kind` and `reason` at line `line` of `app.py`.
    fn gap_at(kind: GapKind, reason: &str, line: u32, key: Option<&str>) -> UsageGap {
        UsageGap {
            kind,
            sdk: Some(Sdk::Openai),
            file: "app.py".into(),
            span: Some(span(line)),
            symbol: None,
            chain: Some("client.chat.completions.create".into()),
            key: key.map(str::to_string),
            reason: reason.into(),
            detail: String::new(),
            values: Vec::new(),
            values_complete: false,
            args: None,
        }
    }

    #[test]
    fn a_call_through_an_unresolved_client_is_checked_with_its_arguments() {
        // #290: the arguments of a call whose client Arbor could not resolve
        // (a subclass of `OpenAI` exported under the same name) were
        // dropped, so `temperature`, `tools` and `max_tokens` went unchecked
        // and the decision read "let the model reason (no change)".
        let at = |args: Option<CallArgs>| UsageGap {
            args,
            values: vec![value("gpt-4.1-mini", ValueOrigin::Literal, 40)],
            ..gap_at(GapKind::UnknownClientFactory, "factory_call", 40, None)
        };
        let read = |kwargs: &[&str], splat: bool| {
            Some(CallArgs {
                kwargs: kwargs.iter().map(|k| k.to_string()).collect(),
                splat,
                spreads: Vec::new(),
            })
        };
        let finding = |gap: UsageGap| {
            let report = run(&[], &[gap], &options("2026-10-18"));
            assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
            report.findings[0].clone()
        };

        let sampled = finding(at(read(
            &["model", "temperature", "max_tokens", "messages"],
            false,
        )));
        assert_eq!(sampled.confidence, Confidence::Gap);
        assert_eq!(sampled.fix, FixKind::Manual, "a gap is never a swap");
        assert_eq!(
            change(&sampled, "max_tokens").and_then(|c| c.to).as_deref(),
            Some("max_completion_tokens")
        );
        assert_eq!(sampled.decisions[0].unchecked_args, None);
        assert_eq!(option(&sampled, "model_default").remove, ["temperature"]);
        assert_eq!(option(&sampled, "reasoning_none").keep, ["temperature"]);

        let tools = finding(at(read(&["model", "tools", "messages"], false)));
        assert_eq!(option(&tools, "model_default").remove, ["tools"]);

        // Arguments it cannot see: the decision says so, and a person checks.
        for (args, why) in [
            (read(&["model", "messages"], true), UncheckedArgs::Splat),
            (None, UncheckedArgs::NotRecorded),
        ] {
            let unseen = finding(at(args));
            assert_eq!(unseen.decisions[0].unchecked_args, Some(why));
            assert!(
                unseen
                    .review
                    .iter()
                    .any(|r| r.message == why.message(&[], Syntax::Python)),
                "{:#?}",
                unseen.review
            );
        }
    }

    #[test]
    fn a_decision_says_when_the_call_sends_arguments_arbor_did_not_read() {
        let mut splat = openai("gpt-4o-mini", &["messages"]);
        splat.splat = true;
        let extra_body = openai("gpt-4o-mini", &["messages", "extra_body"]);
        let plain = openai("gpt-4o-mini", &["messages"]);
        for (call, why) in [
            (splat, Some(UncheckedArgs::Splat)),
            (extra_body, Some(UncheckedArgs::ExtraBody)),
            (plain, None),
        ] {
            let finding = one(&[call]);
            assert_eq!(finding.decisions[0].unchecked_args, why, "{finding:#?}");
            match why {
                Some(why) => {
                    assert!(finding
                        .review
                        .iter()
                        .any(|r| r.message == why.message(&[], Syntax::Python)));
                    assert_eq!(finding.fix, FixKind::Manual);
                }
                // Every argument was read and none conflicts: "no change"
                // holds for letting the model reason.
                None => assert!(option(&finding, "model_default").remove.is_empty()),
            }
        }

        // Claude's thinking choice is marked the same way.
        let mut claude = call(
            Sdk::Anthropic,
            Routing::AnthropicDirect,
            "claude.messages.create",
            "claude-3-5-sonnet-20241022",
            &["max_tokens", "messages"],
            20,
        );
        claude.splat = true;
        let finding = one(&[claude]);
        assert_eq!(finding.decisions[0].choice, "ANT-THINKING-DEFAULT");
        assert_eq!(
            finding.decisions[0].unchecked_args,
            Some(UncheckedArgs::Splat)
        );

        // A model nested in `extra_body` has no decision; the nested check
        // already says to look at its parameters by hand.
        let mut nested = openai("gpt-4o", &["messages", "extra_body"]);
        nested.values[0].key = "extra_body.model".into();
        let finding = one(&[nested]);
        assert!(finding.decisions.is_empty());
        assert!(!finding
            .review
            .iter()
            .any(|r| r.message == UncheckedArgs::ExtraBody.message(&[], Syntax::Python)));
    }

    #[test]
    fn an_argument_a_check_line_flags_is_named_on_every_option() {
        // #291 round 3: "let the model reason (no change)" sat next to "Whether
        // gpt-6-luna accepts temperature ... on the Responses API is not
        // verified", and "let the model think (no change)" next to "Non-default
        // temperature ... return 400 on claude-sonnet-5-5".
        let flags = |finding: &Finding| -> Vec<(String, Vec<String>, Vec<String>)> {
            finding.decisions[0]
                .options
                .iter()
                .map(|o| (o.id.clone(), o.check.clone(), o.unverified.clone()))
                .collect()
        };
        let both = |a: (&str, Vec<String>, Vec<String>), b: (&str, Vec<String>, Vec<String>)| {
            vec![(a.0.to_string(), a.1, a.2), (b.0.to_string(), b.1, b.2)]
        };
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // Responses API: no live call went to /v1/responses, but OpenAI's
        // "Using GPT-6" guide documents the sampling rule there
        // (G6-SAMPLING-RESPONSES, `docs` since the 28 Oct data, #297), so
        // both options name temperature as a check.
        let responses = one(&[call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "client.responses.create",
            "gpt-4o-mini",
            &["temperature", "input"],
            10,
        )]);
        assert!(responses
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-SAMPLING-RESPONSES")));
        assert_eq!(
            flags(&responses),
            both(
                ("reasoning_none", names(&["temperature"]), vec![]),
                ("model_default", names(&["temperature"]), vec![]),
            )
        );

        // Claude: a non-default temperature returns 400 whichever way the
        // thinking choice goes.
        let claude = one(&[call(
            Sdk::Anthropic,
            Routing::AnthropicDirect,
            "claude.messages.create",
            "claude-3-5-sonnet-20241022",
            &["max_tokens", "temperature", "messages"],
            20,
        )]);
        assert_eq!(claude.decisions[0].choice, "ANT-THINKING-DEFAULT");
        assert_eq!(
            flags(&claude),
            both(
                ("thinking_between_tools", names(&["temperature"]), vec![]),
                ("model_default", names(&["temperature"]), vec![]),
            )
        );

        // Chat Completions `stop` returns 400 on gpt-6-luna (live).
        let stop = one(&[openai("gpt-4o-mini", &["stop", "messages"])]);
        assert_eq!(
            flags(&stop),
            both(
                ("reasoning_none", names(&["stop"]), vec![]),
                ("model_default", names(&["stop"]), vec![]),
            )
        );

        // An argument the choice settles is not flagged again: Chat
        // Completions temperature is kept or removed by the options.
        let sampled = one(&[openai("gpt-4o-mini", &["temperature", "messages"])]);
        assert!(flags(&sampled)
            .iter()
            .all(|(_, check, unverified)| check.is_empty() && unverified.is_empty()));
        // Claude's thinking=... is removed by both options: not flagged.
        let thinking = one(&[call(
            Sdk::Anthropic,
            Routing::AnthropicDirect,
            "claude.messages.create",
            "claude-3-5-sonnet-20241022",
            &["max_tokens", "thinking", "messages"],
            20,
        )]);
        assert!(flags(&thinking)
            .iter()
            .all(|(_, check, unverified)| check.is_empty() && unverified.is_empty()));
        // A call with nothing a check names keeps both lists empty.
        let plain = one(&[openai("gpt-4o-mini", &["messages"])]);
        assert!(flags(&plain)
            .iter()
            .all(|(_, check, unverified)| check.is_empty() && unverified.is_empty()));
    }

    #[test]
    fn an_argument_checked_only_with_reasoning_off_cannot_be_vouched_for_otherwise() {
        // #291 round 3 review: `seed=1` on gpt-4o read "let the model reason
        // (no change)" next to Arbor's own note that seed is untested with
        // the model reasoning (LIVE_CHECKS_2026-10-18.md:77, :284).
        let unverified = |finding: &Finding| -> Vec<(String, Vec<String>)> {
            finding.decisions[0]
                .options
                .iter()
                .map(|o| (o.id.clone(), o.unverified.clone()))
                .collect()
        };
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for arg in ["seed", "n"] {
            let finding = one(&[openai("gpt-4o", &[arg, "messages"])]);
            assert_eq!(
                unverified(&finding),
                [
                    ("reasoning_none".to_string(), Vec::new()),
                    ("model_default".to_string(), names(&[arg])),
                ],
                "{arg}"
            );
            assert!(
                finding.decisions[0]
                    .notes
                    .iter()
                    .any(|n| n.starts_with(&format!("{arg} was accepted on gpt-6-luna"))),
                "{:?}",
                finding.decisions[0].notes
            );
        }
        // With temperature too: the reasoning option removes temperature and
        // still cannot vouch for seed.
        let finding = one(&[openai("gpt-4o", &["seed", "temperature", "messages"])]);
        let model_default = &finding.decisions[0].options[1];
        assert_eq!(model_default.remove, names(&["temperature"]));
        assert_eq!(model_default.unverified, names(&["seed"]));
        assert!(finding.decisions[0].options[0].unverified.is_empty());
    }

    #[test]
    fn a_check_line_seen_on_another_api_says_where_it_was_seen() {
        // #291 round 3 review: on the Responses API "stop and logit_bias are
        // not supported on gpt-6-luna (400)" sat next to "could not check
        // logit_bias"; the live check called Chat Completions only.
        let message = |finding: &Finding, rule: &str| -> String {
            finding
                .review
                .iter()
                .find(|r| r.rule.as_deref() == Some(rule))
                .unwrap_or_else(|| panic!("no {rule}: {:#?}", finding.review))
                .message
                .clone()
        };
        let responses = one(&[call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "client.responses.create",
            "gpt-4o-mini",
            &["logit_bias", "input"],
            10,
        )]);
        assert_eq!(
            message(&responses, "G6-STOP-LOGIT-BIAS"),
            "stop and logit_bias are not supported on gpt-6-luna (400). Seen on Chat Completions only; the Responses API was not checked."
        );
        assert!(responses.decisions[0]
            .options
            .iter()
            .all(|o| o.unverified == ["logit_bias"]));
        // On the API the live check called, the line is as before.
        let chat = one(&[openai("gpt-4o-mini", &["logit_bias", "messages"])]);
        assert_eq!(
            message(&chat, "G6-STOP-LOGIT-BIAS"),
            "stop and logit_bias are not supported on gpt-6-luna (400)."
        );
    }

    #[test]
    fn a_spread_dict_is_checked_and_one_arbor_cannot_read_is_named() {
        let spread = |name: Option<&str>, keys: &[&str], complete: bool| crate::usages::Spread {
            name: name.map(str::to_string),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            complete,
        };
        let with = |spreads: Vec<crate::usages::Spread>| {
            let mut call = openai("gpt-4o-mini", &["messages"]);
            call.splat = true;
            call.spreads = spreads;
            one(&[call])
        };

        // `**PARAMS`, a same-file literal holding temperature: checked like
        // an argument written at the call, and nothing is left unseen.
        let read = with(vec![spread(
            Some("PARAMS"),
            &["model", "temperature"],
            true,
        )]);
        assert_eq!(read.decisions[0].unchecked_args, None);
        assert_eq!(option(&read, "model_default").remove, ["temperature"]);
        assert_eq!(option(&read, "reasoning_none").keep, ["temperature"]);
        assert!(read.review.is_empty(), "{:#?}", read.review);

        // Names it could not read are named, never "**kwargs" for all.
        let unread = with(vec![
            spread(Some("PARAMS"), &["temperature"], true),
            spread(Some("settings.llm"), &[], false),
            spread(Some("kwargs"), &[], false),
        ]);
        let decision = &unread.decisions[0];
        assert_eq!(decision.unchecked_args, Some(UncheckedArgs::Splat));
        assert_eq!(decision.unread_spreads, ["settings.llm", "kwargs"]);
        // What it could read still counts.
        assert_eq!(option(&unread, "model_default").remove, ["temperature"]);
        let message = "Some arguments come from `**settings.llm` and `**kwargs`, so Arbor could not check them all.";
        assert!(
            unread.review.iter().any(|r| r.message == message),
            "{:#?}",
            unread.review
        );
        assert_eq!(unread.fix, FixKind::Manual);

        // No name (a conditional), or a scan that recorded no spreads.
        for spreads in [vec![spread(None, &[], false)], Vec::new()] {
            let unnamed = with(spreads);
            assert_eq!(
                unnamed.decisions[0].unchecked_args,
                Some(UncheckedArgs::Splat)
            );
            assert!(unnamed.decisions[0].unread_spreads.is_empty());
            assert!(unnamed.review.iter().any(|r| r.message
                == "Some arguments are spread into the call with `**`, so Arbor could not check them all."));
        }
    }

    #[test]
    fn every_gap_at_a_call_keeps_it_from_reading_as_confirmed() {
        // A retired model at a direct call fails the check on its own...
        let call = openai("gpt-3.5-turbo-0613", &[]);
        let clean = run(std::slice::from_ref(&call), &[], &options("2026-10-18"));
        assert_eq!(clean.findings[0].confidence, Confidence::Confirmed);
        assert_eq!(clean.exit_code, 1);

        // ...and never with any gap at its call: every reason the usage scan
        // names, and a reason it may add later, of every kind.
        let mut cases: Vec<(GapKind, String)> = gaps::GAP_REASONS
            .iter()
            .map(|(kind, reason, _)| (*kind, reason.to_string()))
            .collect();
        for kind in [
            GapKind::UnresolvedModel,
            GapKind::AmbiguousClient,
            GapKind::UnknownClientFactory,
            GapKind::DynamicDispatch,
            GapKind::VendoredPath,
        ] {
            cases.push((kind, "a_reason_added_later".to_string()));
        }
        for (kind, reason) in &cases {
            let key = (*kind == GapKind::UnresolvedModel).then_some("model");
            let gap = gap_at(*kind, reason, 10, key);
            let report = run(
                std::slice::from_ref(&call),
                std::slice::from_ref(&gap),
                &options("2026-10-18"),
            );
            let finding = &report.findings[0];
            let label = format!("{kind}/{reason}");
            assert!(
                finding.confidence != Confidence::Confirmed
                    || finding.routing_class != RoutingClass::Direct,
                "{label} reads as confirmed"
            );
            assert!(!finding.fails, "{label} counts toward the exit code");
            assert_eq!(report.exit_code, 0, "{label}");
            assert_eq!(finding.fix, FixKind::Manual, "{label} is never a swap");
            if *kind == GapKind::UnresolvedModel {
                assert_eq!(finding.confidence, Confidence::ConfirmFirst, "{label}");
                assert_eq!(finding.routing_class, RoutingClass::Direct, "{label}");
                assert_eq!(finding.gap_reason.as_deref(), Some(reason.as_str()));
            } else {
                assert_eq!(finding.routing_class, RoutingClass::Unknown, "{label}");
                assert_eq!(finding.routing_gap.as_deref(), Some(reason.as_str()));
            }
            // Counted when asked.
            let mut opts = options("2026-10-18");
            opts.count_unconfirmed = true;
            let counted = run(
                std::slice::from_ref(&call),
                std::slice::from_ref(&gap),
                &opts,
            );
            assert_eq!(counted.exit_code, 1, "{label} with --count-unconfirmed");
        }
    }

    #[test]
    fn a_value_gap_holds_its_own_key_or_every_key() {
        let mut call = openai("gpt-3.5-turbo-0613", &["extra_body"]);
        let mut fallback = value("gpt-3.5-turbo-0613", ValueOrigin::Literal, 10);
        fallback.key = "extra_body.fallbacks.model".into();
        call.values.push(fallback);
        let confidence = |gaps: &[UsageGap]| -> Vec<(String, Confidence)> {
            run(std::slice::from_ref(&call), gaps, &options("2026-10-18"))
                .findings
                .iter()
                .map(|f| (f.key.clone(), f.confidence))
                .collect()
        };
        // A gap for the fallback leaves the model confirmed.
        let other = gap_at(
            GapKind::UnresolvedModel,
            "env_var",
            10,
            Some("extra_body.fallbacks.model"),
        );
        let found = confidence(std::slice::from_ref(&other));
        assert!(found.contains(&("model".into(), Confidence::Confirmed)));
        assert!(found.contains(&(
            "extra_body.fallbacks.model".into(),
            Confidence::ConfirmFirst
        )));
        // A gap that names no key (more nested values than are read) holds
        // every value at the call.
        let truncated = gap_at(GapKind::UnresolvedModel, "truncated", 10, None);
        assert!(confidence(&[truncated])
            .iter()
            .all(|(_, c)| *c == Confidence::ConfirmFirst));
    }

    #[test]
    fn unanalyzed_parts_and_untracked_methods_are_counted() {
        // A region the scan did not analyze has no callee; it is counted,
        // never matched to a call or listed as an unreadable model.
        let region = UsageGap {
            sdk: None,
            chain: None,
            ..gap_at(GapKind::UnresolvedModel, "truncated", 30, None)
        };
        let dispatch = gap_at(GapKind::DynamicDispatch, "getattr", 40, None);
        let report = run(&[], &[region, dispatch], &options("2026-10-18"));
        assert_eq!(report.summary.not_analyzed, 1);
        assert_eq!(report.summary.untracked_methods, 1);
        assert!(report.unreadable.is_empty());
        assert_eq!(report.exit_code, 0);
    }

    /// Python files that give the usage scan each gap the retirement check
    /// has to keep from reading as confirmed.
    const GAP_FIXTURES: &[(&str, &str)] = &[
        (
            "value_gaps.py",
            r#"import functools
import os

from openai import OpenAI

client = OpenAI()


class Bot:
    MODEL = "gpt-4o"

    def run(self):
        return client.chat.completions.create(model=self.MODEL, messages=[])


def ask(model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[])


@functools.lru_cache
def cached(model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[])


def _private(model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[])


_private("gpt-4o-mini")


def env_fallback():
    return client.chat.completions.create(model=os.getenv("MODEL", "gpt-4o"), messages=[])


def splatted(overrides):
    return client.chat.completions.create(**{"model": "gpt-4o", **overrides}, messages=[])


def no_model():
    return client.chat.completions.create(messages=[])
"#,
        ),
        (
            "dynamic_value.py",
            r#"from openai import OpenAI

MODEL = "gpt-4o"
client = OpenAI()
exec("MODEL = 'gpt-6-luna'")


def run():
    return client.chat.completions.create(model=MODEL, messages=[])


def literal():
    return client.chat.completions.create(model="gpt-4o", messages=[])
"#,
        ),
        (
            "module_config.py",
            r#"import openai


def legacy():
    return openai.ChatCompletion.create(engine="gpt-4o", messages=[])
"#,
        ),
        (
            "endpoint_env.py",
            r#"import os

from openai import OpenAI

os.environ["OPENAI_BASE_URL"] = "http://localhost:4000"
client = OpenAI()


def run():
    return client.chat.completions.create(model="gpt-4o", messages=[])
"#,
        ),
        (
            "base_url_env.py",
            r#"import os

from openai import OpenAI

client = OpenAI(base_url=os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1"))
other = OpenAI(base_url=os.environ.get("OPENAI_BASE_URL"))


def run():
    return client.chat.completions.create(model="gpt-4o", messages=[])


def run_env_only():
    return other.chat.completions.create(model="gpt-4o", messages=[])
"#,
        ),
        (
            "endpoint_option.py",
            r#"from google import genai

client = genai.Client(http_options={"base_url": "https://example.invalid"})


def run():
    return client.models.generate_content(model="gemini-2.0-flash", contents="hi")
"#,
        ),
        (
            "routing_gaps.py",
            r#"import random

from openai import AzureOpenAI, OpenAI


def make():
    return OpenAI()


plain = OpenAI()
either = OpenAI() if random.random() > 0.5 else AzureOpenAI()
moved = OpenAI()
moved.base_url = "http://localhost:4000"
unread = OpenAI(base_url=random.choice(["a", "b"]))
made = make()


def branch():
    return either.chat.completions.create(model="gpt-4o", messages=[])


def assigned():
    return moved.chat.completions.create(model="gpt-4o", messages=[])


def unresolved():
    return unread.chat.completions.create(model="gpt-4o", messages=[])


def factory():
    return made.chat.completions.create(model="gpt-4o", messages=[])


def dispatch():
    return getattr(plain.chat.completions, "create")(model="gpt-4o", messages=[])


def send(c=plain):
    return c.chat.completions.create(model="gpt-4o", messages=[])


def _helper(c):
    return c.chat.completions.create(model="gpt-4o", messages=[])


_helper(plain)
"#,
        ),
    ];

    /// Every `.py` file under `dir`, with its path relative to `root`.
    fn python_files(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, String)>,
    ) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                python_files(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "py") {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }

    #[test]
    fn no_gap_from_the_usage_scan_reads_as_confirmed() {
        // The fixtures above, a file with scopes nested past the scan's cap
        // and a call with more nested values than it reads, and every SDK
        // eval corpus.
        let mut files: Vec<(String, String)> = GAP_FIXTURES
            .iter()
            .map(|(name, source)| (name.to_string(), source.to_string()))
            .collect();
        let mut deep = r#"client.chat.completions.create(model="gpt-4o", messages=[])"#.to_string();
        for _ in 0..70 {
            deep = format!("(lambda: {deep})");
        }
        files.push((
            "truncated_scopes.py".into(),
            format!(
                "from openai import OpenAI\n\nclient = OpenAI()\n\ndeep = {deep}\n\n\ndef shallow():\n    return client.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
            ),
        ));
        let fallbacks = vec![r#"{"model": "gpt-4o"}"#; 40].join(", ");
        files.push((
            "truncated_values.py".into(),
            format!(
                "from openai import OpenAI\n\nclient = OpenAI()\n\n\ndef run():\n    return client.chat.completions.create(model=\"gpt-4o\", messages=[], extra_body={{\"fallbacks\": [{fallbacks}]}})\n"
            ),
        ));
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/corpus");
        let mut corpora = 0;
        for entry in std::fs::read_dir(&corpus).expect("the eval corpus is in the workspace") {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("py-sdk-"))
            {
                corpora += 1;
                python_files(&corpus, &path, &mut files);
            }
        }
        assert!(corpora >= 4, "the SDK eval corpora moved");

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::language())
            .unwrap();
        let (mut calls, mut gaps_found, mut clients) = (Vec::new(), Vec::new(), Vec::new());
        for (name, source) in &files {
            let tree = parser.parse(source, None).unwrap();
            let found = crate::usages::python::extract(&tree, source, name);
            calls.extend(found.calls);
            gaps_found.extend(found.gaps);
            clients.extend(found.clients);
        }

        // Every reason the scan gave is one this check names.
        let seen: BTreeSet<(GapKind, &str)> = gaps_found
            .iter()
            .map(|g| (g.kind, g.reason.as_str()))
            .collect();
        for (kind, reason) in &seen {
            assert!(
                gaps::gap_text(*kind, reason).is_some(),
                "the usage scan gave a {kind} gap with reason `{reason}`, which GAP_REASONS does not name"
            );
        }
        // The fixtures keep covering the gaps this check has to hold.
        let reasons: BTreeSet<&str> = seen.iter().map(|(_, r)| *r).collect();
        for needed in [
            "dynamic_feature",
            "subclass_override",
            "module_config",
            "endpoint_option",
            "endpoint_env",
            "base_url_env",
            "base_url_unresolved",
            "base_url_assigned",
            "multiple_assignments",
            "in_file_callers",
            "splat",
            "unseen_callers",
            "parameter_default",
            "parameter_overridden",
            "missing_model",
            "truncated",
            "env_var",
            "factory_call",
            "getattr",
        ] {
            assert!(reasons.contains(needed), "no fixture gives `{needed}`");
        }

        let opts = CheckOptions {
            today: Date::parse("2026-10-20").unwrap(),
            fail_within: Some(36_500),
            include_routed: true,
            count_unconfirmed: false,
        };
        let input = ScanInput {
            calls: &calls,
            gaps: &gaps_found,
            clients: &clients,
        };
        let report = check(input, bundled().unwrap(), &opts);
        assert!(report.findings.len() > 50, "{}", report.findings.len());
        let mut held = 0;
        for finding in report.findings.iter().chain(&report.routed) {
            let here: Vec<&UsageGap> = gaps_found
                .iter()
                .filter(|g| {
                    g.file == finding.file
                        && g.chain.is_some()
                        && g.span
                            .is_some_and(|s| s.line == finding.line && s.column == finding.column)
                })
                .collect();
            let value_gap = here.iter().any(|g| gaps::holds_value(g, &finding.key));
            let routing_gap = here.iter().any(|g| gaps::holds_routing(g));
            let at = format!("{}:{} {}", finding.file, finding.line, finding.model);
            if value_gap {
                assert_ne!(finding.confidence, Confidence::Confirmed, "{at}");
            }
            if routing_gap {
                assert_ne!(finding.routing_class, RoutingClass::Direct, "{at}");
            }
            if value_gap || routing_gap {
                held += 1;
                assert!(!finding.fails, "{at} counts toward the exit code");
                assert_ne!(finding.fix, FixKind::Swap, "{at}");
                assert_ne!(finding.fix, FixKind::Decide, "{at}");
            }
        }
        assert!(held >= 30, "only {held} findings had a gap at their call");
        assert_eq!(report.summary.not_analyzed, 1, "the nested scopes");
        assert!(report.summary.untracked_methods >= 1, "the getattr call");
    }

    /// TypeScript files that give the usage scan each gap the retirement
    /// check must hold, each calling a retired model.
    const TS_GAP_FIXTURES: &[(&str, &str)] = &[
        (
            "gaps/routes.ts",
            r#"import OpenAI from "openai";
import { GoogleGenAI } from "@google/genai";
process.env.OPENAI_BASE_URL = proxy;
const fromEnv = new OpenAI({ baseURL: process.env.GATEWAY ?? "https://api.openai.com/v1" });
const unread = new OpenAI({ baseURL: `${host}/v1` });
const moved = new OpenAI({ baseURL: "https://api.openai.com/v1" });
moved.baseURL = elsewhere;
let either = new OpenAI({ baseURL: "https://api.openai.com/v1" });
if (flag) either = makeClient();
const cloud = new GoogleGenAI({ vertexai: true, project: "p" });
export async function run(typed: OpenAI) {
  await fromEnv.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
  await unread.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
  await moved.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
  await either.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
  await typed.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
  await cloud.models.generateContent({ model: "gemini-1.5-flash", contents: "x" });
  await new OpenAI().chat.completions.create({ model: "gpt-4", messages: [] });
}
"#,
        ),
        (
            "gaps/values.ts",
            r#"import OpenAI from "openai";
import { MODEL } from "./config";
const client = new OpenAI();
export class Bot {
  model = "gpt-3.5-turbo";
  ask() {
    return client.chat.completions.create({ model: this.model, messages: [] });
  }
}
function overridden(model = "gpt-3.5-turbo") {
  return client.chat.completions.create({ model, messages: [] });
}
overridden("gpt-4");
export function exported(model = "gpt-3.5-turbo") {
  return client.chat.completions.create({ model, messages: [] });
}
function passed(model = "gpt-3.5-turbo") {
  return client.chat.completions.create({ model, messages: [] });
}
app.use(passed);
export function rest(options: object) {
  client.chat.completions.create({ model: "gpt-3.5-turbo", ...options, messages: [] });
  client.chat.completions.create({ messages: [] });
  client.chat.completions.create({ model: process.env.MODEL ?? "gpt-3.5-turbo", messages: [] });
  client.chat.completions.create({ model: MODEL, messages: [] });
  client.chat.completions[method]({ model: "gpt-3.5-turbo", messages: [] });
  return client.chat.completions.create;
}
export function helper(llm: OpenAI) {
  return llm.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
}
helper(client);
"#,
        ),
        (
            "gaps/dynamic.ts",
            r#"import OpenAI from "openai";
eval(code);
const MODEL = "gpt-3.5-turbo";
const client = new OpenAI();
client.chat.completions.create({ model: MODEL, messages: [] });
"#,
        ),
    ];

    /// The TypeScript and JavaScript files under `dir`, declaration files
    /// left out, keyed by their path under `root`.
    fn typescript_files(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, String)>,
    ) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                typescript_files(root, &path, out);
            } else if crate::usages::is_usage_extension(
                path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default(),
            ) && !name.ends_with(".py")
                && !crate::usages::is_declaration_file(&name)
            {
                out.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }

    #[test]
    fn no_gap_from_the_typescript_scan_reads_as_confirmed() {
        let mut files: Vec<(String, String)> = TS_GAP_FIXTURES
            .iter()
            .map(|(name, source)| (name.to_string(), source.to_string()))
            .collect();
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/corpus");
        let mut corpora = 0;
        for entry in std::fs::read_dir(&corpus).expect("the eval corpus is in the workspace") {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ts-sdk-"))
            {
                corpora += 1;
                typescript_files(&corpus, &path, &mut files);
            }
        }
        assert!(corpora >= 2, "the TypeScript SDK eval corpora moved");

        let (mut calls, mut gaps_found, mut clients) = (Vec::new(), Vec::new(), Vec::new());
        for (name, source) in &files {
            let mut parser = tree_sitter::Parser::new();
            let language =
                if name.ends_with(".ts") || name.ends_with(".mts") || name.ends_with(".cts") {
                    tree_sitter_typescript::language_typescript()
                } else {
                    tree_sitter_typescript::language_tsx()
                };
            parser.set_language(&language).unwrap();
            let tree = parser.parse(source, None).unwrap();
            let found = crate::usages::typescript::extract(&tree, source, name);
            calls.extend(found.calls);
            gaps_found.extend(found.gaps);
            clients.extend(found.clients);
        }

        let seen: BTreeSet<(GapKind, &str)> = gaps_found
            .iter()
            .map(|g| (g.kind, g.reason.as_str()))
            .collect();
        for (kind, reason) in &seen {
            assert!(
                gaps::gap_text(*kind, reason).is_some(),
                "the TypeScript scan gave a {kind} gap with reason `{reason}`, which GAP_REASONS does not name"
            );
        }
        let reasons: BTreeSet<&str> = seen.iter().map(|(_, r)| *r).collect();
        for needed in [
            "dynamic_feature",
            "subclass_override",
            "endpoint_option",
            "endpoint_env",
            "base_url_env",
            "base_url_unresolved",
            "base_url_assigned",
            "multiple_assignments",
            "in_file_callers",
            "annotation",
            "splat",
            "unseen_callers",
            "parameter_default",
            "parameter_overridden",
            "missing_model",
            "env_var",
            "imported_name",
            "factory_call",
            "computed_member",
            "method_reference",
        ] {
            assert!(
                reasons.contains(needed),
                "no TypeScript fixture gives `{needed}`"
            );
        }

        let opts = CheckOptions {
            today: Date::parse("2026-10-24").unwrap(),
            fail_within: Some(36_500),
            include_routed: true,
            count_unconfirmed: false,
        };
        let input = ScanInput {
            calls: &calls,
            gaps: &gaps_found,
            clients: &clients,
        };
        let report = check(input, bundled().unwrap(), &opts);
        assert!(report.findings.len() > 20, "{}", report.findings.len());
        let mut held = 0;
        for finding in report.findings.iter().chain(&report.routed) {
            let here: Vec<&UsageGap> = gaps_found
                .iter()
                .filter(|g| {
                    g.file == finding.file
                        && g.chain.is_some()
                        && g.span
                            .is_some_and(|s| s.line == finding.line && s.column == finding.column)
                })
                .collect();
            let value_gap = here.iter().any(|g| gaps::holds_value(g, &finding.key));
            let routing_gap = here.iter().any(|g| gaps::holds_routing(g));
            let at = format!("{}:{} {}", finding.file, finding.line, finding.model);
            if value_gap {
                assert_ne!(finding.confidence, Confidence::Confirmed, "{at}");
            }
            if routing_gap {
                assert_ne!(finding.routing_class, RoutingClass::Direct, "{at}");
            }
            if value_gap || routing_gap {
                held += 1;
                assert!(!finding.fails, "{at} counts toward the exit code");
                assert_ne!(finding.fix, FixKind::Swap, "{at}");
                assert_ne!(finding.fix, FixKind::Decide, "{at}");
            }
        }
        assert!(held >= 15, "only {held} findings had a gap at their call");
    }

    #[test]
    fn python_literals() {
        assert_eq!(python_literal(&serde_json::json!("none")), "\"none\"");
        assert_eq!(
            python_literal(&serde_json::json!({"type": "between_tools"})),
            "{\"type\": \"between_tools\"}"
        );
        assert_eq!(
            python_literal(&serde_json::json!([true, null, 2])),
            "[True, None, 2]"
        );
    }
}
