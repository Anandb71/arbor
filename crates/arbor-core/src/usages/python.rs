//! Python SDK usage extraction.
//!
//! Two passes over the tree-sitter tree:
//!
//! 1. **Collect.** One iterative walk records scopes (module, class,
//!    function, lambda), imports, every assignment target (names, `self.x`,
//!    class attributes, `with ... as`, walrus, `global`), parameters, every
//!    call, every attribute chain used as a value, and where each binding
//!    takes effect. Parent kinds and field names travel down the walk, so no
//!    node is asked for its parent: tree-sitter answers that by descending
//!    from the root, which made deep trees quadratic.
//! 2. **Analyze.** Each call's callee is evaluated against those bindings.
//!    A chain rooted at a tracked SDK binding becomes a construction or a
//!    call; an SDK-shaped chain on an unknown receiver, `getattr` on a
//!    tracked binding and an unresolved model value become typed gaps.
//!
//! Names resolve the way Python scopes them, approximately: function and
//! lambda locals, parameters and imports shadow enclosing functions, class
//! bodies are skipped from inside methods, comprehensions have their own
//! scope, and `global` re-homes an assignment to the module. A use sees its
//! reaching definitions: the nearest definition that dominates it in
//! straight-line code, every definition between that one and the use (in a
//! branch), later ones in a loop that runs the use again without passing
//! the dominating one, and `global` assignments made by functions. Class
//! bodies and list, set and dict comprehensions run where they are written,
//! so their uses are placed there. A use from a function, lambda or
//! generator sees what reaches its definition plus every later definition,
//! since it may run after any of them. With no dominating definition, or
//! past the search limits, every definition of the name counts.
//!
//! An untyped parameter used as a client resolves when every call to its
//! function (or to a lambda assigned to a plain name) in this file passes
//! the same tracked client. Other modules may call a public function too, so
//! there the routing keeps an `ambiguous_client` gap (`in_file_callers`);
//! a nested or `_private` function, or any function in a script, is called
//! only from its file. A client default is combined with what the callers
//! pass, and with itself for a public function. A model parameter default
//! that an in-file caller overrides keeps a `parameter_overridden` gap
//! listing the values the callers pass.

use super::{
    is_capturable_value, is_credential_value, is_identifier_key, is_secret_key, is_value_key,
    model_family, BindingKind, CapturedValue, ClientConstruction, ClientScope, FileUsages, GapKind,
    Receiver, Routing, Sdk, SdkCall, SdkImport, Span, UsageGap, ValueOrigin,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use tree_sitter::{Node, Tree};

/// A file mentioning none of these cannot report anything, so it is neither
/// parsed nor walked.
const PREFILTER: &[&str] = &[
    "openai",
    "anthropic",
    "genai",
    "generativeai",
    "Completion",
    "completions.",
    "responses.",
    "messages.",
    "embeddings.",
    "generate_content",
    "embed_content",
];

/// Whether `source` mentions anything the extractor tracks. A file that does
/// not reports nothing, so callers may skip parsing it.
pub fn may_use_sdk(source: &str) -> bool {
    PREFILTER.iter().any(|needle| source.contains(needle))
}

/// Last member names that send a request to the provider (resource verbs and
/// the Gemini/PaLM call names). A call ending in one is an [`SdkCall::request`];
/// storing or passing one as a value hides the eventual request.
const REQUEST_METHODS: &[&str] = &[
    "create",
    "acreate",
    "stream",
    "parse",
    "retrieve",
    "list",
    "delete",
    "update",
    "cancel",
    "upload",
    "download",
    "content",
    "completion",
    "completion_stream",
    "generate",
    "edit",
    "create_variation",
    "connect",
    "generate_content",
    "generate_content_async",
    "generate_content_stream",
    "embed_content",
    "embed_content_async",
    "batch_embed_contents",
    "count_tokens",
    "count_tokens_async",
    "count_text_tokens",
    "compute_tokens",
    "send_message",
    "send_message_async",
    "send_message_stream",
    "generate_text",
    "generate_embeddings",
    "generate_message",
    "generate_images",
    "generate_videos",
    "chat",
    "upload_file",
    "get_file",
    "list_files",
    "delete_file",
    "list_models",
    "get_model",
];

/// Whether a call's member path ends in a request method.
fn is_request(member: &str) -> bool {
    let last = member.rsplit('.').next().unwrap_or(member);
    REQUEST_METHODS.contains(&last)
}

/// Members that take a model. Like a request, an unresolved model value here
/// is an `unresolved_model` gap, and so is `**kwargs` with no explicit model
/// key.
const MODEL_BEARING: &[&str] = &[
    "chat.completions.create",
    "chat.completions.parse",
    "chat.completions.stream",
    "completions.create",
    "responses.create",
    "responses.parse",
    "responses.stream",
    "embeddings.create",
    "images.generate",
    "images.edit",
    "images.create_variation",
    "audio.transcriptions.create",
    "audio.translations.create",
    "audio.speech.create",
    "moderations.create",
    "fine_tuning.jobs.create",
    "assistants.create",
    "runs.create",
    "realtime.connect",
    "messages.create",
    "messages.stream",
    "messages.count_tokens",
    "models.generate_content",
    "models.generate_content_stream",
    "models.generate_images",
    "models.generate_videos",
    "embed_content",
    "generate_embeddings",
    "generate_text",
    "count_text_tokens",
    "chats.create",
    "ChatCompletion.create",
    "ChatCompletion.acreate",
    "Completion.create",
    "Completion.acreate",
    "Embedding.create",
    "GenerativeModel",
];

/// Members that wrap a resource without changing the request
/// (`client.chat.completions.with_raw_response.create`).
const RESPONSE_WRAPPERS: &[&str] = &["with_raw_response", "with_streaming_response"];

/// Legacy `google.generativeai` functions whose first positional argument is
/// the model (`genai.embed_content("models/embedding-001", text)`).
const LEGACY_POSITIONAL_MODEL: &[&str] = &[
    "embed_content",
    "embed_content_async",
    "generate_embeddings",
    "count_text_tokens",
];

/// Calls whose `default=` (or second positional argument) is the fallback of
/// an environment lookup: python-decouple and django-environ.
const CONFIG_LOOKUPS: &[&str] = &["config", "decouple.config", "env", "env.str"];

/// Maximum expression depth within one evaluation; deeper chains are
/// treated as unresolved. Every name resolution starts a fresh budget.
const MAX_DEPTH: u32 = 64;
/// Maximum nesting of name, attribute and caller resolutions. A value cut
/// off here is memoized with the budget it had (see [`Memo`]), so a long
/// chain of names that each use the previous one more than once stays
/// linear instead of doubling per link.
const MAX_NAME_DEPTH: u32 = 64;
/// Inheritance search bounds: base-class hops, classes visited per lookup and
/// bases read per class. Real hierarchies are far smaller; a pathological
/// file (every class listing every class as a base) stays linear.
const MAX_BASE_DEPTH: u32 = 8;
const MAX_CLASSES_VISITED: usize = 64;
const MAX_BASES: usize = 32;
/// Definitions a reaching-definition search may look at before it gives up
/// and counts every definition of the name.
const MAX_REACH_SCAN: usize = 64;
/// Longest string literal read, in source bytes. A captured value is at
/// most 256 bytes, so a longer literal is refused without being copied.
const MAX_LITERAL_BYTES: usize = 4096;
/// Most arguments an environment, config or dict lookup takes. A call with
/// more is not a lookup and its arguments are not scanned.
const MAX_LOOKUP_ARGS: usize = 8;
/// Container entries a search for nested model keys reads: per call across
/// all of its literal arguments (`fallbacks=[{"model": ...}, ...]`), and
/// per dict literal for what its `**` expansion nests (read once, reused by
/// every call that expands it). The rest of a huge literal is not read.
const MAX_NESTED_ENTRIES: usize = 256;
/// Nested model values listed per call. Past it, and past
/// [`MAX_FILE_NESTED_VALUES`], the call gets a `truncated` gap instead.
const MAX_NESTED_VALUES: usize = 32;
/// Nested model values listed per file, so a dict expanded at thousands of
/// calls cannot multiply the report.
const MAX_FILE_NESTED_VALUES: usize = 4096;
/// Candidate values listed with one gap, and per file.
const MAX_CANDIDATES: usize = 16;
const MAX_FILE_CANDIDATES: usize = 4096;
/// In-file calls read for the values a parameter is given.
const MAX_CALLERS_READ: usize = 256;
/// Definitions, or dict entries, read for the values a name or a lookup
/// with a run-time key may take.
const MAX_CANDIDATE_READS: usize = 64;
/// Longest name, chain or expression shape reported, in bytes: a longer one
/// keeps its start and end around `...`. A name defined once and reported
/// at every use (a function's symbol, a constant behind an alias, a client
/// bound to a long attribute chain) can then not multiply the report.
const MAX_TEXT: usize = 200;
/// Longest member path followed off a tracked binding. SDK paths are a few
/// segments (`beta.threads.runs.steps.list`).
const MAX_PATH: usize = 32;
/// Calls, gaps and clients reported per file. Past it, the rest of the file
/// is one `truncated` gap: the report stays proportional to real code
/// however a generated file repeats itself.
const MAX_FILE_RECORDS: usize = 20_000;

/// Extracts SDK usages from a parsed Python file.
pub fn extract(tree: &Tree, source: &str, file_path: &str) -> FileUsages {
    if !may_use_sdk(source) {
        return FileUsages::new(file_path, "python");
    }
    let mut cx = Ctx::new(source, file_path);
    cx.collect(tree.root_node());
    cx.index_definitions();
    cx.settle_dict_escapes();
    cx.analyze();
    let mut found = cx.finish();
    found.syntax_errors = tree.root_node().has_error();
    found
}

// ============================================================================
// SDK tables
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Pkg {
    Openai,
    Anthropic,
    GoogleGenai,
    GoogleGenerativeai,
}

impl Pkg {
    fn sdk(self) -> Sdk {
        match self {
            Pkg::Openai => Sdk::Openai,
            Pkg::Anthropic => Sdk::Anthropic,
            Pkg::GoogleGenai | Pkg::GoogleGenerativeai => Sdk::Google,
        }
    }

    fn package(self) -> &'static str {
        match self {
            Pkg::Openai => "openai",
            Pkg::Anthropic => "anthropic",
            Pkg::GoogleGenai => "google.genai",
            Pkg::GoogleGenerativeai => "google.generativeai",
        }
    }

    /// The package a dotted module path belongs to, and the rest of the path.
    fn of_module(module: &str) -> Option<(Pkg, Vec<String>)> {
        let parts: Vec<&str> = module.split('.').map(str::trim).collect();
        let (pkg, used) = match parts.as_slice() {
            ["openai", ..] => (Pkg::Openai, 1),
            ["anthropic", ..] => (Pkg::Anthropic, 1),
            ["google", "genai", ..] => (Pkg::GoogleGenai, 2),
            ["google", "generativeai", ..] => (Pkg::GoogleGenerativeai, 2),
            _ => return None,
        };
        Some((pkg, parts[used..].iter().map(|s| s.to_string()).collect()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    OpenaiDirect,
    Azure,
    AnthropicDirect,
    Bedrock,
    Vertex,
    Genai,
    LegacyModel,
    /// A hosted-platform client this version does not classify.
    Platform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClientClass {
    pkg: Pkg,
    name: &'static str,
    family: Family,
}

/// Client constructors, by package. Platform clients route through a cloud
/// vendor this version does not classify (see [`Routing::Unknown`]).
const CLASSES: &[ClientClass] = &[
    class(Pkg::Openai, "OpenAI", Family::OpenaiDirect),
    class(Pkg::Openai, "AsyncOpenAI", Family::OpenaiDirect),
    class(Pkg::Openai, "Client", Family::OpenaiDirect),
    class(Pkg::Openai, "AsyncClient", Family::OpenaiDirect),
    class(Pkg::Openai, "AzureOpenAI", Family::Azure),
    class(Pkg::Openai, "AsyncAzureOpenAI", Family::Azure),
    class(Pkg::Anthropic, "Anthropic", Family::AnthropicDirect),
    class(Pkg::Anthropic, "AsyncAnthropic", Family::AnthropicDirect),
    class(Pkg::Anthropic, "Client", Family::AnthropicDirect),
    class(Pkg::Anthropic, "AsyncClient", Family::AnthropicDirect),
    class(Pkg::Openai, "BedrockOpenAI", Family::Platform),
    class(Pkg::Openai, "AsyncBedrockOpenAI", Family::Platform),
    class(Pkg::Anthropic, "AnthropicAWS", Family::Platform),
    class(Pkg::Anthropic, "AsyncAnthropicAWS", Family::Platform),
    class(Pkg::Anthropic, "AnthropicFoundry", Family::Platform),
    class(Pkg::Anthropic, "AsyncAnthropicFoundry", Family::Platform),
    class(Pkg::Anthropic, "AnthropicGoogleCloud", Family::Platform),
    class(
        Pkg::Anthropic,
        "AsyncAnthropicGoogleCloud",
        Family::Platform,
    ),
    class(Pkg::Anthropic, "AnthropicBedrock", Family::Bedrock),
    class(Pkg::Anthropic, "AsyncAnthropicBedrock", Family::Bedrock),
    class(Pkg::Anthropic, "AnthropicVertex", Family::Vertex),
    class(Pkg::Anthropic, "AsyncAnthropicVertex", Family::Vertex),
    class(Pkg::GoogleGenai, "Client", Family::Genai),
    class(
        Pkg::GoogleGenerativeai,
        "GenerativeModel",
        Family::LegacyModel,
    ),
];

const fn class(pkg: Pkg, name: &'static str, family: Family) -> ClientClass {
    ClientClass { pkg, name, family }
}

fn client_class(pkg: Pkg, name: &str) -> Option<ClientClass> {
    CLASSES
        .iter()
        .copied()
        .find(|class| class.pkg == pkg && class.name == name)
}

impl ClientClass {
    fn is_async(self) -> bool {
        self.name.starts_with("Async")
    }

    /// Routing from the constructor and its keyword names.
    fn routing(self, kwargs: &[String], splat: bool) -> Routing {
        let custom = kwargs.iter().any(|k| is_base_url_key(k));
        // `OpenAI(provider=bedrock(...))` hands requests to a platform adapter.
        let adapter = kwargs.iter().any(|k| k == "provider");
        match self.family {
            Family::OpenaiDirect | Family::AnthropicDirect if adapter => Routing::Unknown,
            Family::OpenaiDirect | Family::AnthropicDirect if custom => Routing::CustomBaseUrl,
            Family::OpenaiDirect | Family::AnthropicDirect if splat => Routing::Unknown,
            Family::OpenaiDirect => Routing::OpenaiDirect,
            Family::AnthropicDirect => Routing::AnthropicDirect,
            Family::Azure => Routing::AzureOpenai,
            Family::Bedrock => Routing::AnthropicBedrock,
            Family::Vertex => Routing::AnthropicVertex,
            Family::Genai => Routing::GoogleGenai,
            Family::LegacyModel => Routing::GoogleGenerativeaiLegacy,
            Family::Platform => Routing::Unknown,
        }
    }

    /// Routing known from a type annotation alone: a direct client may still
    /// have been given a `base_url`, so it is `unknown`.
    fn typed_routing(self) -> Routing {
        match self.family {
            Family::OpenaiDirect | Family::AnthropicDirect => Routing::Unknown,
            _ => self.routing(&[], false),
        }
    }
}

fn is_base_url_key(key: &str) -> bool {
    key == "base_url" || key == "api_base"
}

/// The provider an SDK-shaped chain on an unknown receiver most likely
/// belongs to, and whether the shape alone is distinctive enough to report.
fn sdk_shape(chain: &str) -> Option<(Sdk, bool)> {
    const GOOGLE: &[&str] = &[
        ".models.generate_content",
        ".models.generate_content_stream",
        ".models.embed_content",
        ".generate_content",
        ".generate_content_async",
        ".generate_content_stream",
    ];
    const OPENAI: &[&str] = &[
        ".chat.completions.create",
        ".chat.completions.parse",
        ".chat.completions.stream",
        ".responses.create",
        ".responses.parse",
        ".responses.stream",
        ".completions.create",
        ".embeddings.create",
        ".images.generate",
        ".images.edit",
        ".images.create_variation",
        ".audio.transcriptions.create",
        ".audio.translations.create",
        ".audio.speech.create",
        ".realtime.connect",
        "ChatCompletion.create",
        "ChatCompletion.acreate",
        "Completion.create",
        "Completion.acreate",
    ];
    // `messages.create` is also Twilio's and Slack's shape, so it counts only
    // where the file imports the Anthropic SDK.
    const ANTHROPIC: &[&str] = &[
        ".messages.create",
        ".messages.stream",
        ".messages.count_tokens",
    ];
    let chain = strip_wrappers(chain);
    if GOOGLE.iter().any(|s| chain.ends_with(s)) {
        Some((Sdk::Google, true))
    } else if OPENAI.iter().any(|s| chain.ends_with(s)) {
        Some((Sdk::Openai, true))
    } else if ANTHROPIC.iter().any(|s| chain.ends_with(s)) {
        Some((Sdk::Anthropic, false))
    } else {
        None
    }
}

/// A dotted path without the response wrappers.
fn strip_wrappers(path: &str) -> String {
    path.split('.')
        .filter(|segment| !RESPONSE_WRAPPERS.contains(segment))
        .collect::<Vec<_>>()
        .join(".")
}

// ============================================================================
// Values
// ============================================================================

/// A resolved client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Client {
    pkg: Pkg,
    routing: Routing,
    /// Line of the same-file construction, when there is exactly one.
    line: Option<u32>,
    /// The construction call (its node id), when there is exactly one.
    site: Option<usize>,
    /// Assigned more than one kind of client, or a client and something else.
    ambiguous: bool,
    /// Known only from a type annotation, not from a construction.
    typed: bool,
    /// Why the routing may not hold at a call, reported there as an
    /// `ambiguous_client` gap.
    caveat: Option<ClientCaveat>,
}

/// Why a resolved client's routing may not be the one a request uses.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientCaveat {
    /// Every in-file caller of a function passes this client for
    /// parameter `param`, but other modules may call it with another.
    InFileCallers { param: String },
    /// `client.base_url = ...` after the client is built.
    BaseUrlAssigned,
    /// The file sets an environment variable the SDK reads its endpoint
    /// from (`os.environ["OPENAI_BASE_URL"] = ...`).
    EndpointEnv { var: &'static str },
}

/// Environment variables an SDK reads its endpoint from when the
/// constructor is not given one.
const ENDPOINT_ENV: &[(Pkg, &str)] = &[
    (Pkg::Openai, "OPENAI_BASE_URL"),
    (Pkg::Anthropic, "ANTHROPIC_BASE_URL"),
];

/// Environment variables the module-level OpenAI API (v0 and the v1 module
/// client) reads its endpoint or API type from.
const OPENAI_MODULE_ENV: &[&str] = &["OPENAI_BASE_URL", "OPENAI_API_BASE", "OPENAI_API_TYPE"];

/// Attributes that re-route a built client when assigned.
const BASE_URL_ATTRS: &[&str] = &["base_url", "api_base", "_base_url"];

/// Where an untracked value came from, for gap details.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    /// No binding in this file.
    Unbound,
    /// `None`, a literal or a container: ignored next to a real value.
    Placeholder,
    /// A binding that refers to itself while being resolved.
    Cycle,
    /// The result of calling something that is not a tracked constructor.
    Factory {
        callee: String,
        wraps: Option<Pkg>,
    },
    /// A parameter with no tracked annotation.
    Parameter {
        name: String,
    },
    /// A name imported from a module that is not a tracked SDK.
    Imported {
        module: String,
    },
    /// `self.x` with no assignment in the class.
    UnassignedAttribute {
        name: String,
    },
    /// The value returned by an SDK call that is not a known constructor.
    SdkResult {
        pkg: Option<Pkg>,
    },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Val {
    /// A tracked module, or a member path off one (`openai.ChatCompletion`).
    Module {
        pkg: Pkg,
        path: Vec<String>,
    },
    /// The `google` namespace package, from `import google.genai`.
    GoogleNs,
    /// A client constructor.
    Class(ClientClass),
    /// Another imported SDK name, plus a member path.
    Symbol {
        pkg: Pkg,
        path: Vec<String>,
    },
    /// A client, plus a member path (`client.chat.completions`).
    Client {
        client: Client,
        path: Vec<String>,
    },
    /// The return value of an SDK call that is not a known client.
    Result(Option<Pkg>),
    Untracked(Origin),
}

impl Val {
    fn untracked(origin: Origin) -> Self {
        Val::Untracked(origin)
    }

    fn is_tracked(&self) -> bool {
        !matches!(self, Val::Untracked(_) | Val::Result(_))
    }

    fn pkg(&self) -> Option<Pkg> {
        match self {
            Val::Module { pkg, .. } | Val::Symbol { pkg, .. } => Some(*pkg),
            Val::Class(class) => Some(class.pkg),
            Val::Client { client, .. } => Some(client.pkg),
            Val::Result(pkg) => *pkg,
            _ => None,
        }
    }

    /// The member path after the tracked root.
    fn path(&self) -> &[String] {
        match self {
            Val::Module { path, .. } | Val::Symbol { path, .. } | Val::Client { path, .. } => path,
            _ => &[],
        }
    }
}

/// Why an allowlisted value did not resolve, and the expression's shape.
#[derive(Debug, Clone)]
struct Unresolved {
    reason: &'static str,
    shape: String,
    via: Option<String>,
    /// Values it may take that are known: what in-file callers pass for a
    /// parameter, the entries of a dict looked up with a run-time key, the
    /// assignments in each branch. Listed with the gap for triage, never
    /// as the call's value.
    candidates: Vec<Resolved>,
}

impl Unresolved {
    fn new(reason: &'static str, shape: String) -> Self {
        Self {
            reason,
            shape: clip_owned(shape),
            via: None,
            candidates: Vec::new(),
        }
    }

    fn with_candidates(mut self, candidates: Vec<Resolved>) -> Self {
        self.candidates.extend(candidates);
        self
    }

    fn via(mut self, name: &str) -> Self {
        if self.via.is_none() {
            self.via = Some(clip(name));
        }
        self
    }

    fn phrase(&self) -> &'static str {
        match self.reason {
            "env_var" => "read from the environment",
            "f_string" => "built with an f-string",
            "function_call" => "returned by a function call",
            "attribute" => "an attribute of another object",
            "subscript" => "looked up by subscript",
            "conditional" => "chosen at run time",
            "parameter" => "a parameter whose default is not a resolvable string",
            "reassigned" => "assigned more than once",
            "imported_name" => "imported from another module",
            "unbound" => "not defined in this file",
            "splat" => "possibly passed through **kwargs",
            "parameter_overridden" => "a parameter that in-file callers set to other values",
            "credential" => {
                "credential-shaped (a secret-like name or value), so it is never captured"
            }
            _ => "not a string literal",
        }
    }
}

/// A captured value before it is attached to a key.
#[derive(Debug, Clone)]
struct Resolved {
    value: String,
    origin: ValueOrigin,
    name: Option<String>,
    span: Span,
    /// Why the value may not be the one sent, each reported as an
    /// `unresolved_model` gap next to it: the lookup behind an
    /// [`ValueOrigin::EnvDefault`] fallback, or in-file callers that pass
    /// their own value for a parameter default. A value that has one is
    /// captured only where its gap can be reported.
    caveats: Vec<Unresolved>,
}

impl Resolved {
    /// Names `name` as the binding the value came through: the value's own
    /// name when it was written inline there, and the gaps of its caveats.
    fn through(mut self, name: &str, origin: ValueOrigin) -> Self {
        if self.name.is_none() || self.origin == origin {
            self.name = Some(clip(name));
        }
        self.caveats = self
            .caveats
            .into_iter()
            .map(|caveat| caveat.via(name))
            .collect();
        self
    }
}

/// A model value found inside a container argument or a `**` expansion.
#[derive(Debug, Clone)]
struct NestedValue {
    /// The key path (`fallbacks.model`).
    path: String,
    positional: bool,
    resolved: Resolved,
}

/// A bounded read of nested model values.
struct NestedRead {
    /// Container entries that may still be read.
    left: usize,
    /// Entries were left unread when the budget ran out.
    cut: bool,
    found: Vec<NestedValue>,
}

impl NestedRead {
    fn new() -> Self {
        Self {
            left: MAX_NESTED_ENTRIES,
            cut: false,
            found: Vec::new(),
        }
    }
}

/// What a dict literal's `**` expansion nests, read once.
struct NestedList {
    found: Vec<NestedValue>,
    cut: bool,
}

/// A memoized evaluation. `budget` is `None` when no resolution inside it
/// was cut off at [`MAX_NAME_DEPTH`]: the value is final. Otherwise it is
/// the name budget the evaluation ran with. A lookup with no more budget
/// reuses the value (and counts as cut off itself); one with more budget
/// recomputes it. Each entry is therefore computed at most
/// `MAX_NAME_DEPTH + 1` times, which keeps the work linear in the file
/// however its names branch.
#[derive(Clone)]
struct Memo<V> {
    val: V,
    budget: Option<u32>,
}

/// The memoized value for `key`, if one may be reused with `budget` left,
/// and whether it was cut off.
fn recall<K: std::hash::Hash + Eq, V: Clone>(
    memo: &HashMap<K, Memo<V>>,
    key: &K,
    budget: u32,
) -> Option<(V, bool)> {
    let entry = memo.get(key)?;
    match entry.budget {
        None => Some((entry.val.clone(), false)),
        Some(had) if budget <= had => Some((entry.val.clone(), true)),
        Some(_) => None,
    }
}

// ============================================================================
// Collection state
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Module,
    Class,
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    Positional,
    PositionalOnly,
    KeywordOnly,
    VarArgs,
    VarKwargs,
}

#[derive(Debug, Clone)]
struct Slot {
    name: String,
    kind: SlotKind,
}

struct Scope {
    kind: ScopeKind,
    parent: Option<usize>,
    /// Nearest enclosing class (through functions), for `self`.
    class: Option<usize>,
    /// The symbol name the graph uses for this scope (`Bot.ask`).
    symbol: Option<String>,
    /// For a class or function: its own name.
    name: String,
    globals: HashSet<String>,
    /// For a class: base class names as written.
    bases: Vec<String>,
    /// Byte range of the scope's node.
    start: usize,
    end: usize,
    /// For a function or lambda: its parameters in order.
    slots: Vec<Slot>,
    /// Decorated with something other than `staticmethod`/`classmethod`, so
    /// a framework may call it.
    decorated: bool,
    staticmethod: bool,
    lambda: bool,
    /// A list, set or dict comprehension or a generator expression: its
    /// own scope for its targets, with no name.
    comprehension: bool,
    /// The body runs where it is written (a class body, a list, set or dict
    /// comprehension), not when called later.
    immediate: bool,
    /// For a lambda assigned to a plain name (`ask = lambda q: ...`): that
    /// name, through which its in-file calls are found.
    bound_name: Option<String>,
}

impl Scope {
    fn new(kind: ScopeKind, parent: Option<usize>, start: usize, end: usize) -> Self {
        Self {
            kind,
            parent,
            class: None,
            symbol: None,
            name: String::new(),
            globals: HashSet::new(),
            bases: Vec::new(),
            start,
            end,
            slots: Vec::new(),
            decorated: false,
            staticmethod: false,
            lambda: false,
            comprehension: false,
            immediate: kind == ScopeKind::Class,
            bound_name: None,
        }
    }
}

/// Where a definition takes effect.
#[derive(Clone, Copy, Debug)]
struct Place {
    /// The byte offset after which the binding exists.
    at: usize,
    /// The byte range it certainly holds in, in straight-line code (the rest
    /// of its block after its statement). `None` for bindings that do not
    /// dominate what follows (walrus, loop targets).
    region: Option<(usize, usize)>,
}

impl Place {
    /// Whether the binding certainly holds at `pos`. The end is inclusive,
    /// so a definition holds at the end of its block.
    fn dominates(&self, pos: usize) -> bool {
        self.region
            .is_some_and(|(start, end)| start <= pos && pos <= end)
    }
}

#[derive(Clone, Copy)]
struct Assign<'t> {
    id: usize,
    /// The scope the value expression lives in.
    scope: usize,
    value: Option<Node<'t>>,
    annotation: Option<Node<'t>>,
    /// Bound by a loop, an unpacking or an augmented assignment.
    unknowable: bool,
    place: Place,
}

