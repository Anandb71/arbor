//! Matching SDK calls against the retirement data.
//!
//! For every model value [`crate::usages`] captured at an SDK call, [`check`]
//! decides which provider schedule applies from the client's routing, looks
//! the value up (exactly; never a substring or a case-folded match), and
//! applies the parameter rules for the listed replacement to the call's
//! keyword-argument names. The result says, per call site:
//!
//! - when the model stops working (already retired, or N days left);
//! - what the provider lists as the replacement;
//! - which other arguments must change with it, and what needs a person;
//! - how sure the match is: a literal or constant ([`Confidence::Confirmed`]),
//!   an environment or parameter default the deployed value may override
//!   ([`Confidence::ConfirmFirst`]), or a call whose client Arbor could not
//!   resolve ([`Confidence::Gap`]);
//! - whether the call reaches the provider directly. A provider's dates do
//!   not necessarily apply through Azure, a proxy or another cloud, so those
//!   calls are kept apart in [`RetirementReport::routed`].

use super::{
    Action, DateKind, EntryStatus, ParamRule, RecipeInfo, RetirementData, Retirements, RuleEffect,
    Table, Verified,
};
use super::{Date, ModelEntry};
use crate::usages::{
    model_family, CapturedValue, ClientConstruction, GapKind, Routing, Sdk, SdkCall, UsageGap,
    ValueOrigin,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};

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
    /// Shut down: calls fail (or the provider says they will).
    Retired,
    /// Shuts down on `date`, `days_left` from today.
    Retiring,
    /// Probably shut down, not confirmed for this exact ID.
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
    /// A direct client (`OpenAI()`, `Anthropic()`, `genai.Client()`).
    Direct,
    /// Azure, a custom `base_url`, Bedrock or Vertex for Claude: the
    /// provider's dates may not apply.
    Routed,
    /// Arbor could not tell (a client from a factory, `**kwargs` passed to
    /// the constructor, a client assigned two ways).
    Unknown,
}

/// How sure the match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// A literal or a same-file constant.
    Confirmed,
    /// An environment/config fallback or a parameter default: the deployed
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
    /// Something needs a person: see `review`.
    Manual,
    /// Nothing fails (the provider routes the ID to a newer model).
    None,
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

/// The replacement is itself retiring or retired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainInfo {
    pub model: String,
    pub status: State,
    pub date: Option<Date>,
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
    /// Days from today to `date`; negative once it has passed.
    pub days_left: Option<i64>,
    pub replacement: Option<String>,
    pub replacement_text: Option<String>,
    pub replacement_status: Option<ChainInfo>,
    pub fix: FixKind,
    pub changes: Vec<Change>,
    pub review: Vec<ReviewItem>,
    pub notes: Vec<String>,
    /// The client's routing (`openai_direct`, `azure_openai`...), or
    /// `unresolved_client` for a gap.
    pub routing: String,
    pub routing_class: RoutingClass,
    /// `chat.completions`, `responses`, `messages`...
    pub endpoint: Option<&'static str>,
    pub confidence: Confidence,
    pub origin: Option<ValueOrigin>,
    /// The constant, parameter or environment lookup the value came from.
    pub value_from: Option<String>,
    /// Line of the string literal itself.
    pub value_line: u32,
    /// For a gap: why the client was not resolved (`factory_call`...).
    pub gap_reason: Option<String>,
    /// Arguments were spread into the call with `**`.
    pub splat: bool,
    /// Counts toward the exit code under the current options.
    pub in_window: bool,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataInfo {
    /// The newest date the provider pages were read.
    pub as_of: Date,
    pub source: String,
    pub recipes: Vec<RecipeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportOptions {
    pub fail_within_days: Option<i64>,
    pub include_routed: bool,
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
    /// Calls that count toward the exit code.
    pub in_window: usize,
    pub swap: usize,
    pub manual: usize,
    pub by_provider: BTreeMap<String, usize>,
    /// The soonest date among retiring calls.
    pub next_shutdown: Option<NextShutdown>,
    /// Calls through Azure, a proxy or another cloud that use a listed
    /// model (listed in `routed` only with `include_routed`).
    pub routed_calls: usize,
    /// Requests whose model could not be read.
    pub unreadable_calls: usize,
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

    for call in input.calls {
        let class = call_routing(call, input.clients);
        for value in &call.values {
            let Some(finding) = finding_for(
                data,
                options,
                Site::Call(call),
                &class,
                value,
                confidence_of(value.origin),
            ) else {
                continue;
            };
            if finding.routing_class == RoutingClass::Routed {
                routed.push(finding);
            } else {
                direct.push(finding);
            }
        }
    }
    for gap in input.gaps {
        let class = CallRouting {
            class: RoutingClass::Unknown,
            routing: "unresolved_client".to_string(),
            sdk: gap.sdk,
            table: None,
        };
        for value in &gap.values {
            let Some(finding) = finding_for(
                data,
                options,
                Site::Gap(gap),
                &class,
                value,
                Confidence::Gap,
            ) else {
                continue;
            };
            if finding.routing_class == RoutingClass::Routed {
                routed.push(finding);
            } else {
                direct.push(finding);
            }
        }
    }

    sort_findings(&mut direct);
    sort_findings(&mut routed);
    let routed_calls = routed.len();
    if !options.include_routed {
        routed.clear();
    }
    let unreadable = unreadable(input);

    let summary = summarize(&direct, &routed, routed_calls, unreadable.len());
    let exit_code = i32::from(direct.iter().chain(&routed).any(|f| f.in_window));

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
        },
        summary,
        findings: direct,
        routed,
        unreadable,
        exit_code,
    }
}