#[derive(Clone, Copy)]
struct Param<'t> {
    annotation: Option<Node<'t>>,
    default: Option<Node<'t>>,
}

#[derive(Clone)]
struct ImportDef {
    val: Val,
    place: Place,
}

#[derive(Clone)]
struct OtherImport {
    module: String,
    place: Place,
}

/// One definition of a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefRef {
    Import(usize),
    Other(usize),
    Param,
    Assign(usize),
}

#[derive(Clone, Copy)]
struct DefPos {
    place: Place,
    def: DefRef,
}

/// Every definition of one name in its scope, by position.
struct DefIndex {
    /// Definitions made in the binding scope itself, sorted by `place.at`.
    local: Vec<DefPos>,
    /// `global` assignments made by other functions: they may run at any time.
    foreign: Vec<DefRef>,
    total: usize,
}

/// The definitions that reach a use.
enum Reach {
    All,
    Subset(Vec<DefRef>),
}

/// What kinds of definition reach a use, without listing every one: a
/// name assigned thousands of times costs the same as one assigned twice.
#[derive(Default)]
struct DefKinds {
    param: bool,
    /// A tracked or other import.
    imported: bool,
    assigns: usize,
    /// The assignment, when there is exactly one.
    only_assign: Option<usize>,
}

/// A `x.attr = value` whose object is not `self`/`cls`/a class, checked later
/// for module-level OpenAI configuration.
#[derive(Clone, Copy)]
struct ConfigAssign<'t> {
    object: Node<'t>,
    attr: Node<'t>,
    value: Option<Node<'t>>,
    scope: usize,
}

/// Calls of a method name on `self`/`cls`, by calling class.
struct MethodCalls<'t> {
    /// Some call is on another object.
    foreign: bool,
    by_class: HashMap<usize, CallSites<'t>>,
}

/// Calls and the scope each is in.
type CallSites<'t> = Rc<[(Node<'t>, usize)]>;

/// Calls of one name, by the scope that binds the name at each.
type CallIndex<'t> = Rc<HashMap<usize, CallSites<'t>>>;

/// Calls of a function or method name, and whether the name is also used as
/// a value (passed, stored, decorated, rebound).
#[derive(Default)]
struct Uses<'t> {
    calls: Vec<(Node<'t>, usize)>,
    escapes: bool,
}

/// One node of the collection walk, with the context its ancestors give it.
#[derive(Clone, Copy)]
struct Frame<'t> {
    node: Node<'t>,
    scope: usize,
    parent: Option<Node<'t>>,
    field: Option<&'static str>,
    /// End of the enclosing block (or module).
    block_end: usize,
    /// The statement of that block this node is part of; `None` when the
    /// node is the statement.
    stmt: Option<(usize, usize)>,
    /// Innermost loop around the node in its scope (an index into that
    /// scope's [`Ctx::loops`]).
    inner_loop: Option<usize>,
    /// The parent's parent.
    grand: Option<Node<'t>>,
}

/// A `for` or `while` statement, for loop-carried definitions.
#[derive(Clone, Copy, Debug)]
struct LoopRange {
    start: usize,
    end: usize,
    /// The loop around it in the same scope.
    parent: Option<usize>,
}

impl LoopRange {
    fn contains(&self, pos: usize) -> bool {
        self.start <= pos && pos < self.end
    }
}

/// Where a use runs relative to the scope that binds its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UseAt {
    /// At this byte of the binding scope: code in the scope itself, a class
    /// body or a list, set or dict comprehension, which all run where they
    /// are written.
    At(usize),
    /// Later, from a function, lambda or generator defined at this byte of
    /// the binding scope: it may run after any later definition.
    Deferred(usize),
}

/// How a dict, or a part of one, is used where it appears.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DictUse<'t> {
    /// Read without letting it change: a subscript, `.get`, `.keys()`,
    /// `**d` in a call, a test or a read-only builtin.
    Safe,
    /// Assigned to this plain name: the two refer to one dict.
    Alias(&'t str),
    /// Its entries are handed out (`.items()`, `.values()`, `.copy()`,
    /// `dict(d)`, `{**d}`, iterated): a change to a nested container
    /// changes it.
    Shallow,
    /// Passed, stored, returned or mutated: it may change.
    Escape,
}

/// A part of a dict literal (`D["a"]`, `D.get("a")`, or the whole of `D`
/// for a shallow use) that leaves the dict, decided after collection.
#[derive(Clone, Copy)]
struct Exposed<'t> {
    root: &'t str,
    /// The identifier, subscript or `.get` call.
    node: Node<'t>,
    scope: usize,
    /// Assigned to this plain name.
    alias: Option<&'t str>,
}

/// A dict literal reached through names and subscripts.
struct DictRef<'t> {
    node: Node<'t>,
    scope: usize,
    /// How the code names it (`PARAMS`, `CONFIG["openai"]`), `None` inline.
    name: Option<String>,
    origin: ValueOrigin,
}

enum Lookup<'t> {
    Found(Node<'t>),
    Absent,
    Unknown,
}

/// A dict literal read once: where each key's last entry is, and the
/// entries a `**` expansion needs. Lookups are then constant time however
/// large the literal and however often it is used.
struct DictSummary<'t> {
    last: HashMap<String, (usize, Node<'t>)>,
    /// Index of the last entry that could not be read (`**other`, a computed
    /// key): it may override any key before it.
    last_unknown: Option<usize>,
    /// Identifier-keyed entries a `**` expansion reads (allowlisted keys,
    /// base URLs, containers), the last one per key, at most 256.
    expansion: Vec<(String, Node<'t>)>,
    /// Part of the dict could not be read, or the expansion list was cut.
    opaque: bool,
}

/// A dict expanded with `**`.
struct Expansion<'t> {
    /// The dict literal.
    node: Node<'t>,
    /// Its summary: `expansion` holds the pairs that can matter.
    summary: Rc<DictSummary<'t>>,
    scope: usize,
    name: Option<String>,
    origin: ValueOrigin,
}

impl Expansion<'_> {
    /// Part of it could not be read (`**other`, a computed key).
    fn opaque(&self) -> bool {
        self.summary.opaque
    }
}

/// A `D.get(key, default)` call on a same-file dict literal.
#[derive(Clone, Copy)]
struct DictGet<'t> {
    call: Node<'t>,
    key: Option<Node<'t>>,
    default: Option<Node<'t>>,
    /// The scope the call is in, where its default is evaluated.
    scope: usize,
}

/// The model values in-file calls pass for one parameter.
#[derive(Default)]
struct CallerModels {
    /// Distinct resolved values, at most [`MAX_CANDIDATES`].
    values: Vec<Resolved>,
    /// Some caller passes a value that does not resolve, passes it through
    /// `*args`/`**kwargs`, or was not read.
    unknown: bool,
}

/// How a call passes one parameter (see [`Ctx::param_slot`]).
struct ParamSlot {
    /// Index among the explicit positional arguments, `None` when the
    /// parameter is keyword-only.
    position: Option<usize>,
    /// It may be passed by keyword (it is not positional-only).
    keyword: bool,
    /// The scope the function is defined in, where its defaults run.
    parent: usize,
}

/// Where a parameter's argument is in one call.
enum ArgFor<'t> {
    Found(Node<'t>),
    Missing,
    Unknown,
}

type Key = (usize, String);