fn data_info(data: &RetirementData, as_of: Date) -> DataInfo {
    DataInfo {
        as_of,
        source: data.source.clone(),
        recipes: data
            .recipes
            .iter()
            .map(|r| RecipeRef {
                id: r.id.clone(),
                version: r.version.clone(),
                provider: r.provider.clone(),
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
// Routing
// ---------------------------------------------------------------------------

struct CallRouting {
    class: RoutingClass,
    routing: String,
    sdk: Option<Sdk>,
    /// The schedule a direct client follows.
    table: Option<Table>,
}

fn call_routing(call: &SdkCall, clients: &[ClientConstruction]) -> CallRouting {
    let (class, table) = match call.routing {
        Routing::OpenaiDirect => (RoutingClass::Direct, Some(Table::Openai)),
        Routing::AnthropicDirect => (RoutingClass::Direct, Some(Table::Anthropic)),
        Routing::GoogleGenai if vertex_client(call, clients) => {
            (RoutingClass::Direct, Some(Table::Vertex))
        }
        Routing::GoogleGenai | Routing::GoogleGenerativeaiLegacy => {
            (RoutingClass::Direct, Some(Table::GeminiApi))
        }
        Routing::AzureOpenai
        | Routing::CustomBaseUrl
        | Routing::AnthropicBedrock
        | Routing::AnthropicVertex => (RoutingClass::Routed, None),
        Routing::Unknown => (RoutingClass::Unknown, None),
    };
    CallRouting {
        class,
        routing: call.routing.as_str().to_string(),
        sdk: Some(call.sdk),
        table,
    }
}

/// A `genai.Client(vertexai=...)` construction in the same file.
fn vertex_client(call: &SdkCall, clients: &[ClientConstruction]) -> bool {
    let Some(line) = call.client_line else {
        return false;
    };
    clients.iter().any(|c| {
        c.file == call.file && c.span.line == line && c.kwargs.iter().any(|k| k == "vertexai")
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
        if let Some(fine) = data.fine_tune(table, base) {
            let recipe = data.recipe(&fine.recipe)?;
            return Some(Hit {
                matched: base.to_string(),
                match_kind: MatchKind::FineTuned,
                table,
                recipe,
                status: fine.status,
                date: Date::parse(&fine.date),
                replacement: None,
                replacement_text: fine
                    .replacement_base
                    .as_ref()
                    .map(|b| format!("retrain on {b} (the base model OpenAI lists)")),
                action: Action::ReportOnly,
                rule_set: None,
                entry: None,
                verified: Verified::Docs,
                notes: vec![format!(
                    "OpenAI retires fine-tunes of {base} with their base model's schedule."
                )],
            });
        }
        let entry = data.entry(table, base)?;
        let mut hit = entry_hit(data, entry, MatchKind::FineTuned)?;
        hit.replacement_text = entry
            .replacement
            .as_ref()
            .map(|b| format!("retrain on {b} (the replacement OpenAI lists for {base})"));
        hit.replacement = None;
        hit.action = Action::ReportOnly;
        hit.rule_set = None;
        hit.notes = vec![format!(
            "Fine-tuned models follow the schedule of their base model, {base}."
        )];
        return Some(hit);
    }
    None
}

/// Resolves a value for a call, deciding which schedule applies.
fn resolve<'d>(
    data: &'d Retirements,
    routing: &CallRouting,
    value: &str,
) -> Option<(Hit<'d>, RoutingClass)> {
    if let Some(table) = routing.table {
        // A direct client follows its provider's schedule only.
        return lookup(data, table, value).map(|hit| (hit, routing.class));
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
    let hit = lookup(data, table, value)?;
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
// Findings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Site<'a> {
    Call(&'a SdkCall),
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
    let today = options.today;
    let date_kind = hit.recipe.date_kind;
    let status = effective_state(hit.status, hit.date, date_kind, today);
    let days_left = hit.date.map(|d| today.days_until(d));

    let (file, span, symbol, chain, kwargs, splat, gap_reason, endpoint) = match site {
        Site::Call(call) => (
            call.file.clone(),
            Some(call.span),
            call.symbol.clone(),
            Some(call.chain.clone()),
            call.kwargs.clone(),
            call.splat,
            None,
            endpoint_of(&call.chain, &call.member),
        ),
        Site::Gap(gap) => {
            let chain = gap.chain.clone().unwrap_or_default();
            (
                gap.file.clone(),
                gap.span,
                gap.symbol.clone(),
                gap.chain.clone(),
                Vec::new(),
                false,
                Some(gap.reason.clone()),
                endpoint_of(&chain, &chain),
            )
        }
    };
    let span = span.unwrap_or(value.span);

    let mut changes = Vec::new();
    let mut review = Vec::new();
    let mut notes = hit.notes.clone();
    if let (Some(rule_set), Some(target)) = (hit.rule_set, hit.replacement.as_deref()) {
        let context = RuleContext {
            target,
            source_model: &hit.matched,
            source_family: hit.entry.and_then(|e| e.source_family.as_deref()),
            endpoint,
            routing: &routing.routing,
            kwargs: &kwargs,
        };
        apply_rules(
            data.rules(rule_set),
            &context,
            &mut changes,
            &mut review,
            &mut notes,
        );
    }

    let replacement_status = hit.replacement.as_ref().and_then(|r| {
        let next = data.entry(hit.table, r)?;
        let next_date = next.date.as_deref().and_then(Date::parse);
        let next_kind = data
            .recipe(&next.recipe)
            .map(|r| r.date_kind)
            .unwrap_or(DateKind::Hard);
        let state = effective_state(next.status, next_date, next_kind, today);
        matches!(state, State::Retired | State::Retiring | State::Unconfirmed).then(|| ChainInfo {
            model: r.clone(),
            status: state,
            date: next_date,
        })
    });

    let mut reasons = model_reasons(&hit, status, splat);
    reasons.append(&mut review);
    let review = dedupe_review(reasons);
    let fix = if status == State::AutoRouted {
        FixKind::None
    } else if review.is_empty()
        && hit.action == Action::Rewrite
        && hit.replacement.is_some()
        && hit.match_kind == MatchKind::Exact
    {
        FixKind::Swap
    } else {
        FixKind::Manual
    };

    let in_window = match status {
        State::Retired => true,
        State::Retiring => match (options.fail_within, days_left) {
            (Some(window), Some(days)) => days <= window,
            _ => false,
        },
        _ => false,
    };

    Some(Finding {
        file,
        line: span.line,
        column: span.column,
        end_line: span.end_line,
        symbol,
        chain,
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
        replacement_status,
        fix,
        changes,
        review,
        notes: dedupe(notes),
        routing: routing.routing.clone(),
        routing_class: class,
        endpoint,
        confidence,
        origin: (confidence != Confidence::Gap).then_some(value.origin),
        value_from: value.name.clone(),
        value_line: value.span.line,
        gap_reason,
        splat,
        in_window,
        recipe: hit.recipe.id.clone(),
        recipe_version: hit.recipe.version.clone(),
        verified: hit.verified,
        source_url: hit.recipe.source_urls.first().cloned(),
    })
}

/// The state on `today`: a hard date that has passed means retired; a
/// passed "earliest" date is only a possibility.
fn effective_state(status: EntryStatus, date: Option<Date>, kind: DateKind, today: Date) -> State {
    match status {
        EntryStatus::ShutDown => State::Retired,
        EntryStatus::Retiring => match date {
            Some(date) if date < today => match kind {
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

/// Which API a call uses, from its callee.
fn endpoint_of(chain: &str, member: &str) -> Option<&'static str> {
    let text = if member.contains('.') { member } else { chain };
    let has = |needle: &str| text.contains(needle);
    let endpoint = if has("chat.completions") || has("ChatCompletion") {
        "chat.completions"
    } else if has("responses.") {
        "responses"
    } else if has("assistants.") || has("threads.") {
        "assistants"
    } else if has("completions.") || has("Completion.") {
        "completions_legacy"
    } else if has("messages.") {
        "messages"
    } else if has("generate_content")
        || has("GenerativeModel")
        || has("chats.")
        || has("send_message")
    {
        "generate_content"
    } else if has("interactions.") {
        "interactions"
    } else if has("embeddings.") || has("embed_content") || has("Embedding.") {
        "embeddings"
    } else if has("images.") || has("generate_images") {
        "images"
    } else if has("audio.") {
        "audio"
    } else {
        return None;
    };
    Some(endpoint)
}

/// Why the model itself needs a person, before any parameter rule.
fn model_reasons(hit: &Hit<'_>, status: State, splat: bool) -> Vec<ReviewItem> {
    if status == State::AutoRouted {
        return Vec::new();
    }
    let provider = hit.table.provider();
    let fine_tuned = hit.match_kind == MatchKind::FineTuned
        || hit.entry.and_then(|e| e.kind.as_deref()) == Some("fine_tune_label");
    let category = hit.entry.map(|e| e.category.as_str()).unwrap_or("text");
    let mut messages = Vec::new();
    if fine_tuned {
        messages.push("Fine-tuned model: retrain it on a supported base model.".to_string());
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
    } else if hit.rule_set.is_none() {
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
    if splat {
        messages.push(
            "Some arguments come from `**kwargs`, so Arbor could not check them all.".to_string(),
        );
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

    fn message(&self, text: &str) -> String {
        text.replace("{target}", self.target)
    }
}

fn apply_rules<'r>(
    rules: impl Iterator<Item = &'r ParamRule>,
    context: &RuleContext<'_>,
    changes: &mut Vec<Change>,
    review: &mut Vec<ReviewItem>,
    notes: &mut Vec<String>,
) {
    let rules: Vec<&ParamRule> = rules.collect();
    let changes_args =
        |rule: &&&ParamRule| matches!(rule.effect, RuleEffect::Add | RuleEffect::Rename);

    // First the argument changes, so the other rules see what the swap adds.
    for rule in rules.iter().filter(changes_args) {
        if !context.applies(rule, &[]) {
            continue;
        }
        let because = context.because(rule);
        if let Some(rename) = rule.rename.as_ref().filter(|r| context.has(&r.from)) {
            merge_change(
                changes,
                Change {
                    kind: ChangeKind::Rename,
                    param: rename.from.clone(),
                    to: Some(rename.to.clone()),
                    value: None,
                    because: vec![rename.from.clone()],
                    rules: vec![rule.id.clone()],
                    verified: rule.verified,
                },
            );
        }
        apply_adds(rule, context, &because, changes, review);
    }
    // Renames read first: they are edits to arguments already in the call.
    changes.sort_by_key(|c| c.kind != ChangeKind::Rename);
    let added: Vec<String> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Add)
        .map(|c| c.param.clone())
        .collect();

    let mut keeps = Vec::new();
    for rule in rules.iter().filter(|r| !changes_args(r)) {
        if !context.applies(rule, &added) {
            continue;
        }
        let because = context.because(rule);
        match rule.effect {
            RuleEffect::Review => review.push(ReviewItem {
                rule: Some(rule.id.clone()),
                message: context.message(&rule.message),
                because,
                verified: Some(rule.verified),
            }),
            RuleEffect::Keep => keeps.push((*rule, because)),
            RuleEffect::Note => notes.push(context.message(&rule.message)),
            RuleEffect::Add | RuleEffect::Rename => {}
        }
    }
    // Keep rules depend on the reasoning effort the adds settled on.
    for (rule, because) in keeps {
        let on_target =
            rule.keep_targets.is_empty() || rule.keep_targets.iter().any(|t| t == context.target);
        if !on_target {
            let text = rule.else_message.as_deref().unwrap_or(&rule.message);
            review.push(ReviewItem {
                rule: Some(rule.id.clone()),
                message: context.message(text),
                because,
                verified: Some(rule.verified),
            });
        } else if rule.keep_needs_effort_none && !effort_none(changes) {
            review.push(ReviewItem {
                rule: Some(rule.id.clone()),
                message: format!(
                    "{} {} accepted on {} only at reasoning effort \"none\"; check the effort this call runs at.",
                    because.join(", "),
                    if because.len() == 1 { "is" } else { "are" },
                    context.target
                ),
                because,
                verified: Some(rule.verified),
            });
        } else {
            notes.push(context.message(&rule.message));
        }
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
        .find(|a| a.endpoint == "*" || Some(a.endpoint.as_str()) == context.endpoint);
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
            verified: Some(rule.verified),
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
                python_literal(&spec.value)
            ),
            because: because.to_vec(),
            verified: Some(rule.verified),
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
            verified: rule.verified,
        },
    );
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
        if change.verified == Verified::Live {
            existing.verified = Verified::Live;
        }
        return;
    }
    changes.push(change);
}

/// Whether the changes set reasoning effort to `none`.
fn effort_none(changes: &[Change]) -> bool {
    changes.iter().any(|c| {
        c.kind == ChangeKind::Add
            && match (c.param.as_str(), &c.value) {
                ("reasoning_effort", Some(serde_json::Value::String(v))) => v == "none",
                ("reasoning", Some(serde_json::Value::Object(map))) => {
                    map.get("effort").and_then(|v| v.as_str()) == Some("none")
                }
                _ => false,
            }
    })
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
/// parameter, another module), except where a fallback was captured.
fn unreadable(input: ScanInput<'_>) -> Vec<Unreadable> {
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
    for gap in input.gaps {
        if gap.kind != GapKind::UnresolvedModel || gap.reason == "truncated" {
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
    out
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
            FixKind::Manual => summary.manual += 1,
            FixKind::None => {}
        }
        *summary
            .by_provider
            .entry(finding.provider.as_str().to_string())
            .or_default() += 1;
    }
    summary.in_window = findings
        .iter()
        .chain(routed)
        .filter(|f| f.in_window)
        .count();
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
    use crate::usages::{Receiver, Span};

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

    fn options(today: &str) -> CheckOptions {
        CheckOptions {
            today: Date::parse(today).unwrap(),
            fail_within: None,
            include_routed: false,
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

    #[test]
    fn retiring_model_counts_days_from_a_fixed_clock() {
        let finding = one(&[openai("gpt-4o", &["messages"])]);
        assert_eq!(finding.status, State::Retiring);
        assert_eq!(finding.date.unwrap().iso(), "2027-01-14");
        assert_eq!(finding.days_left, Some(88));
        assert_eq!(finding.replacement.as_deref(), Some("gpt-6-luna"));
        assert_eq!(finding.routing_class, RoutingClass::Direct);
        assert_eq!(finding.confidence, Confidence::Confirmed);
        assert_eq!(finding.fix, FixKind::Swap);
        assert_eq!(finding.endpoint, Some("chat.completions"));
        assert!(!finding.in_window, "retiring is not failing by default");
        assert!(finding.notes.iter().any(|n| n.contains("efficient tier")));

        // The same call once the date has passed.
        let report = run(&[openai("gpt-4o", &[])], &[], &options("2027-01-15"));
        let later = &report.findings[0];
        assert_eq!(later.status, State::Retired);
        assert_eq!(later.days_left, Some(-1));
        assert!(later.in_window);
        assert_eq!(report.exit_code, 1);

        // On the day itself it is still retiring, with 0 days left.
        let report = run(&[openai("gpt-4o", &[])], &[], &options("2027-01-14"));
        assert_eq!(report.findings[0].status, State::Retiring);
        assert_eq!(report.findings[0].days_left, Some(0));
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
        assert!(finding.in_window);
        assert_eq!(report.exit_code, 1);
        assert_eq!(report.summary.retired, 1);
        // Sonnet 5.5 thinks by default: without tools, between_tools is added.
        let thinking = change(finding, "thinking").expect("thinking added");
        assert_eq!(
            python_literal(thinking.value.as_ref().unwrap()),
            "{\"type\": \"between_tools\"}"
        );
    }

    #[test]
    fn co_changes_follow_the_call_arguments() {
        let finding = one(&[openai(
            "gpt-4o-mini",
            &["messages", "max_tokens", "temperature", "tools"],
        )]);
        let rename = change(&finding, "max_tokens").expect("rename");
        assert_eq!(rename.kind, ChangeKind::Rename);
        assert_eq!(rename.to.as_deref(), Some("max_completion_tokens"));
        assert_eq!(rename.verified, Verified::Live);
        let effort = change(&finding, "reasoning_effort").expect("effort none");
        assert_eq!(effort.value, Some(serde_json::json!("none")));
        // One change, every reason that needs it.
        for reason in ["max_tokens", "temperature", "tools"] {
            assert!(
                effort.because.iter().any(|b| b == reason),
                "{reason}: {effort:?}"
            );
        }
        assert_eq!(
            finding
                .changes
                .iter()
                .filter(|c| c.param == "reasoning_effort")
                .count(),
            1
        );
        assert_eq!(finding.fix, FixKind::Swap);
        // With effort none set, the reasoning-default note does not apply.
        assert!(!finding.notes.iter().any(|n| n.contains("medium effort")));
    }

    #[test]
    fn unsupported_parameters_need_a_person() {
        let finding = one(&[openai("gpt-4o", &["messages", "stop", "logit_bias"])]);
        assert_eq!(finding.fix, FixKind::Manual);
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-STOP-LOGIT-BIAS")
                && r.message.contains("gpt-6-luna")));

        // Penalties failed live on gpt-6-luna; n and seed need effort none.
        let finding = one(&[openai("gpt-4o", &["presence_penalty", "seed"])]);
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-PENALTIES")));
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-N-SEED")));
        let finding = one(&[openai("gpt-4o", &["seed", "temperature"])]);
        assert!(!finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("G6-N-SEED")));
        assert!(finding.notes.iter().any(|n| n.contains("n and seed")));
    }

    #[test]
    fn reasoning_sources_and_endpoints_pick_the_right_rules() {
        // o3-mini is a reasoning model: sampling parameters mean manual review.
        let finding = one(&[openai("o3-mini", &["temperature"])]);
        assert!(finding
            .review
            .iter()
            .any(|r| r.rule.as_deref() == Some("OAI-SAMPLING-REASONING-SOURCE")));
        assert!(change(&finding, "reasoning_effort").is_none());

        // The Responses API takes reasoning={"effort": "none"}.
        let responses = call(
            Sdk::Openai,
            Routing::OpenaiDirect,
            "client.responses.create",
            "gpt-3.5-turbo",
            &["input", "temperature"],
            12,
        );
        let finding = one(&[responses]);
        let reasoning = change(&finding, "reasoning").expect("reasoning added");
        assert_eq!(reasoning.value, Some(serde_json::json!({"effort": "none"})));

        // A reasoning_effort the call already sets cannot be read: check it.
        let finding = one(&[openai("gpt-4", &["tools", "reasoning_effort"])]);
        assert!(change(&finding, "reasoning_effort").is_none());
        assert!(finding
            .review
            .iter()
            .any(|r| r.message.contains("already sets reasoning_effort")));

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
    fn env_defaults_and_gaps_carry_their_confidence() {
        let mut env = openai("gpt-4.1", &[]);
        env.values[0].origin = ValueOrigin::EnvDefault;
        env.values[0].name = Some("os.getenv(\"MODEL\")".into());
        let finding = one(&[env]);
        assert_eq!(finding.confidence, Confidence::ConfirmFirst);
        assert_eq!(finding.origin, Some(ValueOrigin::EnvDefault));

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
        };
        let report = run(&[], &[gap], &options("2026-10-18"));
        let finding = &report.findings[0];
        assert_eq!(finding.confidence, Confidence::Gap);
        assert_eq!(finding.routing_class, RoutingClass::Unknown);
        assert_eq!(finding.gap_reason.as_deref(), Some("factory_call"));
        assert_eq!(finding.origin, None);
        assert_eq!(finding.replacement.as_deref(), Some("gpt-6-luna"));
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
        };
        // Line 10 has a captured fallback; line 50 has nothing.
        let report = run(&[env], &[gap(10), gap(50), gap(50)], &options("2026-10-18"));
        assert_eq!(report.unreadable.len(), 1);
        assert_eq!(report.unreadable[0].line, 50);
        assert_eq!(report.summary.unreadable_calls, 1);
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
    fn fine_tunes_and_resource_prefixes() {
        let finding = one(&[openai("ft:gpt-4o-2024-08-06:acme::abc123", &[])]);
        assert_eq!(finding.match_kind, MatchKind::FineTuned);
        assert_eq!(finding.matched, "gpt-4o-2024-08-06");
        assert_eq!(finding.replacement, None);
        assert_eq!(finding.fix, FixKind::Manual);
        assert!(finding.review[0].message.starts_with("Fine-tuned model"));

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

    #[test]
    fn vertex_clients_follow_the_cloud_schedule() {
        let gemini = call(
            Sdk::Google,
            Routing::GoogleGenai,
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
            routing: Routing::GoogleGenai,
            is_async: false,
            kwargs: vec!["vertexai".into(), "project".into()],
            splat: false,
            file: "app.py".into(),
            span: span(1),
            symbol: None,
        };
        // The Gemini API lists no date for gemini-2.5-flash; Vertex retires it.
        let report = run(std::slice::from_ref(&gemini), &[], &options("2026-10-18"));
        assert!(report.findings.is_empty());
        let input = ScanInput {
            calls: std::slice::from_ref(&gemini),
            gaps: &[],
            clients: std::slice::from_ref(&vertex),
        };
        let report = check(input, bundled().unwrap(), &options("2026-10-18"));
        let finding = &report.findings[0];
        assert_eq!(finding.provider, Table::Vertex);
        assert_eq!(finding.date.unwrap().iso(), "2026-10-20");
        assert_eq!(finding.date_kind, DateKind::Earliest);
        assert_eq!(finding.days_left, Some(2));
    }

    #[test]
    fn chains_and_summary() {
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
        let past = &report.findings[0];
        assert_eq!(past.fix, FixKind::Manual);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["findings"][1]["status"], "retiring");
        assert_eq!(json["findings"][1]["date"], "2027-01-14");
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