struct Ctx<'t> {
    src: &'t str,
    file: String,
    ascii: bool,
    /// Continuation bytes before each 64-byte block, for character columns.
    cont_blocks: Vec<u32>,
    scopes: Vec<Scope>,
    /// Tracked imports, keyed by the scope they bind in: a function for an
    /// import inside its body, the module for everything else.
    imports: HashMap<Key, Vec<ImportDef>>,
    /// Other imports (for gap details), keyed the same way.
    other_imports: HashMap<Key, Vec<OtherImport>>,
    sdk_imported: HashSet<Sdk>,
    assigns: HashMap<Key, Vec<Assign<'t>>>,
    attr_assigns: HashMap<Key, Vec<Assign<'t>>>,
    params: HashMap<Key, Param<'t>>,
    /// Function definitions, keyed by the scope their name binds in.
    functions: HashMap<Key, Vec<usize>>,
    classes_by_name: HashMap<String, Vec<usize>>,
    config_assigns: Vec<ConfigAssign<'t>>,
    calls: Vec<(Node<'t>, usize)>,
    refs: Vec<(Node<'t>, usize)>,
    call_scope: HashMap<usize, usize>,
    /// Parents of calls and of the wrappers between a call and its binding.
    parents: HashMap<usize, Node<'t>>,
    name_uses: HashMap<&'t str, Uses<'t>>,
    attr_uses: HashMap<&'t str, Uses<'t>>,
    /// Names whose dict literal may change after it is written: mutated
    /// (`D[k] = v`, `D["a"]["b"] = v`, `D["a"].update()`), passed to a
    /// function, stored, returned, or aliased by a name that is. Their
    /// literal is never read.
    escaped: HashSet<&'t str>,
    /// Dict parts that leave their dict, and plain-name aliases.
    exposed: Vec<Exposed<'t>>,
    def_index: HashMap<Key, DefIndex>,
    /// `for`/`while` statements per scope, in source order.
    loops: HashMap<usize, Vec<LoopRange>>,
    /// Start bytes of the expressions that read the process environment
    /// (`os.environ`, `os.getenv`, `getenv(...)`, `environ[...]`), in
    /// order, so "does this expression read the environment?" is a binary
    /// search instead of a walk.
    env_reads: Vec<usize>,
    constructions: HashMap<usize, usize>,
    name_memo: HashMap<Key, Memo<Val>>,
    attr_memo: HashMap<Key, Memo<Val>>,
    def_memo: HashMap<usize, Memo<Val>>,
    param_memo: HashMap<Key, Memo<Val>>,
    call_memo: HashMap<(usize, usize), Memo<Val>>,
    dict_cache: HashMap<usize, Rc<DictSummary<'t>>>,
    dict_contents_memo: HashMap<usize, (bool, Rc<[&'t str]>)>,
    /// What each expanded dict literal nests, by node id.
    dict_nested: HashMap<usize, Rc<NestedList>>,
    /// Nested values that may still be listed in this file.
    nested_left: usize,
    /// Candidate values that may still be listed with gaps in this file.
    candidates_left: usize,
    attr_lookup_memo: HashMap<Key, Rc<[Assign<'t>]>>,
    caller_memo: HashMap<Key, Memo<Option<Val>>>,
    /// Model values in-file callers pass for a parameter, by (function,
    /// parameter).
    caller_models: HashMap<Key, Rc<CallerModels>>,
    /// The values a dict literal's entries hold, by node id and the name
    /// the dict is used through.
    dict_candidates_memo: HashMap<(usize, Option<String>), Rc<[Resolved]>>,
    /// Candidate values are being collected: a nested lookup does not
    /// collect its own, so the work stays linear.
    collecting: u32,
    name_call_index: HashMap<String, CallIndex<'t>>,
    method_call_index: HashMap<String, Rc<MethodCalls<'t>>>,
    /// Resolutions in progress: (kind, scope or id, name).
    visiting: HashSet<(u8, usize, String)>,
    /// Current nesting of name resolutions, and how many were cut off at
    /// [`MAX_NAME_DEPTH`]; a value computed across a cut-off is memoized
    /// with its budget (see [`Memo`]).
    name_depth: u32,
    cutoffs: u64,
    next_id: usize,
    openai_routing: Routing,
    /// The file is a script (no extension, or `__main__.py`): other modules
    /// do not import it, so its in-file callers are all the callers.
    script: bool,
    /// Environment variables the file sets by name
    /// (`os.environ["OPENAI_BASE_URL"] = ...`, `os.environ.setdefault(...)`,
    /// `os.environ.update(...)`, `os.putenv(...)`).
    env_writes: HashSet<String>,
    /// Construction calls whose client has `base_url` assigned later, and
    /// packages with such a client that could not be singled out.
    rerouted: HashSet<usize>,
    rerouted_pkgs: HashSet<Pkg>,
    /// Why the module-level OpenAI API's routing may not hold.
    openai_caveat: Option<ClientCaveat>,
    out: FileUsages,
}

impl<'t> Ctx<'t> {
    fn new(src: &'t str, file: &str) -> Self {
        let ascii = src.is_ascii();
        let mut cont_blocks = Vec::new();
        if !ascii {
            let mut total = 0u32;
            cont_blocks.push(0);
            for chunk in src.as_bytes().chunks(64) {
                total += chunk.iter().filter(|b| (**b & 0xC0) == 0x80).count() as u32;
                cont_blocks.push(total);
            }
        }
        Self {
            src,
            file: file.to_string(),
            ascii,
            cont_blocks,
            scopes: vec![Scope::new(ScopeKind::Module, None, 0, src.len())],
            imports: HashMap::new(),
            other_imports: HashMap::new(),
            sdk_imported: HashSet::new(),
            assigns: HashMap::new(),
            attr_assigns: HashMap::new(),
            params: HashMap::new(),
            functions: HashMap::new(),
            classes_by_name: HashMap::new(),
            config_assigns: Vec::new(),
            calls: Vec::new(),
            refs: Vec::new(),
            call_scope: HashMap::new(),
            parents: HashMap::new(),
            name_uses: HashMap::new(),
            attr_uses: HashMap::new(),
            escaped: HashSet::new(),
            exposed: Vec::new(),
            def_index: HashMap::new(),
            loops: HashMap::new(),
            env_reads: Vec::new(),
            constructions: HashMap::new(),
            name_memo: HashMap::new(),
            attr_memo: HashMap::new(),
            def_memo: HashMap::new(),
            param_memo: HashMap::new(),
            call_memo: HashMap::new(),
            dict_cache: HashMap::new(),
            dict_contents_memo: HashMap::new(),
            dict_nested: HashMap::new(),
            nested_left: MAX_FILE_NESTED_VALUES,
            candidates_left: MAX_FILE_CANDIDATES,
            attr_lookup_memo: HashMap::new(),
            caller_memo: HashMap::new(),
            caller_models: HashMap::new(),
            dict_candidates_memo: HashMap::new(),
            collecting: 0,
            name_call_index: HashMap::new(),
            method_call_index: HashMap::new(),
            visiting: HashSet::new(),
            name_depth: 0,
            cutoffs: 0,
            next_id: 0,
            openai_routing: Routing::OpenaiDirect,
            script: is_script_path(file),
            env_writes: HashSet::new(),
            rerouted: HashSet::new(),
            rerouted_pkgs: HashSet::new(),
            openai_caveat: None,
            out: FileUsages::new(file, "python"),
        }
    }

    fn text(&self, node: Node<'t>) -> &'t str {
        self.src.get(node.byte_range()).unwrap_or("")
    }

    fn span(&self, node: Node<'t>) -> Span {
        let start = node.start_position();
        let end = node.end_position();
        Span {
            line: start.row as u32 + 1,
            column: self.char_column(node.start_byte(), start.column),
            end_line: end.row as u32 + 1,
            end_column: self.char_column(node.end_byte(), end.column),
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    /// 1-based column in Unicode scalar values for a byte offset whose
    /// tree-sitter column (in bytes) is `byte_column`. Constant time per
    /// call, so a long minified line is not rescanned for every record.
    fn char_column(&self, byte: usize, byte_column: usize) -> u32 {
        if self.ascii {
            return byte_column as u32 + 1;
        }
        let line_start = byte.saturating_sub(byte_column);
        let continuation = self.continuation_before(byte) - self.continuation_before(line_start);
        (byte_column - continuation.min(byte_column)) as u32 + 1
    }

    fn continuation_before(&self, at: usize) -> usize {
        let at = at.min(self.src.len());
        let block = at / 64;
        let base = self.cont_blocks.get(block).copied().unwrap_or(0) as usize;
        base + self.src.as_bytes()[block * 64..at]
            .iter()
            .filter(|b| (**b & 0xC0) == 0x80)
            .count()
    }

    fn symbol(&self, scope: usize) -> Option<String> {
        self.scopes[scope].symbol.clone()
    }

    fn fresh_id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    // ------------------------------------------------------------------
    // Pass 1: collect
    // ------------------------------------------------------------------

    fn collect(&mut self, root: Node<'t>) {
        self.scopes[0].start = root.start_byte();
        self.scopes[0].end = root.end_byte();
        let mut stack = vec![Frame {
            node: root,
            scope: 0,
            parent: None,
            field: None,
            block_end: root.end_byte(),
            stmt: None,
            inner_loop: None,
            grand: None,
        }];
        while let Some(frame) = stack.pop() {
            let node = frame.node;
            let scope = frame.scope;
            let stmt = frame.stmt.unwrap_or((node.start_byte(), node.end_byte()));
            // A statement-level binding holds from the end of its statement
            // to the end of its block.
            let place = Place {
                at: stmt.1,
                region: Some((stmt.1, frame.block_end)),
            };
            let mut inner = scope;
            let mut child_loop = frame.inner_loop;
            if node.is_named() {
                match node.kind() {
                    "function_definition" => {
                        inner = self.enter_function(node, scope, frame.parent);
                        child_loop = None;
                    }
                    "class_definition" => {
                        inner = self.enter_class(node, scope);
                        child_loop = None;
                    }
                    "lambda" => {
                        inner = self.enter_lambda(node, scope, frame.parent, frame.field);
                        child_loop = None;
                    }
                    "list_comprehension"
                    | "set_comprehension"
                    | "dictionary_comprehension"
                    | "generator_expression" => {
                        inner = self.enter_comprehension(node, scope);
                        child_loop = None;
                    }
                    "for_statement" | "while_statement" => {
                        let loops = self.loops.entry(scope).or_default();
                        loops.push(LoopRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            parent: frame.inner_loop,
                        });
                        child_loop = Some(loops.len() - 1);
                        if let Some(left) = node.child_by_field_name("left") {
                            let place = Place {
                                at: left.end_byte(),
                                region: None,
                            };
                            self.record_target(left, None, None, scope, true, place);
                        }
                    }
                    "for_in_clause" => {
                        if let Some(left) = node.child_by_field_name("left") {
                            let place = Place {
                                at: left.end_byte(),
                                region: None,
                            };
                            self.record_target(left, None, None, scope, true, place);
                        }
                    }
                    "import_statement" => self.import_statement(node, scope, place),
                    "import_from_statement" => self.import_from(node, scope, place),
                    "assignment" => {
                        if let Some(left) = node.child_by_field_name("left") {
                            let value = node.child_by_field_name("right");
                            let annotation = node.child_by_field_name("type");
                            self.record_target(left, value, annotation, scope, false, place);
                        }
                    }
                    "augmented_assignment" => {
                        if let Some(left) = node.child_by_field_name("left") {
                            self.record_target(left, None, None, scope, true, place);
                        }
                    }
                    "with_item" => {
                        // Bound before the body runs, for the rest of the block.
                        let place = Place {
                            at: node.end_byte(),
                            region: Some((node.end_byte(), frame.block_end)),
                        };
                        self.record_with_item(node, scope, place);
                    }
                    "named_expression" => {
                        if let Some(name) = node.child_by_field_name("name") {
                            let value = node.child_by_field_name("value");
                            // A walrus in a comprehension binds in the
                            // enclosing function or module (PEP 572).
                            let mut home = scope;
                            while self.scopes[home].comprehension {
                                home = self.scopes[home].parent.unwrap_or(0);
                            }
                            let place = Place {
                                at: node.end_byte(),
                                region: None,
                            };
                            self.record_target(name, value, None, home, false, place);
                        }
                    }
                    "global_statement" => {
                        if self.scopes[scope].kind == ScopeKind::Function {
                            for name in named_children(node) {
                                let name = self.text(name).to_string();
                                self.scopes[scope].globals.insert(name);
                            }
                        }
                    }
                    "delete_statement" => {
                        for target in named_children(node) {
                            self.note_mutated_root(target);
                        }
                    }
                    "call" => {
                        self.calls.push((node, scope));
                        self.call_scope.insert(node.id(), scope);
                        if let Some(parent) = frame.parent {
                            self.parents.insert(node.id(), parent);
                        }
                        self.note_dict_mutation(node);
                        self.note_env_read(node);
                        self.note_env_write(node);
                        if is_get_call(node, |n| self.text(n)) {
                            self.note_dict_part(node, &frame, scope);
                        }
                    }
                    "subscript" => {
                        self.note_env_read(node);
                        self.note_dict_part(node, &frame, scope);
                    }
                    "parenthesized_expression"
                    | "boolean_operator"
                    | "conditional_expression"
                    | "await" => {
                        if let Some(parent) = frame.parent {
                            self.parents.insert(node.id(), parent);
                        }
                    }
                    "attribute" => {
                        if is_value_reference(frame.parent, frame.field) {
                            self.refs.push((node, scope));
                        }
                        self.note_attr_use(node, &frame, scope);
                        self.note_env_read(node);
                    }
                    "identifier" => {
                        self.note_name_use(node, &frame, scope);
                        self.note_dict_name(node, &frame, scope);
                    }
                    _ => {}
                }
            }
            let (block_end, child_stmt) = match node.kind() {
                "module" | "block" => (node.end_byte(), None),
                _ => (frame.block_end, Some(stmt)),
            };
            // Punctuation and keywords carry nothing: only named children
            // are walked, which also keeps the stack small on deep chains.
            for (child, field) in children_with_fields(node).into_iter().rev() {
                if !child.is_named() {
                    continue;
                }
                stack.push(Frame {
                    node: child,
                    scope: inner,
                    parent: Some(node),
                    field,
                    block_end,
                    stmt: child_stmt,
                    inner_loop: child_loop,
                    grand: frame.parent,
                });
            }
        }
        // The walk is pre-order, so this is already sorted; keep it so.
        self.env_reads.sort_unstable();
    }

    /// Records a call or value use of a bare name, for resolving parameters
    /// from in-file callers.
    fn note_name_use(&mut self, node: Node<'t>, frame: &Frame<'t>, scope: usize) {
        let Some(parent) = frame.parent else {
            return;
        };
        let declaration = match (parent.kind(), frame.field) {
            ("call", Some("function")) => {
                let name = self.text(node);
                self.name_uses
                    .entry(name)
                    .or_default()
                    .calls
                    .push((parent, scope));
                return;
            }
            _ => is_declaration(parent.kind(), frame.field),
        };
        if !declaration {
            let name = self.text(node);
            self.name_uses.entry(name).or_default().escapes = true;
        }
    }

    /// Records a call or value use of `x.name`, for resolving method
    /// parameters from in-file callers.
    fn note_attr_use(&mut self, node: Node<'t>, frame: &Frame<'t>, scope: usize) {
        let Some(attr) = node.child_by_field_name("attribute") else {
            return;
        };
        let name = self.text(attr);
        let uses = self.attr_uses.entry(name).or_default();
        match frame.parent {
            Some(parent) if parent.kind() == "call" && frame.field == Some("function") => {
                uses.calls.push((parent, scope));
            }
            _ => uses.escapes = true,
        }
    }

    /// Records `node` when it reads the process environment: `os.environ`,
    /// `os.getenv`, `environ.get`, `getenv(...)` or `environ[...]`.
    fn note_env_read(&mut self, node: Node<'t>) {
        let (object, member) = match node.kind() {
            "attribute" => (
                node.child_by_field_name("object"),
                node.child_by_field_name("attribute"),
            ),
            "call" => (node.child_by_field_name("function"), None),
            "subscript" => (node.child_by_field_name("value"), None),
            _ => return,
        };
        let Some(object) = object.filter(|o| o.kind() == "identifier") else {
            return;
        };
        let reads = match (node.kind(), self.text(object), member.map(|m| self.text(m))) {
            ("attribute", "os", Some(member)) => matches!(member, "environ" | "getenv"),
            ("attribute", "environ", Some(member)) => matches!(member, "get" | "setdefault"),
            ("call", name, _) => name == "getenv",
            ("subscript", name, _) => name == "environ",
            _ => false,
        };
        if reads {
            self.env_reads.push(node.start_byte());
        }
    }

    /// Records the names of environment variables set by `node`: an
    /// assignment target `os.environ["NAME"]`, or a call to
    /// `os.environ.setdefault("NAME", ...)`, `os.environ.update(...)` or
    /// `os.putenv("NAME", ...)`. Only names written as literals are seen.
    fn note_env_write(&mut self, node: Node<'t>) {
        let environ =
            |n: Node<'t>| matches!(self.render_links(n, 4).as_str(), "os.environ" | "environ");
        let mut names: Vec<String> = Vec::new();
        match node.kind() {
            "subscript" => {
                if node.child_by_field_name("value").is_some_and(environ) {
                    if let Some(name) = single_subscript(node).and_then(|k| self.literal(k).ok()) {
                        names.push(name);
                    }
                }
            }
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                let (positional, keywords) = call_arguments(node, |n| self.text(n));
                let callee = self.render_links(function, 4);
                let method = function
                    .child_by_field_name("attribute")
                    .map(|a| self.text(a))
                    .unwrap_or_default();
                let on_environ = function.child_by_field_name("object").is_some_and(environ);
                if matches!(callee.as_str(), "os.putenv" | "putenv")
                    || (on_environ && method == "setdefault")
                {
                    if let Some(name) = positional.first().and_then(|k| self.literal(*k).ok()) {
                        names.push(name);
                    }
                } else if on_environ && method == "update" {
                    names.extend(keywords.iter().map(|(k, _)| k.clone()));
                    for arg in positional {
                        if arg.kind() == "dictionary" {
                            for (key, _) in
                                dict_entries(arg, |n| self.literal(n).ok(), |n| self.text(n))
                                    .into_iter()
                                    .flatten()
                            {
                                names.push(key);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        self.env_writes.extend(names);
    }

    /// `D[k] = v`, `del D[k]`, `D["a"]["b"] = v`, `H.d["k"] = v`: the
    /// name at the root of the target may hold a dict that changes.
    fn note_mutated_root(&mut self, target: Node<'t>) {
        if matches!(target.kind(), "subscript" | "attribute") {
            if let Some(root) = chain_root_name(target, |n| self.text(n)) {
                self.escaped.insert(root);
            }
        }
    }

    /// `D.update(...)`, `D["a"].setdefault(...)`, `D["fallbacks"].append(...)`.
    fn note_dict_mutation(&mut self, call: Node<'t>) {
        let Some(function) = call
            .child_by_field_name("function")
            .filter(|f| f.kind() == "attribute")
        else {
            return;
        };
        let (Some(object), Some(attr)) = (
            function.child_by_field_name("object"),
            function.child_by_field_name("attribute"),
        ) else {
            return;
        };
        if MUTATING_METHODS.contains(&self.text(attr)) {
            if let Some(root) = chain_root_name(object, |n| self.text(n)) {
                self.escaped.insert(root);
            }
        }
    }

    /// A bare name: whether a dict bound to it may change or escape here.
    fn note_dict_name(&mut self, node: Node<'t>, frame: &Frame<'t>, scope: usize) {
        let name = self.text(node);
        match self.dict_use(frame, false) {
            DictUse::Safe => {}
            DictUse::Escape => {
                self.escaped.insert(name);
            }
            use_ @ (DictUse::Shallow | DictUse::Alias(_)) => self.exposed.push(Exposed {
                root: name,
                node,
                scope,
                alias: match use_ {
                    DictUse::Alias(target) => Some(target),
                    _ => None,
                },
            }),
        }
    }

    /// `D["a"]` or `D.get("a")` on a name: whether that part leaves the
    /// dict here. What it holds is checked after collection: handing out a
    /// string cannot change the dict, handing out a nested container can.
    fn note_dict_part(&mut self, node: Node<'t>, frame: &Frame<'t>, scope: usize) {
        let Some(root) = chain_root_name(node, |n| self.text(n)) else {
            return;
        };
        let alias = match self.dict_use(frame, true) {
            DictUse::Safe => return,
            DictUse::Alias(target) => Some(target),
            DictUse::Shallow | DictUse::Escape => None,
        };
        self.exposed.push(Exposed {
            root,
            node,
            scope,
            alias,
        });
    }

    /// How the dict or dict part at `frame` is used, from its parent, the
    /// field it fills there and its grandparent. `part` is set for a
    /// subscript or `.get` call: iterating one hands out its elements.
    fn dict_use(&self, frame: &Frame<'t>, part: bool) -> DictUse<'t> {
        let Some(parent) = frame.parent else {
            return DictUse::Safe;
        };
        match (parent.kind(), frame.field) {
            // A longer chain decides at its outer end; a target is a
            // definition or a mutation, recorded where it is assigned.
            ("subscript", Some("value"))
            | ("assignment" | "augmented_assignment", Some("left"))
            | ("delete_statement", _) => DictUse::Safe,
            ("attribute", Some("object")) => {
                let attr = parent
                    .child_by_field_name("attribute")
                    .map(|a| self.text(a))
                    .unwrap_or_default();
                if READ_ONLY_METHODS.contains(&attr) {
                    DictUse::Safe
                } else if SHALLOW_METHODS.contains(&attr) {
                    DictUse::Shallow
                } else {
                    DictUse::Escape
                }
            }
            ("dictionary_splat", _) => match frame.grand.map(|g| g.kind()) {
                Some("argument_list") => DictUse::Safe,
                _ => DictUse::Shallow,
            },
            ("list_splat", _) | ("for_statement" | "for_in_clause", Some("right")) => {
                // A dict iterates its keys; a nested list hands out its
                // elements.
                if part {
                    DictUse::Shallow
                } else {
                    DictUse::Safe
                }
            }
            ("if_statement" | "elif_clause" | "while_statement", Some("condition"))
            | ("comparison_operator" | "not_operator" | "expression_statement", _)
            | ("interpolation", _) => DictUse::Safe,
            ("argument_list", _) => {
                let callee = frame
                    .grand
                    .and_then(|call| call.child_by_field_name("function"))
                    .map(|f| self.render_links(f, 4))
                    .unwrap_or_default();
                if READ_ONLY_CALLS.contains(&callee.as_str()) {
                    DictUse::Safe
                } else if SHALLOW_CALLS.contains(&callee.as_str()) {
                    DictUse::Shallow
                } else {
                    DictUse::Escape
                }
            }
            ("assignment", Some("right")) => match parent.child_by_field_name("left") {
                Some(target) if target.kind() == "identifier" => DictUse::Alias(self.text(target)),
                _ => DictUse::Escape,
            },
            (kind, field) if !part && is_declaration(kind, field) => DictUse::Safe,
            _ => DictUse::Escape,
        }
    }

    /// Decides the exposed dict parts and aliases once every definition is
    /// known: a part that hands out a nested container, or an unknown entry,
    /// lets its dict change; a name held in the dict is exposed itself; and
    /// every name aliased with an escaped one escapes too.
    fn settle_dict_escapes(&mut self) {
        let mut links: Vec<(&'t str, &'t str)> = Vec::new();
        for exposed in std::mem::take(&mut self.exposed) {
            let (alters, names) = self.exposed_contents(&exposed);
            match exposed.alias {
                Some(target) => {
                    if alters {
                        links.push((target, exposed.root));
                    }
                    for name in names.iter() {
                        links.push((target, name));
                    }
                }
                None => {
                    if alters {
                        self.escaped.insert(exposed.root);
                    }
                    self.escaped.extend(names.iter().copied());
                }
            }
        }
        // Union the aliases; a class with one escaped name escapes whole.
        let mut parent: HashMap<&'t str, &'t str> = HashMap::new();
        fn find<'a>(parent: &mut HashMap<&'a str, &'a str>, name: &'a str) -> &'a str {
            let mut root = name;
            while let Some(&next) = parent.get(root) {
                if next == root {
                    break;
                }
                root = next;
            }
            let mut current = name;
            while current != root {
                let next = parent.get(current).copied().unwrap_or(root);
                parent.insert(current, root);
                current = next;
            }
            root
        }
        for (a, b) in &links {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let names: Vec<&'t str> = links.iter().flat_map(|(a, b)| [*a, *b]).collect();
        let escaped_roots: HashSet<&'t str> = names
            .iter()
            .filter(|n| self.escaped.contains(*n))
            .map(|n| find(&mut parent, n))
            .collect();
        for name in names {
            if escaped_roots.contains(find(&mut parent, name)) {
                self.escaped.insert(name);
            }
        }
    }

    /// What an exposed part hands out: whether a change through it could
    /// alter its dict (a nested container, or an entry that cannot be
    /// read), and the names stored in it (which may be dicts themselves).
    fn exposed_contents(&mut self, exposed: &Exposed<'t>) -> (bool, Rc<[&'t str]>) {
        let none: Rc<[&'t str]> = Rc::from(Vec::new());
        // The literal keys from the root to the part.
        let mut keys: Vec<Option<String>> = Vec::new();
        let mut node = exposed.node;
        for _ in 0..MAX_DEPTH {
            match node.kind() {
                "identifier" => break,
                "subscript" => {
                    keys.push(single_subscript(node).and_then(|k| self.literal(k).ok()));
                    match node.child_by_field_name("value") {
                        Some(value) => node = value,
                        None => return (true, none),
                    }
                }
                "call" => {
                    let key = call_arguments(node, |n| self.text(n)).0.first().copied();
                    keys.push(key.and_then(|k| self.literal(k).ok()));
                    match node
                        .child_by_field_name("function")
                        .and_then(|f| f.child_by_field_name("object"))
                    {
                        Some(object) => node = object,
                        None => return (true, none),
                    }
                }
                "parenthesized_expression" => match node.named_child(0) {
                    Some(inner) => node = inner,
                    None => return (true, none),
                },
                _ => return (true, none),
            }
        }
        if node.kind() != "identifier" {
            return (true, none);
        }
        // Not a same-file dict literal here: nothing to protect.
        let Some(mut dict) = self.dict_node(node, exposed.scope, 0) else {
            return (false, none);
        };
        keys.reverse();
        let last = keys.len();
        for (i, key) in keys.into_iter().enumerate() {
            let Some(key) = key else {
                // A key known only at run time: any entry may leave.
                return self.dict_contents(dict.node);
            };
            let value = match self.dict_lookup(dict.node, &key) {
                Lookup::Found(value) => value,
                Lookup::Absent => return (false, none),
                Lookup::Unknown => return (true, none),
            };
            if i + 1 == last {
                return match value.kind() {
                    "identifier" => (false, Rc::from(vec![self.text(value)])),
                    _ => (is_mutable_literal(value, |n| self.text(n)), none),
                };
            }
            match self.dict_node(value, dict.scope, 0) {
                Some(inner) => dict = inner,
                // An entry that is not a dict literal: not readable as one.
                None => return (false, none),
            }
        }
        // The whole dict is handed out (`.items()`, `dict(D)`, an alias).
        if exposed.alias.is_some() && exposed.node.kind() == "identifier" {
            return (true, none);
        }
        self.dict_contents(dict.node)
    }

    /// Whether a dict literal holds a container or an unreadable entry, and
    /// the names it holds. Computed once per literal.
    fn dict_contents(&mut self, dict: Node<'t>) -> (bool, Rc<[&'t str]>) {
        if let Some(found) = self.dict_contents_memo.get(&dict.id()) {
            return found.clone();
        }
        let mut alters = false;
        let mut names = Vec::new();
        for entry in dict_entries(dict, |n| self.literal(n).ok(), |n| self.text(n)) {
            match entry {
                None => alters = true,
                Some((_, value)) if value.kind() == "identifier" => names.push(self.text(value)),
                Some((_, value)) => alters |= is_mutable_literal(value, |n| self.text(n)),
            }
        }
        let found = (alters, Rc::from(names));
        self.dict_contents_memo.insert(dict.id(), found.clone());
        found
    }

    fn enter_function(
        &mut self,
        node: Node<'t>,
        parent: usize,
        wrapper: Option<Node<'t>>,
    ) -> usize {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        let class = match self.scopes[parent].kind {
            ScopeKind::Class => Some(parent),
            _ => self.scopes[parent].class,
        };
        // Same naming as the graph's Python extractor: `Class.method` inside
        // a class (including functions nested in a method), else the name.
        let symbol = match class {
            Some(class) => format!("{}.{}", self.scopes[class].name, name),
            None => name.clone(),
        };
        let mut scope = Scope::new(
            ScopeKind::Function,
            Some(parent),
            node.start_byte(),
            node.end_byte(),
        );
        scope.class = class;
        scope.symbol = Some(clip_owned(symbol));
        scope.name = name.clone();
        if let Some(wrapper) = wrapper.filter(|w| w.kind() == "decorated_definition") {
            for decorator in named_children(wrapper) {
                if decorator.kind() != "decorator" {
                    continue;
                }
                match decorator.named_child(0).map(|d| self.text(d)) {
                    Some("staticmethod") => scope.staticmethod = true,
                    Some("classmethod") => {}
                    _ => scope.decorated = true,
                }
            }
        }
        let id = self.scopes.len();
        self.scopes.push(scope);
        if let Some(params) = node.child_by_field_name("parameters") {
            self.record_params(params, id);
        }
        // A method binds in its class; any other `def` where an import would.
        let home = match self.scopes[parent].kind {
            ScopeKind::Class => parent,
            _ => self.import_home(parent, &name),
        };
        self.functions.entry((home, name)).or_default().push(id);
        id
    }

    /// A comprehension or generator expression: its own scope for its
    /// targets (Python 3), named after the enclosing symbol like a lambda.
    /// List, set and dict comprehensions run where they are written; a
    /// generator runs when consumed.
    fn enter_comprehension(&mut self, node: Node<'t>, parent: usize) -> usize {
        let class = match self.scopes[parent].kind {
            ScopeKind::Class => Some(parent),
            _ => self.scopes[parent].class,
        };
        let mut scope = Scope::new(
            ScopeKind::Function,
            Some(parent),
            node.start_byte(),
            node.end_byte(),
        );
        scope.class = class;
        scope.symbol = self.scopes[parent].symbol.clone();
        scope.name = "<comprehension>".to_string();
        scope.lambda = true;
        scope.comprehension = true;
        scope.immediate = node.kind() != "generator_expression";
        let id = self.scopes.len();
        self.scopes.push(scope);
        id
    }

    fn enter_lambda(
        &mut self,
        node: Node<'t>,
        parent: usize,
        holder: Option<Node<'t>>,
        field: Option<&'static str>,
    ) -> usize {
        let class = match self.scopes[parent].kind {
            ScopeKind::Class => Some(parent),
            _ => self.scopes[parent].class,
        };
        let mut scope = Scope::new(
            ScopeKind::Function,
            Some(parent),
            node.start_byte(),
            node.end_byte(),
        );
        scope.class = class;
        // The graph has no node for a lambda: calls in it belong to the
        // enclosing function.
        scope.symbol = self.scopes[parent].symbol.clone();
        scope.name = "<lambda>".to_string();
        scope.lambda = true;
        scope.bound_name = holder
            .filter(|h| h.kind() == "assignment" && field == Some("right"))
            .and_then(|h| h.child_by_field_name("left"))
            .filter(|left| left.kind() == "identifier")
            .map(|left| self.text(left).to_string());
        let id = self.scopes.len();
        self.scopes.push(scope);
        if let Some(params) = node.child_by_field_name("parameters") {
            self.record_params(params, id);
        }
        id
    }

    fn record_params(&mut self, params: Node<'t>, scope: usize) {
        let mut kind = SlotKind::Positional;
        let mut slots: Vec<Slot> = Vec::new();
        for param in named_children(params) {
            let (name, annotation, default, slot) = match param.kind() {
                "identifier" => (Some(param), None, None, kind),
                "typed_parameter" => {
                    let inner = param.named_child(0);
                    let (name, slot) = match inner.map(|n| n.kind()) {
                        Some("list_splat_pattern") => {
                            kind = SlotKind::KeywordOnly;
                            (inner.and_then(|n| n.named_child(0)), SlotKind::VarArgs)
                        }
                        Some("dictionary_splat_pattern") => {
                            (inner.and_then(|n| n.named_child(0)), SlotKind::VarKwargs)
                        }
                        _ => (inner, kind),
                    };
                    (name, param.child_by_field_name("type"), None, slot)
                }
                "default_parameter" => (
                    param.child_by_field_name("name"),
                    None,
                    param.child_by_field_name("value"),
                    kind,
                ),
                "typed_default_parameter" => (
                    param.child_by_field_name("name"),
                    param.child_by_field_name("type"),
                    param.child_by_field_name("value"),
                    kind,
                ),
                "list_splat_pattern" => {
                    kind = SlotKind::KeywordOnly;
                    (param.named_child(0), None, None, SlotKind::VarArgs)
                }
                "dictionary_splat_pattern" => {
                    (param.named_child(0), None, None, SlotKind::VarKwargs)
                }
                "keyword_separator" => {
                    kind = SlotKind::KeywordOnly;
                    continue;
                }
                "positional_separator" => {
                    for slot in &mut slots {
                        if slot.kind == SlotKind::Positional {
                            slot.kind = SlotKind::PositionalOnly;
                        }
                    }
                    continue;
                }
                // A tuple parameter (Python 2) still takes a position.
                _ => (None, None, None, kind),
            };
            let name = name.filter(|n| n.kind() == "identifier");
            slots.push(Slot {
                name: name.map(|n| self.text(n).to_string()).unwrap_or_default(),
                kind: slot,
            });
            if let Some(name) = name {
                let key = (scope, self.text(name).to_string());
                self.params.insert(
                    key,
                    Param {
                        annotation,
                        default,
                    },
                );
            }
        }
        self.scopes[scope].slots = slots;
    }

    fn enter_class(&mut self, node: Node<'t>, parent: usize) -> usize {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        let mut bases = Vec::new();
        if let Some(list) = node.child_by_field_name("superclasses") {
            for base in named_children(list) {
                if matches!(base.kind(), "identifier" | "attribute") {
                    bases.push(self.text(base).to_string());
                }
            }
        }
        let id = self.scopes.len();
        let mut scope = Scope::new(
            ScopeKind::Class,
            Some(parent),
            node.start_byte(),
            node.end_byte(),
        );
        scope.class = Some(id);
        scope.symbol = Some(clip(&name));
        scope.name = name.clone();
        scope.bases = bases;
        self.scopes.push(scope);
        self.classes_by_name.entry(name).or_default().push(id);
        id
    }

    fn record_with_item(&mut self, node: Node<'t>, scope: usize, place: Place) {
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if value.kind() != "as_pattern" {
            return;
        }
        let expr = value.named_child(0);
        let target = value
            .child_by_field_name("alias")
            .and_then(|alias| alias.named_child(0));
        if let Some(target) = target {
            self.record_target(target, expr, None, scope, false, place);
        }
    }

    fn record_target(
        &mut self,
        target: Node<'t>,
        value: Option<Node<'t>>,
        annotation: Option<Node<'t>>,
        scope: usize,
        unknowable: bool,
        place: Place,
    ) {
        stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
            self.record_target_inner(target, value, annotation, scope, unknowable, place)
        })
    }

    fn record_target_inner(
        &mut self,
        target: Node<'t>,
        value: Option<Node<'t>>,
        annotation: Option<Node<'t>>,
        scope: usize,
        unknowable: bool,
        place: Place,
    ) {
        let assign = Assign {
            id: self.fresh_id(),
            scope,
            value,
            annotation,
            unknowable,
            place,
        };
        match target.kind() {
            "identifier" => {
                let name = self.text(target).to_string();
                let home = if self.scopes[scope].kind == ScopeKind::Function
                    && self.scopes[scope].globals.contains(&name)
                {
                    0
                } else {
                    scope
                };
                self.assigns.entry((home, name)).or_default().push(assign);
            }
            "attribute" => {
                let (Some(object), Some(attr)) = (
                    target.child_by_field_name("object"),
                    target.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                let attr_name = self.text(attr).to_string();
                if object.kind() == "identifier" {
                    let object_name = self.text(object);
                    if matches!(object_name, "self" | "cls") {
                        if let Some(class) = self.scopes[scope].class {
                            self.attr_assigns
                                .entry((class, attr_name))
                                .or_default()
                                .push(assign);
                            return;
                        }
                    }
                    if let Some(classes) = self.classes_by_name.get(object_name) {
                        if let [class] = classes.as_slice() {
                            let class = *class;
                            self.attr_assigns
                                .entry((class, attr_name))
                                .or_default()
                                .push(assign);
                            return;
                        }
                    }
                }
                self.note_mutated_root(object);
                self.config_assigns.push(ConfigAssign {
                    object,
                    attr,
                    value,
                    scope,
                });
            }
            "subscript" => {
                self.note_env_write(target);
                self.note_mutated_root(target)
            }
            "pattern_list"
            | "tuple_pattern"
            | "list_pattern"
            | "tuple"
            | "list"
            | "list_splat_pattern"
            | "parenthesized_expression" => {
                for child in named_children(target) {
                    self.record_target(child, None, None, scope, true, place);
                }
            }
            _ => {}
        }
    }

    /// The scope an import or definition of `name` in `scope` binds in:
    /// the function itself (unless the name is declared `global`), a class
    /// for a method, else the module. Class-body imports are treated as
    /// module-level.
    fn import_home(&self, scope: usize, name: &str) -> usize {
        let current = &self.scopes[scope];
        if current.kind == ScopeKind::Function && !current.globals.contains(name) {
            scope
        } else {
            0
        }
    }

    fn other_import(&mut self, scope: usize, local: String, module: String, place: Place) {
        let home = self.import_home(scope, &local);
        self.other_imports
            .entry((home, local))
            .or_default()
            .push(OtherImport { module, place });
    }

    fn import_statement(&mut self, node: Node<'t>, scope: usize, place: Place) {
        let mut cursor = node.walk();
        let names: Vec<Node<'t>> = node.children_by_field_name("name", &mut cursor).collect();
        for item in names {
            let (module_node, alias) = match item.kind() {
                "aliased_import" => (
                    item.child_by_field_name("name"),
                    item.child_by_field_name("alias"),
                ),
                _ => (Some(item), None),
            };
            let Some(module_node) = module_node else {
                continue;
            };
            let module = self.text(module_node).to_string();
            let Some((pkg, rest)) = Pkg::of_module(&module) else {
                let local = match alias {
                    Some(alias) => self.text(alias).to_string(),
                    None => module.split('.').next().unwrap_or("").to_string(),
                };
                self.other_import(scope, local, module, place);
                continue;
            };
            let (local, val) = match alias {
                Some(alias) => (
                    self.text(alias).to_string(),
                    Val::Module { pkg, path: rest },
                ),
                // `import google.genai` binds `google`; `import openai.types`
                // binds `openai`.
                None => match pkg {
                    Pkg::GoogleGenai | Pkg::GoogleGenerativeai => {
                        ("google".to_string(), Val::GoogleNs)
                    }
                    _ => (
                        module.split('.').next().unwrap_or("").to_string(),
                        Val::Module {
                            pkg,
                            path: Vec::new(),
                        },
                    ),
                },
            };
            let binding = Binding {
                local: &local,
                pkg,
                module: &module,
                name: None,
                kind: BindingKind::Module,
            };
            self.add_import(binding, val, scope, item, place);
        }
    }

    fn import_from(&mut self, node: Node<'t>, scope: usize, place: Place) {
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        let module = self.text(module_node).to_string();
        let relative = module_node.kind() == "relative_import";

        let mut cursor = node.walk();
        let names: Vec<Node<'t>> = node.children_by_field_name("name", &mut cursor).collect();
        let wildcard = named_children(node)
            .iter()
            .any(|c| c.kind() == "wildcard_import");

        let target = if relative {
            None
        } else {
            Pkg::of_module(&module)
        };

        if wildcard {
            if let Some((pkg, _)) = target {
                for class in CLASSES.iter().filter(|c| c.pkg == pkg) {
                    let key = (self.import_home(scope, class.name), class.name.to_string());
                    self.imports.entry(key).or_default().push(ImportDef {
                        val: Val::Class(*class),
                        place,
                    });
                }
                let binding = Binding {
                    local: "*",
                    pkg,
                    module: &module,
                    name: Some("*".to_string()),
                    kind: BindingKind::Symbol,
                };
                self.record_import(binding, node);
            }
            return;
        }

        for item in names {
            let (name_node, alias) = match item.kind() {
                "aliased_import" => (
                    item.child_by_field_name("name"),
                    item.child_by_field_name("alias"),
                ),
                _ => (Some(item), None),
            };
            let Some(name_node) = name_node else {
                continue;
            };
            let name = self.text(name_node).to_string();
            let local = alias
                .map(|a| self.text(a).to_string())
                .unwrap_or_else(|| name.clone());

            // `from google import genai` / `from google import generativeai`.
            if !relative && module == "google" {
                let pkg = match name.as_str() {
                    "genai" => Pkg::GoogleGenai,
                    "generativeai" => Pkg::GoogleGenerativeai,
                    _ => {
                        self.other_import(scope, local, module.clone(), place);
                        continue;
                    }
                };
                let val = Val::Module {
                    pkg,
                    path: Vec::new(),
                };
                let binding = Binding {
                    local: &local,
                    pkg,
                    module: &module,
                    name: Some(name),
                    kind: BindingKind::Module,
                };
                self.add_import(binding, val, scope, item, place);
                continue;
            }

            let Some((pkg, rest)) = target.clone() else {
                self.other_import(scope, local, module.clone(), place);
                continue;
            };
            let (val, kind) = match client_class(pkg, &name) {
                Some(class) => (Val::Class(class), BindingKind::ClientClass),
                None => {
                    let mut path = rest;
                    path.push(name.clone());
                    (Val::Symbol { pkg, path }, BindingKind::Symbol)
                }
            };
            let binding = Binding {
                local: &local,
                pkg,
                module: &module,
                name: Some(name),
                kind,
            };
            self.add_import(binding, val, scope, item, place);
        }
    }

    /// Binds a tracked import in the scope it appears in and records it.
    fn add_import(
        &mut self,
        binding: Binding<'_>,
        val: Val,
        scope: usize,
        node: Node<'t>,
        place: Place,
    ) {
        let key = (
            self.import_home(scope, binding.local),
            binding.local.to_string(),
        );
        self.imports
            .entry(key)
            .or_default()
            .push(ImportDef { val, place });
        self.record_import(binding, node);
    }

    fn record_import(&mut self, binding: Binding<'_>, node: Node<'t>) {
        self.sdk_imported.insert(binding.pkg.sdk());
        let span = self.span(node);
        self.out.imports.push(SdkImport {
            sdk: binding.pkg.sdk(),
            package: binding.pkg.package().to_string(),
            module: binding.module.to_string(),
            name: binding.name,
            local: binding.local.to_string(),
            kind: binding.kind,
            file: self.file.clone(),
            span,
        });
    }

    /// Orders every name's definitions by position, for reaching-definition
    /// queries.
    fn index_definitions(&mut self) {
        let mut keys: HashSet<Key> = HashSet::new();
        keys.extend(self.imports.keys().cloned());
        keys.extend(self.other_imports.keys().cloned());
        keys.extend(self.params.keys().cloned());
        keys.extend(self.assigns.keys().cloned());
        for key in keys {
            let home = key.0;
            let mut local = Vec::new();
            let mut foreign = Vec::new();
            for (i, import) in self.imports.get(&key).into_iter().flatten().enumerate() {
                local.push(DefPos {
                    place: import.place,
                    def: DefRef::Import(i),
                });
            }
            for (i, import) in self
                .other_imports
                .get(&key)
                .into_iter()
                .flatten()
                .enumerate()
            {
                local.push(DefPos {
                    place: import.place,
                    def: DefRef::Other(i),
                });
            }
            if self.params.contains_key(&key) {
                let scope = &self.scopes[home];
                local.push(DefPos {
                    place: Place {
                        at: scope.start,
                        region: Some((scope.start, scope.end)),
                    },
                    def: DefRef::Param,
                });
            }
            for (i, assign) in self.assigns.get(&key).into_iter().flatten().enumerate() {
                if assign.scope == home {
                    local.push(DefPos {
                        place: assign.place,
                        def: DefRef::Assign(i),
                    });
                } else {
                    foreign.push(DefRef::Assign(i));
                }
            }
            local.sort_by_key(|d| d.place.at);
            let total = local.len() + foreign.len();
            self.def_index.insert(
                key,
                DefIndex {
                    local,
                    foreign,
                    total,
                },
            );
        }
    }

    /// The definitions of `key` that reach a use.
    ///
    /// A use [`UseAt::At`] a byte sees the nearest definition that dominates
    /// it, every definition between that one and the use (in a branch), and
    /// the definitions later in a loop that runs the use again without
    /// passing the dominating one first. A [`UseAt::Deferred`] use (from a
    /// function defined at that byte) sees what reaches the definition plus
    /// every later definition: it may be called after any of them.
    fn reaching(&self, key: &Key, at: UseAt) -> Reach {
        let Some(index) = self.def_index.get(key) else {
            return Reach::All;
        };
        if index.total <= 1 {
            return Reach::All;
        }
        let (pos, deferred) = match at {
            UseAt::At(pos) => (pos, false),
            UseAt::Deferred(pos) => (pos, true),
        };
        let local = &index.local;
        let before = local.partition_point(|d| d.place.at <= pos);
        let mut nearest = None;
        for (steps, i) in (0..before).rev().enumerate() {
            if steps >= MAX_REACH_SCAN {
                return Reach::All;
            }
            if local[i].place.dominates(pos) {
                nearest = Some(i);
                break;
            }
        }
        let first = match nearest {
            Some(i) => i,
            None if before > MAX_REACH_SCAN => return Reach::All,
            None => 0,
        };
        let mut chosen: Vec<DefRef> = local[first..before].iter().map(|d| d.def).collect();
        let later_until = if deferred {
            Some(usize::MAX)
        } else {
            let dominating_at = nearest.map(|i| local[i].place.at);
            match self.reentry_loop(key.0, pos, dominating_at) {
                Some(found) => found,
                None => return Reach::All,
            }
        };
        if let Some(until) = later_until {
            for d in &local[before..] {
                if d.place.at > until {
                    break;
                }
                if chosen.len() >= MAX_REACH_SCAN {
                    return Reach::All;
                }
                chosen.push(d.def);
            }
        }
        if chosen.is_empty() || chosen.len() + index.foreign.len() >= index.total {
            return Reach::All;
        }
        chosen.extend(index.foreign.iter().copied());
        Reach::Subset(chosen)
    }

    /// The end of the outermost loop of `scope` that runs byte `pos` again
    /// without passing a dominating definition at `dominating_at` first:
    /// definitions later in that loop reach `pos` on its next iteration.
    /// `Some(None)` when there is no such loop, `None` past the search
    /// limit.
    fn reentry_loop(
        &self,
        scope: usize,
        pos: usize,
        dominating_at: Option<usize>,
    ) -> Option<Option<usize>> {
        let Some(loops) = self.loops.get(&scope) else {
            return Some(None);
        };
        // The last loop starting before `pos`; every loop around `pos` is
        // it or one of its ancestors, since loops nest.
        let last = loops.partition_point(|l| l.start <= pos);
        let mut current = last.checked_sub(1);
        let mut found = None;
        let mut steps = 0;
        while let Some(i) = current {
            steps += 1;
            if steps > MAX_REACH_SCAN {
                return None;
            }
            let range = loops[i];
            if range.contains(pos) {
                // This loop re-runs the dominating definition before the
                // use, and so does every loop around it.
                if dominating_at.is_some_and(|at| range.start < at && at <= range.end) {
                    break;
                }
                found = Some(range.end);
            }
            current = range.parent;
        }
        Some(found)
    }

    /// Where a use at byte `pos` of `scope` runs, seen from `home`, the
    /// scope that binds the name: at its position when only immediate
    /// scopes (class bodies, comprehensions) lie between them, else deferred
    /// from where the outermost nested scope is defined.
    fn use_at(&self, scope: usize, home: usize, pos: usize) -> UseAt {
        let mut s = scope;
        let mut outer = None;
        let mut deferred = false;
        while s != home {
            let current = &self.scopes[s];
            deferred |= !current.immediate;
            outer = Some(s);
            match current.parent {
                Some(parent) => s = parent,
                None => break,
            }
        }
        match outer {
            Some(outer) if deferred => UseAt::Deferred(self.scopes[outer].start),
            _ => UseAt::At(pos),
        }
    }

    /// The binding scope of `name` used at byte `pos` of `scope`, and where
    /// that use runs in it.
    fn locate(&self, name: &str, scope: usize, pos: usize) -> (Key, UseAt) {
        let home = self.binding_scope(name, scope);
        let at = self.use_at(scope, home, pos);
        ((home, name.to_string()), at)
    }

    /// Every definition of `key`, in a fixed order.
    fn all_defs(&self, key: &Key) -> Vec<DefRef> {
        let mut defs = Vec::new();
        defs.extend((0..self.imports.get(key).map_or(0, Vec::len)).map(DefRef::Import));
        defs.extend((0..self.other_imports.get(key).map_or(0, Vec::len)).map(DefRef::Other));
        if self.params.contains_key(key) {
            defs.push(DefRef::Param);
        }
        defs.extend((0..self.assigns.get(key).map_or(0, Vec::len)).map(DefRef::Assign));
        defs
    }

    /// The kinds of definition of `name` a use at `pos` in `scope` sees,
    /// and the key they belong to. Constant time when every definition
    /// reaches the use, however many there are.
    fn defs_at(&self, name: &str, scope: usize, pos: usize) -> (Key, DefKinds) {
        let (key, at) = self.locate(name, scope, pos);
        let kinds = match self.reaching(&key, at) {
            Reach::All => {
                let assigns = self.assigns.get(&key).map_or(0, Vec::len);
                DefKinds {
                    param: self.params.contains_key(&key),
                    imported: self.imports.contains_key(&key)
                        || self.other_imports.contains_key(&key),
                    assigns,
                    only_assign: (assigns == 1).then_some(0),
                }
            }
            Reach::Subset(defs) => {
                let mut kinds = DefKinds::default();
                for def in defs {
                    match def {
                        DefRef::Import(_) | DefRef::Other(_) => kinds.imported = true,
                        DefRef::Param => kinds.param = true,
                        DefRef::Assign(i) => {
                            kinds.assigns += 1;
                            kinds.only_assign = (kinds.assigns == 1).then_some(i);
                        }
                    }
                }
                kinds
            }
        };
        (key, kinds)
    }

    // ------------------------------------------------------------------
    // Evaluation
    // ------------------------------------------------------------------

    fn eval(&mut self, node: Node<'t>, scope: usize, depth: u32) -> Val {
        if depth > MAX_DEPTH {
            return Val::untracked(Origin::Other);
        }
        stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
            self.eval_inner(node, scope, depth)
        })
    }

    fn eval_inner(&mut self, node: Node<'t>, scope: usize, depth: u32) -> Val {
        match node.kind() {
            "identifier" => {
                let name = self.text(node).to_string();
                self.resolve_name(&name, scope, node.start_byte())
            }
            "attribute" => {
                let (Some(object), Some(attr)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("attribute"),
                ) else {
                    return Val::untracked(Origin::Other);
                };
                let attr = self.text(attr).to_string();
                if let Some(class) = self.class_receiver(object, scope) {
                    return self.resolve_attr(class, &attr);
                }
                let base = self.eval(object, scope, depth + 1);
                self.member(base, &attr)
            }
            "call" => {
                // Each call is evaluated once per scope: a chain of calls
                // (`x.a().b().c()...`) is then linear, not quadratic.
                let key = (node.id(), scope);
                let budget = self.budget();
                if let Some(val) = self.reuse(recall(&self.call_memo, &key, budget)) {
                    return val;
                }
                let Some(function) = node.child_by_field_name("function") else {
                    return Val::untracked(Origin::Other);
                };
                let cutoffs = self.cutoffs;
                let callee = self.eval(function, scope, depth + 1);
                let val = self.apply_call(callee, node, scope, depth + 1);
                let memo = self.memo(val.clone(), cutoffs, budget);
                self.call_memo.insert(key, memo);
                val
            }
            "parenthesized_expression" | "await" => match node.named_child(0) {
                Some(inner) => self.eval(inner, scope, depth + 1),
                None => Val::untracked(Origin::Other),
            },
            "assignment" => match node.child_by_field_name("right") {
                Some(right) => self.eval(right, scope, depth + 1),
                None => Val::untracked(Origin::Other),
            },
            "boolean_operator" => {
                let mut vals = Vec::new();
                for field in ["left", "right"] {
                    if let Some(side) = node.child_by_field_name(field) {
                        vals.push(self.eval(side, scope, depth + 1));
                    }
                }
                combine(vals)
            }
            "conditional_expression" => {
                // `a if cond else b`: the first and last named children.
                let children = named_children(node);
                let mut vals = Vec::new();
                for side in [children.first(), children.last()].into_iter().flatten() {
                    vals.push(self.eval(*side, scope, depth + 1));
                }
                combine(vals)
            }
            "none"
            | "true"
            | "false"
            | "integer"
            | "float"
            | "string"
            | "concatenated_string"
            | "list"
            | "dictionary"
            | "tuple"
            | "set" => Val::untracked(Origin::Placeholder),
            _ => Val::untracked(Origin::Other),
        }
    }

    /// The class `self.x` / `cls.x` / `ClassName.x` refers to, when the
    /// object is one of those.
    fn class_receiver(&self, object: Node<'t>, scope: usize) -> Option<usize> {
        if object.kind() != "identifier" {
            return None;
        }
        let name = self.text(object);
        if matches!(name, "self" | "cls") {
            return self.scopes[scope].class;
        }
        match self.classes_by_name.get(name).map(Vec::as_slice) {
            Some([class]) if !self.is_bound_locally(name, scope) => Some(*class),
            _ => None,
        }
    }

    /// Whether `name` is assigned or a parameter in a function scope between
    /// `scope` and the module (which would shadow a class of that name).
    fn is_bound_locally(&self, name: &str, scope: usize) -> bool {
        let mut current = Some(scope);
        while let Some(s) = current {
            if self.scopes[s].kind == ScopeKind::Function && self.binds(s, name) {
                return true;
            }
            current = self.scopes[s].parent;
        }
        false
    }

    /// Whether scope `s` itself binds `name` (assignment, parameter, import
    /// or `def`).
    fn binds(&self, s: usize, name: &str) -> bool {
        let key = (s, name.to_string());
        self.assigns.contains_key(&key)
            || self.params.contains_key(&key)
            || self.imports.contains_key(&key)
            || self.other_imports.contains_key(&key)
            || self.functions.contains_key(&key)
    }

    /// The scope whose binding of `name` is visible from `scope`.
    fn binding_scope(&self, name: &str, scope: usize) -> usize {
        let mut s = scope;
        loop {
            let current = &self.scopes[s];
            match current.kind {
                ScopeKind::Function => {
                    if current.globals.contains(name) {
                        return 0;
                    }
                    if self.binds(s, name) {
                        return s;
                    }
                }
                ScopeKind::Class => {
                    // Only code directly in the class body sees its names.
                    if s == scope && self.assigns.contains_key(&(s, name.to_string())) {
                        return s;
                    }
                }
                ScopeKind::Module => return 0,
            }
            // Methods do not see class-body names: skip enclosing classes.
            let mut parent = current.parent.unwrap_or(0);
            while self.scopes[parent].kind == ScopeKind::Class {
                parent = self.scopes[parent].parent.unwrap_or(0);
            }
            s = parent;
        }
    }

    /// Enters a nested name resolution; `false` past [`MAX_NAME_DEPTH`].
    fn enter_name(&mut self) -> bool {
        if self.name_depth >= MAX_NAME_DEPTH {
            self.cutoffs += 1;
            return false;
        }
        self.name_depth += 1;
        true
    }

    fn leave_name(&mut self) {
        self.name_depth -= 1;
    }

    /// Name resolutions that may still nest below this point.
    fn budget(&self) -> u32 {
        MAX_NAME_DEPTH.saturating_sub(self.name_depth)
    }

    /// A value [`recall`] found; reusing a cut-off value makes the current
    /// evaluation cut off too.
    fn reuse<V>(&mut self, found: Option<(V, bool)>) -> Option<V> {
        let (val, cut) = found?;
        if cut {
            self.cutoffs += 1;
        }
        Some(val)
    }

    /// `val` as computed with `budget`, final unless a resolution was cut
    /// off since `cutoffs` was read.
    fn memo<V>(&self, val: V, cutoffs: u64, budget: u32) -> Memo<V> {
        Memo {
            val,
            budget: (self.cutoffs != cutoffs).then_some(budget),
        }
    }

    /// The value of `name` used at byte `pos` of `scope`.
    fn resolve_name(&mut self, name: &str, scope: usize, pos: usize) -> Val {
        if !self.enter_name() {
            return Val::untracked(Origin::Other);
        }
        let (key, at) = self.locate(name, scope, pos);
        let val = match self.reaching(&key, at) {
            Reach::All => self.resolve_all(key),
            Reach::Subset(defs) => {
                let vals = self.def_values(&key, &defs);
                if vals.is_empty() {
                    self.no_value(&key, &defs)
                } else {
                    combine(vals)
                }
            }
        };
        self.leave_name();
        val
    }

    /// Every definition of `key` combined, memoized.
    fn resolve_all(&mut self, key: Key) -> Val {
        let budget = self.budget();
        if let Some(val) = self.reuse(recall(&self.name_memo, &key, budget)) {
            return val;
        }
        let guard = (0u8, key.0, key.1.clone());
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }
        let cutoffs = self.cutoffs;
        let defs = self.all_defs(&key);
        let vals = self.def_values(&key, &defs);
        let val = if vals.is_empty() {
            self.no_value(&key, &defs)
        } else {
            combine(vals)
        };
        self.visiting.remove(&guard);
        let memo = self.memo(val.clone(), cutoffs, budget);
        self.name_memo.insert(key, memo);
        val
    }

    /// Values of the tracked imports, parameter and assignments among `defs`.
    /// Other imports matter only when nothing else binds the name.
    fn def_values(&mut self, key: &Key, defs: &[DefRef]) -> Vec<Val> {
        let mut vals = Vec::new();
        for def in defs {
            match *def {
                DefRef::Import(i) => {
                    if let Some(import) = self.imports.get(key).and_then(|v| v.get(i)) {
                        vals.push(import.val.clone());
                    }
                }
                DefRef::Other(_) => {}
                DefRef::Param => {
                    if self.params.contains_key(key) {
                        vals.push(self.param_value_memo(key));
                    }
                }
                DefRef::Assign(i) => {
                    if let Some(assign) = self.assigns.get(key).and_then(|v| v.get(i)).copied() {
                        vals.push(self.assign_value_memo(assign));
                    }
                }
            }
        }
        vals
    }

    fn no_value(&self, key: &Key, defs: &[DefRef]) -> Val {
        let module = defs.iter().find_map(|def| match def {
            DefRef::Other(i) => self
                .other_imports
                .get(key)
                .and_then(|v| v.get(*i))
                .map(|o| o.module.clone()),
            _ => None,
        });
        match module {
            Some(module) => Val::untracked(Origin::Imported {
                module: clip_owned(module),
            }),
            None => Val::untracked(Origin::Unbound),
        }
    }

    fn assign_value_memo(&mut self, assign: Assign<'t>) -> Val {
        let budget = self.budget();
        if let Some(val) = self.reuse(recall(&self.def_memo, &assign.id, budget)) {
            return val;
        }
        let guard = (2u8, assign.id, String::new());
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }
        let cutoffs = self.cutoffs;
        let val = self.assign_value(assign);
        self.visiting.remove(&guard);
        let memo = self.memo(val.clone(), cutoffs, budget);
        self.def_memo.insert(assign.id, memo);
        val
    }

    /// The value of parameter `key.1` of function `key.0`, memoized.
    fn param_value_memo(&mut self, key: &Key) -> Val {
        let budget = self.budget();
        if let Some(val) = self.reuse(recall(&self.param_memo, key, budget)) {
            return val;
        }
        let Some(param) = self.params.get(key).copied() else {
            return Val::untracked(Origin::Unbound);
        };
        let guard = (4u8, key.0, key.1.clone());
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }
        let cutoffs = self.cutoffs;
        let val = self.param_value(&key.1, param, key.0);
        self.visiting.remove(&guard);
        let memo = self.memo(val.clone(), cutoffs, budget);
        self.param_memo.insert(key.clone(), memo);
        val
    }

    fn param_value(&mut self, name: &str, param: Param<'t>, scope: usize) -> Val {
        if let Some(annotation) = param.annotation {
            if let Some(val) = self.typed(annotation, scope) {
                return val;
            }
        }
        // A client default holds only where a caller leaves the argument out.
        let mut default_client = None;
        if let Some(default) = param.default {
            let parent = self.scopes[scope].parent.unwrap_or(0);
            let val = self.eval(default, parent, 0);
            if matches!(val, Val::Client { .. }) {
                default_client = Some(val);
            }
        }
        if let Some(val) = self.param_from_callers(scope, name, default_client.clone()) {
            return val;
        }
        default_client.unwrap_or_else(|| {
            Val::untracked(Origin::Parameter {
                name: name.to_string(),
            })
        })
    }

    /// What the in-file calls of `func` pass for parameter `name`, memoized.
    /// With a client default (`default_client`), every caller's argument is
    /// combined with it: a caller passing another client makes the binding
    /// ambiguous. Without one, the client every caller passes. `None` when
    /// the calls cannot all be seen or (without a default) they disagree.
    fn param_from_callers(
        &mut self,
        func: usize,
        name: &str,
        default_client: Option<Val>,
    ) -> Option<Val> {
        let key = (func, name.to_string());
        let budget = self.budget();
        if let Some(found) = self.reuse(recall(&self.caller_memo, &key, budget)) {
            return found;
        }
        let guard = (3u8, func, name.to_string());
        if !self.visiting.insert(guard.clone()) {
            return Some(Val::untracked(Origin::Cycle));
        }
        let cutoffs = self.cutoffs;
        if !self.enter_name() {
            self.visiting.remove(&guard);
            return None;
        }
        let found = self.callers_agree(func, name, default_client);
        self.leave_name();
        self.visiting.remove(&guard);
        let memo = self.memo(found.clone(), cutoffs, budget);
        self.caller_memo.insert(key, memo);
        found
    }

    fn callers_agree(
        &mut self,
        func: usize,
        name: &str,
        default_client: Option<Val>,
    ) -> Option<Val> {
        let vals = self.caller_vals(func, name)?;
        if vals.is_empty() {
            return None;
        }
        let private = self.is_private(func);
        if let Some(default) = default_client {
            // Other modules may call a public function without the
            // argument, so its default stays possible.
            let mut all = vals;
            if !private {
                all.push(default);
            }
            return Some(combine(all));
        }
        let first = vals.first()?.clone();
        let agree = vals.iter().all(|v| *v == first);
        match first {
            Val::Client { path, .. } if path.is_empty() => match combine(vals) {
                Val::Client { mut client, path } if path.is_empty() && !client.ambiguous => {
                    if !private && client.caveat.is_none() {
                        client.caveat = Some(ClientCaveat::InFileCallers {
                            param: name.to_string(),
                        });
                    }
                    Some(Val::Client { client, path })
                }
                _ => None,
            },
            Val::Module { .. } | Val::Symbol { .. } | Val::Class(_) | Val::GoogleNs if agree => {
                Some(first)
            }
            _ => None,
        }
    }

    /// The value each in-file call of `func` passes for parameter `name`
    /// (the default where a call leaves it out; unknown through `*args` or
    /// `**kwargs`), self-references dropped. `None` when the calls cannot
    /// all be seen or a call leaves out a parameter that has no default.
    fn caller_vals(&mut self, func: usize, name: &str) -> Option<Vec<Val>> {
        let slot = self.param_slot(func, name)?;
        let calls = self.calls_of(func)?;
        let default = self
            .params
            .get(&(func, name.to_string()))
            .and_then(|p| p.default);
        let keyword = slot.keyword.then_some(name);
        let mut vals = Vec::new();
        for &(call, call_scope) in calls.iter() {
            let val = match argument_for(call, slot.position, keyword, |n| self.text(n)) {
                ArgFor::Found(expr) => self.eval(expr, call_scope, 0),
                ArgFor::Missing => self.eval(default?, slot.parent, 0),
                ArgFor::Unknown => Val::untracked(Origin::Other),
            };
            if !matches!(val, Val::Untracked(Origin::Cycle)) {
                vals.push(val);
            }
        }
        Some(vals)
    }

    /// How a call passes parameter `name` of `func`: its position among
    /// the explicit arguments (not for keyword-only parameters), whether it
    /// may be passed by keyword, and the scope its default is evaluated in.
    /// `None` for `self`/`cls`, `*args` and `**kwargs`.
    fn param_slot(&self, func: usize, name: &str) -> Option<ParamSlot> {
        let scope = &self.scopes[func];
        let parent = scope.parent?;
        let method = self.scopes[parent].kind == ScopeKind::Class && !scope.lambda;
        let shift = usize::from(method && !scope.staticmethod);
        let index = scope.slots.iter().position(|s| s.name == name)?;
        let slot = scope.slots[index].kind;
        if index < shift || matches!(slot, SlotKind::VarArgs | SlotKind::VarKwargs) {
            return None;
        }
        Some(ParamSlot {
            position: matches!(slot, SlotKind::Positional | SlotKind::PositionalOnly)
                .then(|| index - shift),
            keyword: slot != SlotKind::PositionalOnly,
            parent,
        })
    }

    /// Whether every call of `func` can be in this file: a function nested
    /// in another function or lambda, a `_private` (not dunder) function or
    /// method, or anything in a script (an extensionless file or
    /// `__main__.py`), which other modules do not import.
    fn is_private(&self, func: usize) -> bool {
        if self.script {
            return true;
        }
        let scope = &self.scopes[func];
        let Some(parent) = scope.parent else {
            return false;
        };
        if self.scopes[parent].kind == ScopeKind::Function {
            return true;
        }
        let name = match &scope.bound_name {
            Some(name) if scope.lambda => name.as_str(),
            _ => scope.name.as_str(),
        };
        name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__"))
    }

    /// Every in-file call of function or method `func`, or `None` when some
    /// may not be seen: it is decorated or a lambda not assigned to a plain
    /// name, its name is used as a value or bound more than once, or (for a
    /// method) it is called on something other than `self`/`cls` of its
    /// class. Calls are indexed by binding scope (by class for methods) once
    /// per name, so a file with thousands of same-named functions stays
    /// linear.
    fn calls_of(&mut self, func: usize) -> Option<CallSites<'t>> {
        let scope = &self.scopes[func];
        if scope.decorated || scope.comprehension {
            return None;
        }
        let lambda = scope.lambda;
        let fname = if lambda {
            scope.bound_name.clone()?
        } else {
            scope.name.clone()
        };
        let parent = scope.parent?;
        let empty = || Rc::from(Vec::new());
        if self.scopes[parent].kind == ScopeKind::Class {
            if lambda || self.attr_uses.get(fname.as_str())?.escapes {
                return None;
            }
            let index = self.method_calls(&fname);
            if index.foreign {
                return None;
            }
            return match index.by_class.len() {
                0 => Some(empty()),
                1 => match index.by_class.get(&parent) {
                    Some(calls) => Some(Rc::clone(calls)),
                    // Only another class calls a method of this name on
                    // `self`: it may inherit this one.
                    None => None,
                },
                _ => None,
            };
        }
        let home = self.import_home(parent, &fname);
        let key = (home, fname.clone());
        let defined_once = if lambda {
            self.assigns.get(&key).map_or(0, Vec::len) == 1 && !self.functions.contains_key(&key)
        } else {
            self.functions.get(&key).map_or(0, Vec::len) == 1 && !self.assigns.contains_key(&key)
        };
        let only_def = defined_once
            && !self.imports.contains_key(&key)
            && !self.other_imports.contains_key(&key)
            && !self.params.contains_key(&key);
        if !only_def || self.name_uses.get(fname.as_str())?.escapes {
            return None;
        }
        let index = self.name_calls(&fname);
        Some(index.get(&home).map_or_else(empty, Rc::clone))
    }

    /// The calls of bare name `name`, grouped by the scope that binds the
    /// name at each call. Built once per name.
    fn name_calls(&mut self, name: &str) -> CallIndex<'t> {
        if let Some(index) = self.name_call_index.get(name) {
            return Rc::clone(index);
        }
        let mut grouped: HashMap<usize, Vec<(Node<'t>, usize)>> = HashMap::new();
        if let Some(uses) = self.name_uses.get(name) {
            for &(call, call_scope) in &uses.calls {
                let home = self.binding_scope(name, call_scope);
                grouped.entry(home).or_default().push((call, call_scope));
            }
        }
        let index: CallIndex<'t> = Rc::new(
            grouped
                .into_iter()
                .map(|(home, calls)| (home, Rc::from(calls)))
                .collect(),
        );
        self.name_call_index
            .insert(name.to_string(), Rc::clone(&index));
        index
    }

    /// The calls of method name `name` on `self`/`cls`, grouped by the
    /// calling class. Built once per name.
    fn method_calls(&mut self, name: &str) -> Rc<MethodCalls<'t>> {
        if let Some(index) = self.method_call_index.get(name) {
            return Rc::clone(index);
        }
        let mut foreign = false;
        let mut grouped: HashMap<usize, Vec<(Node<'t>, usize)>> = HashMap::new();
        if let Some(uses) = self.attr_uses.get(name) {
            for &(call, call_scope) in &uses.calls {
                let object = call
                    .child_by_field_name("function")
                    .and_then(|f| f.child_by_field_name("object"));
                let class = self.scopes[call_scope].class;
                match (object, class) {
                    (Some(object), Some(class))
                        if object.kind() == "identifier"
                            && matches!(self.text(object), "self" | "cls") =>
                    {
                        grouped.entry(class).or_default().push((call, call_scope));
                    }
                    _ => foreign = true,
                }
            }
        }
        let index = Rc::new(MethodCalls {
            foreign,
            by_class: grouped
                .into_iter()
                .map(|(class, calls)| (class, Rc::from(calls)))
                .collect(),
        });
        self.method_call_index
            .insert(name.to_string(), Rc::clone(&index));
        index
    }

    fn assign_value(&mut self, assign: Assign<'t>) -> Val {
        if assign.unknowable {
            return Val::untracked(Origin::Other);
        }
        let value = assign.value.map(|v| self.eval(v, assign.scope, 0));
        if let Some(val @ Val::Client { .. }) = &value {
            return val.clone();
        }
        if let Some(annotation) = assign.annotation {
            if let Some(val) = self.typed(annotation, assign.scope) {
                return val;
            }
        }
        value.unwrap_or(Val::untracked(Origin::Placeholder))
    }

    /// A client typed by an annotation (`client: OpenAI`,
    /// `Optional[AzureOpenAI]`).
    fn typed(&mut self, annotation: Node<'t>, scope: usize) -> Option<Val> {
        // Each entry: a node and whether its parent is an attribute.
        let mut stack = vec![(annotation, false)];
        while let Some((node, in_attribute)) = stack.pop() {
            let top_chain = matches!(node.kind(), "identifier" | "attribute") && !in_attribute;
            if top_chain {
                if let Val::Class(class) = self.eval(node, scope, 0) {
                    return Some(Val::Client {
                        client: Client {
                            pkg: class.pkg,
                            routing: class.typed_routing(),
                            line: None,
                            site: None,
                            ambiguous: false,
                            typed: true,
                            caveat: None,
                        },
                        path: Vec::new(),
                    });
                }
                continue;
            }
            let is_attribute = node.kind() == "attribute";
            for child in named_children(node).into_iter().rev() {
                stack.push((child, is_attribute));
            }
        }
        None
    }

    /// All assignments to attribute `attr` of `class`: class-body names and
    /// `self.attr = ...`, falling back to same-file base classes. Each class
    /// is visited once per lookup and the answer is memoized, so a file
    /// whose classes all list each other as bases stays cheap.
    fn attr_assignments(&mut self, class: usize, attr: &str) -> Rc<[Assign<'t>]> {
        let key = (class, attr.to_string());
        if let Some(found) = self.attr_lookup_memo.get(&key) {
            return Rc::clone(found);
        }
        let mut visited = HashSet::new();
        let found: Rc<[Assign<'t>]> = self
            .attr_assignments_in(class, attr, 0, &mut visited)
            .into();
        self.attr_lookup_memo.insert(key, Rc::clone(&found));
        found
    }

    fn attr_assignments_in(
        &self,
        class: usize,
        attr: &str,
        depth: u32,
        visited: &mut HashSet<usize>,
    ) -> Vec<Assign<'t>> {
        if !visited.insert(class) {
            return Vec::new();
        }
        let key = (class, attr.to_string());
        let mut found: Vec<Assign<'t>> = self.assigns.get(&key).cloned().unwrap_or_default();
        found.extend(self.attr_assigns.get(&key).cloned().unwrap_or_default());
        if !found.is_empty() || depth >= MAX_BASE_DEPTH {
            return found;
        }
        for base in self.scopes[class].bases.iter().take(MAX_BASES) {
            if visited.len() >= MAX_CLASSES_VISITED {
                break;
            }
            if let Some([base_class]) = self.classes_by_name.get(base).map(Vec::as_slice) {
                let inherited = self.attr_assignments_in(*base_class, attr, depth + 1, visited);
                if !inherited.is_empty() {
                    return inherited;
                }
            }
        }
        found
    }

    fn resolve_attr(&mut self, class: usize, attr: &str) -> Val {
        let key = (class, attr.to_string());
        let budget = self.budget();
        if let Some(val) = self.reuse(recall(&self.attr_memo, &key, budget)) {
            return val;
        }
        let guard = (1u8, class, attr.to_string());
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }
        let cutoffs = self.cutoffs;
        if !self.enter_name() {
            self.visiting.remove(&guard);
            return Val::untracked(Origin::Other);
        }
        let assigns = self.attr_assignments(class, attr);
        let val = if assigns.is_empty() {
            Val::untracked(Origin::UnassignedAttribute {
                name: attr.to_string(),
            })
        } else {
            let vals = assigns.iter().map(|a| self.assign_value_memo(*a)).collect();
            combine(vals)
        };
        self.leave_name();
        self.visiting.remove(&guard);
        let memo = self.memo(val.clone(), cutoffs, budget);
        self.attr_memo.insert(key, memo);
        val
    }

    fn member(&mut self, base: Val, attr: &str) -> Val {
        if base.path().len() >= MAX_PATH {
            return Val::untracked(Origin::Other);
        }
        let clipped = clip(attr);
        let attr = clipped.as_str();
        match base {
            Val::Module { pkg, mut path } => {
                if let Some(class) = client_class(pkg, attr) {
                    return Val::Class(class);
                }
                path.push(attr.to_string());
                Val::Module { pkg, path }
            }
            Val::GoogleNs => match attr {
                "genai" => Val::Module {
                    pkg: Pkg::GoogleGenai,
                    path: Vec::new(),
                },
                "generativeai" => Val::Module {
                    pkg: Pkg::GoogleGenerativeai,
                    path: Vec::new(),
                },
                _ => Val::untracked(Origin::Other),
            },
            Val::Class(class) => Val::Symbol {
                pkg: class.pkg,
                path: vec![class.name.to_string(), attr.to_string()],
            },
            Val::Symbol { pkg, mut path } => {
                path.push(attr.to_string());
                Val::Symbol { pkg, path }
            }
            Val::Client { client, mut path } => {
                path.push(attr.to_string());
                Val::Client { client, path }
            }
            Val::Result(pkg) => Val::Result(pkg),
            Val::Untracked(origin) => Val::Untracked(origin),
        }
    }

    fn apply_call(&mut self, callee: Val, call: Node<'t>, scope: usize, depth: u32) -> Val {
        match callee {
            Val::Class(class) => Val::Client {
                client: self.construction(call, class),
                path: Vec::new(),
            },
            Val::Client { client, path } => {
                let pkg = client.pkg;
                let joined = path.join(".");
                match joined.as_str() {
                    // Copies of the same client, re-routed by their options.
                    "with_options" | "copy" => Val::Client {
                        client: self.copied_client(client, call, scope),
                        path: Vec::new(),
                    },
                    // Chat sessions that keep the client's routing.
                    "start_chat" | "chats.create" | "aio.chats.create" => Val::Client {
                        client,
                        path: Vec::new(),
                    },
                    _ => Val::Result(Some(pkg)),
                }
            }
            Val::Module { pkg, .. } | Val::Symbol { pkg, .. } => Val::Result(Some(pkg)),
            Val::Result(pkg) => Val::Result(pkg),
            Val::GoogleNs => Val::untracked(Origin::Other),
            Val::Untracked(_) => {
                let callee = call
                    .child_by_field_name("function")
                    .map(|f| self.render_links(f, 16))
                    .unwrap_or_else(|| "<expr>".to_string());
                let wraps = self.wrapped_pkg(call, scope, depth);
                Val::untracked(Origin::Factory { callee, wraps })
            }
        }
    }

    /// `client.with_options(...)` / `client.copy(...)`: both SDKs accept
    /// `base_url` there, so a direct client given one is a custom route, and
    /// options passed through an unreadable `**kwargs` make it unknown.
    fn copied_client(&mut self, mut client: Client, call: Node<'t>, scope: usize) -> Client {
        let args = self.arguments(call);
        let mut base_url = args.kwargs.iter().any(|k| is_base_url_key(k));
        let mut opaque = false;
        for splat in &args.splats {
            match self.expand_splat(*splat, scope) {
                Some(expansion) => {
                    base_url |= expansion
                        .summary
                        .expansion
                        .iter()
                        .any(|(k, _)| is_base_url_key(k));
                    opaque |= expansion.opaque();
                }
                None => opaque = true,
            }
        }
        let direct = matches!(
            client.routing,
            Routing::OpenaiDirect | Routing::AnthropicDirect
        );
        if direct && base_url {
            client.routing = Routing::CustomBaseUrl;
        } else if direct && opaque {
            client.routing = Routing::Unknown;
        }
        client
    }

    /// The SDK of a tracked client or constructor passed to a call
    /// (`instructor.from_openai(OpenAI())`).
    fn wrapped_pkg(&mut self, call: Node<'t>, scope: usize, depth: u32) -> Option<Pkg> {
        let args = call.child_by_field_name("arguments")?;
        for arg in named_children(args) {
            let expr = if arg.kind() == "keyword_argument" {
                arg.child_by_field_name("value")
            } else {
                Some(arg)
            };
            if let Some(expr) = expr {
                if let val @ (Val::Client { .. } | Val::Class(_)) =
                    self.eval(expr, scope, depth + 1)
                {
                    return val.pkg();
                }
            }
        }
        None
    }

    /// The client built by `call`, recorded once.
    fn construction(&mut self, call: Node<'t>, class: ClientClass) -> Client {
        let site = Some(call.id());
        if let Some(&index) = self.constructions.get(&call.id()) {
            let built = &self.out.clients[index];
            let kwargs = built.kwargs.clone();
            return Client {
                pkg: class.pkg,
                routing: built.routing,
                line: Some(built.span.line),
                site,
                ambiguous: false,
                typed: false,
                caveat: self.endpoint_env(class, &kwargs),
            };
        }
        let scope = self.call_scope.get(&call.id()).copied().unwrap_or(0);
        let args = self.arguments(call);
        let mut routing = class.routing(&args.kwargs, args.splat);
        let caveat = self.endpoint_env(class, &args.kwargs);
        if caveat.is_some() {
            routing = Routing::Unknown;
        }
        let (binding, binding_scope) = self.binding_target(call, scope);
        let chain = call
            .child_by_field_name("function")
            .map(|f| self.render(f))
            .unwrap_or_default();
        let span = self.span(call);
        let record = ClientConstruction {
            sdk: class.pkg.sdk(),
            package: class.pkg.package().to_string(),
            constructor: format!("{}.{}", class.pkg.package(), class.name),
            chain,
            binding,
            scope: binding_scope,
            routing,
            is_async: class.is_async(),
            kwargs: args.kwargs,
            splat: args.splat,
            file: self.file.clone(),
            span,
            symbol: self.symbol(scope),
        };
        self.constructions.insert(call.id(), self.out.clients.len());
        self.out.clients.push(record);
        Client {
            pkg: class.pkg,
            routing,
            line: Some(span.line),
            site,
            ambiguous: false,
            typed: false,
            caveat,
        }
    }

    /// The endpoint variable this file sets for a client built by `class`
    /// with keyword arguments `kwargs`, when the constructor does not name
    /// its own endpoint: the SDK reads the variable instead.
    fn endpoint_env(&self, class: ClientClass, kwargs: &[String]) -> Option<ClientCaveat> {
        let own = match class.family {
            Family::OpenaiDirect | Family::AnthropicDirect => {
                kwargs.iter().any(|k| is_base_url_key(k))
            }
            _ => return None,
        };
        if own {
            return None;
        }
        ENDPOINT_ENV
            .iter()
            .find(|(pkg, var)| *pkg == class.pkg && self.env_writes.contains(*var))
            .map(|(_, var)| ClientCaveat::EndpointEnv { var })
    }

    /// The name a constructed client is assigned to, and how it is held.
    fn binding_target(&self, call: Node<'t>, scope: usize) -> (Option<String>, ClientScope) {
        let mut child = call;
        let mut parent = self.parents.get(&call.id()).copied();
        while let Some(p) = parent {
            if matches!(
                p.kind(),
                "parenthesized_expression"
                    | "boolean_operator"
                    | "conditional_expression"
                    | "await"
            ) && p.is_named()
            {
                child = p;
                parent = self.parents.get(&p.id()).copied();
            } else {
                break;
            }
        }
        let Some(parent) = parent else {
            return (None, ClientScope::Inline);
        };
        let target = match parent.kind() {
            "assignment"
                if parent.child_by_field_name("right").map(|r| r.id()) == Some(child.id()) =>
            {
                parent.child_by_field_name("left")
            }
            "as_pattern" => parent
                .child_by_field_name("alias")
                .and_then(|alias| alias.named_child(0)),
            "named_expression" => parent.child_by_field_name("name"),
            _ => None,
        };
        let Some(target) = target else {
            return (None, ClientScope::Inline);
        };
        match target.kind() {
            "identifier" => {
                let name = self.text(target).to_string();
                let held = match self.scopes[scope].kind {
                    ScopeKind::Module => ClientScope::Module,
                    ScopeKind::Class => ClientScope::Class,
                    ScopeKind::Function if self.scopes[scope].globals.contains(&name) => {
                        ClientScope::Module
                    }
                    ScopeKind::Function => ClientScope::Local,
                };
                (Some(clip_owned(name)), held)
            }
            "attribute" => (Some(self.render(target)), ClientScope::Attribute),
            _ => (Some(self.render(target)), ClientScope::Inline),
        }
    }

    // ------------------------------------------------------------------
    // Pass 2: analyze
    // ------------------------------------------------------------------

    fn analyze(&mut self) {
        self.openai_routing = self.module_openai_routing();
        if let Some(var) = OPENAI_MODULE_ENV
            .iter()
            .find(|var| self.env_writes.contains(**var))
        {
            if self.openai_routing == Routing::OpenaiDirect {
                self.openai_routing = Routing::Unknown;
            }
            self.openai_caveat = Some(ClientCaveat::EndpointEnv { var });
        }
        self.note_rerouted_clients();
        for (call, scope) in self.calls.clone() {
            if self.records() >= MAX_FILE_RECORDS {
                return self.truncate_file(call, scope);
            }
            self.analyze_call(call, scope);
        }
        for (reference, scope) in self.refs.clone() {
            if self.records() >= MAX_FILE_RECORDS {
                return self.truncate_file(reference, scope);
            }
            self.analyze_reference(reference, scope);
        }
    }

    /// Calls, gaps and clients reported so far.
    fn records(&self) -> usize {
        self.out.calls.len() + self.out.gaps.len() + self.out.clients.len()
    }

    /// Stops the analysis at `node`: the rest of the file is one gap.
    fn truncate_file(&mut self, node: Node<'t>, scope: usize) {
        self.out.gaps.push(UsageGap {
            kind: GapKind::UnresolvedModel,
            sdk: None,
            file: self.file.clone(),
            span: Some(self.span(node)),
            symbol: self.symbol(scope),
            chain: None,
            key: None,
            reason: "truncated".to_string(),
            detail: format!(
                "this file reports more than {MAX_FILE_RECORDS} calls, clients and gaps; nothing from this line on is analyzed"
            ),
            values: Vec::new(),
        });
    }

    /// Routing of the module-level OpenAI API, from `openai.api_type`,
    /// `openai.api_base` / `base_url` and `openai.azure_endpoint`
    /// assignments anywhere in the file.
    fn module_openai_routing(&mut self) -> Routing {
        let (mut azure, mut unknown, mut custom) = (false, false, false);
        for assign in self.config_assigns.clone() {
            let target = self.eval(assign.object, assign.scope, 0);
            if !matches!(&target, Val::Module { pkg: Pkg::Openai, path } if path.is_empty()) {
                continue;
            }
            let attr = self.text(assign.attr);
            match attr {
                "api_type" => match assign.value.and_then(|v| self.literal(v).ok()) {
                    Some(kind) => match kind.to_ascii_lowercase().as_str() {
                        "azure" | "azure_ad" | "azuread" => azure = true,
                        "openai" | "open_ai" => {}
                        // `amazon-bedrock` and anything newer.
                        _ => unknown = true,
                    },
                    None => unknown = true,
                },
                "azure_endpoint" | "azure_ad_token" | "azure_ad_token_provider" => azure = true,
                "api_base" | "base_url" => custom = true,
                _ if attr.starts_with("bedrock_") => unknown = true,
                _ => {}
            }
        }
        if azure {
            Routing::AzureOpenai
        } else if unknown {
            Routing::Unknown
        } else if custom {
            Routing::CustomBaseUrl
        } else {
            Routing::OpenaiDirect
        }
    }

    /// Clients given a new `base_url` after they are built
    /// (`client.base_url = "https://openrouter.ai/api/v1"`).
    fn note_rerouted_clients(&mut self) {
        for assign in self.config_assigns.clone() {
            if !BASE_URL_ATTRS.contains(&self.text(assign.attr)) {
                continue;
            }
            if let Val::Client { client, path } = self.eval(assign.object, assign.scope, 0) {
                if !path.is_empty() {
                    continue;
                }
                match client.site {
                    Some(site) => {
                        self.rerouted.insert(site);
                    }
                    None => {
                        self.rerouted_pkgs.insert(client.pkg);
                    }
                }
            }
        }
    }

    /// `client` with its routing unknown when a `base_url` is assigned to it
    /// after it is built.
    fn reroute(&self, mut client: Client) -> Client {
        let assigned = client
            .site
            .is_some_and(|site| self.rerouted.contains(&site))
            || (self.rerouted_pkgs.contains(&client.pkg)
                && matches!(
                    client.routing,
                    Routing::OpenaiDirect | Routing::AnthropicDirect
                ));
        if assigned && !client.ambiguous {
            if matches!(
                client.routing,
                Routing::OpenaiDirect | Routing::AnthropicDirect
            ) {
                client.routing = Routing::Unknown;
            }
            client.caveat = Some(ClientCaveat::BaseUrlAssigned);
        }
        client
    }

    /// Why the module-level API of `pkg` may not route as reported.
    fn module_caveat(&self, pkg: Pkg) -> Option<ClientCaveat> {
        match pkg {
            Pkg::Openai => self.openai_caveat.clone(),
            _ => None,
        }
    }

    fn module_routing(&self, pkg: Pkg) -> Routing {
        match pkg {
            Pkg::Openai => self.openai_routing,
            Pkg::Anthropic => Routing::AnthropicDirect,
            Pkg::GoogleGenai => Routing::GoogleGenai,
            Pkg::GoogleGenerativeai => Routing::GoogleGenerativeaiLegacy,
        }
    }

    fn analyze_call(&mut self, call: Node<'t>, scope: usize) {
        let Some(function) = call.child_by_field_name("function") else {
            return;
        };

        if function.kind() == "identifier" && self.text(function) == "getattr" {
            self.check_getattr(call, scope);
            return;
        }

        let callee = self.eval(function, scope, 0);
        match callee {
            Val::Class(class) => {
                self.construction(call, class);
                if class.family == Family::LegacyModel {
                    let receiver = if function.kind() == "attribute" {
                        Receiver::Module
                    } else {
                        Receiver::Symbol
                    };
                    let target = CallTarget {
                        pkg: class.pkg,
                        member: class.name.to_string(),
                        receiver,
                        routing: Routing::GoogleGenerativeaiLegacy,
                        client_line: None,
                        ambiguous: false,
                        caveat: None,
                    };
                    self.record_call(call, function, scope, target);
                }
            }
            Val::Module { pkg, path } if !path.is_empty() => {
                let target = CallTarget {
                    pkg,
                    member: path.join("."),
                    receiver: Receiver::Module,
                    routing: self.module_routing(pkg),
                    client_line: None,
                    ambiguous: false,
                    caveat: self.module_caveat(pkg),
                };
                self.record_call(call, function, scope, target);
            }
            Val::Symbol { pkg, path } => {
                let target = CallTarget {
                    pkg,
                    member: path.join("."),
                    receiver: Receiver::Symbol,
                    routing: self.module_routing(pkg),
                    client_line: None,
                    ambiguous: false,
                    caveat: self.module_caveat(pkg),
                };
                self.record_call(call, function, scope, target);
            }
            Val::Client { client, path } if !path.is_empty() => {
                let client = self.reroute(client);
                let target = CallTarget {
                    pkg: client.pkg,
                    member: path.join("."),
                    receiver: Receiver::Client,
                    routing: client.routing,
                    client_line: client.line,
                    ambiguous: client.ambiguous,
                    caveat: client.caveat,
                };
                self.record_call(call, function, scope, target);
            }
            Val::Untracked(origin) => self.check_unknown_receiver(call, function, scope, origin),
            Val::Result(pkg) => {
                let origin = Origin::SdkResult { pkg };
                self.check_unknown_receiver(call, function, scope, origin);
            }
            _ => {}
        }
    }

    fn record_call(
        &mut self,
        call: Node<'t>,
        function: Node<'t>,
        scope: usize,
        target: CallTarget,
    ) {
        let chain = self.render(function);
        let span = self.span(call);
        let symbol = self.symbol(scope);
        let args = self.arguments(call);
        let sdk = target.pkg.sdk();
        let member = clip_owned(strip_wrappers(&target.member));
        let request = is_request(&member);
        let model_bearing = is_model_bearing(&member);

        // An unresolved model matters where it reaches a request, a model
        // object, or any call on a client or SDK module; on an SDK type
        // (`ChatCompletion(model=model)`) the call still lists `model` in
        // its kwargs, without a gap.
        let on_sdk_object = matches!(target.receiver, Receiver::Client | Receiver::Module)
            && member.split('.').next() != Some("types");
        let gap_worthy = request || model_bearing || on_sdk_object;
        let site = Site {
            sdk,
            call,
            chain: &chain,
            scope,
            gap_worthy,
        };

        let gaps_before = self.out.gaps.len();
        let mut values = Vec::new();
        let mut keyed = false;
        for (key, expr) in &args.value_args {
            keyed = true;
            let result = self.resolve_value(*expr, scope, 0);
            self.take_value(&site, key, false, result, &mut values);
        }

        // `GenerativeModel("gemini-pro")`, `genai.embed_content("models/...", text)`:
        // the model is positional.
        if !keyed {
            if let (Some(key), Some(first)) = (
                positional_model_key(target.pkg, target.receiver, &member),
                args.first_positional,
            ) {
                keyed = true;
                let result = self.resolve_value(first, scope, 0);
                let site = Site {
                    gap_worthy: true,
                    ..site
                };
                self.take_value(&site, key, true, result, &mut values);
            }
        }

        // Model keys nested in literal arguments share one read budget per
        // call; what an expanded dict nests is read once per dict.
        let mut read = NestedRead::new();
        for (keyword, expr) in &args.literal_args {
            let prefix = keyword
                .as_ref()
                .map(|k| format!("{k}."))
                .unwrap_or_default();
            self.nested_values_in(*expr, &prefix, keyword.is_none(), scope, 0, &mut read);
        }
        let mut nested = read.found;
        let mut nested_cut = read.cut;

        // `**PARAMS` where `PARAMS` is a same-file dict literal.
        let mut opaque_splat = false;
        let mut expanded: Vec<usize> = Vec::new();
        for splat in &args.splats {
            let Some(expansion) = self.expand_splat(*splat, scope) else {
                opaque_splat = true;
                continue;
            };
            opaque_splat |= expansion.opaque();
            // `f(**P, **P)` is a TypeError; read each dict once.
            if expanded.contains(&expansion.node.id()) {
                continue;
            }
            expanded.push(expansion.node.id());
            for (key, value) in expansion.summary.expansion.iter() {
                if !is_value_key(key) {
                    continue;
                }
                keyed = true;
                let label = expansion.name.as_ref().map(|n| format!("{n}[\"{key}\"]"));
                let result = self
                    .resolve_value(*value, expansion.scope, 0)
                    .map(|mut r| {
                        if r.origin == ValueOrigin::Literal {
                            r.origin = expansion.origin;
                            r.name = label.clone();
                        }
                        r
                    })
                    .map_err(|u| match &label {
                        Some(label) => u.via(label),
                        None => u,
                    });
                self.take_value(&site, key, false, result, &mut values);
            }
            let list = self.dict_nested(&expansion);
            nested.extend(list.found.iter().cloned());
            nested_cut |= list.cut;
        }
        self.take_nested(&site, nested, nested_cut, &mut values);

        let model_key = if member == "GenerativeModel" {
            "model_name"
        } else {
            "model"
        };
        if !keyed && opaque_splat && (request || model_bearing) {
            let unresolved = Unresolved::new("splat", "**kwargs".to_string());
            self.unresolved_gap(&site, model_key, &unresolved, None);
        } else if !keyed && model_bearing && !args.list_splat {
            // No model at all: the SDK's or the server's default applies,
            // and it may be one that retires.
            let unresolved = Unresolved::new("missing_model", String::new());
            self.unresolved_gap(&site, model_key, &unresolved, None);
        }

        if target.ambiguous {
            self.out.gaps.push(UsageGap {
                kind: GapKind::AmbiguousClient,
                sdk: Some(sdk),
                file: self.file.clone(),
                span: Some(span),
                symbol: symbol.clone(),
                chain: Some(chain.clone()),
                key: None,
                reason: "multiple_assignments".to_string(),
                detail: format!(
                    "the receiver of `{chain}` is assigned more than one kind of client or a non-client value, so its routing depends on a runtime branch"
                ),
                values: Vec::new(),
            });
        } else if let Some(caveat) = &target.caveat {
            let (reason, detail) = match caveat {
                ClientCaveat::InFileCallers { param } => (
                    "in_file_callers",
                    format!(
                        "the receiver of `{chain}` is parameter `{param}`: every caller in this file passes the same client, but other modules may call the function with another one"
                    ),
                ),
                ClientCaveat::BaseUrlAssigned => (
                    "base_url_assigned",
                    format!(
                        "the client behind `{chain}` is given a `base_url` after it is built, so its requests may go to another endpoint"
                    ),
                ),
                ClientCaveat::EndpointEnv { var } => (
                    "endpoint_env",
                    format!(
                        "this file sets `{var}`, which the SDK reads its endpoint from, so `{chain}` may send its requests elsewhere"
                    ),
                ),
            };
            self.out.gaps.push(UsageGap {
                kind: GapKind::AmbiguousClient,
                sdk: Some(sdk),
                file: self.file.clone(),
                span: Some(span),
                symbol: symbol.clone(),
                chain: Some(chain.clone()),
                key: None,
                reason: reason.to_string(),
                detail,
                values: Vec::new(),
            });
        }

        // The same value or gap reached twice (one dict expanded twice, one
        // fallback behind two names) is listed once.
        let mut seen_values = HashSet::new();
        values.retain(|v| seen_values.insert((v.key.clone(), v.span.start_byte, v.value.clone())));
        let mut seen_gaps = HashSet::new();
        for gap in self.out.gaps.split_off(gaps_before) {
            if seen_gaps.insert((
                gap.kind,
                gap.key.clone(),
                gap.reason.clone(),
                gap.detail.clone(),
            )) {
                self.out.gaps.push(gap);
            }
        }

        self.out.calls.push(SdkCall {
            sdk,
            package: target.pkg.package().to_string(),
            chain,
            request,
            member,
            receiver: target.receiver,
            routing: target.routing,
            client_line: target.client_line,
            file: self.file.clone(),
            span,
            symbol,
            kwargs: args.kwargs,
            positional: args.positional,
            splat: args.splat,
            values,
        });
    }

    /// Attaches a resolved value to `key`, or reports why it did not
    /// resolve. A fallback captured from an environment or config lookup is
    /// both a value and a gap.
    fn take_value(
        &mut self,
        site: &Site<'_, 't>,
        key: &str,
        positional: bool,
        result: Result<Resolved, Unresolved>,
        values: &mut Vec<CapturedValue>,
    ) {
        match result {
            // A value whose gap cannot be reported here (an SDK type such as
            // `ChatCompletion(model=os.getenv(...))`) is not captured: it
            // would read as confirmed.
            Ok(resolved) if !resolved.caveats.is_empty() && !site.gap_worthy => {}
            Ok(mut resolved) => {
                let caveats = std::mem::take(&mut resolved.caveats);
                let origin = resolved.origin;
                values.push(captured(key, resolved, positional));
                for caveat in &caveats {
                    self.unresolved_gap(site, key, caveat, Some(origin));
                }
            }
            Err(unresolved) if site.gap_worthy => self.unresolved_gap(site, key, &unresolved, None),
            Err(_) => {}
        }
    }

    /// Attaches nested model values, at most [`MAX_NESTED_VALUES`] per call
    /// and [`MAX_FILE_NESTED_VALUES`] per file. Values cut by either limit,
    /// or a read cut short after model values were found, leave a
    /// `truncated` gap: the call may carry values the report does not list.
    fn take_nested(
        &mut self,
        site: &Site<'_, 't>,
        nested: Vec<NestedValue>,
        read_cut: bool,
        values: &mut Vec<CapturedValue>,
    ) {
        if nested.is_empty() {
            return;
        }
        let mut seen: HashSet<(u32, String)> = HashSet::new();
        let mut listed = 0;
        let mut dropped = false;
        for item in nested {
            if !seen.insert((item.resolved.span.start_byte, item.path.clone())) {
                continue;
            }
            if listed >= MAX_NESTED_VALUES || self.nested_left == 0 {
                dropped = true;
                continue;
            }
            listed += 1;
            self.nested_left -= 1;
            self.take_value(site, &item.path, item.positional, Ok(item.resolved), values);
        }
        if (dropped || read_cut) && site.gap_worthy {
            let chain = site.chain;
            self.out.gaps.push(UsageGap {
                kind: GapKind::UnresolvedModel,
                sdk: Some(site.sdk),
                file: self.file.clone(),
                span: Some(self.span(site.call)),
                symbol: self.symbol(site.scope),
                chain: Some(chain.to_string()),
                key: None,
                reason: "truncated".to_string(),
                detail: format!(
                    "`{chain}` passes more nested model values than are read or listed ({listed} listed); the rest are not reported"
                ),
                values: Vec::new(),
            });
        }
    }

    /// An `unresolved_model` gap for `key` at `site`. `captured` is the
    /// origin of the value captured next to it, when the gap is that
    /// value's caveat.
    fn unresolved_gap(
        &mut self,
        site: &Site<'_, 't>,
        key: &str,
        unresolved: &Unresolved,
        captured: Option<ValueOrigin>,
    ) {
        let chain = site.chain;
        let phrase = unresolved.phrase();
        let shape = &unresolved.shape;
        let mut detail = match unresolved.via.as_deref() {
            _ if unresolved.reason == "splat" => {
                format!("`{key}` for `{chain}` is not passed by name and may come from `**kwargs`")
            }
            _ if unresolved.reason == "missing_model" => {
                format!("`{key}` for `{chain}` is not passed, so the SDK or server default applies")
            }
            Some(via) if !shape.is_empty() && via != shape => {
                format!("`{key}` for `{chain}` comes from `{via}`, which is {phrase} (`{shape}`)")
            }
            Some(via) => format!("`{key}` for `{chain}` comes from `{via}`, which is {phrase}"),
            None => format!("`{key}` for `{chain}` is `{shape}`, which is {phrase}"),
        };
        match captured {
            Some(origin) if unresolved.reason == "parameter_overridden" => {
                detail.push_str(&format!(
                    "; its default is captured with origin `{}` and the values callers pass are listed",
                    origin.as_str()
                ));
            }
            Some(_) => {
                detail.push_str("; its literal fallback is captured with origin `env_default`");
                if !unresolved.candidates.is_empty() {
                    detail.push_str(", and other values it may take are listed");
                }
            }
            None if !unresolved.candidates.is_empty() => {
                detail.push_str("; values it may take are listed")
            }
            None => {}
        }
        let values = self.candidate_values(key, &unresolved.candidates);
        self.out.gaps.push(UsageGap {
            kind: GapKind::UnresolvedModel,
            sdk: Some(site.sdk),
            file: self.file.clone(),
            span: Some(self.span(site.call)),
            symbol: self.symbol(site.scope),
            chain: Some(chain.to_string()),
            key: Some(key.to_string()),
            reason: unresolved.reason.to_string(),
            detail,
            values,
        });
    }

    /// Candidate values listed with a gap for `key`: distinct, at most
    /// [`MAX_CANDIDATES`] per gap and [`MAX_FILE_CANDIDATES`] per file.
    fn candidate_values(&mut self, key: &str, candidates: &[Resolved]) -> Vec<CapturedValue> {
        let mut seen = HashSet::new();
        let mut listed = Vec::new();
        for candidate in candidates {
            if listed.len() >= MAX_CANDIDATES || self.candidates_left == 0 {
                break;
            }
            if !seen.insert(candidate.value.as_str()) {
                continue;
            }
            self.candidates_left -= 1;
            let mut candidate = candidate.clone();
            candidate.caveats.clear();
            listed.push(captured(key, candidate, false));
        }
        listed
    }

    /// `getattr(client, name)` on a tracked binding.
    fn check_getattr(&mut self, call: Node<'t>, scope: usize) {
        let Some(args) = call.child_by_field_name("arguments") else {
            return;
        };
        let Some(first) = args
            .named_child(0)
            .filter(|a| a.kind() != "keyword_argument")
        else {
            return;
        };
        let target = self.eval(first, scope, 0);
        if !target.is_tracked() {
            return;
        }
        let rendered = self.render(first);
        self.out.gaps.push(UsageGap {
            kind: GapKind::DynamicDispatch,
            sdk: target.pkg().map(Pkg::sdk),
            file: self.file.clone(),
            span: Some(self.span(call)),
            symbol: self.symbol(scope),
            chain: Some(format!("getattr({rendered}, ...)")),
            key: None,
            reason: "getattr".to_string(),
            detail: format!(
                "`getattr` on `{rendered}` picks the SDK member at run time; calls through it are not tracked"
            ),
            values: Vec::new(),
        });
    }

    /// An SDK-shaped call on a receiver that is not a tracked client.
    fn check_unknown_receiver(
        &mut self,
        call: Node<'t>,
        function: Node<'t>,
        scope: usize,
        origin: Origin,
    ) {
        if sdk_shape(&self.render_links(function, 8)).is_none() {
            return;
        }
        let chain = self.render(function);
        let Some((shape_sdk, strong)) = sdk_shape(&chain) else {
            return;
        };
        if !strong && !self.sdk_imported.contains(&shape_sdk) {
            return;
        }
        let root = chain_root(&chain);
        let (reason, detail, sdk) = match &origin {
            Origin::Factory { callee, wraps } => (
                "factory_call",
                format!("`{root}` comes from `{callee}`, which is not a tracked SDK constructor"),
                wraps.map(Pkg::sdk).unwrap_or(shape_sdk),
            ),
            Origin::Parameter { name } => (
                "parameter",
                format!("`{root}` comes from parameter `{name}`, which has no SDK type annotation and no in-file caller that passes a known client"),
                shape_sdk,
            ),
            Origin::Imported { module } => (
                "imported_name",
                format!("`{root}` is imported from `{module}`; cross-file resolution is not in this version"),
                shape_sdk,
            ),
            Origin::UnassignedAttribute { name } => (
                "unassigned_attribute",
                format!("`{root}` is never assigned in this class (`{name}`)"),
                shape_sdk,
            ),
            Origin::SdkResult { pkg } => (
                "sdk_call_result",
                format!("`{root}` is returned by an SDK call that is not a known client constructor"),
                pkg.map(Pkg::sdk).unwrap_or(shape_sdk),
            ),
            Origin::Unbound => (
                "unbound",
                format!("`{root}` is not assigned in this file"),
                shape_sdk,
            ),
            _ => (
                "unresolved_receiver",
                format!("the receiver of `{chain}` could not be resolved"),
                shape_sdk,
            ),
        };

        let args = self.arguments(call);
        let mut values = Vec::new();
        for (key, expr) in &args.value_args {
            if let Ok(resolved) = self.resolve_value(*expr, scope, 0) {
                values.push(captured(key, resolved, false));
            }
        }
        self.out.gaps.push(UsageGap {
            kind: GapKind::UnknownClientFactory,
            sdk: Some(sdk),
            file: self.file.clone(),
            span: Some(self.span(call)),
            symbol: self.symbol(scope),
            chain: Some(chain),
            key: None,
            reason: reason.to_string(),
            detail,
            values,
        });
    }

    /// An SDK request method used as a value (`create = client.chat.completions.create`).
    fn analyze_reference(&mut self, reference: Node<'t>, scope: usize) {
        let val = self.eval(reference, scope, 0);
        if !val.is_tracked() {
            return;
        }
        let Some(last) = val.path().last() else {
            return;
        };
        if !REQUEST_METHODS.contains(&last.as_str()) {
            return;
        }
        let rendered = self.render(reference);
        self.out.gaps.push(UsageGap {
            kind: GapKind::DynamicDispatch,
            sdk: val.pkg().map(Pkg::sdk),
            file: self.file.clone(),
            span: Some(self.span(reference)),
            symbol: self.symbol(scope),
            chain: Some(rendered.clone()),
            key: None,
            reason: "method_reference".to_string(),
            detail: format!(
                "`{rendered}` is stored or passed as a value; calls through it are not tracked"
            ),
            values: Vec::new(),
        });
    }

    // ------------------------------------------------------------------
    // Arguments and values
    // ------------------------------------------------------------------

    fn arguments(&self, call: Node<'t>) -> Args<'t> {
        let mut args = Args::default();
        let Some(list) = call.child_by_field_name("arguments") else {
            return args;
        };
        if list.kind() != "argument_list" {
            // A bare generator argument: `f(x for x in y)`.
            args.positional = 1;
            return args;
        }
        for arg in named_children(list) {
            match arg.kind() {
                "keyword_argument" => {
                    let Some(name) = arg.child_by_field_name("name") else {
                        continue;
                    };
                    let name = self.text(name).to_string();
                    if let Some(value) = arg.child_by_field_name("value") {
                        if is_value_key(&name) {
                            args.value_args.push((name.clone(), value));
                        } else if is_container(value) && !is_secret_key(&name) {
                            args.literal_args.push((Some(name.clone()), value));
                        }
                    }
                    args.kwargs.push(name);
                }
                "dictionary_splat" => {
                    args.splat = true;
                    if let Some(inner) = arg.named_child(0) {
                        args.splats.push(inner);
                    }
                }
                "list_splat" => args.list_splat = true,
                "comment" => {}
                _ => {
                    if args.positional == 0 {
                        args.first_positional = Some(arg);
                    }
                    if is_container(arg) {
                        args.literal_args.push((None, arg));
                    }
                    args.positional += 1;
                }
            }
        }
        args
    }

    /// What the containers in a dict literal's `**` expansion nest
    /// (`**{"extra_body": {"model": ...}}`), read once per dict with one
    /// budget and reused by every call that expands it.
    fn dict_nested(&mut self, expansion: &Expansion<'t>) -> Rc<NestedList> {
        if let Some(list) = self.dict_nested.get(&expansion.node.id()) {
            return Rc::clone(list);
        }
        let mut read = NestedRead::new();
        for (key, value) in expansion.summary.expansion.iter() {
            if !is_value_key(key) && is_container(*value) {
                let prefix = format!("{key}.");
                self.nested_values_in(*value, &prefix, false, expansion.scope, 0, &mut read);
            }
        }
        let list = Rc::new(NestedList {
            found: read.found,
            cut: read.cut,
        });
        self.dict_nested
            .insert(expansion.node.id(), Rc::clone(&list));
        list
    }

    /// Allowlisted keys inside dict literals passed to a call
    /// (`fallbacks=[{"model": "claude-opus-4-8"}]`). Only identifier-shaped
    /// keys are followed and only keys [`is_value_key`] accepts are read; an
    /// unresolvable nested value is skipped, not reported, because the
    /// argument may not configure the request at all. `read` bounds the
    /// entries read (see [`MAX_NESTED_ENTRIES`]).
    fn nested_values_in(
        &mut self,
        node: Node<'t>,
        prefix: &str,
        positional: bool,
        scope: usize,
        depth: u32,
        read: &mut NestedRead,
    ) {
        if depth > 4 {
            return;
        }
        let mut cursor = node.walk();
        match node.kind() {
            "dictionary" => {
                for pair in node.named_children(&mut cursor) {
                    if read.left == 0 {
                        read.cut = true;
                        return;
                    }
                    read.left -= 1;
                    if pair.kind() != "pair" {
                        continue;
                    }
                    let (Some(key), Some(value)) = (
                        pair.child_by_field_name("key"),
                        pair.child_by_field_name("value"),
                    ) else {
                        continue;
                    };
                    let Ok(key) = self.literal(key) else {
                        continue;
                    };
                    // Nothing under a credential-shaped or non-identifier key
                    // is read, at any depth (`extra_headers={"auth": {...}}`).
                    if !is_identifier_key(&key) {
                        continue;
                    }
                    let path = format!("{prefix}{key}");
                    if is_value_key(&key) {
                        if let Ok(resolved) = self.resolve_value(value, scope, 0) {
                            read.found.push(NestedValue {
                                path,
                                positional,
                                resolved,
                            });
                        }
                    } else if is_container(value) {
                        let nested = format!("{path}.");
                        self.nested_values_in(value, &nested, positional, scope, depth + 1, read);
                    }
                }
            }
            "list" | "tuple" | "set" | "parenthesized_expression" => {
                for item in node.named_children(&mut cursor) {
                    if read.left == 0 {
                        read.cut = true;
                        return;
                    }
                    read.left -= 1;
                    self.nested_values_in(item, prefix, positional, scope, depth + 1, read);
                }
            }
            _ => {}
        }
    }

    /// A plain string literal's value. `Err` carries the reason it is not.
    /// A literal longer than [`MAX_LITERAL_BYTES`] is never read.
    fn literal(&self, node: Node<'t>) -> Result<String, &'static str> {
        if node.end_byte() - node.start_byte() > MAX_LITERAL_BYTES {
            return Err("non_literal");
        }
        match node.kind() {
            "string" => {
                let mut value = String::new();
                for part in children(node) {
                    match part.kind() {
                        "string_start" => {
                            let prefix = self.text(part).to_ascii_lowercase();
                            if prefix.contains('b') {
                                return Err("non_literal");
                            }
                        }
                        "string_content" => value.push_str(self.text(part)),
                        "interpolation" => return Err("f_string"),
                        _ => {}
                    }
                }
                if value.contains('\\') {
                    return Err("non_literal");
                }
                Ok(value)
            }
            "concatenated_string" => {
                let mut value = String::new();
                for part in named_children(node) {
                    value.push_str(&self.literal(part)?);
                }
                Ok(value)
            }
            _ => Err("non_literal"),
        }
    }

    fn resolve_value(
        &mut self,
        expr: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        if depth > 16 {
            return Err(Unresolved::new("non_literal", self.render(expr)));
        }
        stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
            self.resolve_value_inner(expr, scope, depth)
        })
    }

    fn resolve_value_inner(
        &mut self,
        expr: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        match expr.kind() {
            "string" | "concatenated_string" => match self.literal(expr) {
                Ok(value) if is_capturable_value(&value) => Ok(Resolved {
                    value,
                    origin: ValueOrigin::Literal,
                    name: None,
                    span: self.span(expr),
                    caveats: Vec::new(),
                }),
                Ok(value) if is_credential_value(&value) => {
                    Err(Unresolved::new("credential", self.render(expr)))
                }
                Ok(_) => Err(Unresolved::new("non_literal", self.render(expr))),
                Err(reason) => Err(Unresolved::new(reason, self.render(expr))),
            },
            "parenthesized_expression" => match expr.named_child(0) {
                Some(inner) => self.resolve_value(inner, scope, depth + 1),
                None => Err(Unresolved::new("non_literal", self.render(expr))),
            },
            "identifier" => {
                let name = self.text(expr).to_string();
                self.resolve_constant(&name, scope, expr.start_byte(), depth)
            }
            "attribute" => {
                let rendered = self.render(expr);
                if let (Some(object), Some(attr)) = (
                    expr.child_by_field_name("object"),
                    expr.child_by_field_name("attribute"),
                ) {
                    let attr = self.text(attr).to_string();
                    // `self.secret`, `ApiKeys.openai`, `cfg.credentials.model`.
                    if is_secret_key(&attr) || self.chain_has_secret_name(object) {
                        return Err(Unresolved::new("credential", rendered));
                    }
                    if let Some(class) = self.class_receiver(object, scope) {
                        let assigns = self.attr_assignments(class, &attr);
                        let only = match assigns.as_ref() {
                            [assign] => Some(*assign),
                            _ => None,
                        };
                        return match self.single_constant(
                            only,
                            assigns.len(),
                            ValueOrigin::ClassConstant,
                            depth,
                        ) {
                            Ok(resolved) => {
                                Ok(resolved.through(&rendered, ValueOrigin::ClassConstant))
                            }
                            Err(unresolved) if unresolved.reason == "reassigned" => {
                                let candidates = self.assign_candidates(
                                    &assigns,
                                    ValueOrigin::ClassConstant,
                                    &rendered,
                                    depth,
                                );
                                Err(unresolved.via(&rendered).with_candidates(candidates))
                            }
                            Err(unresolved) => Err(unresolved.via(&rendered)),
                        };
                    }
                }
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "attribute"
                };
                Err(Unresolved::new(reason, rendered))
            }
            "call" => {
                if let Some(result) = self.fallback_value(expr, scope, depth) {
                    return result;
                }
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "function_call"
                };
                Err(Unresolved::new(reason, self.render(expr)))
            }
            "subscript" => {
                if let Some(result) = self.subscript_value(expr, scope, depth) {
                    return result;
                }
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "subscript"
                };
                // `MODELS[os.getenv("TIER")]`: the dict's values are the
                // candidates.
                let candidates = match expr.child_by_field_name("value") {
                    Some(base) => match self.dict_node(base, scope, 0) {
                        Some(dict) => self.dict_candidates(&dict).to_vec(),
                        None => Vec::new(),
                    },
                    None => Vec::new(),
                };
                Err(Unresolved::new(reason, self.render(expr)).with_candidates(candidates))
            }
            "boolean_operator" => {
                if let Some(result) = self.or_fallback(expr, scope, depth) {
                    return result;
                }
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "conditional"
                };
                Err(Unresolved::new(reason, self.render(expr)))
            }
            "conditional_expression" => {
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "conditional"
                };
                Err(Unresolved::new(reason, self.render(expr)))
            }
            _ => Err(Unresolved::new("non_literal", self.render(expr))),
        }
    }

    /// Whether a name written in the attribute chain `node` looks like a
    /// credential's (`ApiKeys` in `ApiKeys.openai`, `keys` in
    /// `cfg.keys.openai`). `self` and `cls` are not names of their own.
    fn chain_has_secret_name(&self, node: Node<'t>) -> bool {
        let mut node = node;
        for _ in 0..MAX_DEPTH {
            match node.kind() {
                "identifier" => return is_secret_key(self.text(node)),
                "attribute" => {
                    let secret = node
                        .child_by_field_name("attribute")
                        .is_some_and(|a| is_secret_key(self.text(a)));
                    match node.child_by_field_name("object") {
                        Some(object) if !secret => node = object,
                        _ => return secret,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn resolve_constant(
        &mut self,
        name: &str,
        scope: usize,
        pos: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        // A value that flows through a secret-named binding is never read.
        if is_secret_key(name) {
            return Err(Unresolved::new("credential", name.to_string()));
        }
        let (key, defs) = self.defs_at(name, scope, pos);
        let home = key.0;
        let imported = defs.imported;
        if defs.param {
            if defs.assigns > 0 || imported {
                let reaching = self.reaching_assigns(name, scope, pos);
                let origin = constant_origin(self.scopes[home].kind);
                let candidates = self.assign_candidates(&reaching, origin, name, depth);
                return Err(
                    Unresolved::new("reassigned", name.to_string()).with_candidates(candidates)
                );
            }
            let default = self.params.get(&key).and_then(|p| p.default);
            let Some(default) = default else {
                // What in-file callers pass, for triage.
                let passed = self.caller_models(home, name);
                return Err(Unresolved::new("parameter", name.to_string())
                    .with_candidates(passed.values.clone()));
            };
            // Defaults are evaluated where the function is defined.
            let parent = self.scopes[home].parent.unwrap_or(0);
            return match self.resolve_value(default, parent, depth + 1) {
                Ok(mut resolved) => {
                    if resolved.origin != ValueOrigin::EnvDefault {
                        resolved.origin = ValueOrigin::ParameterDefault;
                        resolved.name = Some(name.to_string());
                    }
                    resolved.caveats = resolved
                        .caveats
                        .into_iter()
                        .map(|caveat| caveat.via(name))
                        .collect();
                    // In-file callers that pass another value make the
                    // default one of several.
                    if let Some(caveat) = self.overridden(home, name, &resolved.value) {
                        resolved.caveats.push(caveat);
                    }
                    Ok(resolved)
                }
                Err(unresolved) if unresolved.reason == "credential" => Err(unresolved.via(name)),
                Err(unresolved) => {
                    let passed = self.caller_models(home, name);
                    Err(Unresolved::new("parameter", name.to_string())
                        .with_candidates(unresolved.candidates)
                        .with_candidates(passed.values.clone()))
                }
            };
        }
        if defs.assigns > 0 {
            if imported {
                return Err(Unresolved::new("reassigned", name.to_string()));
            }
            let origin = constant_origin(self.scopes[home].kind);
            let only = defs
                .only_assign
                .and_then(|i| self.assigns.get(&key).and_then(|v| v.get(i)).copied());
            return match self.single_constant(only, defs.assigns, origin, depth) {
                Ok(resolved) => Ok(resolved.through(name, origin)),
                // Assigned in more than one branch: what each assigns.
                Err(unresolved) if unresolved.reason == "reassigned" => {
                    let reaching = self.reaching_assigns(name, scope, pos);
                    let candidates = self.assign_candidates(&reaching, origin, name, depth);
                    Err(unresolved.via(name).with_candidates(candidates))
                }
                Err(unresolved) => Err(unresolved.via(name)),
            };
        }
        if imported {
            Err(Unresolved::new("imported_name", name.to_string()))
        } else {
            Err(Unresolved::new("unbound", name.to_string()))
        }
    }

    /// The model values in-file calls of `func` pass for parameter `name`,
    /// read once per parameter (at most [`MAX_CALLERS_READ`] calls). Empty
    /// while candidates are being collected for another lookup.
    fn caller_models(&mut self, func: usize, name: &str) -> Rc<CallerModels> {
        let key = (func, name.to_string());
        if let Some(found) = self.caller_models.get(&key) {
            return Rc::clone(found);
        }
        if self.collecting > 0 {
            return Rc::new(CallerModels::default());
        }
        let mut models = CallerModels::default();
        if let (Some(slot), Some(calls)) = (self.param_slot(func, name), self.calls_of(func)) {
            let keyword = slot.keyword.then_some(name);
            self.collecting += 1;
            for (i, &(call, call_scope)) in calls.iter().enumerate() {
                if i >= MAX_CALLERS_READ {
                    models.unknown = true;
                    break;
                }
                match argument_for(call, slot.position, keyword, |n| self.text(n)) {
                    ArgFor::Found(expr) => match self.resolve_value(expr, call_scope, 0) {
                        Ok(resolved) => push_candidate(&mut models.values, resolved),
                        Err(_) => models.unknown = true,
                    },
                    ArgFor::Missing => {}
                    ArgFor::Unknown => models.unknown = true,
                }
            }
            self.collecting -= 1;
        }
        let models = Rc::new(models);
        self.caller_models.insert(key, Rc::clone(&models));
        models
    }

    /// A `parameter_overridden` caveat when an in-file call passes `name`
    /// a value other than its `default` (or one that does not resolve).
    fn overridden(&mut self, func: usize, name: &str, default: &str) -> Option<Unresolved> {
        let passed = self.caller_models(func, name);
        let others: Vec<Resolved> = passed
            .values
            .iter()
            .filter(|r| r.value != default)
            .cloned()
            .collect();
        if others.is_empty() && !passed.unknown {
            return None;
        }
        Some(Unresolved::new("parameter_overridden", name.to_string()).with_candidates(others))
    }

    /// The assignments of `name` that reach a use at byte `pos` of `scope`,
    /// at most [`MAX_CANDIDATE_READS`].
    fn reaching_assigns(&self, name: &str, scope: usize, pos: usize) -> Vec<Assign<'t>> {
        let (key, at) = self.locate(name, scope, pos);
        let Some(assigns) = self.assigns.get(&key) else {
            return Vec::new();
        };
        match self.reaching(&key, at) {
            Reach::All => assigns.iter().take(MAX_CANDIDATE_READS).copied().collect(),
            Reach::Subset(defs) => defs
                .iter()
                .filter_map(|def| match def {
                    DefRef::Assign(i) => assigns.get(*i).copied(),
                    _ => None,
                })
                .take(MAX_CANDIDATE_READS)
                .collect(),
        }
    }

    /// The values `assigns` give a name, for a `reassigned` gap: each
    /// assignment resolved on its own (without collecting candidates of
    /// its own), distinct, at most [`MAX_CANDIDATES`].
    fn assign_candidates(
        &mut self,
        assigns: &[Assign<'t>],
        origin: ValueOrigin,
        name: &str,
        depth: u32,
    ) -> Vec<Resolved> {
        if self.collecting > 0 {
            return Vec::new();
        }
        self.collecting += 1;
        let mut found = Vec::new();
        for assign in assigns.iter().take(MAX_CANDIDATE_READS) {
            let Some(value) = assign.value.filter(|_| !assign.unknowable) else {
                continue;
            };
            if let Ok(mut resolved) = self.resolve_value(value, assign.scope, depth + 1) {
                if resolved.origin == ValueOrigin::Literal {
                    resolved.origin = origin;
                }
                push_candidate(&mut found, resolved.through(name, origin));
            }
        }
        self.collecting -= 1;
        found
    }

    /// The values a dict literal's entries hold, for a lookup whose key is
    /// known only at run time: the first [`MAX_CANDIDATE_READS`] entries
    /// under identifier keys (never under a secret-named or
    /// credential-shaped key), distinct, at most [`MAX_CANDIDATES`]. Read
    /// once per dict.
    fn dict_candidates(&mut self, dict: &DictRef<'t>) -> Rc<[Resolved]> {
        let key = (dict.node.id(), dict.name.clone());
        if let Some(found) = self.dict_candidates_memo.get(&key) {
            return Rc::clone(found);
        }
        if self.collecting > 0 {
            return Rc::from(Vec::new());
        }
        self.collecting += 1;
        let mut found = Vec::new();
        let entries = dict_entries(dict.node, |n| self.literal(n).ok(), |n| self.text(n));
        for (entry_key, value) in entries.into_iter().take(MAX_CANDIDATE_READS).flatten() {
            if !is_identifier_key(&entry_key) {
                continue;
            }
            if let Ok(resolved) = self.dict_entry(dict, &entry_key, value, 0) {
                push_candidate(&mut found, resolved);
            }
        }
        self.collecting -= 1;
        let found: Rc<[Resolved]> = Rc::from(found);
        self.dict_candidates_memo.insert(key, Rc::clone(&found));
        found
    }

    /// A name or attribute assigned exactly once (`only`, out of `count`
    /// assignments), to something that itself resolves to a string.
    fn single_constant(
        &mut self,
        only: Option<Assign<'t>>,
        count: usize,
        origin: ValueOrigin,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        match (only, count) {
            (_, 0) => Err(Unresolved::new("unbound", String::new())),
            (Some(assign), 1) => {
                let Some(value) = assign.value.filter(|_| !assign.unknowable) else {
                    return Err(Unresolved::new("non_literal", String::new()));
                };
                let mut resolved = self.resolve_value(value, assign.scope, depth + 1)?;
                if resolved.origin == ValueOrigin::Literal {
                    resolved.origin = origin;
                }
                Ok(resolved)
            }
            (_, many) => Err(Unresolved::new("reassigned", format!("{many} assignments"))),
        }
    }

    /// The literal fallback of an environment or config lookup, or an exact
    /// lookup in a same-file dict literal (`MODELS.get("fast", "gpt-4o")`).
    /// `None` when `call` is neither.
    fn fallback_value(
        &mut self,
        call: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Option<Result<Resolved, Unresolved>> {
        let function = call.child_by_field_name("function")?;
        let rendered = self.render_links(function, 4);
        let dict_get = function.kind() == "attribute"
            && function
                .child_by_field_name("attribute")
                .is_some_and(|a| self.text(a) == "get");
        let lookup = dict_get
            || CONFIG_LOOKUPS.contains(&rendered.as_str())
            || matches!(
                rendered.as_str(),
                "os.getenv"
                    | "getenv"
                    | "os.environ.get"
                    | "environ.get"
                    | "os.environ.setdefault"
                    | "environ.setdefault"
                    | "getattr"
            );
        // Lookups take a key and a default: a call with many arguments is
        // something else, and its arguments are not scanned.
        let few_args = call
            .child_by_field_name("arguments")
            .is_some_and(|a| a.named_child_count() <= MAX_LOOKUP_ARGS);
        if !lookup || !few_args {
            return None;
        }
        let (positional, keywords) = call_arguments(call, |n| self.text(n));
        let keyword = |name: &str| {
            keywords
                .iter()
                .find(|(k, _)| k.as_str() == name)
                .map(|(_, v)| *v)
        };
        let mut candidates: Vec<Resolved> = Vec::new();
        let (lookup_key, default, reason) = match rendered.as_str() {
            "os.getenv"
            | "getenv"
            | "os.environ.get"
            | "environ.get"
            | "os.environ.setdefault"
            | "environ.setdefault" => (
                positional.first().copied(),
                positional.get(1).copied().or_else(|| keyword("default")),
                "env_var",
            ),
            _ if CONFIG_LOOKUPS.contains(&rendered.as_str()) => (
                positional.first().copied(),
                keyword("default").or_else(|| positional.get(1).copied()),
                "env_var",
            ),
            "getattr" => (
                positional.get(1).copied(),
                positional.get(2).copied(),
                "attribute",
            ),
            _ if function.kind() == "attribute"
                && function
                    .child_by_field_name("attribute")
                    .is_some_and(|a| self.text(a) == "get") =>
            {
                let key = positional.first().copied();
                let default = positional.get(1).copied().or_else(|| keyword("default"));
                if let Some(object) = function.child_by_field_name("object") {
                    if let Some(dict) = self.dict_node(object, scope, 0) {
                        let get = DictGet {
                            call,
                            key,
                            default,
                            scope,
                        };
                        if let Some(found) = self.dict_get_value(&dict, get, depth) {
                            return Some(found);
                        }
                        // A key known only at run time: the dict's values
                        // are the candidates.
                        candidates = self.dict_candidates(&dict).to_vec();
                    }
                }
                let reason = if self.contains_env(function) {
                    "env_var"
                } else {
                    "function_call"
                };
                (key, default, reason)
            }
            _ => return None,
        };
        let shape = self.render(call);
        // The name being looked up must not be a credential's.
        if let Some(lookup_key) = lookup_key {
            let looked_up = self
                .literal(lookup_key)
                .unwrap_or_else(|_| self.text(lookup_key).to_string());
            if is_secret_key(&looked_up) {
                return Some(Err(Unresolved::new("credential", shape)));
            }
        }
        let Some(default) = default else {
            if candidates.is_empty() {
                return None;
            }
            return Some(Err(
                Unresolved::new(reason, shape).with_candidates(candidates)
            ));
        };
        Some(match self.resolve_value(default, scope, depth + 1) {
            Ok(mut resolved) => {
                resolved.origin = ValueOrigin::EnvDefault;
                resolved
                    .caveats
                    .push(Unresolved::new(reason, shape).with_candidates(candidates));
                Ok(resolved)
            }
            Err(unresolved) if unresolved.reason == "credential" => Err(unresolved),
            Err(_) => Err(Unresolved::new(reason, shape).with_candidates(candidates)),
        })
    }

    /// `os.getenv("MODEL") or "gpt-4o"`: the right side is the fallback.
    fn or_fallback(
        &mut self,
        expr: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Option<Result<Resolved, Unresolved>> {
        let operator = expr.child_by_field_name("operator")?;
        if self.text(operator) != "or" {
            return None;
        }
        let left = expr.child_by_field_name("left")?;
        let right = expr.child_by_field_name("right")?;
        let lookup = self.contains_env(left)
            || (left.kind() == "call"
                && left
                    .child_by_field_name("function")
                    .is_some_and(|f| CONFIG_LOOKUPS.contains(&self.render(f).as_str())));
        if !lookup {
            return None;
        }
        match self.resolve_value(right, scope, depth + 1) {
            Ok(mut resolved) => {
                resolved.origin = ValueOrigin::EnvDefault;
                resolved
                    .caveats
                    .push(Unresolved::new("env_var", self.render(left)));
                Some(Ok(resolved))
            }
            Err(unresolved) if unresolved.reason == "credential" => Some(Err(unresolved)),
            Err(_) => None,
        }
    }

    /// `MODELS["fast"]` where `MODELS` is a same-file dict literal.
    fn subscript_value(
        &mut self,
        expr: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Option<Result<Resolved, Unresolved>> {
        let base = expr.child_by_field_name("value")?;
        let index = single_subscript(expr)?;
        let key = self.literal(index).ok()?;
        if is_secret_key(&key) {
            return Some(Err(Unresolved::new("credential", self.render(expr))));
        }
        let dict = self.dict_node(base, scope, 0)?;
        match self.dict_lookup(dict.node, &key) {
            Lookup::Found(value) => Some(self.dict_entry(&dict, &key, value, depth)),
            Lookup::Absent | Lookup::Unknown => None,
        }
    }

    /// The value of `dict.get(key, default)` on a same-file dict literal:
    /// the entry when the key is there, the default when it certainly is not.
    /// The default is an argument of the call, so it is resolved where the
    /// call runs, not where the dict is written.
    fn dict_get_value(
        &mut self,
        dict: &DictRef<'t>,
        get: DictGet<'t>,
        depth: u32,
    ) -> Option<Result<Resolved, Unresolved>> {
        let key = self.literal(get.key?).ok()?;
        if is_secret_key(&key) {
            // The key is a literal: name the call, never the key.
            return Some(Err(Unresolved::new("credential", self.render(get.call))));
        }
        match self.dict_lookup(dict.node, &key) {
            Lookup::Found(value) => Some(self.dict_entry(dict, &key, value, depth)),
            Lookup::Absent => {
                let default = get.default?;
                Some(self.resolve_value(default, get.scope, depth + 1))
            }
            Lookup::Unknown => None,
        }
    }

    fn dict_entry(
        &mut self,
        dict: &DictRef<'t>,
        key: &str,
        value: Node<'t>,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        let label = dict.name.as_ref().map(|name| {
            if is_identifier_key(key) {
                format!("{name}[\"{key}\"]")
            } else {
                format!("{name}[...]")
            }
        });
        self.resolve_value(value, dict.scope, depth + 1)
            .map(|mut r| {
                if r.origin == ValueOrigin::Literal {
                    r.origin = dict.origin;
                    r.name = label.clone();
                }
                r
            })
            .map_err(|u| match &label {
                Some(label) => u.via(label),
                None => u,
            })
    }

    /// The dict literal `expr` evaluates to: a `{...}` or `dict(...)`
    /// written inline, a name assigned exactly one (and never mutated or
    /// passed on, see [`Ctx::escaped`]), or a
    /// constant key of one of those.
    fn dict_node(&mut self, expr: Node<'t>, scope: usize, depth: u32) -> Option<DictRef<'t>> {
        if depth > 8 {
            return None;
        }
        match expr.kind() {
            "dictionary" => Some(DictRef {
                node: expr,
                scope,
                name: None,
                origin: ValueOrigin::Literal,
            }),
            "call" if is_dict_call(expr, |n| self.text(n)) => Some(DictRef {
                node: expr,
                scope,
                name: None,
                origin: ValueOrigin::Literal,
            }),
            "parenthesized_expression" => {
                let inner = expr.named_child(0)?;
                self.dict_node(inner, scope, depth + 1)
            }
            "identifier" => {
                let name = self.text(expr);
                if is_secret_key(name) || self.escaped.contains(name) {
                    return None;
                }
                let (key, defs) = self.defs_at(name, scope, expr.start_byte());
                if defs.param || defs.imported || defs.assigns != 1 {
                    return None;
                }
                let assign = self.assigns.get(&key)?.get(defs.only_assign?).copied()?;
                if assign.unknowable {
                    return None;
                }
                let mut dict = self.dict_node(assign.value?, assign.scope, depth + 1)?;
                if dict.name.is_none() {
                    dict.name = Some(name.to_string());
                    dict.origin = constant_origin(self.scopes[key.0].kind);
                }
                Some(dict)
            }
            "subscript" => {
                let base = expr.child_by_field_name("value")?;
                let key = self.literal(single_subscript(expr)?).ok()?;
                if !is_identifier_key(&key) {
                    return None;
                }
                let outer = self.dict_node(base, scope, depth + 1)?;
                let Lookup::Found(value) = self.dict_lookup(outer.node, &key) else {
                    return None;
                };
                let mut dict = self.dict_node(value, outer.scope, depth + 1)?;
                if dict.name.is_none() {
                    dict.name = outer.name.map(|n| format!("{n}[\"{key}\"]"));
                    dict.origin = outer.origin;
                }
                Some(dict)
            }
            _ => None,
        }
    }

    /// The entries of a dict expanded with `**` that can matter.
    fn expand_splat(&mut self, expr: Node<'t>, scope: usize) -> Option<Expansion<'t>> {
        let dict = self.dict_node(expr, scope, 0)?;
        let summary = self.dict_summary(dict.node);
        Some(Expansion {
            node: dict.node,
            summary,
            scope: dict.scope,
            name: dict.name,
            origin: dict.origin,
        })
    }

    /// Looks `key` up in a dict literal: the last matching entry wins, as in
    /// Python, unless an unreadable entry after it could override it.
    fn dict_lookup(&mut self, dict: Node<'t>, key: &str) -> Lookup<'t> {
        let summary = self.dict_summary(dict);
        match summary.last.get(key) {
            Some((index, value)) if summary.last_unknown.is_none_or(|u| u < *index) => {
                Lookup::Found(*value)
            }
            Some(_) => Lookup::Unknown,
            None if summary.last_unknown.is_some() => Lookup::Unknown,
            None => Lookup::Absent,
        }
    }

    fn dict_summary(&mut self, dict: Node<'t>) -> Rc<DictSummary<'t>> {
        if let Some(summary) = self.dict_cache.get(&dict.id()) {
            return Rc::clone(summary);
        }
        let mut summary = DictSummary {
            last: HashMap::new(),
            last_unknown: None,
            expansion: Vec::new(),
            opaque: false,
        };
        let entries = dict_entries(dict, |n| self.literal(n).ok(), |n| self.text(n));
        for (index, entry) in entries.into_iter().enumerate() {
            let Some((key, value)) = entry else {
                summary.last_unknown = Some(index);
                summary.opaque = true;
                continue;
            };
            // A credential-named or non-identifier key cannot be a model
            // argument; it is never read through an expansion.
            let relevant = is_identifier_key(&key)
                && (is_value_key(&key) || is_base_url_key(&key) || is_container(value));
            if relevant {
                if let Some(slot) = summary.expansion.iter_mut().find(|(k, _)| *k == key) {
                    slot.1 = value;
                } else if summary.expansion.len() < 256 {
                    summary.expansion.push((key.clone(), value));
                } else {
                    summary.opaque = true;
                }
            }
            summary.last.insert(key, (index, value));
        }
        let summary = Rc::new(summary);
        self.dict_cache.insert(dict.id(), Rc::clone(&summary));
        summary
    }

    /// Whether `node` reads the process environment
    /// (`os.getenv(...)`, `os.environ[...]`, `os.environ.get(...)`): a
    /// binary search over the reads recorded during collection, so asking
    /// it of every nested call in a deep expression stays linear.
    fn contains_env(&self, node: Node<'t>) -> bool {
        let first = self
            .env_reads
            .partition_point(|&start| start < node.start_byte());
        self.env_reads
            .get(first)
            .is_some_and(|&start| start < node.end_byte())
    }

    /// An expression's shape with every argument and literal elided, so a
    /// rendering can never carry a value. The spine (attribute objects, call
    /// functions, subscript values) is walked iteratively, at most
    /// [`MAX_DEPTH`] links deep, so rendering is linear in the chain.
    fn render(&self, node: Node<'t>) -> String {
        self.render_links(node, MAX_DEPTH)
    }

    /// [`Self::render`] keeping at most `links` links of the spine.
    fn render_links(&self, node: Node<'t>, links: u32) -> String {
        let mut suffixes: Vec<String> = Vec::new();
        let mut current = Some(node);
        let mut depth = 0;
        let base = loop {
            let Some(node) = current else {
                break String::new();
            };
            if depth > links {
                break "<expr>".to_string();
            }
            depth += 1;
            match node.kind() {
                "identifier" => break self.text(node).to_string(),
                "attribute" => {
                    let attr = node
                        .child_by_field_name("attribute")
                        .map(|a| self.text(a))
                        .unwrap_or_default();
                    suffixes.push(format!(".{attr}"));
                    current = node.child_by_field_name("object");
                }
                "call" => {
                    suffixes.push(if self.has_args(node) { "(...)" } else { "()" }.to_string());
                    current = node.child_by_field_name("function");
                }
                "subscript" => {
                    suffixes.push("[...]".to_string());
                    current = node.child_by_field_name("value");
                }
                "parenthesized_expression" | "await" => match node.named_child(0) {
                    Some(inner) => current = Some(inner),
                    None => break "<expr>".to_string(),
                },
                "string" => {
                    let interpolated = children(node).iter().any(|c| c.kind() == "interpolation");
                    break if interpolated { "f\"...\"" } else { "\"...\"" }.to_string();
                }
                "concatenated_string" => break "\"...\"".to_string(),
                "conditional_expression" => break "<conditional>".to_string(),
                "boolean_operator" => break "<boolean>".to_string(),
                _ => break "<expr>".to_string(),
            }
        };
        let mut rendered = base;
        for suffix in suffixes.iter().rev() {
            rendered.push_str(suffix);
        }
        clip_owned(rendered)
    }

    fn has_args(&self, call: Node<'t>) -> bool {
        call.child_by_field_name("arguments")
            .is_some_and(|args| args.named_child_count() > 0)
    }

    fn finish(mut self) -> FileUsages {
        self.out.imports.sort_by_key(|i| i.span.start_byte);
        self.out
            .clients
            .sort_by_key(|c| (c.span.start_byte, c.span.end_byte));
        // A call chained on another (`m(...).start_chat()`) starts at the same
        // byte; the inner, shorter one comes first.
        self.out
            .calls
            .sort_by_key(|c| (c.span.start_byte, c.span.end_byte));
        self.out
            .gaps
            .sort_by_key(|g| (g.span.map(|s| s.start_byte).unwrap_or(0), g.kind));
        self.out
    }
}

/// `text`, or its first and last bytes around `...` when it is longer than
/// [`MAX_TEXT`].
fn clip(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_string();
    }
    let half = (MAX_TEXT - 3) / 2;
    let mut head = half;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - half;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}...{}", &text[..head], &text[tail..])
}

/// [`clip`] for an owned string, which is returned as is when short.
fn clip_owned(text: String) -> String {
    if text.len() <= MAX_TEXT {
        text
    } else {
        clip(&text)
    }
}

/// Whether `file` is a script other modules do not import: no extension
/// (a shebang script) or `__main__.py`.
fn is_script_path(file: &str) -> bool {
    let path = std::path::Path::new(file);
    path.extension().is_none()
        || path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("__main__.py"))
}

/// What an import statement binds, for [`SdkImport`].
struct Binding<'a> {
    local: &'a str,
    pkg: Pkg,
    module: &'a str,
    name: Option<String>,
    kind: BindingKind,
}

/// Everything the analysis needs to know about a call's target.
struct CallTarget {
    pkg: Pkg,
    member: String,
    receiver: Receiver,
    routing: Routing,
    client_line: Option<u32>,
    ambiguous: bool,
    caveat: Option<ClientCaveat>,
}

/// The call a value or gap is reported for.
#[derive(Clone, Copy)]
struct Site<'a, 't> {
    sdk: Sdk,
    call: Node<'t>,
    chain: &'a str,
    scope: usize,
    /// Whether an unresolved value here is reported as a gap.
    gap_worthy: bool,
}

#[derive(Default)]
struct Args<'t> {
    kwargs: Vec<String>,
    positional: u32,
    splat: bool,
    first_positional: Option<Node<'t>>,
    /// Allowlisted keyword arguments and their value expressions.
    value_args: Vec<(String, Node<'t>)>,
    /// Other arguments that are dict/list/tuple literals, with the keyword
    /// they were passed under (`None` when positional).
    literal_args: Vec<(Option<String>, Node<'t>)>,
    /// The expressions passed with `**`.
    splats: Vec<Node<'t>>,
    /// Arguments were spread with `*`.
    list_splat: bool,
}

/// Adds `resolved` to `values` unless its value is already there, keeping at
/// most [`MAX_CANDIDATES`].
fn push_candidate(values: &mut Vec<Resolved>, mut resolved: Resolved) {
    if values.len() < MAX_CANDIDATES && values.iter().all(|v| v.value != resolved.value) {
        resolved.caveats.clear();
        values.push(resolved);
    }
}

fn captured(key: &str, resolved: Resolved, positional: bool) -> CapturedValue {
    let family = model_family(&resolved.value);
    CapturedValue {
        key: key.to_string(),
        value: resolved.value,
        origin: resolved.origin,
        name: resolved.name.map(clip_owned),
        positional,
        span: resolved.span,
        model_family: family,
    }
}

fn constant_origin(kind: ScopeKind) -> ValueOrigin {
    match kind {
        ScopeKind::Module => ValueOrigin::ModuleConstant,
        ScopeKind::Class => ValueOrigin::ClassConstant,
        ScopeKind::Function => ValueOrigin::LocalConstant,
    }
}

/// The key a call's first positional argument is captured under, when that
/// argument is the model.
fn positional_model_key(pkg: Pkg, receiver: Receiver, member: &str) -> Option<&'static str> {
    if member == "GenerativeModel" {
        return Some("model_name");
    }
    let last = member.rsplit('.').next().unwrap_or(member);
    (pkg == Pkg::GoogleGenerativeai
        && receiver != Receiver::Client
        && LEGACY_POSITIONAL_MODEL.contains(&last))
    .then_some("model")
}

/// A dict, list, tuple or set literal.
fn is_container(node: Node<'_>) -> bool {
    matches!(node.kind(), "dictionary" | "list" | "tuple" | "set")
}

fn is_model_bearing(member: &str) -> bool {
    MODEL_BEARING
        .iter()
        .any(|m| member == *m || member.ends_with(&format!(".{m}")))
}

/// Methods that read a dict or list without changing it or handing out
/// its entries.
const READ_ONLY_METHODS: &[&str] = &[
    "get",
    "keys",
    "__contains__",
    "__len__",
    "__getitem__",
    "count",
    "index",
];

/// Methods that hand out a dict's entries (or a shallow copy of them).
const SHALLOW_METHODS: &[&str] = &["items", "values", "copy", "__iter__"];

/// Methods that change a dict, list or set in place.
const MUTATING_METHODS: &[&str] = &[
    "update",
    "setdefault",
    "pop",
    "popitem",
    "clear",
    "__setitem__",
    "__delitem__",
    "__ior__",
    "append",
    "extend",
    "insert",
    "remove",
    "sort",
    "reverse",
    "add",
    "discard",
];

/// Calls that only read their argument.
const READ_ONLY_CALLS: &[&str] = &[
    "len",
    "print",
    "repr",
    "str",
    "sorted",
    "list",
    "tuple",
    "set",
    "frozenset",
    "iter",
    "bool",
    "isinstance",
    "type",
    "id",
    "hash",
    "enumerate",
    "any",
    "all",
    "json.dumps",
    "copy.deepcopy",
    "deepcopy",
];

/// Calls that return a shallow copy, sharing nested containers.
const SHALLOW_CALLS: &[&str] = &["dict", "copy.copy", "copy"];

/// Whether a field of a node of kind `parent` names a binding rather than
/// using one: a definition's name, a parameter, an import, a keyword
/// argument's name, an attribute name or an assignment target.
fn is_declaration(parent: &str, field: Option<&str>) -> bool {
    matches!(
        (parent, field),
        ("function_definition" | "class_definition", Some("name"))
            | ("keyword_argument", Some("name"))
            | ("attribute", Some("attribute"))
            | (
                "default_parameter" | "typed_default_parameter",
                Some("name")
            )
            | ("assignment" | "augmented_assignment", Some("left"))
            | (
                "parameters"
                    | "lambda_parameters"
                    | "typed_parameter"
                    | "list_splat_pattern"
                    | "dictionary_splat_pattern"
                    | "dotted_name"
                    | "aliased_import"
                    | "import_from_statement"
                    | "global_statement"
                    | "nonlocal_statement",
                _,
            )
    )
}

/// The name at the root of a subscript, attribute or `.get` chain
/// (`CFG` in `CFG["a"]["b"]`, `H` in `H.d["k"]`, `D` in `D.get("a")["b"]`).
fn chain_root_name<'t>(mut node: Node<'t>, text: impl Fn(Node<'t>) -> &'t str) -> Option<&'t str> {
    for _ in 0..MAX_DEPTH {
        node = match node.kind() {
            "identifier" => return Some(text(node)),
            "subscript" => node.child_by_field_name("value")?,
            "attribute" => node.child_by_field_name("object")?,
            "call" => node.child_by_field_name("function")?,
            "parenthesized_expression" => node.named_child(0)?,
            _ => return None,
        };
    }
    None
}

/// `x.get(...)`.
fn is_get_call<'t>(call: Node<'t>, text: impl Fn(Node<'t>) -> &'t str) -> bool {
    call.child_by_field_name("function")
        .filter(|f| f.kind() == "attribute")
        .and_then(|f| f.child_by_field_name("attribute"))
        .is_some_and(|a| text(a) == "get")
}

/// A dict, list or set literal, or a `dict(...)`/`list(...)` call: a
/// container that can be changed in place. Strings, numbers and tuples of
/// them cannot.
fn is_mutable_literal<'t>(node: Node<'t>, text: impl Fn(Node<'t>) -> &'t str + Copy) -> bool {
    match node.kind() {
        "dictionary"
        | "list"
        | "set"
        | "dictionary_comprehension"
        | "list_comprehension"
        | "set_comprehension" => true,
        "tuple" | "parenthesized_expression" => named_children(node)
            .into_iter()
            .any(|n| is_mutable_literal(n, text)),
        "call" => node
            .child_by_field_name("function")
            .is_some_and(|f| matches!(text(f), "dict" | "list" | "set")),
        _ => false,
    }
}

/// `dict(model=..., ...)`.
fn is_dict_call<'t>(call: Node<'t>, text: impl Fn(Node<'t>) -> &'t str) -> bool {
    call.child_by_field_name("function")
        .is_some_and(|f| f.kind() == "identifier" && text(f) == "dict")
}

/// The one index of `x[i]`, if there is exactly one.
fn single_subscript(expr: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = expr.walk();
    let mut indexes = expr.children_by_field_name("subscript", &mut cursor);
    let first = indexes.next()?;
    indexes.next().is_none().then_some(first)
}

/// The entries of a dict literal or `dict(...)` call: `Some((key, value))`
/// for a string or keyword key, `None` for a part that cannot be read
/// (`**other`, a computed key, a positional argument to `dict`).
fn dict_entries<'t>(
    dict: Node<'t>,
    literal: impl Fn(Node<'t>) -> Option<String>,
    text: impl Fn(Node<'t>) -> &'t str,
) -> Vec<Option<(String, Node<'t>)>> {
    let mut entries = Vec::new();
    if dict.kind() == "dictionary" {
        for item in named_children(dict) {
            match item.kind() {
                "pair" => {
                    let key = item.child_by_field_name("key").and_then(&literal);
                    match (key, item.child_by_field_name("value")) {
                        (Some(key), Some(value)) => entries.push(Some((key, value))),
                        _ => entries.push(None),
                    }
                }
                "comment" => {}
                _ => entries.push(None),
            }
        }
    } else if let Some(args) = dict.child_by_field_name("arguments") {
        if args.kind() != "argument_list" {
            entries.push(None);
            return entries;
        }
        for arg in named_children(args) {
            match arg.kind() {
                "keyword_argument" => {
                    match (
                        arg.child_by_field_name("name"),
                        arg.child_by_field_name("value"),
                    ) {
                        (Some(name), Some(value)) => {
                            entries.push(Some((text(name).to_string(), value)))
                        }
                        _ => entries.push(None),
                    }
                }
                "comment" => {}
                _ => entries.push(None),
            }
        }
    }
    entries
}

/// Positional and keyword arguments of a call, as written.
fn call_arguments<'t>(
    call: Node<'t>,
    text: impl Fn(Node<'t>) -> &'t str,
) -> (Vec<Node<'t>>, Vec<(String, Node<'t>)>) {
    let mut positional = Vec::new();
    let mut keywords = Vec::new();
    let Some(args) = call
        .child_by_field_name("arguments")
        .filter(|a| a.kind() == "argument_list")
    else {
        return (positional, keywords);
    };
    for arg in named_children(args) {
        match arg.kind() {
            "keyword_argument" => {
                if let (Some(name), Some(value)) = (
                    arg.child_by_field_name("name"),
                    arg.child_by_field_name("value"),
                ) {
                    keywords.push((text(name).to_string(), value));
                }
            }
            "list_splat" | "dictionary_splat" | "comment" => {}
            _ => positional.push(arg),
        }
    }
    (positional, keywords)
}

/// The argument a call passes for a parameter at `position` (counting from
/// the first explicit argument) or named `keyword`.
fn argument_for<'t>(
    call: Node<'t>,
    position: Option<usize>,
    keyword: Option<&str>,
    text: impl Fn(Node<'t>) -> &'t str,
) -> ArgFor<'t> {
    let Some(args) = call.child_by_field_name("arguments") else {
        return ArgFor::Missing;
    };
    if args.kind() != "argument_list" {
        return ArgFor::Unknown;
    }
    let mut index = 0;
    let mut star = false;
    let mut double_star = false;
    let mut found = None;
    for arg in named_children(args) {
        match arg.kind() {
            "keyword_argument" => {
                let name = arg.child_by_field_name("name").map(&text);
                if keyword.is_some() && name == keyword {
                    if let Some(value) = arg.child_by_field_name("value") {
                        return ArgFor::Found(value);
                    }
                }
            }
            "list_splat" => star = true,
            "dictionary_splat" => double_star = true,
            "comment" => {}
            _ => {
                if !star && Some(index) == position {
                    found = Some(arg);
                }
                index += 1;
            }
        }
    }
    match found {
        Some(arg) => ArgFor::Found(arg),
        None if (star && position.is_some()) || double_star => ArgFor::Unknown,
        None => ArgFor::Missing,
    }
}

/// The first segment of a rendered chain (`self.client` for
/// `self.client.chat.completions.create`).
fn chain_root(chain: &str) -> String {
    let mut parts = chain.split('.');
    let first = parts.next().unwrap_or_default();
    if matches!(first, "self" | "cls") {
        if let Some(second) = parts.next() {
            return format!("{first}.{second}");
        }
    }
    first.to_string()
}

/// Whether an attribute node is used as a value: not the callee of a call,
/// not the object of a longer chain, not an assignment target. Decided from
/// the parent's kind and the field the node fills in it.
fn is_value_reference(parent: Option<Node<'_>>, field: Option<&str>) -> bool {
    let Some(parent) = parent else {
        return false;
    };
    match parent.kind() {
        "attribute" => field != Some("object"),
        "call" => field != Some("function"),
        "assignment" | "augmented_assignment" => field != Some("left"),
        "as_pattern_target" | "delete_statement" => false,
        _ => true,
    }
}

/// Every child, in order, in time linear in their number.
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// Every named child, in order, in time linear in their number
/// (`Node::named_child(i)` walks from the first child on every call).
fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// Every child with the field it fills in `node`.
fn children_with_fields(node: Node<'_>) -> Vec<(Node<'_>, Option<&'static str>)> {
    let mut out = Vec::with_capacity(node.child_count());
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            out.push((cursor.node(), cursor.field_name()));
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

/// Combines every value a binding is given. Placeholders (`None`, literals)
/// and self-references are ignored next to a real value. A client mixed with
/// a different client or with an unknown value is ambiguous.
fn combine(vals: Vec<Val>) -> Val {
    let real: Vec<Val> = vals
        .iter()
        .filter(|v| {
            !matches!(
                v,
                Val::Untracked(Origin::Placeholder) | Val::Untracked(Origin::Cycle)
            )
        })
        .cloned()
        .collect();
    if real.is_empty() {
        return vals
            .into_iter()
            .next()
            .unwrap_or(Val::untracked(Origin::Unbound));
    }
    if real.len() == 1 {
        return real
            .into_iter()
            .next()
            .unwrap_or(Val::untracked(Origin::Other));
    }

    let clients: Vec<&Client> = real
        .iter()
        .filter_map(|v| match v {
            Val::Client { client, path } if path.is_empty() => Some(client),
            _ => None,
        })
        .collect();
    // A construction says more than an annotation; an annotation alone says
    // more than an unknown factory next to it (`client: OpenAI` in the class
    // body, `self.client = make_client()` in `__init__`).
    let constructed: Vec<&Client> = clients.iter().copied().filter(|c| !c.typed).collect();
    if constructed.is_empty() {
        if let Some(typed) = clients.first() {
            return Val::Client {
                client: (*typed).clone(),
                path: Vec::new(),
            };
        }
    } else {
        let first = constructed[0].clone();
        let others = real.len() - clients.len();
        let same_routing = constructed.iter().all(|c| c.routing == first.routing);
        let same_pkg = constructed.iter().all(|c| c.pkg == first.pkg);
        let same_line = constructed.iter().all(|c| c.line == first.line);
        let same_site = constructed.iter().all(|c| c.site == first.site);
        let ambiguous =
            others > 0 || !same_routing || !same_pkg || constructed.iter().any(|c| c.ambiguous);
        return Val::Client {
            client: Client {
                pkg: first.pkg,
                routing: if ambiguous {
                    Routing::Unknown
                } else {
                    first.routing
                },
                line: if same_line { first.line } else { None },
                site: if same_site { first.site } else { None },
                ambiguous,
                typed: false,
                caveat: constructed.iter().find_map(|c| c.caveat.clone()),
            },
            path: Vec::new(),
        };
    }

    // No client: a tracked import beats a fallback; otherwise keep the most
    // informative origin.
    if let Some(tracked) = real.iter().find(|v| v.is_tracked()) {
        return tracked.clone();
    }
    let rank = |v: &Val| match v {
        Val::Untracked(Origin::Factory { .. }) => 0,
        Val::Untracked(Origin::Parameter { .. }) => 1,
        Val::Untracked(Origin::Imported { .. }) => 2,
        Val::Untracked(Origin::UnassignedAttribute { .. }) => 3,
        Val::Untracked(Origin::Other) => 4,
        _ => 5,
    };
    real.into_iter()
        .min_by_key(rank)
        .unwrap_or(Val::untracked(Origin::Other))
}

#[cfg(test)]
mod tests;
