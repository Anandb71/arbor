//! Python SDK usage extraction.
//!
//! # Soundness policy
//!
//! A value or route reported with no gap is confirmed: a consumer acts on it
//! without asking. Python is too dynamic to make that hold by resolving every
//! shape, so this module resolves a small **supported grammar** exactly and
//! turns everything else into a gap. `docs/SDK_USAGES.md` states the grammar
//! in full; in short, a value or route has no gap only when it is:
//!
//! - a string literal written at the call (never downgraded);
//! - a name with one reaching assignment of such a value, in the scope that
//!   binds it (module constant, local, enclosing local);
//! - a class attribute assigned once, read by class name or through
//!   `self`/`cls`, in a class with no decorator, class keyword or second
//!   base, at most one same-file level of inheritance and at most 16
//!   direct subclasses (a model value read through `self`/`cls` in a class
//!   another module may subclass keeps a `subclass_override` gap). Every
//!   binding of the name in a class body (`def`, `class`, an import, a loop
//!   target...) is an assignment of it, in the class, its base and its
//!   subclasses alike; and a method that binds its `self`/`cls` name other
//!   than as that parameter reads nothing through it;
//! - a parameter default of a function whose calls are all in this file (a
//!   nested or `_private` function, a private class's `__init__`, anything
//!   in a script), none of which passes another value;
//! - an entry of a same-file dict literal read by literal key, when the dict
//!   is only ever used through its one name and no `**` expansion or
//!   computed key follows the entry (in a dict written at the call too);
//! - a client from a tracked constructor with no `base_url`, or one that
//!   resolves to a string (the provider's own host is direct, another host
//!   is `custom_base_url`), and for Gemini no `vertexai=True`, endpoint
//!   `http_options` or `**` expansion, in a file that does not configure
//!   the endpoint some other way;
//! - the module-level API, configured only in the top-of-file setup block:
//!   plain `=` statements, each routing setting once, on a name only
//!   imports bind, before the first `def`, `class`, decorator, bare
//!   expression or compound statement (and before any module-level call in
//!   the block). No code of the file exists yet there, so none can run
//!   first; import-time execution is not followed any further.
//!
//! The collection walk also records **dynamic features** that can change a
//! resolved name, class, dict or the environment (`exec`, `globals()`,
//! `sys.modules`, `__dict__`, `setattr` with a run-time name, attribute
//! hooks, decorators and metaclasses on a class whose attributes are read,
//! multiple or deep inheritance, a class used as a value, environment
//! writes with run-time names, ...). A value or route one of them may affect
//! keeps a `dynamic_feature` gap naming the feature and its line; the most
//! dynamic ones affect the whole file (namespace builtins reached under
//! any name, `builtins` or `sys` imported under another name or used as a
//! value, a frame's `f_globals`, scopes nested past
//! [`MAX_SCOPE_DEPTH`]). Replacing a precise but costly resolution with
//! such a gap is always acceptable, so each lookup does a bounded amount of
//! work (see the `MAX_*` constants), every walk up the scopes is bounded by
//! [`MAX_SCOPE_DEPTH`], and the whole pass is linear in the file.
//!
//! # Passes
//!
//! Two passes over the tree-sitter tree:
//!
//! 1. **Collect.** One iterative walk records scopes (module, class,
//!    function, lambda), imports, every assignment target (names, `self.x`,
//!    class attributes, `with ... as`, `except ... as`, `match` captures,
//!    walrus, `global`, `nonlocal`, `globals()["NAME"]`, `del`, and a wildcard
//!    import from another module, which may bind any module-level name),
//!    parameters, every call, every attribute chain used as a value, where
//!    each binding takes effect, and the dynamic features. Parent kinds and
//!    field names travel down the walk, so no node is asked for its parent:
//!    tree-sitter answers that by descending from the root, which made deep
//!    trees quadratic.
//! 2. **Analyze.** Each call's callee is evaluated against those bindings.
//!    A chain rooted at a tracked SDK binding becomes a construction or a
//!    call; an SDK-shaped chain on an unknown receiver, `getattr` on a
//!    tracked binding and an unresolved model value become typed gaps.
//!
//! Names resolve the way Python scopes them, approximately: function and
//! lambda locals, parameters and imports shadow enclosing functions, class
//! bodies are skipped from inside methods, comprehensions have their own
//! scope, `global` re-homes an assignment to the module and `nonlocal` to
//! the enclosing function that binds the name. A use sees its
//! reaching definitions: the nearest definition that dominates it in
//! straight-line code, every definition between that one and the use (in a
//! branch), later ones in a loop that runs the use again without passing
//! the dominating one, and `global` or `nonlocal` assignments made by
//! nested or other functions, which may run at any time. Class
//! bodies and list, set and dict comprehensions run where they are written,
//! so their uses are placed there. A use from a function, lambda or
//! generator sees what reaches its definition plus every later definition,
//! since it may run after any of them. With no dominating definition, or
//! past the search limits, every definition of the name counts.
//!
//! An untyped parameter used as a client resolves when every call to its
//! function (or to a lambda assigned to a plain name) in this file passes
//! the same tracked client. Other modules may call a public function too, so
//! there the routing keeps an `ambiguous_client` gap (`in_file_callers`),
//! and a public function's model or client default keeps a
//! `parameter_default` gap. The calls of `__init__` are the calls of its
//! class; a method's are every call of its name, on any object. A function
//! whose calls cannot all be seen (it is decorated, passed, stored or bound
//! twice, or instances are built without naming their class) keeps an
//! `unseen_callers` gap on a client or model default. A model parameter
//! default that an in-file caller overrides keeps a `parameter_overridden`
//! gap listing the values the callers pass.

use super::{
    is_capturable_value, is_credential_value, is_identifier_key, is_secret_key, is_value_key,
    model_family, BindingKind, CapturedValue, ClientConstruction, ClientScope, FileUsages, GapKind,
    Receiver, Routing, Sdk, SdkCall, SdkImport, Span, UsageGap, ValueOrigin, VertexAi,
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
    // `from langchain.chat_models import ChatOpenAI` names no SDK in lower
    // case.
    "langchain",
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
    // LangChain chat models (`AzureChatOpenAI` is not: its deployment, not
    // a `model`, decides what it serves).
    "ChatOpenAI",
    "ChatAnthropic",
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
/// Maximum nesting of scopes (functions, classes, lambdas, comprehensions)
/// the collection pass follows. Every scope walk (name binding, `self`,
/// import time) is then bounded by it. A deeper scope is not analyzed: it
/// is a `truncated` gap, and a dynamic feature of the whole file, since
/// what it binds or writes is not seen.
const MAX_SCOPE_DEPTH: u32 = 64;
/// Bases read per class when same-file subclasses are indexed.
const MAX_BASES: usize = 32;
/// Direct same-file subclasses a `self.x` read looks at, and assignments
/// of the attribute taken from each. Past either, the read is a
/// `dynamic_feature` gap instead of a longer walk.
const MAX_SUBCLASSES: usize = 16;
const MAX_SUBCLASS_ASSIGNS: usize = 4;
/// Assignments of one attribute read per lookup. More is a gap anyway
/// (`reassigned`), so the rest are not copied.
const MAX_ATTR_ASSIGNS: usize = 64;
/// Definitions a reaching-definition search may look at before it gives up
/// and counts every definition of the name.
const MAX_REACH_SCAN: usize = 64;
/// Wildcard imports from other modules placed as definitions of every
/// module-level name. Past it, every module-level name may come from one
/// at any point.
const MAX_STARS: usize = 16;
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
const MAX_CANDIDATES: usize = 32;
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
/// Imports, calls, gaps and clients reported per file. Past it, the rest of
/// the file is one `truncated` gap: the report stays proportional to real
/// code however a generated file repeats itself.
const MAX_FILE_RECORDS: usize = 20_000;
/// Names bound by import statements per file. A file that binds more is
/// not analyzed past its imports (one `truncated` gap): a name it no longer
/// binds could change what any call resolves to.
const MAX_FILE_IMPORTS: usize = MAX_FILE_RECORDS;

/// Extracts SDK usages from a parsed Python file.
pub fn extract(tree: &Tree, source: &str, file_path: &str) -> FileUsages {
    if !may_use_sdk(source) {
        return FileUsages::new(file_path, "python");
    }
    let mut cx = Ctx::new(source, file_path);
    cx.collect(tree.root_node());
    cx.place_nonlocal_assigns();
    cx.note_rebound_receivers();
    cx.bind_definitions();
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
    /// LangChain's chat models, which call a provider's SDK for you: the
    /// partner packages (`langchain_openai`, `langchain_anthropic`), and the
    /// deprecated copies in `langchain_community.chat_models` and
    /// `langchain.chat_models`. Only their chat-model classes are tracked
    /// (see [`LANGCHAIN_REEXPORTS`]).
    LangchainOpenai,
    LangchainAnthropic,
    LangchainCommunityOpenai,
    LangchainCommunityAnthropic,
    LangchainLegacyOpenai,
    LangchainLegacyAnthropic,
}

impl Pkg {
    fn sdk(self) -> Sdk {
        match self {
            Pkg::Openai
            | Pkg::LangchainOpenai
            | Pkg::LangchainCommunityOpenai
            | Pkg::LangchainLegacyOpenai => Sdk::Openai,
            Pkg::Anthropic
            | Pkg::LangchainAnthropic
            | Pkg::LangchainCommunityAnthropic
            | Pkg::LangchainLegacyAnthropic => Sdk::Anthropic,
            Pkg::GoogleGenai | Pkg::GoogleGenerativeai => Sdk::Google,
        }
    }

    fn package(self) -> &'static str {
        match self {
            Pkg::Openai => "openai",
            Pkg::Anthropic => "anthropic",
            Pkg::GoogleGenai => "google.genai",
            Pkg::GoogleGenerativeai => "google.generativeai",
            Pkg::LangchainOpenai => "langchain_openai",
            Pkg::LangchainAnthropic => "langchain_anthropic",
            Pkg::LangchainCommunityOpenai | Pkg::LangchainCommunityAnthropic => {
                "langchain_community"
            }
            Pkg::LangchainLegacyOpenai | Pkg::LangchainLegacyAnthropic => "langchain",
        }
    }

    /// A LangChain package: only its chat-model classes are tracked, and
    /// what they are built with is the model the requests use.
    fn is_langchain(self) -> bool {
        !matches!(
            self,
            Pkg::Openai | Pkg::Anthropic | Pkg::GoogleGenai | Pkg::GoogleGenerativeai
        )
    }

    /// A LangChain OpenAI chat model that may run on the pre-1.0 `openai`
    /// SDK: given no `base_url`, it sends its requests through the
    /// module-level API, so the file's `openai.api_base` / `api_type`
    /// settings apply to it. `langchain_openai` needs `openai>=1`.
    fn reads_module_config(self) -> bool {
        matches!(
            self,
            Pkg::LangchainCommunityOpenai | Pkg::LangchainLegacyOpenai
        )
    }

    /// The package a dotted module path belongs to, and the rest of the
    /// path: at most [`MAX_PATH`] segments, each clipped, so a generated
    /// module path is not copied in full into every name it imports.
    fn of_module(module: &str) -> Option<(Pkg, Vec<String>)> {
        let mut parts = module.split('.').map(str::trim);
        let pkg = match (parts.next(), parts.clone().next()) {
            (Some("openai"), _) => Pkg::Openai,
            (Some("anthropic"), _) => Pkg::Anthropic,
            (Some("google"), Some("genai")) => Pkg::GoogleGenai,
            (Some("google"), Some("generativeai")) => Pkg::GoogleGenerativeai,
            (Some("langchain_openai"), _) => Pkg::LangchainOpenai,
            (Some("langchain_anthropic"), _) => Pkg::LangchainAnthropic,
            _ => return None,
        };
        if matches!(pkg, Pkg::GoogleGenai | Pkg::GoogleGenerativeai) {
            parts.next();
        }
        Some((pkg, parts.take(MAX_PATH).map(clip).collect()))
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
    // LangChain chat models route like the client each one builds.
    class(Pkg::LangchainOpenai, "ChatOpenAI", Family::OpenaiDirect),
    class(Pkg::LangchainOpenai, "AzureChatOpenAI", Family::Azure),
    class(
        Pkg::LangchainAnthropic,
        "ChatAnthropic",
        Family::AnthropicDirect,
    ),
    class(
        Pkg::LangchainCommunityOpenai,
        "ChatOpenAI",
        Family::OpenaiDirect,
    ),
    class(
        Pkg::LangchainCommunityOpenai,
        "AzureChatOpenAI",
        Family::Azure,
    ),
    class(
        Pkg::LangchainCommunityAnthropic,
        "ChatAnthropic",
        Family::AnthropicDirect,
    ),
    class(
        Pkg::LangchainLegacyOpenai,
        "ChatOpenAI",
        Family::OpenaiDirect,
    ),
    class(Pkg::LangchainLegacyOpenai, "AzureChatOpenAI", Family::Azure),
    class(
        Pkg::LangchainLegacyAnthropic,
        "ChatAnthropic",
        Family::AnthropicDirect,
    ),
];

const fn class(pkg: Pkg, name: &'static str, family: Family) -> ClientClass {
    ClientClass { pkg, name, family }
}

/// The deprecated LangChain modules that re-export several providers' chat
/// models, with the packages their OpenAI and Anthropic classes are
/// recorded under. A name imported from one of them, or from a submodule
/// (`langchain_community.chat_models.openai`), is tracked only when it is
/// one of those classes; the modules themselves are not followed.
const LANGCHAIN_REEXPORTS: &[(&str, [Pkg; 2])] = &[
    (
        "langchain_community.chat_models",
        [
            Pkg::LangchainCommunityOpenai,
            Pkg::LangchainCommunityAnthropic,
        ],
    ),
    (
        "langchain.chat_models",
        [Pkg::LangchainLegacyOpenai, Pkg::LangchainLegacyAnthropic],
    ),
];

/// The packages of the [`LANGCHAIN_REEXPORTS`] module `module` is, or is a
/// submodule of. Only its first two segments are read.
fn langchain_reexport(module: &str) -> Option<[Pkg; 2]> {
    let mut parts = module.split('.').map(str::trim);
    let head = (parts.next()?, parts.next()?);
    LANGCHAIN_REEXPORTS
        .iter()
        .find_map(|(prefix, pkgs)| (prefix.split_once('.') == Some(head)).then_some(*pkgs))
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

    /// Routing from the constructor, its keyword names and what its
    /// `base_url` is.
    fn routing(self, kwargs: &[String], splat: bool, base_url: BaseUrl) -> Routing {
        // `OpenAI(provider=bedrock(...))` hands requests to a platform adapter.
        let adapter = kwargs.iter().any(|k| k == "provider");
        match self.family {
            Family::OpenaiDirect | Family::AnthropicDirect if adapter => Routing::Unknown,
            Family::OpenaiDirect | Family::AnthropicDirect
                if matches!(base_url, BaseUrl::Custom | BaseUrl::CustomFallback) =>
            {
                Routing::CustomBaseUrl
            }
            Family::OpenaiDirect | Family::AnthropicDirect
                if matches!(base_url, BaseUrl::Unknown { .. }) =>
            {
                Routing::Unknown
            }
            Family::OpenaiDirect | Family::AnthropicDirect
                if splat && base_url == BaseUrl::Absent =>
            {
                Routing::Unknown
            }
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
    /// have been given a `base_url` (a Gemini client `vertexai=True` or an
    /// endpoint), so it is `unknown`.
    fn typed_routing(self) -> Routing {
        match self.family {
            Family::OpenaiDirect | Family::AnthropicDirect | Family::Genai => Routing::Unknown,
            _ => self.routing(&[], false, BaseUrl::Absent),
        }
    }
}

/// Keyword names that set a client's endpoint: the SDKs' own, and
/// LangChain's field names for it (`base_url` is their alias).
fn is_base_url_key(key: &str) -> bool {
    matches!(
        key,
        "base_url" | "api_base" | "openai_api_base" | "anthropic_api_url"
    )
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
    /// The client is the default of parameter `param`: other modules may
    /// call the (public) function with another one, or (`unseen`) the
    /// function is decorated, passed or stored, so callers this file does
    /// not show may.
    ParameterDefault { param: String, unseen: bool },
    /// `client.base_url = ...` after the client is built.
    BaseUrlAssigned,
    /// The file sets an environment variable the SDK reads its endpoint
    /// from (`os.environ["OPENAI_BASE_URL"] = ...`).
    EndpointEnv { var: &'static str },
    /// The file writes the environment with a name known only at run time
    /// or through an alias of `os.environ`, which may set the endpoint
    /// variable.
    EnvWrite(Feature),
    /// The `base_url` given to the constructor is read from the environment
    /// with no fallback (`env`), falls back to the provider's own host
    /// (`fallback`), or does not resolve to a string.
    BaseUrl { env: bool, fallback: bool },
    /// A dynamic feature of the file may rebind or replace the client.
    Dynamic(Feature),
    /// A Gemini client built with an option that may send its requests to
    /// Vertex AI or another endpoint (`vertexai=True`, `http_options=` with
    /// a `base_url`, `**options`), or the legacy SDK configured with
    /// `client_options`.
    EndpointOption { option: &'static str },
    /// A module-level API call passes `engine` or `deployment_id`, which
    /// Azure deployments take, in a file that does not configure the API:
    /// another module may route it to Azure (the SDK's module state is
    /// process-wide).
    ModuleConfig,
    /// The module-level API's routing is configured outside the top-of-file
    /// setup block (in a function, a branch, after a `def`...), so it may
    /// not hold when a call runs (see [`Ctx::module_openai_routing`]).
    ModuleSetup(Feature),
    /// A LangChain OpenAI chat model that may run on the pre-1.0 SDK
    /// ([`Pkg::reads_module_config`]) in a file that configures the
    /// module-level API, which such a model's requests then go through.
    LangchainModuleConfig,
}

/// A Python feature that can change what a name, attribute, dict or the
/// environment holds at run time (see "Dynamic features" in the module
/// documentation), and the line it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Feature {
    what: &'static str,
    line: u32,
}

impl Feature {
    fn describe(self) -> String {
        format!("{} at line {}", self.what, self.line)
    }
}

/// How the `base_url` given to a direct client routes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseUrl {
    /// None given, or `None`: the SDK's default host or its endpoint
    /// variable.
    Absent,
    /// The provider's own host (`https://api.openai.com/v1`).
    Provider,
    /// Another host: a gateway, a proxy or a local server.
    Custom,
    /// The provider's own host as the fallback of an environment lookup.
    ProviderFallback,
    /// Another host as the fallback of an environment lookup: the
    /// deployment may point it anywhere, the provider included.
    CustomFallback,
    /// Read from the environment with no fallback (`env`), or not a string
    /// this file resolves.
    Unknown { env: bool },
}

/// The provider's own API host, for [`BaseUrl::Provider`].
fn provider_host(pkg: Pkg) -> &'static str {
    match pkg.sdk() {
        Sdk::Openai => "api.openai.com",
        Sdk::Anthropic => "api.anthropic.com",
        Sdk::Google => "generativelanguage.googleapis.com",
    }
}

/// The host of a URL, lowercase, without user information or port.
fn url_host(url: &str) -> String {
    let rest = url.trim().split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// [`BaseUrl::Provider`] when `url` is the provider's own host, else
/// [`BaseUrl::Custom`].
fn host_kind(pkg: Pkg, url: &str) -> BaseUrl {
    if url_host(url) == provider_host(pkg) {
        BaseUrl::Provider
    } else {
        BaseUrl::Custom
    }
}

/// Environment variables an SDK reads its endpoint from when the
/// constructor is not given one.
fn endpoint_env_vars(pkg: Pkg) -> &'static [&'static str] {
    match pkg {
        Pkg::Openai => &["OPENAI_BASE_URL"],
        Pkg::Anthropic => &["ANTHROPIC_BASE_URL"],
        // `google.genai.Client()` reads whether to use Vertex AI, and its
        // endpoints, from these.
        Pkg::GoogleGenai => &[
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_GEMINI_BASE_URL",
            "GOOGLE_VERTEX_BASE_URL",
        ],
        Pkg::GoogleGenerativeai => &[],
        // LangChain reads its own variable first, then the SDK it builds
        // reads the SDK's; the partner packages also fall back to the
        // LangSmith gateway.
        Pkg::LangchainOpenai | Pkg::LangchainCommunityOpenai | Pkg::LangchainLegacyOpenai => {
            &["OPENAI_API_BASE", "OPENAI_BASE_URL", "LANGSMITH_GATEWAY"]
        }
        Pkg::LangchainAnthropic
        | Pkg::LangchainCommunityAnthropic
        | Pkg::LangchainLegacyAnthropic => &[
            "ANTHROPIC_API_URL",
            "ANTHROPIC_BASE_URL",
            "LANGSMITH_GATEWAY",
        ],
    }
}

/// Environment variables the module-level OpenAI API (v0 and the v1 module
/// client) reads its endpoint or API type from.
const OPENAI_MODULE_ENV: &[&str] = &["OPENAI_BASE_URL", "OPENAI_API_BASE", "OPENAI_API_TYPE"];

/// Attributes that re-route a built client when assigned.
const BASE_URL_ATTRS: &[&str] = &[
    "base_url",
    "api_base",
    "_base_url",
    "openai_api_base",
    "anthropic_api_url",
];

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
    /// A client constructor next to another that routes differently, or
    /// next to something that is not a tracked constructor
    /// (`AzureOpenAI if azure else build`, `try: from openai import
    /// AzureOpenAI as Maker` / `except ImportError: from mylib import
    /// FakeClient as Maker`); calling it builds an ambiguous client of the
    /// first constructor's package.
    MixedClasses(Pkg),
    /// A dynamic feature of the file may rebind or replace the value.
    Dynamic(Feature),
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
    /// assignments in each branch, the items a loop runs over. Listed with
    /// the gap for triage, never as the call's value.
    candidates: Vec<Resolved>,
    /// Whether `candidates` (with the value captured next to the gap, if
    /// any) are every value it can take: `Some(true)` only when every
    /// source added was complete.
    complete: Option<bool>,
}

impl Unresolved {
    fn new(reason: &'static str, shape: String) -> Self {
        Self {
            reason,
            shape: clip_owned(shape),
            via: None,
            candidates: Vec::new(),
            complete: None,
        }
    }

    /// Adds the values one source says it may take, and whether that
    /// source listed all of its values.
    fn with_candidates(mut self, candidates: Vec<Resolved>, complete: bool) -> Self {
        for candidate in candidates {
            if !push_candidate(&mut self.candidates, candidate) {
                self.complete = Some(false);
            }
        }
        self.complete = Some(self.complete.unwrap_or(true) && complete);
        self
    }

    /// A `dynamic_feature` gap naming `feature`.
    fn dynamic(feature: Feature) -> Self {
        Self::new("dynamic_feature", feature.describe())
    }

    /// Marks the candidates as possibly partial.
    fn incomplete(mut self) -> Self {
        self.complete = Some(false);
        self
    }

    /// Whether the candidates are known to be every value.
    fn complete(&self) -> bool {
        self.complete == Some(true) && !self.candidates.is_empty()
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
            "unseen_callers" => {
                "a parameter of a function that is decorated, passed or stored, so callers this file does not show may set it"
            }
            "credential" => {
                "credential-shaped (a secret-like name or value), so it is never captured"
            }
            "parameter_default" => {
                "a parameter of a function other modules may call with another value"
            }
            "dynamic_feature" => "something this file may change at run time",
            "subclass_override" => {
                "an attribute read through `self` or `cls` in a public class (or one with a public subclass), which a subclass in another module may override"
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
    /// Whether the value is the only one its source gives: no caveat, and
    /// not a parameter default (another module may pass the parameter).
    fn is_certain(&self) -> bool {
        self.caveats.is_empty() && self.origin != ValueOrigin::ParameterDefault
    }

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
    /// Key paths of model values that a `**` expansion or computed key
    /// written after them may replace (`{"model": "a", **extra}`): each is
    /// a gap, not a value.
    overridden: Vec<String>,
}

impl NestedRead {
    fn new() -> Self {
        Self {
            left: MAX_NESTED_ENTRIES,
            cut: false,
            found: Vec::new(),
            overridden: Vec::new(),
        }
    }
}

/// Where a nested read is: the key path so far, whether the outer argument
/// is positional, the scope its values resolve in, and whether an enclosing
/// entry may be replaced by a later `**` expansion or computed key.
#[derive(Clone, Copy)]
struct NestedPlace<'p> {
    prefix: &'p str,
    positional: bool,
    scope: usize,
    overridden: bool,
}

/// What a dict literal's `**` expansion nests, read once.
struct NestedList {
    found: Vec<NestedValue>,
    overridden: Vec<String>,
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
    /// Names declared `nonlocal` in this function.
    nonlocals: HashSet<String>,
    /// For a class: base class names as written.
    bases: Vec<String>,
    /// Byte range of the scope's node.
    start: usize,
    end: usize,
    /// For a function or lambda: its parameters in order.
    slots: Vec<Slot>,
    /// Decorated with something other than `staticmethod`/`classmethod`, so
    /// a framework may call it. For a class: decorated at all
    /// (`@dataclass`), so its annotated attributes may be fields.
    decorated: bool,
    /// For a class: a base that is not a same-file class (`BaseModel`,
    /// `NamedTuple`, `mod.Base`) or a class keyword (`metaclass=...`), whose
    /// constructor may set annotated attributes from its arguments.
    external_bases: bool,
    /// For a class: why attributes read through it may change at run time
    /// whatever its body says: a decorator, a metaclass or other class
    /// keyword, or more than one base.
    class_feature: Option<Feature>,
    /// The 1-based line the scope starts on (set for classes, functions
    /// and lambdas).
    line: u32,
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
    /// For a function or lambda: where its body starts. A walrus before it
    /// (in a default or an annotation) binds in the enclosing scope.
    body_start: usize,
    /// The class whose instance (`self_recv`) or which class itself
    /// (`cls_recv`) the name `self` / `cls` holds here, and the method that
    /// binds it: the first parameter of a method directly in the class body
    /// that is not a `staticmethod`, seen from that method and the
    /// functions, lambdas and comprehensions nested in it that do not bind
    /// the name again.
    self_recv: Option<(usize, usize)>,
    cls_recv: Option<(usize, usize)>,
    /// How many scopes enclose this one (the module is 0); at most
    /// [`MAX_SCOPE_DEPTH`].
    depth: u32,
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
            nonlocals: HashSet::new(),
            bases: Vec::new(),
            start,
            end,
            slots: Vec::new(),
            decorated: false,
            external_bases: false,
            class_feature: None,
            line: 0,
            staticmethod: false,
            lambda: false,
            comprehension: false,
            immediate: kind == ScopeKind::Class,
            bound_name: None,
            body_start: 0,
            self_recv: None,
            cls_recv: None,
            depth: 0,
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
    /// For a plain-name loop target: what the loop runs over.
    iter: Option<Node<'t>>,
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
    /// The module as written, clipped, shared by every name one statement
    /// imports.
    module: Rc<str>,
    place: Place,
}

/// One definition of a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefRef {
    Import(usize),
    Other(usize),
    Param,
    Assign(usize),
    /// `from other import *` at module level (an index into
    /// [`Ctx::module_stars`]): it may bind any module-level name.
    Star(usize),
    /// A write to the module namespace whose name is known only at run
    /// time (`globals().update(...)`, `globals()[name] = v`), or more
    /// wildcard imports than [`MAX_STARS`]: any module-level name, at any
    /// time.
    Dynamic,
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

/// A `x.attr = value` or `setattr(x, "attr", value)` whose object is not
/// `self`/`cls`/a class, checked later for module-level OpenAI
/// configuration and clients re-routed after they are built.
#[derive(Clone, Copy)]
struct ConfigAssign<'t> {
    object: Node<'t>,
    attr: &'t str,
    value: Option<Node<'t>>,
    scope: usize,
    /// The write is a statement of its own directly in the module body
    /// (not in a branch, loop, `try`, `with`, class body or function, and
    /// not part of a larger expression): it runs once, at import.
    top: bool,
    /// A plain `=` assignment (not `+=`, `setattr` or `del`).
    plain: bool,
    /// Where the write is.
    at: usize,
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
    /// Inside a type annotation, where a name is a type, not a value.
    annotation: bool,
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
    /// base URLs, containers): the last one per key with its index, at most
    /// 256.
    expansion: Vec<(String, usize, Node<'t>)>,
    /// Part of the dict could not be read, or the expansion list was cut.
    opaque: bool,
}

impl DictSummary<'_> {
    /// Whether an entry that cannot be read (`**other`, a computed key)
    /// comes after entry `index`, so it may replace that entry's value:
    /// the entry is then not the dict's value for its key (rules 1 and 5).
    fn overridable(&self, index: usize) -> bool {
        self.last_unknown.is_some_and(|unknown| unknown > index)
    }
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
    /// Some calls cannot be seen: the function is decorated, passed,
    /// stored or bound more than once (see [`Ctx::visible_calls`]).
    unseen: bool,
    /// `values` are every value the parameter is given: every call is
    /// seen and read, each passes a value that resolves (or none), and the
    /// function is called only from this file.
    complete: bool,
}

/// What an attribute read through a class (`Bot.model`, `self.model`) may
/// hold: the assignments the supported grammar sees, and the dynamic
/// feature that may change it whatever they say.
struct AttrLookup<'t> {
    assigns: Rc<[Assign<'t>]>,
    dynamic: Option<Feature>,
}

/// A class's place in its hierarchy, for attribute reads.
#[derive(Clone, Copy)]
struct ClassShape {
    /// Its one same-file base class.
    base: Option<usize>,
    /// It or that base has a base outside this file, whose metaclass or
    /// constructor may turn class attributes into fields.
    external: bool,
    /// Why its attributes may change whatever its body says.
    feature: Option<Feature>,
}

/// Candidate values read once for a dict literal.
struct DictCandidates {
    values: Vec<Resolved>,
    complete: bool,
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

/// Candidate values read once, and whether they are every value.
type CandidateList = Rc<(Vec<Resolved>, bool)>;

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
    /// Assignments to a `nonlocal` name, by the declaring function: placed
    /// in the scope that binds the name once every binding is known.
    nonlocal_assigns: Vec<(usize, String, Assign<'t>)>,
    params: HashMap<Key, Param<'t>>,
    /// Function definitions, keyed by the scope their name binds in.
    functions: HashMap<Key, Vec<usize>>,
    classes_by_name: HashMap<String, Vec<usize>>,
    /// The same-file classes that list each class as a base.
    subclasses: HashMap<usize, Vec<usize>>,
    config_assigns: Vec<ConfigAssign<'t>>,
    /// Attribute writes on objects other than `self`/`cls`/a class, by
    /// attribute name (the first one): any instance's or class's attribute
    /// of that name may be one of them.
    outside_attr_writes: HashMap<&'t str, Feature>,
    /// Writes whose attribute name is known only at run time
    /// (`setattr(self, name, v)`, `delattr`, `__setattr__`) on `self`/`cls`
    /// of a class: any attribute of its instances may be one of them.
    wild_attr_writes: HashMap<usize, Feature>,
    /// The same on any other object: any attribute of anything.
    wild_attr_any: Option<Feature>,
    /// The objects written that way, for clients re-routed after they are
    /// built.
    wild_attr_objects: Vec<(Node<'t>, usize)>,
    /// The first feature that may rebind any name, attribute or dict of the
    /// file (`exec`, `eval`, `compile`, `globals()`, `locals()`, `vars()`,
    /// `sys.modules`, `__dict__`, `import __main__`, importing the module
    /// itself): every value not written at its call, and every route, is
    /// then a `dynamic_feature` gap.
    file_dynamic: Option<Feature>,
    /// A class defines `__getattr__`, `__getattribute__`, `__setattr__`,
    /// `__delattr__` or `__init_subclass__`: any attribute read may run it.
    attr_hooks: Option<Feature>,
    /// The environment is written with a name known only at run time
    /// (`os.environ[key] = v`, `os.environ |= {...}`, `update(**cfg)`) or
    /// through an alias of `os.environ` (`env = os.environ`).
    env_wild: Option<Feature>,
    /// The same through a bare `environ`, which counts only when the file
    /// imports it from `os` (a WSGI `environ` is a request's).
    bare_environ_wild: Option<Feature>,
    /// Instances may be built without naming their class
    /// (`type(self)(...)`, `self.__class__(...)`, `cls(...)`): no class's
    /// constructor calls are all seen.
    dynamic_constructors: Option<Feature>,
    /// Names used as a value other than to read an attribute, call or
    /// subclass it (stored, aliased, passed, returned): a class of that
    /// name may be changed by whatever receives it.
    class_escapes: HashMap<&'t str, Feature>,
    /// Base names that more than one same-file class has: their classes
    /// may have subclasses this file cannot attribute.
    ambiguous_base_names: HashSet<String>,
    /// A method or module function may be called by a name known only at
    /// run time (`getattr(obj, name)(...)`, `globals()[name](...)`): no
    /// method's (or, for the namespace, no function's) calls are all seen.
    dynamic_method_calls: bool,
    dynamic_function_calls: bool,
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
    /// Constructions recorded so far: the index of the record and why its
    /// routing may not hold.
    constructions: HashMap<usize, (usize, Option<ClientCaveat>)>,
    name_memo: HashMap<Key, Memo<Val>>,
    attr_memo: HashMap<Key, Memo<Val>>,
    /// The same for `self.x` / `cls.x`, read with subclasses and outside
    /// writes.
    dyn_attr_memo: HashMap<Key, Memo<Val>>,
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
    /// A class's own assignments of an attribute (its body and
    /// `self.x = ...`), read once per class and name.
    own_attr_memo: HashMap<Key, Rc<[Assign<'t>]>>,
    /// Attribute lookups through a class name, and through `self`/`cls`.
    class_attr_memo: HashMap<Key, Rc<AttrLookup<'t>>>,
    self_attr_memo: HashMap<Key, Rc<AttrLookup<'t>>>,
    /// The items a loop runs over, read once per iterable and origin.
    loop_items_memo: HashMap<(usize, ValueOrigin), CandidateList>,
    /// Candidates of a name every definition of which reaches the use,
    /// read once per name and origin.
    reach_all_memo: HashMap<(Key, ValueOrigin), CandidateList>,
    caller_memo: HashMap<Key, Memo<Option<Val>>>,
    /// Model values in-file callers pass for a parameter, by (function,
    /// parameter).
    caller_models: HashMap<Key, Rc<CallerModels>>,
    /// The values a dict literal's entries hold, by node id and the name
    /// the dict is used through.
    dict_candidates_memo: HashMap<(usize, Option<String>), Rc<DictCandidates>>,
    /// Candidate values are being collected: a nested lookup does not
    /// collect its own, so the work stays linear.
    collecting: u32,
    name_call_index: HashMap<String, CallIndex<'t>>,
    method_call_index: HashMap<String, CallSites<'t>>,
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
    /// A `base_url` written on an object this file cannot identify, which
    /// may be any of its clients.
    rerouted_any: Option<Feature>,
    /// Why the module-level OpenAI API's routing may not hold.
    openai_caveat: Option<ClientCaveat>,
    /// Namespace builtins used as a value, by name (see
    /// [`Ctx::note_builtin_value`]).
    builtin_values: HashMap<&'t str, Feature>,
    /// `C.m` read as a value (not called, not written): when `C` is a class
    /// and `m` one of its methods, the method may run on another class's
    /// instance (`B2.go = B.go`, `go = B.go` in another class body).
    member_values: HashSet<(&'t str, &'t str)>,
    /// Attribute writes through a method's `self`/`cls`: the method, the
    /// attribute and the object written.
    method_self_writes: Vec<(usize, &'t str, Node<'t>)>,
    /// Methods that bind their `self` (`true`) or `cls` (`false`) other
    /// than as the first parameter, themselves or in a scope nested in them
    /// (`self = other`, `for self in ...`, `nonlocal self`...): a read
    /// through the name there is a `dynamic_feature` gap, and a write an
    /// outside write (see [`Ctx::note_rebound_receivers`]).
    rebound_receivers: HashMap<(usize, bool), Feature>,
    /// Why the legacy Gemini SDK's module-level routing may not hold.
    legacy_caveat: Option<ClientCaveat>,
    /// The file imports the legacy Gemini SDK (`google.generativeai`).
    legacy_imported: bool,
    /// Names `__all__` lists, and whether it is built from names
    /// (`__all__ = _EXPORTS`), when any module-level name may be exported.
    exported: HashSet<&'t str>,
    exports_dynamic: bool,
    /// An SDK member replaced in this file (`client.chat.completions.create
    /// = patched`, `openai.OpenAI = Sub`), by package: every route of that
    /// package is then a `dynamic_feature` gap.
    sdk_patched: HashMap<Pkg, Feature>,
    /// Names bound by `def` and `class` statements and class-body imports,
    /// where they bind: a name also assigned there gets an assignment of no
    /// known value for each (see [`Ctx::bind_definitions`]).
    def_bindings: Vec<(Key, usize, Place)>,
    /// The last write that configures the module-level OpenAI API's
    /// routing, and where it is: a call that runs at import before it is
    /// not routed by it.
    openai_last_config: Option<(usize, Feature)>,
    /// The attribute write or `setattr` call being recorded is a statement
    /// of its own directly in the module body (see [`ConfigAssign::top`]),
    /// and a plain `=` assignment ([`ConfigAssign::plain`]).
    config_top: bool,
    config_plain: bool,
    /// Where the top-of-file setup block ends (see
    /// [`Ctx::setup_block_end`]): the module-level API's configuration
    /// counts only before it. Computed on first use: most files never
    /// configure the module-level API.
    setup_end: Option<usize>,
    /// The module node.
    root: Option<Node<'t>>,
    /// Scopes nested deeper than [`MAX_SCOPE_DEPTH`], which are not walked,
    /// and the scope each is in: one `truncated` gap each.
    scope_cuts: Vec<(Node<'t>, usize)>,
    /// Wildcard imports from other modules, in order, and whether there
    /// were more than [`MAX_STARS`] or the module namespace is written with
    /// a run-time name (see [`DefRef::Dynamic`]).
    module_stars: Vec<(Place, Rc<str>)>,
    dynamic_globals: bool,
    /// While a plain-name loop target is recorded: what the loop runs over.
    loop_iter: Option<Node<'t>>,
    /// Names bound by imports so far, and the import statement at which
    /// [`MAX_FILE_IMPORTS`] was reached.
    import_bindings: usize,
    import_cut: Option<Node<'t>>,
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
            nonlocal_assigns: Vec::new(),
            params: HashMap::new(),
            functions: HashMap::new(),
            classes_by_name: HashMap::new(),
            subclasses: HashMap::new(),
            config_assigns: Vec::new(),
            outside_attr_writes: HashMap::new(),
            wild_attr_writes: HashMap::new(),
            wild_attr_any: None,
            wild_attr_objects: Vec::new(),
            file_dynamic: None,
            attr_hooks: None,
            env_wild: None,
            bare_environ_wild: None,
            dynamic_constructors: None,
            class_escapes: HashMap::new(),
            ambiguous_base_names: HashSet::new(),
            dynamic_method_calls: false,
            dynamic_function_calls: false,
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
            dyn_attr_memo: HashMap::new(),
            def_memo: HashMap::new(),
            param_memo: HashMap::new(),
            call_memo: HashMap::new(),
            dict_cache: HashMap::new(),
            dict_contents_memo: HashMap::new(),
            dict_nested: HashMap::new(),
            nested_left: MAX_FILE_NESTED_VALUES,
            candidates_left: MAX_FILE_CANDIDATES,
            own_attr_memo: HashMap::new(),
            class_attr_memo: HashMap::new(),
            self_attr_memo: HashMap::new(),
            loop_items_memo: HashMap::new(),
            reach_all_memo: HashMap::new(),
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
            rerouted_any: None,
            openai_caveat: None,
            builtin_values: HashMap::new(),
            member_values: HashSet::new(),
            method_self_writes: Vec::new(),
            rebound_receivers: HashMap::new(),
            legacy_caveat: None,
            legacy_imported: false,
            exported: HashSet::new(),
            exports_dynamic: false,
            sdk_patched: HashMap::new(),
            def_bindings: Vec::new(),
            openai_last_config: None,
            config_top: false,
            config_plain: false,
            setup_end: None,
            root: None,
            scope_cuts: Vec::new(),
            module_stars: Vec::new(),
            dynamic_globals: false,
            loop_iter: None,
            import_bindings: 0,
            import_cut: None,
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
        self.root = Some(root);
        let mut stack = vec![Frame {
            node: root,
            scope: 0,
            parent: None,
            field: None,
            block_end: root.end_byte(),
            stmt: None,
            inner_loop: None,
            grand: None,
            annotation: false,
        }];
        // One cursor, reset at each node, walks every node's children.
        let mut cursor = root.walk();
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
                    "function_definition"
                    | "class_definition"
                    | "lambda"
                    | "list_comprehension"
                    | "set_comprehension"
                    | "dictionary_comprehension"
                    | "generator_expression"
                        if self.scopes[scope].depth >= MAX_SCOPE_DEPTH =>
                    {
                        // Nothing inside is walked: every scope walk stays
                        // bounded, and what it binds or writes is unknown.
                        self.note_file_dynamic(
                            "scopes nested more than 64 deep, which are not analyzed",
                            node,
                        );
                        self.scope_cuts.push((node, scope));
                        continue;
                    }
                    "function_definition" => {
                        inner = self.enter_function(node, scope, frame.parent);
                        child_loop = None;
                    }
                    "class_definition" => {
                        inner = self.enter_class(node, scope, frame.parent);
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
                        self.record_loop_target(node, scope);
                    }
                    "for_in_clause" => self.record_loop_target(node, scope),
                    "import_statement" => self.import_statement(node, scope, place),
                    "import_from_statement" => self.import_from(node, scope, place),
                    "assignment" | "augmented_assignment"
                        if node
                            .child_by_field_name("left")
                            .is_some_and(|l| self.text(l) == "__all__")
                            && node.child_by_field_name("right").is_some() =>
                    {
                        if let Some(right) = node.child_by_field_name("right") {
                            self.note_exports(right);
                        }
                        if let Some(left) = node.child_by_field_name("left") {
                            self.record_target(left, None, None, scope, true, place);
                        }
                    }
                    "assignment" => {
                        if let Some(left) = node.child_by_field_name("left") {
                            let value = node.child_by_field_name("right");
                            let annotation = node.child_by_field_name("type");
                            self.config_top = is_top_statement(&frame);
                            self.config_plain = true;
                            self.record_target(left, value, annotation, scope, false, place);
                            self.config_top = false;
                            self.config_plain = false;
                        }
                        self.note_chained_alias(node);
                    }
                    "augmented_assignment" => {
                        if let Some(left) = node.child_by_field_name("left") {
                            self.config_top = is_top_statement(&frame);
                            self.record_target(left, None, None, scope, true, place);
                            self.config_top = false;
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
                            // enclosing function or module (PEP 572), and
                            // one in a default or annotation of a `def` or
                            // `lambda` where that header runs.
                            let mut home = scope;
                            loop {
                                let current = &self.scopes[home];
                                let header = current.kind == ScopeKind::Function
                                    && node.start_byte() < current.body_start;
                                if home == 0 || !(current.comprehension || header) {
                                    break;
                                }
                                home = current.parent.unwrap_or(0);
                            }
                            let place = Place {
                                at: node.end_byte(),
                                region: None,
                            };
                            self.record_target(name, value, None, home, false, place);
                        }
                    }
                    "type_alias_statement" => self.record_type_alias(node, scope, place),
                    "case_clause" => {
                        for pattern in named_children(node) {
                            if pattern.kind() == "case_pattern" {
                                self.record_pattern_captures(pattern, scope);
                            }
                        }
                    }
                    "except_clause" | "except_group_clause" => {
                        // `except E as name:` binds `name` in the handler
                        // only (it is deleted after it).
                        for child in named_children(node) {
                            let target = (child.kind() == "as_pattern")
                                .then(|| child.child_by_field_name("alias"))
                                .flatten()
                                .and_then(|alias| alias.named_child(0));
                            if let Some(target) = target {
                                let place = Place {
                                    at: target.end_byte(),
                                    region: None,
                                };
                                self.record_target(target, None, None, scope, true, place);
                            }
                        }
                    }
                    "global_statement" | "nonlocal_statement" => {
                        // In a class body too: `class C: global M; M = 1`
                        // binds the module's `M`, not a class attribute.
                        if self.scopes[scope].kind != ScopeKind::Module {
                            let global = node.kind() == "global_statement";
                            for name in named_children(node) {
                                let name = self.text(name).to_string();
                                let declared = &mut self.scopes[scope];
                                if global {
                                    declared.globals.insert(name);
                                } else {
                                    declared.nonlocals.insert(name);
                                }
                            }
                        }
                    }
                    "delete_statement" => {
                        for target in named_children(node) {
                            let targets = match target.kind() {
                                "expression_list" => named_children(target),
                                _ => vec![target],
                            };
                            for target in targets {
                                // `del M` and `del Bot.model` unbind what a
                                // later read sees (a base's attribute, or
                                // nothing): an assignment of no value.
                                if matches!(target.kind(), "identifier" | "attribute") {
                                    self.config_top =
                                        frame.parent.is_some_and(|p| p.kind() == "module");
                                    self.record_target(target, None, None, scope, true, place);
                                    self.config_top = false;
                                } else {
                                    self.note_mutated_root(target);
                                }
                            }
                        }
                    }
                    "call" => {
                        let callee = node.child_by_field_name("function");
                        // `__all__.extend([...])`, `__all__.append("name")`.
                        if callee
                            .and_then(|f| f.child_by_field_name("object"))
                            .is_some_and(|o| self.text(o) == "__all__")
                        {
                            if let Some(args) = node.child_by_field_name("arguments") {
                                self.note_exports(args);
                            }
                        }
                        self.note_dynamic_call(node, scope);
                        self.config_top = is_top_statement(&frame);
                        self.note_attr_write_call(node, scope, place);
                        self.config_top = false;
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
                        self.note_namespace_lookup(node);
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
                        self.note_dict_attribute(node, &frame);
                        self.note_dynamic_attribute(node, &frame);
                        self.note_member_value(node, &frame);
                    }
                    "identifier" => {
                        self.note_name_use(node, &frame, scope);
                        self.note_dict_name(node, &frame, scope);
                        self.note_class_use(node, &frame);
                        self.note_builtin_value(node, &frame);
                        if is_environ_name(self.text(node)) {
                            self.note_environ_use(node, &frame, true);
                        }
                    }
                    _ => {}
                }
            }
            // An import's names were read above and bind nothing else: its
            // children (one per imported name) are not walked.
            if matches!(
                node.kind(),
                "import_statement" | "import_from_statement" | "future_import_statement"
            ) {
                continue;
            }
            let (block_end, child_stmt) = match node.kind() {
                "module" | "block" => (node.end_byte(), None),
                _ => (frame.block_end, Some(stmt)),
            };
            // Punctuation and keywords carry nothing: only named children
            // are walked, which also keeps the stack small on deep chains.
            // They are pushed in order, then that run is reversed, so the
            // walk stays pre-order without a list per node.
            let first = stack.len();
            let annotation = frame.annotation || node.kind() == "type";
            cursor.reset(node);
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.is_named() {
                        stack.push(Frame {
                            node: child,
                            scope: inner,
                            parent: Some(node),
                            field: cursor.field_name(),
                            block_end,
                            stmt: child_stmt,
                            inner_loop: child_loop,
                            grand: frame.parent,
                            annotation,
                        });
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            stack[first..].reverse();
        }
        // The walk is pre-order, so this is already sorted; keep it so.
        self.env_reads.sort_unstable();
    }

    /// The target of a `for` statement or clause: bound once per item, so it
    /// never dominates; a plain name keeps what the loop runs over, whose
    /// items are the values it may take.
    fn record_loop_target(&mut self, node: Node<'t>, scope: usize) {
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        let place = Place {
            at: left.end_byte(),
            region: None,
        };
        if left.kind() == "identifier" {
            self.loop_iter = node.child_by_field_name("right");
        }
        self.record_target(left, None, None, scope, true, place);
        self.loop_iter = None;
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
            // An annotation names a type; it does not pass the value on.
            ("type", _) => true,
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
        self.note_mangled_use(name);
    }

    /// `__all__ = [...]`, `__all__ += [...]`, `__all__.extend([...])`: a
    /// `_private` function or class listed there is exported, so other
    /// modules may call or subclass it (its calls are not all in this
    /// file). A list built from names (`__all__ = _EXPORTS`, `extend(names)`)
    /// may export any module-level name ([`Ctx::exports_dynamic`]).
    fn note_exports(&mut self, node: Node<'t>) {
        let mut cursor = node.walk();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if let Some(name) = self.plain_literal(current) {
                self.name_uses.entry(name).or_default().escapes = true;
                self.exported.insert(name);
                continue;
            }
            if current.kind() == "identifier" {
                self.exports_dynamic = true;
            }
            for child in current.named_children(&mut cursor) {
                stack.push(child);
            }
        }
    }

    /// `_A__go`, the mangled name of `A.__go` used from outside `A`: a call
    /// this file cannot attribute, so `__go`'s calls are not all seen.
    fn note_mangled_use(&mut self, name: &'t str) {
        if let Some(rest) = name.strip_prefix('_') {
            if let Some(at) = rest.find("__").filter(|at| *at > 0) {
                let mangled = &rest[at..];
                if mangled.len() > 2 && !mangled.ends_with("__") {
                    self.attr_uses.entry(mangled).or_default().escapes = true;
                }
            }
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
            ("attribute", "os", Some(member)) => {
                matches!(member, "environ" | "environb" | "getenv" | "getenvb")
            }
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
    /// `os.environ.setdefault("NAME", ...)`, `os.environ.update(...)`,
    /// `os.environ.__setitem__("NAME", ...)` or `os.putenv("NAME", ...)`. A
    /// name that is not a literal may be any variable: the write is a
    /// dynamic feature ([`Ctx::env_wild`]).
    fn note_env_write(&mut self, node: Node<'t>) {
        // `os.environ`, or the same through any other object (`_os.environ`
        // for `import os as _os`, `import_module("os").environ`): matched by
        // the attribute name, so an alias of `os` cannot hide a write.
        let environ = |n: Node<'t>| match n.kind() {
            "identifier" => is_environ_name(self.text(n)),
            "attribute" => n
                .child_by_field_name("attribute")
                .is_some_and(|a| is_environ_name(self.text(a))),
            _ => false,
        };
        let mut names: Vec<String> = Vec::new();
        let mut wild = false;
        let mut bare = false;
        match node.kind() {
            "subscript" => {
                if let Some(value) = node.child_by_field_name("value").filter(|v| environ(*v)) {
                    bare = value.kind() == "identifier";
                    match single_subscript(node).and_then(|k| self.literal(k).ok()) {
                        Some(name) => names.push(name),
                        None => wild = true,
                    }
                }
            }
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                // Most calls are something else: decide from the last name
                // before rendering anything.
                let last = match function.kind() {
                    "identifier" => Some(function),
                    "attribute" => function.child_by_field_name("attribute"),
                    _ => None,
                };
                if !last.is_some_and(|n| {
                    matches!(
                        self.text(n),
                        "putenv" | "setdefault" | "update" | "__setitem__"
                    )
                }) {
                    return;
                }
                let (positional, keywords) = call_arguments(node, |n| self.text(n));
                let putenv = last.is_some_and(|n| self.text(n) == "putenv");
                let method = function
                    .child_by_field_name("attribute")
                    .map(|a| self.text(a))
                    .unwrap_or_default();
                let object = function.child_by_field_name("object");
                let on_environ = object.is_some_and(environ);
                bare = object.is_some_and(|o| o.kind() == "identifier");
                if putenv || (on_environ && matches!(method, "setdefault" | "__setitem__")) {
                    bare = false;
                    match positional.first().and_then(|k| self.literal(*k).ok()) {
                        Some(name) => names.push(name),
                        None => wild = true,
                    }
                } else if on_environ && method == "update" {
                    names.extend(keywords.iter().map(|(k, _)| k.clone()));
                    let splat = node.child_by_field_name("arguments").is_some_and(|args| {
                        named_children(args)
                            .iter()
                            .any(|a| matches!(a.kind(), "dictionary_splat" | "list_splat"))
                    });
                    wild |= splat;
                    for arg in positional {
                        if arg.kind() != "dictionary" {
                            // A name, a call or pairs: any variable.
                            wild = true;
                            continue;
                        }
                        for entry in dict_entries(arg, |n| self.literal(n).ok(), |n| self.text(n)) {
                            match entry {
                                Some((key, _)) => names.push(key),
                                None => wild = true,
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if wild {
            let feature =
                Self::feature("an environment variable written with a run-time name", node);
            if bare {
                self.bare_environ_wild.get_or_insert(feature);
            } else {
                self.env_wild.get_or_insert(feature);
            }
        }
        self.env_writes.extend(names);
    }

    /// The 1-based line `node` starts on.
    fn line_of(node: Node<'t>) -> u32 {
        node.start_position().row as u32 + 1
    }

    fn feature(what: &'static str, node: Node<'t>) -> Feature {
        Feature {
            what,
            line: Self::line_of(node),
        }
    }

    /// Records the first feature that may rebind anything in the file.
    fn note_file_dynamic(&mut self, what: &'static str, node: Node<'t>) {
        self.file_dynamic.get_or_insert(Self::feature(what, node));
    }

    /// `exec`, `eval`, `compile`, `globals()`, `vars(obj)` (an object's
    /// `__dict__`), `locals()` and `vars()` outside a function (where they
    /// are the module's or the class body's namespace; a function's
    /// `locals()` cannot rebind its names), the module importing itself,
    /// and instances built without naming their class (`type(self)(...)`,
    /// `cls(...)`).
    fn note_dynamic_call(&mut self, call: Node<'t>, scope: usize) {
        let Some(function) = call.child_by_field_name("function") else {
            return;
        };
        // `__import__(__name__)`, `importlib.import_module(__name__)`, and
        // the same for `builtins` and `sys`, whose namespace features are
        // then reached under a name this file does not spell.
        let imported = |cx: &Self| {
            call_arguments(call, |n| cx.text(n))
                .0
                .first()
                .and_then(|arg| match arg.kind() {
                    "identifier" if cx.text(*arg) == "__name__" => {
                        Some("the module importing itself")
                    }
                    _ => match cx.plain_literal(*arg) {
                        Some("__main__") => Some("the module importing itself"),
                        Some("builtins" | "sys") => Some("`builtins` or `sys` imported by name"),
                        _ => None,
                    },
                })
        };
        match function.kind() {
            "identifier" => {
                let what = match self.text(function) {
                    "exec" => "`exec`",
                    "eval" => "`eval`",
                    "compile" => "`compile`",
                    "globals" => "`globals()`",
                    "locals" | "vars"
                        if self.scopes[scope].kind == ScopeKind::Function
                            && !self.scopes[scope].comprehension
                            && call_arguments(call, |n| self.text(n)).0.is_empty() =>
                    {
                        return;
                    }
                    "locals" => "`locals()`",
                    "vars" => "`vars()`",
                    "__import__" | "import_module" => match imported(self) {
                        Some(what) => what,
                        None => return,
                    },
                    "cls" => {
                        self.dynamic_constructors
                            .get_or_insert(Self::feature("`cls(...)`", call));
                        return;
                    }
                    _ => return,
                };
                self.note_file_dynamic(what, call);
            }
            "attribute" => {
                let imports = function
                    .child_by_field_name("attribute")
                    .is_some_and(|a| matches!(self.text(a), "import_module" | "reload"));
                if let Some(what) = imports.then(|| imported(self)).flatten() {
                    self.note_file_dynamic(what, call);
                }
            }
            "call" => {
                // `type(self)(...)`.
                let inner = function.child_by_field_name("function");
                if inner.is_some_and(|f| f.kind() == "identifier" && self.text(f) == "type")
                    && call_arguments(function, |n| self.text(n)).0.len() == 1
                {
                    self.dynamic_constructors
                        .get_or_insert(Self::feature("`type(...)(...)`", call));
                }
            }
            _ => {}
        }
    }

    /// `exec`, `eval`, `compile`, `globals`, `locals`, `vars` or
    /// `__import__` used as a value (`run = exec`, `g = globals`), which a
    /// call through another name may then run; and `__builtins__`, through
    /// which any of them can be reached. A name the file binds itself (a
    /// parameter `eval`) is not the builtin: that is checked once every
    /// binding is known ([`Ctx::settle_builtin_values`]).
    fn note_builtin_value(&mut self, node: Node<'t>, frame: &Frame<'t>) {
        let name = self.text(node);
        if matches!(name, "builtins" | "sys") {
            // The module itself handed on (`s = sys`, `f(builtins)`): its
            // namespace features are then reached under another name.
            let used = frame.parent.is_some_and(|parent| {
                !(parent.kind() == "attribute" && frame.field == Some("object"))
                    && !is_declaration(parent.kind(), frame.field)
            });
            if used && !frame.annotation {
                self.note_file_dynamic("`builtins` or `sys` used as a value", node);
            }
            return;
        }
        if !is_namespace_builtin(name) && name != "__builtins__" {
            return;
        }
        let Some(parent) = frame.parent else {
            return;
        };
        let called = parent.kind() == "call" && frame.field == Some("function");
        if (called && name != "__builtins__") || is_declaration(parent.kind(), frame.field) {
            return;
        }
        self.builtin_values.entry(name).or_insert(Self::feature(
            "a builtin that reads or rebinds names (`exec`, `eval`, `globals`...) used as a value",
            node,
        ));
    }

    /// Whole-file features from [`Ctx::note_builtin_value`] whose name the
    /// file does not bind anywhere itself.
    fn settle_builtin_values(&mut self) {
        if self.builtin_values.is_empty() {
            return;
        }
        let bound: HashSet<&str> = self
            .assigns
            .keys()
            .chain(self.params.keys())
            .chain(self.functions.keys())
            .chain(self.imports.keys())
            .chain(self.other_imports.keys())
            .map(|(_, name)| name.as_str())
            .collect();
        let mut found: Vec<Feature> = self
            .builtin_values
            .iter()
            .filter(|(name, _)| !bound.contains(**name))
            .map(|(_, feature)| *feature)
            .collect();
        found.sort_by_key(|f| f.line);
        if let Some(feature) = found.first() {
            self.file_dynamic.get_or_insert(*feature);
        }
    }

    /// Imports that hand out a dynamic feature under another name:
    /// `from sys import modules`, `from builtins import exec` (and the other
    /// namespace builtins), and `os.environ` or `os.putenv` imported under
    /// another name (`from os import environ as E`), which is an alias of
    /// the process environment.
    fn note_dynamic_import(&mut self, module: &str, name: &str, local: &str, node: Node<'t>) {
        match (module, name) {
            ("sys", "modules") => self.note_file_dynamic("`sys.modules` imported by name", node),
            ("builtins", name) if is_namespace_builtin(name) => self.note_file_dynamic(
                "a builtin that reads or rebinds names (`exec`, `eval`, `globals`...) imported by name",
                node,
            ),
            ("os", "environ" | "environb" | "putenv") if local != name => {
                self.env_wild.get_or_insert(Self::feature(
                    "`os.environ` or `os.putenv` imported under another name",
                    node,
                ));
            }
            _ => {}
        }
    }

    /// The file's own module name (its stem), when other modules can import
    /// it by that name: not a package `__init__`, a `__main__` or a script.
    fn own_module_name(&self) -> Option<&str> {
        let stem = std::path::Path::new(&self.file).file_stem()?.to_str()?;
        (!self.script && !matches!(stem, "__init__" | "__main__")).then_some(stem)
    }

    /// `import app` in `app.py`: the module importing itself, through which
    /// it may rebind its own names or call its private functions. A tracked
    /// SDK's name is left alone (`import openai` in an `openai.py` example).
    fn note_self_import(&mut self, written: &str, node: Node<'t>) {
        let last = written.rsplit('.').next().unwrap_or(written);
        if Pkg::of_module(written).is_none() && self.own_module_name() == Some(last) {
            self.note_file_dynamic("the module importing itself", node);
        }
    }

    /// `x.__dict__`, `sys.modules`, `x.__class__` and uses of `os.environ`.
    fn note_dynamic_attribute(&mut self, node: Node<'t>, frame: &Frame<'t>) {
        let Some(attr) = node.child_by_field_name("attribute") else {
            return;
        };
        let object = node.child_by_field_name("object");
        let on =
            |name: &str| object.is_some_and(|o| o.kind() == "identifier" && self.text(o) == name);
        match self.text(attr) {
            "__dict__" => self.note_file_dynamic("`__dict__`", node),
            "modules" if on("sys") => self.note_file_dynamic("`sys.modules`", node),
            "__class__" => {
                self.dynamic_constructors
                    .get_or_insert(Self::feature("`__class__`", node));
            }
            // `os.environ` (or `os.environb`, which shares its data on
            // POSIX), or through an alias of `os`.
            "environ" | "environb" => self.note_environ_use(node, frame, false),
            // A frame's namespaces (`inspect.currentframe().f_globals`,
            // `sys._getframe(1).f_locals`) rebind names at run time.
            "f_globals" | "f_locals" | "f_builtins" => {
                self.note_file_dynamic("a frame's namespace (`f_globals`, `f_locals`)", node);
            }
            name if is_namespace_builtin(name) && (on("builtins") || on("__builtins__")) => {
                self.note_file_dynamic(
                    "a builtin that reads or rebinds names (`exec`, `eval`, `globals`...) reached through `builtins`",
                    node,
                );
            }
            _ => {}
        }
    }

    /// `os.environ` (or a bare `environ`) used other than to read it or to
    /// write a variable named by a literal: assigned to another name,
    /// passed to a call, updated in place. Any variable, the SDK's endpoint
    /// variable among them, may then be set.
    fn note_environ_use(&mut self, node: Node<'t>, frame: &Frame<'t>, bare: bool) {
        let Some(parent) = frame.parent else {
            return;
        };
        let reads = match (parent.kind(), frame.field) {
            // `os.environ[...]`: a read, or a write `note_env_write` sees.
            ("subscript", Some("value")) => true,
            ("attribute", Some("object")) => {
                let method = parent
                    .child_by_field_name("attribute")
                    .map(|a| self.text(a))
                    .unwrap_or_default();
                // Writes through `setdefault`, `update` and `__setitem__` are
                // read by `note_env_write`; removals cannot add an endpoint.
                matches!(
                    method,
                    "get"
                        | "copy"
                        | "keys"
                        | "items"
                        | "values"
                        | "__contains__"
                        | "__getitem__"
                        | "setdefault"
                        | "update"
                        | "__setitem__"
                        | "pop"
                        | "popitem"
                        | "clear"
                        | "__delitem__"
                )
            }
            // `{**os.environ, ...}` copies it.
            ("comparison_operator", _)
            | ("expression_statement", _)
            | ("dictionary_splat", _)
            | ("for_statement" | "for_in_clause", Some("right")) => true,
            ("argument_list", _) => {
                let callee = frame
                    .grand
                    .and_then(|call| call.child_by_field_name("function"))
                    .map(|f| self.render_links(f, 4))
                    .unwrap_or_default();
                READ_ONLY_CALLS.contains(&callee.as_str())
                    || SHALLOW_CALLS.contains(&callee.as_str())
            }
            (kind, field) => bare && is_declaration(kind, field),
        };
        if reads {
            return;
        }
        let feature = Self::feature("`os.environ` aliased, passed or updated in place", node);
        if bare {
            self.bare_environ_wild.get_or_insert(feature);
        } else {
            self.env_wild.get_or_insert(feature);
        }
    }

    /// A name used as a value other than to read or write one of its
    /// attributes, call it, subclass it, test it or annotate with it: if a
    /// class has that name, whatever receives it may change its attributes.
    fn note_class_use(&mut self, node: Node<'t>, frame: &Frame<'t>) {
        let Some(parent) = frame.parent else {
            return;
        };
        if frame.annotation || self.class_escapes.contains_key(self.text(node)) {
            return;
        }
        let safe = match (parent.kind(), frame.field) {
            ("attribute", Some("object")) | ("call", Some("function")) => true,
            ("argument_list", _) => match frame.grand {
                Some(grand) if grand.kind() == "class_definition" => true,
                Some(grand) if grand.kind() == "call" => grand
                    .child_by_field_name("function")
                    .filter(|f| f.kind() == "identifier")
                    .is_some_and(|f| {
                        matches!(
                            self.text(f),
                            "isinstance"
                                | "issubclass"
                                | "type"
                                | "super"
                                | "print"
                                | "repr"
                                | "str"
                                | "id"
                                | "hash"
                        )
                    }),
                _ => false,
            },
            ("comparison_operator", _) | ("expression_statement", _) | ("type", _) => true,
            (kind, field) => is_declaration(kind, field),
        };
        if !safe {
            let name = self.text(node);
            self.class_escapes
                .entry(name)
                .or_insert_with(|| Self::feature("the class used as a value", node));
        }
    }

    /// `D[k] = v`, `del D[k]`, `D["a"]["b"] = v`, `H.d["k"] = v`,
    /// `self.cfg["k"] = v`: the name at the root of the target, and every
    /// attribute name along it (`d`, `cfg`: a class or instance attribute
    /// may alias a dict), may hold a dict that changes.
    fn note_mutated_root(&mut self, target: Node<'t>) {
        if matches!(target.kind(), "subscript" | "attribute") {
            let names = self.chain_names(target);
            self.escaped.extend(names);
        }
    }

    /// The root name of a subscript, attribute or call chain and every
    /// attribute name along it, plus the literal key of
    /// `globals()["NAME"]` / `vars()["NAME"]`, which names a module-level
    /// binding.
    fn chain_names(&self, node: Node<'t>) -> Vec<&'t str> {
        let mut names = Vec::new();
        let mut node = node;
        for _ in 0..MAX_DEPTH {
            node = match node.kind() {
                "identifier" => {
                    names.push(self.text(node));
                    break;
                }
                "subscript" => {
                    let Some(value) = node.child_by_field_name("value") else {
                        break;
                    };
                    if is_namespace_call(value, |n| self.text(n)) {
                        if let Some(key) =
                            single_subscript(node).and_then(|k| self.plain_literal(k))
                        {
                            names.push(key);
                        }
                        break;
                    }
                    value
                }
                "attribute" => {
                    if let Some(attr) = node.child_by_field_name("attribute") {
                        names.push(self.text(attr));
                    }
                    match node.child_by_field_name("object") {
                        Some(object) => object,
                        None => break,
                    }
                }
                "call" => match node.child_by_field_name("function") {
                    Some(function) => function,
                    None => break,
                },
                "parenthesized_expression" => match node.named_child(0) {
                    Some(inner) => inner,
                    None => break,
                },
                _ => break,
            };
        }
        names
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
            let names = self.chain_names(object);
            self.escaped.extend(names);
        }
    }

    /// `x = y = D` and `self.d = x = D`: `D` is aliased or stored, so it
    /// escapes (the inner target is also seen by [`Self::dict_use`]).
    fn note_chained_alias(&mut self, assignment: Node<'t>) {
        let mut value = assignment.child_by_field_name("right");
        let mut chained = false;
        for _ in 0..MAX_DEPTH {
            match value {
                Some(inner) if inner.kind() == "assignment" => {
                    chained = true;
                    value = inner.child_by_field_name("right");
                }
                _ => break,
            }
        }
        if let (Some(value), true) = (value, chained) {
            if value.kind() == "identifier" {
                self.escaped.insert(self.text(value));
            }
        }
    }

    /// `K.cfg` or `self.cfg` used where a dict held there may be changed
    /// (assigned to a name, passed, iterated): the attribute name
    /// escapes, so a dict aliased by an attribute of that name is not read.
    fn note_dict_attribute(&mut self, node: Node<'t>, frame: &Frame<'t>) {
        if matches!(self.dict_use(frame, true), DictUse::Safe) {
            return;
        }
        if let Some(attr) = node.child_by_field_name("attribute") {
            self.escaped.insert(self.text(attr));
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
            // A whole dict or list aliased by another name may change
            // through it: escaped (the supported grammar reads a container
            // only through its one name). A part assigned to a name escapes
            // its dict when that part is itself a container.
            ("assignment", Some("right")) if part => match parent.child_by_field_name("left") {
                Some(target) if target.kind() == "identifier" => DictUse::Alias(self.text(target)),
                _ => DictUse::Escape,
            },
            (kind, field) if !part && is_declaration(kind, field) => DictUse::Safe,
            _ => DictUse::Escape,
        }
    }

    /// Decides the exposed dict parts once every definition is known: a
    /// part that hands out a nested container or an entry that cannot be
    /// read, whether to a call, a loop or another name, lets its dict
    /// change, so the dict escapes; a name held in the dict is handed out
    /// and escapes too. There is no alias tracking to get wrong: a whole
    /// container assigned to another name has already escaped.
    fn settle_dict_escapes(&mut self) {
        for exposed in std::mem::take(&mut self.exposed) {
            let (alters, names) = self.exposed_contents(&exposed);
            if alters {
                self.escaped.insert(exposed.root);
            }
            self.escaped.extend(names.iter().copied());
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
        // Not a dict literal this file can read here (or past the depth
        // limit): it escapes, so no later read can trust it.
        let Some(mut dict) = self.dict_node(node, exposed.scope, 0) else {
            return (true, none);
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
        scope.line = node.start_position().row as u32 + 1;
        scope.name = name.clone();
        if self.scopes[parent].kind == ScopeKind::Class
            && matches!(
                name.as_str(),
                "__getattr__"
                    | "__getattribute__"
                    | "__setattr__"
                    | "__delattr__"
                    | "__init_subclass__"
            )
        {
            // Any attribute read or write may run it.
            self.attr_hooks
                .get_or_insert(Self::feature("an attribute hook (`__getattr__`...)", node));
        }
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
        scope.body_start = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        let id = self.scopes.len();
        scope.depth = self.scopes[parent].depth + 1;
        self.scopes.push(scope);
        if let Some(params) = node.child_by_field_name("parameters") {
            self.record_params(params, id);
        }
        self.bind_receivers(id);
        // A method binds in its class; any other `def` where an import would.
        let home = match self.scopes[parent].kind {
            ScopeKind::Class => parent,
            _ => self.import_home(parent, &name),
        };
        self.def_bindings.push((
            (home, name.clone()),
            parent,
            Place {
                at: node.end_byte(),
                region: None,
            },
        ));
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
        scope.self_recv = self.scopes[parent].self_recv;
        scope.cls_recv = self.scopes[parent].cls_recv;
        let id = self.scopes.len();
        scope.depth = self.scopes[parent].depth + 1;
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
        scope.line = node.start_position().row as u32 + 1;
        scope.lambda = true;
        scope.body_start = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        scope.bound_name = holder
            .filter(|h| h.kind() == "assignment" && field == Some("right"))
            .and_then(|h| h.child_by_field_name("left"))
            .filter(|left| left.kind() == "identifier")
            .map(|left| self.text(left).to_string());
        let id = self.scopes.len();
        scope.depth = self.scopes[parent].depth + 1;
        self.scopes.push(scope);
        if let Some(params) = node.child_by_field_name("parameters") {
            self.record_params(params, id);
        }
        self.bind_receivers(id);
        id
    }

    /// What `self` and `cls` hold in function or lambda `id` (see
    /// [`Scope::self_recv`]): the method's own class when it is a method
    /// directly in a class body, not a `staticmethod`, and the name is its
    /// first parameter; nothing when it binds the name some other way (a
    /// nested `def inner(self)`, a later parameter); else what the enclosing
    /// scope's name holds.
    fn bind_receivers(&mut self, id: usize) {
        let Some(parent) = self.scopes[id].parent else {
            return;
        };
        // A lambda in a class body is a method only when the body assigns it
        // to a name (`go = lambda self: ...`), not when it is passed
        // (`map(lambda self: ..., items)`).
        let method = self.scopes[parent].kind == ScopeKind::Class
            && !self.scopes[id].staticmethod
            && (!self.scopes[id].lambda || self.scopes[id].bound_name.is_some());
        let first = self.scopes[id].slots.first().map(|s| s.name.clone());
        for name in ["self", "cls"] {
            let bound = self.params.contains_key(&(id, name.to_string()));
            let recv = if !bound {
                match name {
                    "self" => self.scopes[parent].self_recv,
                    _ => self.scopes[parent].cls_recv,
                }
            } else if method && first.as_deref() == Some(name) {
                Some((parent, id))
            } else {
                None
            };
            match name {
                "self" => self.scopes[id].self_recv = recv,
                _ => self.scopes[id].cls_recv = recv,
            }
        }
    }

    /// The class and method `self` / `cls` (`name`) refers to in `scope`.
    fn receiver(&self, name: &str, scope: usize) -> Option<(usize, usize)> {
        match name {
            "self" => self.scopes[scope].self_recv,
            "cls" => self.scopes[scope].cls_recv,
            _ => None,
        }
    }

    /// `C.m` read as a value, recorded for [`Ctx::member_values`].
    fn note_member_value(&mut self, node: Node<'t>, frame: &Frame<'t>) {
        let (Some(object), Some(attr)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("attribute"),
        ) else {
            return;
        };
        if object.kind() != "identifier" || matches!(self.text(object), "self" | "cls") {
            return;
        }
        if let Some(parent) = frame.parent {
            let called = parent.kind() == "call" && frame.field == Some("function");
            if called || is_declaration(parent.kind(), frame.field) {
                return;
            }
        }
        self.member_values
            .insert((self.text(object), self.text(attr)));
    }

    /// Whether method `method` of its class is read as a value through the
    /// class (`B.go`), so it may run with another class's instance as
    /// `self`.
    fn is_borrowed(&self, method: usize) -> bool {
        let Some(class) = self.scopes[method].parent else {
            return false;
        };
        self.member_values.contains(&(
            self.scopes[class].name.as_str(),
            self.scopes[method].name.as_str(),
        ))
    }

    /// Lead decision D3: a method that binds its `self` or `cls` in any way
    /// other than as its first parameter, in its body or in a scope nested
    /// in it (an assignment, `+=`, a `for`, `with`, `except` or walrus
    /// target, a `match` capture, `del`, an import, a `def` or `class`, a
    /// closure's `nonlocal` or `global`), reads nothing through that name:
    /// every such read is a `dynamic_feature` gap. One pass over the
    /// scopes, after `nonlocal` assignments are placed.
    fn note_rebound_receivers(&mut self) {
        let mut found: Vec<(usize, bool)> = Vec::new();
        for s in 0..self.scopes.len() {
            for (name, is_self) in [("self", true), ("cls", false)] {
                let Some((_, method)) = self.receiver(name, s) else {
                    continue;
                };
                let key = (s, name.to_string());
                if self.assigns.contains_key(&key)
                    || self.imports.contains_key(&key)
                    || self.other_imports.contains_key(&key)
                    || self.functions.contains_key(&key)
                    || self.scopes[s].globals.contains(name)
                {
                    found.push((method, is_self));
                }
            }
        }
        for ((home, name), _, _) in &self.def_bindings {
            let is_self = name == "self";
            if is_self || name == "cls" {
                if let Some((_, method)) = self.receiver(name, *home) {
                    found.push((method, is_self));
                }
            }
        }
        for (method, is_self) in found {
            let line = self.scopes[method].line;
            self.rebound_receivers
                .entry((method, is_self))
                .or_insert(Feature {
                    what: "`self` or `cls` rebound in the method",
                    line,
                });
        }
    }

    /// The feature that makes a read through `object` unknown when it is
    /// `self` / `cls` of a method that rebinds the name (D3).
    fn rebound_receiver(&self, object: Node<'t>, scope: usize) -> Option<Feature> {
        if object.kind() != "identifier" || self.rebound_receivers.is_empty() {
            return None;
        }
        let name = self.text(object);
        let is_self = match name {
            "self" => true,
            "cls" => false,
            _ => return None,
        };
        let (_, method) = self.receiver(name, scope)?;
        self.rebound_receivers.get(&(method, is_self)).copied()
    }

    /// A write through `self` in a method another class may borrow sets
    /// that class's instances too, and one through a rebound `self` (D3)
    /// sets whatever the name holds then: an outside write of the
    /// attribute. So is a write with a run-time name through a rebound
    /// `self`.
    fn settle_borrowed_methods(&mut self) {
        if self.member_values.is_empty() && self.rebound_receivers.is_empty() {
            return;
        }
        for (method, attr, object) in std::mem::take(&mut self.method_self_writes) {
            let what = if self.is_borrowed(method) {
                "a write through `self` in a method another class may borrow"
            } else if self.rebound_receivers.contains_key(&(method, true))
                || self.rebound_receivers.contains_key(&(method, false))
            {
                "a write through `self` or `cls` in a method that rebinds it"
            } else {
                continue;
            };
            self.outside_attr_writes
                .entry(attr)
                .or_insert_with(|| Self::feature(what, object));
        }
        for (object, scope) in self.wild_attr_objects.clone() {
            if let Some(feature) = self.rebound_receiver(object, scope) {
                self.wild_attr_any.get_or_insert(feature);
            }
        }
    }

    /// Whether other modules can name `class` to subclass it: not
    /// `_private` (nor inside a private class) unless `__all__` exports
    /// it, not defined in a function, and not in a script.
    fn is_public_class(&self, class: usize) -> bool {
        if self.script {
            return false;
        }
        let mut current = Some(class);
        while let Some(s) = current {
            let scope = &self.scopes[s];
            match scope.kind {
                ScopeKind::Module => return true,
                ScopeKind::Function => return false,
                // `__all__` may export a private module-level class.
                ScopeKind::Class if scope.name.starts_with('_') => {
                    return scope.parent == Some(0)
                        && (self.exports_dynamic || self.exported.contains(scope.name.as_str()));
                }
                ScopeKind::Class => current = scope.parent,
            }
        }
        true
    }

    /// Whether another module may subclass `class` and set an attribute its
    /// methods read through `self`/`cls`: it is public, or one of its direct
    /// same-file subclasses is (deeper inheritance is a dynamic feature
    /// already, and so are more than [`MAX_SUBCLASSES`] subclasses).
    fn may_be_subclassed_elsewhere(&self, class: usize) -> bool {
        self.is_public_class(class)
            || self.subclasses.get(&class).is_some_and(|subs| {
                subs.iter()
                    .take(MAX_SUBCLASSES)
                    .any(|&sub| self.is_public_class(sub))
            })
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

    fn enter_class(&mut self, node: Node<'t>, parent: usize, wrapper: Option<Node<'t>>) -> usize {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        let mut bases = Vec::new();
        let mut external_bases = false;
        let mut other_bases = 0usize;
        let mut keywords = false;
        if let Some(list) = node.child_by_field_name("superclasses") {
            for base in named_children(list) {
                match base.kind() {
                    "identifier" | "attribute" => {
                        let text = self.text(base);
                        // Bases that add no attributes and no metaclass
                        // that rewrites them.
                        if matches!(text, "object" | "ABC" | "abc.ABC") {
                            continue;
                        }
                        external_bases |= !self.classes_by_name.contains_key(text);
                        bases.push(text.to_string());
                    }
                    "keyword_argument" | "dictionary_splat" => {
                        keywords = true;
                        external_bases = true;
                    }
                    "comment" => {}
                    _ => {
                        other_bases += 1;
                        external_bases = true;
                    }
                }
            }
        }
        let decorated = wrapper.is_some_and(|w| w.kind() == "decorated_definition");
        let line = node.start_position().row as u32 + 1;
        let what = if decorated {
            Some("a class decorator")
        } else if keywords {
            Some("a metaclass or class keyword")
        } else if bases.len() + other_bases > 1 {
            Some("multiple inheritance")
        } else {
            None
        };
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
        scope.external_bases = external_bases;
        scope.decorated = decorated;
        scope.class_feature = what.map(|what| Feature { what, line });
        scope.line = line;
        scope.depth = self.scopes[parent].depth + 1;
        self.scopes.push(scope);
        let home = match self.scopes[parent].kind {
            ScopeKind::Class => parent,
            _ => self.import_home(parent, &name),
        };
        self.def_bindings.push((
            (home, name.clone()),
            parent,
            Place {
                at: node.end_byte(),
                region: None,
            },
        ));
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
            iter: self.loop_iter,
        };
        match target.kind() {
            "identifier" => {
                let name = self.text(target).to_string();
                let current = &self.scopes[scope];
                let home = if current.kind != ScopeKind::Module && current.globals.contains(&name) {
                    0
                } else if current.kind != ScopeKind::Module && current.nonlocals.contains(&name) {
                    self.nonlocal_assigns.push((scope, name, assign));
                    return;
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
                let attr = self.text(attr);
                self.note_mutated_root(object);
                self.record_attr_write(object, attr, assign, scope);
            }
            "subscript" => {
                self.note_env_write(target);
                self.note_mutated_root(target);
                // `globals()["MODEL"] = v` rebinds a module-level name.
                if target
                    .child_by_field_name("value")
                    .is_some_and(|v| self.is_module_namespace(v, scope))
                {
                    match single_subscript(target).and_then(|k| self.plain_literal(k)) {
                        Some(name) => {
                            self.assigns
                                .entry((0, name.to_string()))
                                .or_default()
                                .push(assign);
                        }
                        None => self.dynamic_globals = true,
                    }
                    return;
                }
                // `obj.__dict__["model"] = v`, `vars(obj)[name] = v`.
                if let Some(owner) = target
                    .child_by_field_name("value")
                    .and_then(|v| self.attribute_dict_owner(v))
                {
                    match single_subscript(target).and_then(|k| self.plain_literal(k)) {
                        Some(attr) => self.record_attr_write(owner, attr, assign, scope),
                        None => self.note_wild_attr_write(owner, scope),
                    }
                }
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

    /// Records `object.attr = value` (or the same through `setattr` or
    /// `__dict__`): on `self`/`cls` or a class by name it is an assignment
    /// of that class's attribute; on anything else it is an outside write
    /// that may set any instance's attribute of that name.
    fn record_attr_write(
        &mut self,
        object: Node<'t>,
        attr: &'t str,
        assign: Assign<'t>,
        scope: usize,
    ) {
        // `super().__setattr__(...)` writes the instance.
        if is_super_call(object, |n| self.text(n)) {
            if let Some(class) = self.scopes[scope].class {
                self.attr_assigns
                    .entry((class, attr.to_string()))
                    .or_default()
                    .push(assign);
                return;
            }
        }
        // `sys.modules[__name__].MODEL = v` rebinds a module-level name.
        if self.is_module_object(object) {
            self.assigns
                .entry((0, attr.to_string()))
                .or_default()
                .push(assign);
            return;
        }
        if object.kind() == "identifier" {
            let object_name = self.text(object);
            // `self`/`cls` of a method: its class's attribute. Anywhere else
            // (a `staticmethod`, a nested `def inner(self)`) the name is not
            // the instance: an outside write, below.
            if let Some((class, method)) = self.receiver(object_name, scope) {
                self.attr_assigns
                    .entry((class, attr.to_string()))
                    .or_default()
                    .push(assign);
                self.method_self_writes.push((method, attr, object));
                return;
            }
            if let Some([class]) = self.classes_by_name.get(object_name).map(Vec::as_slice) {
                let class = *class;
                self.attr_assigns
                    .entry((class, attr.to_string()))
                    .or_default()
                    .push(assign);
                return;
            }
        }
        // Any instance's or class's attribute of that name may be this one:
        // reads of it are `dynamic_feature` gaps (the values are not
        // copied into every class's lookup).
        self.outside_attr_writes
            .entry(attr)
            .or_insert_with(|| Self::feature("a write of the attribute on another object", object));
        self.config_assigns.push(ConfigAssign {
            object,
            attr,
            value: assign.value,
            scope,
            top: self.config_top && scope == 0,
            plain: self.config_plain,
            at: object.start_byte(),
        });
    }

    /// The names a `match` case pattern binds: captures (`case model:`,
    /// `case [a, *rest]`, `case {"k": v, **kw}`, `case P(x=m)`) and
    /// `as` targets (`case str() as model`). A class name, a value pattern
    /// (`Color.RED`) and a keyword name bind nothing. Each holds only where
    /// its case matched, so none dominates what follows.
    fn record_pattern_captures(&mut self, pattern: Node<'t>, scope: usize) {
        // Each pattern is walked once: linear in its size.
        let mut stack = vec![(pattern, "")];
        while let Some((node, parent)) = stack.pop() {
            let bound = match node.kind() {
                "dotted_name"
                    if matches!(parent, "case_pattern" | "keyword_pattern" | "union_pattern") =>
                {
                    match named_children(node).as_slice() {
                        [only] => Some(*only),
                        _ => None,
                    }
                }
                "splat_pattern" => node.named_child(0),
                "identifier" if parent == "as_pattern" => Some(node),
                _ => None,
            };
            if let Some(target) = bound.filter(|t| t.kind() == "identifier" && self.text(*t) != "_")
            {
                let place = Place {
                    at: target.end_byte(),
                    region: None,
                };
                self.record_target(target, None, None, scope, true, place);
                continue;
            }
            if matches!(node.kind(), "dotted_name" | "identifier") {
                continue;
            }
            let mut cursor = node.walk();
            for (index, child) in node.named_children(&mut cursor).enumerate() {
                // A class pattern's first child is the class; a dict
                // pattern's keys are values.
                let skip = (node.kind() == "class_pattern" && index == 0)
                    || (node.kind() == "dict_pattern"
                        && child.kind() != "case_pattern"
                        && child.kind() != "splat_pattern");
                if !skip {
                    stack.push((child, node.kind()));
                }
            }
        }
    }

    /// `globals()["ask"]` hands out `ask`; `globals()[name]` may hand out
    /// any module-level function.
    fn note_namespace_lookup(&mut self, node: Node<'t>) {
        if !node
            .child_by_field_name("value")
            .is_some_and(|v| is_namespace_call(v, |n| self.text(n)))
        {
            return;
        }
        match single_subscript(node).and_then(|k| self.plain_literal(k)) {
            Some(name) => self.name_uses.entry(name).or_default().escapes = true,
            None => self.dynamic_function_calls = true,
        }
    }

    /// `sys.modules[...]`: a module object, taken to be this one.
    fn is_module_object(&self, node: Node<'t>) -> bool {
        node.kind() == "subscript" && self.render_links(node, 4) == "sys.modules[...]"
    }

    /// A write to an attribute named only at run time.
    fn note_wild_attr_write(&mut self, object: Node<'t>, scope: usize) {
        if self.is_module_object(object) {
            self.dynamic_globals = true;
            return;
        }
        let own = if object.kind() == "identifier" {
            self.receiver(self.text(object), scope)
                .map(|(class, _)| class)
        } else if is_super_call(object, |n| self.text(n)) {
            self.scopes[scope].class
        } else {
            None
        };
        let feature = Self::feature("an attribute written with a run-time name", object);
        match own {
            Some(class) => {
                self.wild_attr_writes.entry(class).or_insert(feature);
            }
            None => {
                self.wild_attr_any.get_or_insert(feature);
            }
        }
        self.wild_attr_objects.push((object, scope));
    }

    /// `setattr(obj, "attr", value)`, `setattr(obj, name, value)`, the same
    /// through `__setattr__`, `obj.__dict__.update(...)`,
    /// `vars(obj).update(...)`, and `getattr(obj, name)`, which may hand
    /// out any method.
    fn note_attr_write_call(&mut self, call: Node<'t>, scope: usize, place: Place) {
        let Some(function) = call.child_by_field_name("function") else {
            return;
        };
        let last = match function.kind() {
            "identifier" => self.text(function),
            "attribute" => function
                .child_by_field_name("attribute")
                .map(|a| self.text(a))
                .unwrap_or_default(),
            _ => return,
        };
        // Most calls are none of these: decide before reading arguments.
        if !matches!(
            last,
            "getattr"
                | "setattr"
                | "delattr"
                | "__setattr__"
                | "__delattr__"
                | "update"
                | "setdefault"
                | "__setitem__"
                | "__init__"
        ) {
            return;
        }
        let (positional, _) = call_arguments(call, |n| self.text(n));
        if function.kind() == "identifier" && self.text(function) == "getattr" {
            // A method or function looked up by name may be called with
            // any arguments.
            match positional
                .get(1)
                .map(|name| (name, self.plain_literal(*name)))
            {
                Some((_, Some(name))) => {
                    self.attr_uses.entry(name).or_default().escapes = true;
                    self.name_uses.entry(name).or_default().escapes = true;
                    // `getattr(_A(), "_A__go")`.
                    self.note_mangled_use(name);
                    // `getattr(os, "environ")`, `getattr(builtins, "exec")`.
                    if matches!(name, "environ" | "environb" | "putenv") {
                        self.env_wild.get_or_insert(Self::feature(
                            "`os.environ` or `os.putenv` looked up by name",
                            call,
                        ));
                    } else if is_namespace_builtin(name) || name == "modules" {
                        self.note_file_dynamic(
                            "a builtin that reads or rebinds names (`exec`, `eval`, `globals`...) looked up by name",
                            call,
                        );
                    }
                }
                Some((_, None)) => {
                    self.dynamic_method_calls = true;
                    if positional
                        .first()
                        .is_some_and(|object| self.is_module_object(*object))
                    {
                        self.dynamic_function_calls = true;
                    }
                }
                None => {}
            }
            return;
        }
        // `setattr(obj, name, v)`, `object.__setattr__(obj, name, v)`,
        // `obj.__setattr__(name, v)`, and the same with `delattr`, which
        // writes no value (the read falls back to whatever else holds it).
        let dunder =
            function.kind() == "attribute" && matches!(last, "__setattr__" | "__delattr__");
        let builtin = function.kind() == "identifier" && matches!(last, "setattr" | "delattr");
        let deleting = matches!(last, "delattr" | "__delattr__");
        let explicit = if deleting { 2 } else { 3 };
        let value_at = |i: usize| (!deleting).then(|| positional.get(i)).flatten();
        let target = if builtin || (dunder && positional.len() >= explicit) {
            Some((positional.first().copied(), positional.get(1), value_at(2)))
        } else if dunder {
            Some((
                function.child_by_field_name("object"),
                positional.first(),
                value_at(1),
            ))
        } else {
            None
        };
        if let Some((object, name, value)) = target {
            let (Some(object), Some(&name)) = (object, name) else {
                return;
            };
            let value = value.copied();
            match self.plain_literal(name) {
                Some(attr) if is_environ_name(attr) && !deleting => {
                    // `setattr(os, "environ", {...})`, like `os.environ =
                    // {...}`, replaces the process environment.
                    self.env_wild.get_or_insert(Self::feature(
                        "`os.environ` replaced through `setattr`",
                        call,
                    ));
                }
                Some(attr) => {
                    let assign = Assign {
                        id: self.fresh_id(),
                        scope,
                        value,
                        annotation: None,
                        unknowable: value.is_none(),
                        place,
                        iter: None,
                    };
                    self.record_attr_write(object, attr, assign, scope);
                }
                None => self.note_wild_attr_write(object, scope),
            }
            return;
        }
        if function.kind() != "attribute" {
            return;
        }
        let method = function
            .child_by_field_name("attribute")
            .map(|a| self.text(a))
            .unwrap_or_default();
        if matches!(method, "update" | "setdefault" | "__setitem__" | "__init__") {
            let object = function.child_by_field_name("object");
            if object.is_some_and(|o| self.is_module_namespace(o, scope)) {
                self.dynamic_globals = true;
            } else if let Some(owner) = object.and_then(|o| self.attribute_dict_owner(o)) {
                self.note_wild_attr_write(owner, scope);
            }
        }
    }

    /// `globals()`, or `vars()` / `locals()` at module level: the module
    /// namespace, which a write rebinds names in.
    fn is_module_namespace(&self, node: Node<'t>, scope: usize) -> bool {
        is_namespace_call(node, |n| self.text(n))
            && (scope == 0
                || node
                    .child_by_field_name("function")
                    .is_some_and(|f| self.text(f) == "globals"))
    }

    /// `obj` for `obj.__dict__` or `vars(obj)`.
    fn attribute_dict_owner(&self, node: Node<'t>) -> Option<Node<'t>> {
        match node.kind() {
            "attribute"
                if node
                    .child_by_field_name("attribute")
                    .is_some_and(|a| self.text(a) == "__dict__") =>
            {
                node.child_by_field_name("object")
            }
            "call"
                if node
                    .child_by_field_name("function")
                    .is_some_and(|f| f.kind() == "identifier" && self.text(f) == "vars") =>
            {
                call_arguments(node, |n| self.text(n)).0.first().copied()
            }
            _ => None,
        }
    }

    /// The text of a plain string literal (no prefix, interpolation or
    /// escape), borrowed from the source.
    fn plain_literal(&self, node: Node<'t>) -> Option<&'t str> {
        if node.kind() != "string" {
            return None;
        }
        let parts = children(node);
        let [start, content, _end] = parts.as_slice() else {
            return None;
        };
        let text = self.text(*content);
        (content.kind() == "string_content"
            && matches!(self.text(*start), "\"" | "'")
            && !text.contains('\\'))
        .then_some(text)
    }

    /// The scope an import or definition of `name` in `scope` binds in:
    /// the function itself (unless the name is declared `global`), a class
    /// for a method, else the module. Class-body imports are treated as
    /// module-level.
    fn import_home(&self, scope: usize, name: &str) -> usize {
        let current = &self.scopes[scope];
        if current.kind != ScopeKind::Function || current.globals.contains(name) {
            return 0;
        }
        if current.nonlocals.contains(name) {
            // `nonlocal x` then `import x` / `def x()`: the enclosing
            // function's binding.
            let mut parent = current.parent;
            while let Some(p) = parent {
                match self.scopes[p].kind {
                    ScopeKind::Function => return p,
                    ScopeKind::Module => return 0,
                    ScopeKind::Class => parent = self.scopes[p].parent,
                }
            }
        }
        scope
    }

    fn other_import(&mut self, scope: usize, local: String, module: &Rc<str>, place: Place) {
        self.class_body_import(scope, &local, place);
        let home = self.import_home(scope, &local);
        self.other_imports
            .entry((home, local))
            .or_default()
            .push(OtherImport {
                module: Rc::clone(module),
                place,
            });
    }

    /// Counts one more imported name at `statement`: `false` once the file
    /// has bound [`MAX_FILE_IMPORTS`], and from then on.
    fn import_budget(&mut self, statement: Node<'t>) -> bool {
        if self.import_cut.is_some() {
            return false;
        }
        if self.import_bindings >= MAX_FILE_IMPORTS {
            self.import_cut = Some(statement);
            return false;
        }
        self.import_bindings += 1;
        true
    }

    fn import_statement(&mut self, node: Node<'t>, scope: usize, place: Place) {
        let mut cursor = node.walk();
        let names: Vec<Node<'t>> = node.children_by_field_name("name", &mut cursor).collect();
        for item in names {
            if !self.import_budget(node) {
                return;
            }
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
            let written = self.text(module_node);
            if written == "__main__" {
                // The running script's namespace, which may be this one.
                self.note_file_dynamic("`import __main__`", node);
            }
            if alias.is_some() && matches!(written, "builtins" | "sys") {
                // `import builtins as B; B.exec(...)`, `import sys as S;
                // S.modules[...]`: the names this file matches are spelled
                // differently.
                self.note_file_dynamic("`builtins` or `sys` imported under another name", node);
            }
            self.note_self_import(written, node);
            let module = clip(written);
            let Some((pkg, rest)) = Pkg::of_module(written) else {
                let local = match alias {
                    Some(alias) => self.text(alias).to_string(),
                    None => written.split('.').next().unwrap_or("").to_string(),
                };
                self.other_import(scope, local, &Rc::from(module), place);
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
                        written.split('.').next().unwrap_or("").to_string(),
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
        if self.import_cut.is_some() {
            return;
        }
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        // Read and clipped once: every name below shares it.
        let written = self.text(module_node);
        let module = clip(written);
        let shared: Rc<str> = Rc::from(module.as_str());
        let relative = module_node.kind() == "relative_import";

        let mut cursor = node.walk();
        let names: Vec<Node<'t>> = node.children_by_field_name("name", &mut cursor).collect();
        let wildcard = named_children(node)
            .iter()
            .any(|c| c.kind() == "wildcard_import");

        let target = if relative {
            None
        } else {
            Pkg::of_module(written)
        };
        let reexports = if relative {
            None
        } else {
            langchain_reexport(written)
        };
        if !relative {
            self.note_self_import(written, node);
        }

        if wildcard {
            if target.is_none() {
                // `from settings import *` may rebind any module-level name
                // (`MODEL` among them) from here on.
                if self.import_budget(node) {
                    if self.module_stars.len() < MAX_STARS {
                        self.module_stars.push((place, Rc::clone(&shared)));
                    } else {
                        self.dynamic_globals = true;
                    }
                }
                return;
            }
            if let Some((pkg, _)) = target {
                for class in CLASSES.iter().filter(|c| c.pkg == pkg) {
                    if !self.import_budget(node) {
                        return;
                    }
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
            if !self.import_budget(node) {
                return;
            }
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
            if !relative {
                self.note_dynamic_import(written, &name, &local, node);
            }

            // `from google import genai` / `from google import generativeai`.
            if !relative && written == "google" {
                let pkg = match name.as_str() {
                    "genai" => Pkg::GoogleGenai,
                    "generativeai" => Pkg::GoogleGenerativeai,
                    _ => {
                        self.other_import(scope, local, &shared, place);
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

            // `from langchain_community.chat_models import ChatOpenAI`: the
            // class's provider decides its package.
            let reexport =
                reexports.and_then(|pkgs| pkgs.iter().find_map(|pkg| client_class(*pkg, &name)));
            if let Some(class) = reexport {
                let binding = Binding {
                    local: &local,
                    pkg: class.pkg,
                    module: &module,
                    name: Some(name),
                    kind: BindingKind::ClientClass,
                };
                self.add_import(binding, Val::Class(class), scope, item, place);
                continue;
            }

            let Some((pkg, rest)) = target.clone() else {
                self.other_import(scope, local, &shared, place);
                continue;
            };
            let (val, kind) = match client_class(pkg, &name) {
                Some(class) => (Val::Class(class), BindingKind::ClientClass),
                // Any other name from LangChain (`OpenAIEmbeddings`,
                // `ChatOllama`) is not followed.
                None if pkg.is_langchain() => {
                    self.other_import(scope, local, &shared, place);
                    continue;
                }
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
        self.class_body_import(scope, binding.local, place);
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
        // Only the SDK itself makes a bare `messages.create` its shape.
        if !binding.pkg.is_langchain() {
            self.sdk_imported.insert(binding.pkg.sdk());
        }
        self.legacy_imported |= binding.pkg == Pkg::GoogleGenerativeai;
        let span = self.span(node);
        self.out.imports.push(SdkImport {
            sdk: binding.pkg.sdk(),
            package: binding.pkg.package().to_string(),
            module: binding.module.to_string(),
            name: binding.name.map(clip_owned),
            local: clip(binding.local),
            kind: binding.kind,
            file: self.file.clone(),
            span,
        });
    }

    /// An import in a class body binds a class attribute too (`class Bot:
    /// model = "gpt-4o"; from cfg import model`): when the class also
    /// assigns that name, it is one more assignment, of no known value (see
    /// [`Ctx::bind_definitions`]). The import is still bound at module
    /// level, as before.
    fn class_body_import(&mut self, scope: usize, local: &str, place: Place) {
        if self.scopes[scope].kind == ScopeKind::Class {
            self.def_bindings
                .push(((scope, local.to_string()), scope, place));
        }
    }

    /// `type M = ...` binds `M` to a type alias object: an assignment of no
    /// known value. tree-sitter also reads `type(self).model = v` as a type
    /// alias statement whose left side is an attribute (`(self).model`):
    /// that is a write of `model` on an object this file cannot identify.
    fn record_type_alias(&mut self, node: Node<'t>, scope: usize, place: Place) {
        let parts = named_children(node);
        let (Some(left), value) = (
            parts.first().and_then(|t| t.named_child(0)),
            parts.get(1).and_then(|t| t.named_child(0)),
        ) else {
            return;
        };
        match left.kind() {
            "identifier" => self.record_target(left, None, None, scope, true, place),
            "generic_type" => {
                if let Some(name) = left.named_child(0).filter(|n| n.kind() == "identifier") {
                    self.record_target(name, None, None, scope, true, place);
                }
            }
            "attribute" => {
                let (Some(object), Some(attr)) = (
                    left.child_by_field_name("object"),
                    left.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                let attr = self.text(attr);
                let assign = Assign {
                    id: self.fresh_id(),
                    scope,
                    value,
                    annotation: None,
                    unknowable: value.is_none(),
                    place,
                    iter: None,
                };
                self.note_mutated_root(object);
                self.record_attr_write(object, attr, assign, scope);
            }
            _ => {}
        }
    }

    /// A `def`, `class` or class-body import that rebinds an assigned name
    /// (`MODEL = "gpt-4o"` then `def MODEL(): ...`) is one more definition
    /// of it, of no known value. Elsewhere names never assigned are left
    /// alone, so functions, classes and imports still resolve as before.
    /// In a class body every one of them is a write of that class
    /// attribute (lead decision D4), whatever the class assigns: a read
    /// through the class, a subclass or a base that may see it then has an
    /// assignment of no known value, and a gap.
    fn bind_definitions(&mut self) {
        for (key, scope, place) in std::mem::take(&mut self.def_bindings) {
            let class_body = self.scopes[key.0].kind == ScopeKind::Class;
            if !class_body && !self.assigns.contains_key(&key) {
                continue;
            }
            // `scope` is where the statement runs: a `global` def in a
            // function binds the module's name whenever it is called.
            let assign = Assign {
                id: self.fresh_id(),
                scope,
                value: None,
                annotation: None,
                unknowable: true,
                place,
                iter: None,
            };
            self.assigns.entry(key).or_default().push(assign);
        }
    }

    /// Moves each assignment to a `nonlocal` name into the nearest
    /// enclosing function that binds the name (Python's rule), or the
    /// nearest enclosing function when none does. Its value expression
    /// stays in the nested function, so it reaches every use there as a
    /// definition that may run at any time.
    fn place_nonlocal_assigns(&mut self) {
        for (declared, name, assign) in std::mem::take(&mut self.nonlocal_assigns) {
            let mut nearest = None;
            let mut home = None;
            let mut current = self.scopes[declared].parent;
            while let Some(s) = current {
                let scope = &self.scopes[s];
                match scope.kind {
                    ScopeKind::Module => break,
                    ScopeKind::Class => {}
                    ScopeKind::Function => {
                        nearest.get_or_insert(s);
                        if !scope.nonlocals.contains(&name) && self.binds(s, &name) {
                            home = Some(s);
                            break;
                        }
                    }
                }
                current = scope.parent;
            }
            let home = home.or(nearest).unwrap_or(declared);
            self.assigns.entry((home, name)).or_default().push(assign);
        }
    }

    /// Orders every name's definitions by position, for reaching-definition
    /// queries.
    fn index_definitions(&mut self) {
        for class in 0..self.scopes.len() {
            if self.scopes[class].kind != ScopeKind::Class {
                continue;
            }
            for base in self.scopes[class].bases.iter().take(MAX_BASES) {
                match self.classes_by_name.get(base).map(Vec::as_slice) {
                    Some([parent]) if *parent != class => {
                        self.subclasses.entry(*parent).or_default().push(class);
                    }
                    // Any class of that name may be the base: none of them
                    // knows its subclasses (no walk over every pair).
                    Some(many) if many.len() > 1 => {
                        self.ambiguous_base_names.insert(base.clone());
                    }
                    _ => {}
                }
            }
        }
        let mut keys: HashSet<Key> = HashSet::new();
        let module_wide = !self.module_stars.is_empty() || self.dynamic_globals;
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
            if home == 0 && module_wide {
                // It may not bind this name: it never dominates.
                for (i, (place, _)) in self.module_stars.iter().enumerate() {
                    local.push(DefPos {
                        place: Place {
                            at: place.at,
                            region: None,
                        },
                        def: DefRef::Star(i),
                    });
                }
                if self.dynamic_globals {
                    foreign.push(DefRef::Dynamic);
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
        // Every foreign definition reaches every use: past the scan limit,
        // listing them per use would make many uses quadratic.
        if chosen.is_empty()
            || index.foreign.len() > MAX_REACH_SCAN
            || chosen.len() + index.foreign.len() >= index.total
        {
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
        if key.0 == 0 {
            defs.extend((0..self.module_stars.len()).map(DefRef::Star));
            if self.dynamic_globals {
                defs.push(DefRef::Dynamic);
            }
        }
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
                        || self.other_imports.contains_key(&key)
                        || (key.0 == 0 && (!self.module_stars.is_empty() || self.dynamic_globals)),
                    assigns,
                    only_assign: (assigns == 1).then_some(0),
                }
            }
            Reach::Subset(defs) => {
                let mut kinds = DefKinds::default();
                for def in defs {
                    match def {
                        DefRef::Import(_)
                        | DefRef::Other(_)
                        | DefRef::Star(_)
                        | DefRef::Dynamic => kinds.imported = true,
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
                if let Some(feature) = self.rebound_receiver(object, scope) {
                    return Val::untracked(Origin::Dynamic(feature));
                }
                if let Some((class, dynamic)) = self.class_receiver(object, scope) {
                    return self.resolve_attr(class, &attr, dynamic);
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
    /// object is one of those, and whether it is `self`/`cls`: an instance
    /// that may be a subclass's, or have its attributes written from
    /// outside.
    fn class_receiver(&self, object: Node<'t>, scope: usize) -> Option<(usize, bool)> {
        if object.kind() != "identifier" {
            return None;
        }
        let name = self.text(object);
        if matches!(name, "self" | "cls") {
            // Only a method's own first parameter is its instance (or
            // class), and not when another class may borrow the method.
            let (class, method) = self.receiver(name, scope)?;
            if self.rebound_receiver(object, scope).is_some() {
                return None;
            }
            return (!self.is_borrowed(method)).then_some((class, true));
        }
        // The one class of that name, which nothing else rebinds (`Bot =
        // wrap(Bot)`, an import, a function of the same name).
        match self.classes_by_name.get(name).map(Vec::as_slice) {
            Some([class])
                if !self.is_bound_locally(name, scope)
                    && !self.binds(self.class_home(*class), name) =>
            {
                Some((*class, false))
            }
            _ => None,
        }
    }

    /// The scope a class definition binds its name in: the enclosing
    /// function, else the module.
    fn class_home(&self, class: usize) -> usize {
        let mut parent = self.scopes[class].parent;
        while let Some(p) = parent {
            match self.scopes[p].kind {
                ScopeKind::Function => return p,
                ScopeKind::Module => return 0,
                ScopeKind::Class => parent = self.scopes[p].parent,
            }
        }
        0
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

    /// Values of the imports, parameter and assignments among `defs`. A
    /// name imported from another module is an untracked value next to
    /// the others (`except ImportError: from mylib import Client`).
    fn def_values(&mut self, key: &Key, defs: &[DefRef]) -> Vec<Val> {
        let mut vals = Vec::new();
        for def in defs {
            match *def {
                DefRef::Import(i) => {
                    if let Some(import) = self.imports.get(key).and_then(|v| v.get(i)) {
                        vals.push(import.val.clone());
                    }
                }
                DefRef::Other(i) => {
                    if let Some(import) = self.other_imports.get(key).and_then(|v| v.get(i)) {
                        vals.push(Val::untracked(Origin::Imported {
                            module: import.module.to_string(),
                        }));
                    }
                }
                DefRef::Star(i) => {
                    if let Some((_, module)) = self.module_stars.get(i) {
                        vals.push(Val::untracked(Origin::Imported {
                            module: module.to_string(),
                        }));
                    }
                }
                DefRef::Dynamic => vals.push(Val::untracked(Origin::Other)),
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
                .map(|o| o.module.to_string()),
            DefRef::Star(i) => self.module_stars.get(*i).map(|(_, m)| m.to_string()),
            _ => None,
        });
        match module {
            Some(module) => Val::untracked(Origin::Imported { module }),
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
        match default_client {
            // No in-file call passes the argument, or the calls cannot all
            // be seen: the default holds only where nobody else calls.
            Some(Val::Client { mut client, path }) => {
                // Calls past the read limit are not seen either.
                let unseen = self
                    .calls_of(scope)
                    .is_none_or(|calls| calls.len() > MAX_CALLERS_READ);
                if (unseen || !self.is_private(scope)) && client.caveat.is_none() {
                    client.caveat = Some(ClientCaveat::ParameterDefault {
                        param: name.to_string(),
                        unseen,
                    });
                }
                Val::Client { client, path }
            }
            Some(other) => other,
            None => Val::untracked(Origin::Parameter {
                name: name.to_string(),
            }),
        }
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
            // argument, so its default stays possible, and with another
            // client, so the result keeps a gap.
            let mut all = vals;
            if !private {
                all.push(default);
            }
            return Some(match combine(all) {
                Val::Client { mut client, path } if !private => {
                    if !client.ambiguous && client.caveat.is_none() {
                        client.caveat = Some(ClientCaveat::ParameterDefault {
                            param: name.to_string(),
                            unseen: false,
                        });
                    }
                    Val::Client { client, path }
                }
                other => other,
            });
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
        if calls.len() > MAX_CALLERS_READ {
            return None;
        }
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
        // A constructor is called wherever its class is: as private as the
        // class.
        if self.scopes[parent].kind == ScopeKind::Class && scope.name == "__init__" && !scope.lambda
        {
            return self.is_private(parent);
        }
        let name = match &scope.bound_name {
            Some(name) if scope.lambda => name.as_str(),
            _ => scope.name.as_str(),
        };
        name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__"))
    }

    /// Every in-file call of function or method `func`, or `None` when some
    /// may not be seen (see [`Self::visible_calls`]).
    fn calls_of(&mut self, func: usize) -> Option<CallSites<'t>> {
        match self.visible_calls(func) {
            Some((calls, true)) => Some(calls),
            _ => None,
        }
    }

    /// The in-file calls of function or method `func` that can be seen, and
    /// whether they are all of its calls. They are not when it is
    /// decorated, its name is used as a value (passed, stored, partially
    /// applied, `ask.delay(...)`) or bound more than once; `None` for a
    /// comprehension or a lambda not assigned to a plain name, which is
    /// never called by name. A method's calls are every call of its name on
    /// any object (a superset: an unrelated method of the same name only
    /// adds candidates), and `__init__`'s are the calls of its class. Calls
    /// are indexed once per name (by binding scope for functions), so a file
    /// with thousands of same-named functions stays linear.
    fn visible_calls(&mut self, func: usize) -> Option<(CallSites<'t>, bool)> {
        let scope = &self.scopes[func];
        if scope.comprehension {
            return None;
        }
        let lambda = scope.lambda;
        let decorated = scope.decorated;
        let fname = if lambda {
            scope.bound_name.clone()?
        } else {
            scope.name.clone()
        };
        let parent = scope.parent?;
        let empty = || Rc::from(Vec::new());
        if self.scopes[parent].kind == ScopeKind::Class {
            if lambda {
                return None;
            }
            if fname == "__init__" {
                let (calls, all) = self.constructor_calls(parent);
                return Some((calls, all && !decorated));
            }
            let escapes = self.dynamic_method_calls
                || self
                    .attr_uses
                    .get(fname.as_str())
                    .is_some_and(|uses| uses.escapes);
            let calls = self.method_calls(&fname);
            return Some((calls, !escapes && !decorated));
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
        let escapes = ((self.dynamic_function_calls || self.exports_dynamic) && home == 0)
            || self
                .name_uses
                .get(fname.as_str())
                .is_some_and(|uses| uses.escapes);
        let index = self.name_calls(&fname);
        let calls = index.get(&home).map_or_else(empty, Rc::clone);
        Some((calls, only_def && !escapes && !decorated))
    }

    /// The calls that run `__init__` of `class`: calls of the class name
    /// where it binds to this class. They are all of them only when the
    /// class is the one class of that name, its name is bound nowhere else
    /// and is never used as a value (subclassed, passed, stored, its
    /// `__init__` called explicitly).
    fn constructor_calls(&mut self, class: usize) -> (CallSites<'t>, bool) {
        let name = self.scopes[class].name.clone();
        let Some(parent) = self.scopes[class].parent else {
            return (Rc::from(Vec::new()), false);
        };
        let home = self.import_home(parent, &name);
        let key = (home, name.clone());
        // A class decorator may wrap or replace the class, and an instance
        // built without naming its class (`type(self)(...)`, `cls(...)`,
        // `self.__class__(...)`) is a constructor call this index misses.
        let unique = self.classes_by_name.get(&name).map_or(0, Vec::len) == 1
            && !self.binds(home, &name)
            && !self.scopes[class].decorated
            && self.dynamic_constructors.is_none();
        let escapes = (self.exports_dynamic && home == 0)
            || self
                .name_uses
                .get(name.as_str())
                .is_some_and(|uses| uses.escapes);
        let index = self.name_calls(&key.1);
        let calls = index
            .get(&home)
            .map_or_else(|| Rc::from(Vec::new()), Rc::clone);
        (calls, unique && !escapes)
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

    /// Every call of method name `name`, on any object. Built once per name.
    fn method_calls(&mut self, name: &str) -> CallSites<'t> {
        if let Some(calls) = self.method_call_index.get(name) {
            return Rc::clone(calls);
        }
        let calls: CallSites<'t> = match self.attr_uses.get(name) {
            Some(uses) => Rc::from(uses.calls.as_slice()),
            None => Rc::from(Vec::new()),
        };
        self.method_call_index
            .insert(name.to_string(), Rc::clone(&calls));
        calls
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

    /// A class's own assignments of attribute `attr`: class-body names and
    /// `self.attr = ...`, `cls.attr = ...` or `Class.attr = ...`, read once
    /// per class and name.
    fn own_attrs(&mut self, class: usize, attr: &str) -> Rc<[Assign<'t>]> {
        let key = (class, attr.to_string());
        if let Some(found) = self.own_attr_memo.get(&key) {
            return Rc::clone(found);
        }
        let mut found: Vec<Assign<'t>> = Vec::new();
        found.extend(self.assigns.get(&key).into_iter().flatten().copied());
        found.extend(self.attr_assigns.get(&key).into_iter().flatten().copied());
        let found: Rc<[Assign<'t>]> = Rc::from(found);
        self.own_attr_memo.insert(key, Rc::clone(&found));
        found
    }

    /// Where `class` sits in its hierarchy: its one same-file base (one
    /// level only), whether a base comes from outside the file, and why its
    /// attributes may change whatever its body says (see [`ClassShape`]).
    fn class_shape(&self, class: usize) -> ClassShape {
        let scope = &self.scopes[class];
        let mut shape = ClassShape {
            base: None,
            external: scope.external_bases,
            feature: scope.class_feature,
        };
        let [base] = scope.bases.as_slice() else {
            return shape;
        };
        let at = |what| {
            Some(Feature {
                what,
                line: scope.line,
            })
        };
        match self.classes_by_name.get(base).map(Vec::as_slice) {
            Some([parent]) if *parent != class && !self.binds(0, base) => {
                let parent_scope = &self.scopes[*parent];
                shape.base = Some(*parent);
                shape.external |= parent_scope.external_bases;
                if shape.feature.is_none() {
                    if parent_scope.class_feature.is_some() {
                        shape.feature =
                            at("a base class with a decorator, a metaclass or several bases");
                    } else if parent_scope
                        .bases
                        .iter()
                        .any(|b| self.classes_by_name.contains_key(b))
                    {
                        shape.feature = at("inheritance more than one level deep");
                    }
                }
            }
            Some(_) if shape.feature.is_none() => {
                shape.feature = at("a base class name bound more than once");
            }
            _ => {}
        }
        shape
    }

    /// What attribute `attr` read through `class` may hold: by the class's
    /// name (`Bot.model`) or, when `through_self`, through `self`/`cls` in
    /// its methods, where the instance may also be a direct subclass's.
    ///
    /// The supported grammar: the class's own assignments, else its one
    /// same-file base's, plus (through `self`) those of at most
    /// [`MAX_SUBCLASSES`] direct subclasses that have no subclasses of
    /// their own. Anything else that may change the attribute is a dynamic
    /// feature, reported as a gap instead of resolved: a decorator,
    /// metaclass or several bases on the class, its base or a subclass;
    /// deeper inheritance; an attribute hook; a write of that attribute
    /// name on any other object; a write with a run-time name; the class
    /// used as a value; a method of the same name; a class attribute of a
    /// class with a base outside the file. Memoized; constant work per
    /// lookup, however many classes and writes the file has.
    fn attr_lookup(&mut self, class: usize, attr: &str, through_self: bool) -> Rc<AttrLookup<'t>> {
        let key = (class, attr.to_string());
        let memo = if through_self {
            &self.self_attr_memo
        } else {
            &self.class_attr_memo
        };
        if let Some(found) = memo.get(&key) {
            return Rc::clone(found);
        }
        let shape = self.class_shape(class);
        let line = self.scopes[class].line;
        let at = |what| Feature { what, line };
        let shadowed = |cx: &Self, c: usize| cx.functions.contains_key(&(c, attr.to_string()));
        let mut dynamic = shape
            .feature
            .or(self.attr_hooks)
            .or(self.wild_attr_any)
            .or_else(|| self.outside_attr_writes.get(attr).copied())
            .or_else(|| self.wild_attr_writes.get(&class).copied())
            .or_else(|| {
                shape
                    .base
                    .and_then(|b| self.wild_attr_writes.get(&b).copied())
            })
            .or_else(|| {
                self.class_escapes
                    .get(self.scopes[class].name.as_str())
                    .copied()
            });
        if dynamic.is_none()
            && (shadowed(self, class) || shape.base.is_some_and(|b| shadowed(self, b)))
        {
            dynamic = Some(at("a method of the same name"));
        }
        let mut from = class;
        let mut assigns = self.own_attrs(class, attr);
        if assigns.is_empty() {
            if let Some(base) = shape.base {
                from = base;
                assigns = self.own_attrs(base, attr);
            }
        }
        // A class attribute of a class with a base from outside the file
        // may be a field its constructor sets (a dataclass, a pydantic or
        // attrs model, a NamedTuple) or one its metaclass rewrites (an
        // enum member).
        let external = if from == class {
            shape.external
        } else {
            self.scopes[from].external_bases
        };
        if dynamic.is_none() && external && assigns.iter().any(|a| a.scope == from) {
            dynamic = Some(at(
                "a class attribute of a class with a base outside this file",
            ));
        }
        let mut extra: Vec<Assign<'t>> = Vec::new();
        if through_self {
            if dynamic.is_none() && self.ambiguous_base_names.contains(&self.scopes[class].name) {
                dynamic = Some(at("a base class name more than one class has"));
            }
            let subs: Vec<usize> = self
                .subclasses
                .get(&class)
                .map(|subs| subs.iter().take(MAX_SUBCLASSES + 1).copied().collect())
                .unwrap_or_default();
            if subs.len() > MAX_SUBCLASSES {
                dynamic.get_or_insert(at("more than 16 subclasses"));
            }
            for &sub in subs.iter().take(MAX_SUBCLASSES) {
                let sub_line = self.scopes[sub].line;
                let found = if self.subclasses.contains_key(&sub) {
                    Some("inheritance more than one level deep")
                } else if self.scopes[sub].class_feature.is_some() {
                    Some("a subclass with a decorator, a metaclass or several bases")
                } else if self.wild_attr_writes.contains_key(&sub) {
                    Some("an attribute written with a run-time name")
                } else if shadowed(self, sub) {
                    Some("a method of the same name")
                } else {
                    None
                };
                if let Some(what) = found {
                    dynamic.get_or_insert(Feature {
                        what,
                        line: sub_line,
                    });
                }
                let own = self.own_attrs(sub, attr);
                if own.len() > MAX_SUBCLASS_ASSIGNS {
                    dynamic.get_or_insert(Feature {
                        what: "more assignments in a subclass than are read",
                        line: sub_line,
                    });
                }
                extra.extend(own.iter().take(MAX_SUBCLASS_ASSIGNS).copied());
            }
            // Set only by subclasses: an abstract attribute, which
            // subclasses in other modules may set too.
            if assigns.is_empty() && !extra.is_empty() {
                dynamic.get_or_insert(at("an attribute only subclasses assign"));
            }
        }
        if assigns.len() > MAX_ATTR_ASSIGNS {
            dynamic.get_or_insert(at("more assignments of the attribute than are read"));
        }
        let assigns: Rc<[Assign<'t>]> = if extra.is_empty() && assigns.len() <= MAX_ATTR_ASSIGNS {
            assigns
        } else {
            assigns
                .iter()
                .take(MAX_ATTR_ASSIGNS)
                .copied()
                .chain(extra)
                .collect()
        };
        let found = Rc::new(AttrLookup { assigns, dynamic });
        let memo = if through_self {
            &mut self.self_attr_memo
        } else {
            &mut self.class_attr_memo
        };
        memo.insert(key, Rc::clone(&found));
        found
    }

    /// The value of attribute `attr` of `class`, read through `self`/`cls`
    /// when `through_self` (see [`Self::attr_lookup`]). A dynamic feature
    /// is combined in as a value of its own: a client next to it keeps a
    /// `dynamic_feature` gap.
    fn resolve_attr(&mut self, class: usize, attr: &str, through_self: bool) -> Val {
        let key = (class, attr.to_string());
        let budget = self.budget();
        let memo = if through_self {
            &self.dyn_attr_memo
        } else {
            &self.attr_memo
        };
        if let Some(val) = self.reuse(recall(memo, &key, budget)) {
            return val;
        }
        let guard = (
            if through_self { 5u8 } else { 1u8 },
            class,
            attr.to_string(),
        );
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }
        let cutoffs = self.cutoffs;
        if !self.enter_name() {
            self.visiting.remove(&guard);
            return Val::untracked(Origin::Other);
        }
        let lookup = self.attr_lookup(class, attr, through_self);
        let mut vals: Vec<Val> = lookup
            .assigns
            .iter()
            .map(|a| self.assign_value_memo(*a))
            .collect();
        if let Some(feature) = lookup.dynamic {
            vals.push(Val::untracked(Origin::Dynamic(feature)));
        }
        let val = if vals.is_empty() {
            Val::untracked(Origin::UnassignedAttribute {
                name: attr.to_string(),
            })
        } else {
            combine(vals)
        };
        self.leave_name();
        self.visiting.remove(&guard);
        let memo = self.memo(val.clone(), cutoffs, budget);
        if through_self {
            self.dyn_attr_memo.insert(key, memo);
        } else {
            self.attr_memo.insert(key, memo);
        }
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
                    // A LangChain chat model's `copy(update=...)` may change
                    // any field, so its copy is not followed.
                    "with_options" | "copy" if !pkg.is_langchain() => Val::Client {
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
            Val::Untracked(Origin::MixedClasses(pkg)) => Val::Client {
                client: Client {
                    pkg,
                    routing: Routing::Unknown,
                    line: None,
                    site: None,
                    ambiguous: true,
                    typed: false,
                    caveat: None,
                },
                path: Vec::new(),
            },
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
    /// `base_url` there, so a direct client given one routes by what it is
    /// (see [`Self::base_url_kind`]), and options passed through an
    /// unreadable `**kwargs` make it unknown.
    fn copied_client(&mut self, mut client: Client, call: Node<'t>, scope: usize) -> Client {
        let args = self.arguments(call);
        let mut base_url = BaseUrl::Absent;
        let mut opaque = false;
        if let Some(expr) = keyword_value(call, |n| self.text(n), is_base_url_key) {
            base_url = self.base_url_kind(client.pkg, expr, scope);
        }
        for splat in &args.splats {
            match self.expand_splat(*splat, scope) {
                Some(expansion) => {
                    let given = expansion
                        .summary
                        .expansion
                        .iter()
                        .find(|(k, _, _)| is_base_url_key(k))
                        .map(|(_, index, v)| (*index, *v));
                    match given {
                        // A later `**other` may replace it.
                        Some((index, _)) if expansion.summary.overridable(index) => {
                            base_url = BaseUrl::Unknown { env: false };
                        }
                        Some((_, expr)) => {
                            base_url = self.base_url_kind(client.pkg, expr, expansion.scope);
                        }
                        None => {}
                    }
                    opaque |= expansion.opaque();
                }
                None => opaque = true,
            }
        }
        let direct = matches!(
            client.routing,
            Routing::OpenaiDirect | Routing::AnthropicDirect
        );
        if !direct {
            return client;
        }
        match base_url {
            BaseUrl::Custom => client.routing = Routing::CustomBaseUrl,
            BaseUrl::CustomFallback => {
                client.routing = Routing::CustomBaseUrl;
                client.caveat.get_or_insert(ClientCaveat::BaseUrl {
                    env: true,
                    fallback: true,
                });
            }
            BaseUrl::Unknown { env } => {
                client.routing = Routing::Unknown;
                client.caveat.get_or_insert(ClientCaveat::BaseUrl {
                    env,
                    fallback: false,
                });
            }
            BaseUrl::ProviderFallback => {
                client.caveat.get_or_insert(ClientCaveat::BaseUrl {
                    env: false,
                    fallback: true,
                });
            }
            // `with_options(**opts)`: the expansion may hold a `base_url`.
            BaseUrl::Absent if opaque => {
                client.routing = Routing::Unknown;
                client.caveat.get_or_insert(ClientCaveat::BaseUrl {
                    env: false,
                    fallback: false,
                });
            }
            BaseUrl::Absent | BaseUrl::Provider => {}
        }
        client
    }

    /// What a `base_url` argument of a `pkg` client routes it to (P6): the
    /// provider's own host is direct, any other host a custom route, the
    /// provider's host as an environment fallback direct with a gap, and an
    /// environment lookup with no fallback, or anything that does not
    /// resolve to a string, unknown with a gap. The URL is never reported.
    fn base_url_kind(&mut self, pkg: Pkg, expr: Node<'t>, scope: usize) -> BaseUrl {
        if expr.kind() == "none" {
            return BaseUrl::Absent;
        }
        // A literal at the call: its host decides, credentials or not.
        if let Ok(url) = self.literal(expr) {
            return host_kind(pkg, &url);
        }
        match self.resolve_value(expr, scope, 0) {
            Ok(resolved) if resolved.origin == ValueOrigin::EnvDefault => {
                match host_kind(pkg, &resolved.value) {
                    BaseUrl::Provider => BaseUrl::ProviderFallback,
                    _ => BaseUrl::CustomFallback,
                }
            }
            Ok(resolved) if resolved.caveats.is_empty() => host_kind(pkg, &resolved.value),
            Ok(_) => BaseUrl::Unknown { env: false },
            Err(unresolved) => BaseUrl::Unknown {
                env: unresolved.reason == "env_var",
            },
        }
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
        if let Some((index, caveat)) = self.constructions.get(&call.id()).cloned() {
            let built = &self.out.clients[index];
            return Client {
                pkg: class.pkg,
                routing: built.routing,
                line: Some(built.span.line),
                site,
                ambiguous: false,
                typed: false,
                caveat,
            };
        }
        let scope = self.call_scope.get(&call.id()).copied().unwrap_or(0);
        let args = self.arguments(call);
        let direct = matches!(class.family, Family::OpenaiDirect | Family::AnthropicDirect);
        let base_url = match keyword_value(call, |n| self.text(n), is_base_url_key) {
            Some(expr) if direct => self.base_url_kind(class.pkg, expr, scope),
            Some(_) => BaseUrl::Custom,
            None => BaseUrl::Absent,
        };
        let mut routing = class.routing(&args.kwargs, args.splat, base_url);
        let mut caveat = match self.genai_endpoint_option(call, class, &args) {
            Some(option) => Some(ClientCaveat::EndpointOption { option }),
            None => self.endpoint_env(class, base_url),
        };
        if caveat.is_some() {
            routing = Routing::Unknown;
        }
        if direct {
            match base_url {
                BaseUrl::CustomFallback => {
                    caveat.get_or_insert(ClientCaveat::BaseUrl {
                        env: true,
                        fallback: true,
                    });
                }
                BaseUrl::Unknown { env } => {
                    caveat.get_or_insert(ClientCaveat::BaseUrl {
                        env,
                        fallback: false,
                    });
                }
                BaseUrl::ProviderFallback => {
                    caveat.get_or_insert(ClientCaveat::BaseUrl {
                        env: false,
                        fallback: true,
                    });
                }
                // `OpenAI(**options)`: the expansion may hold a `base_url`.
                BaseUrl::Absent if args.splat => {
                    caveat.get_or_insert(ClientCaveat::BaseUrl {
                        env: false,
                        fallback: false,
                    });
                }
                _ => {}
            }
        }
        let (binding, binding_scope) = self.binding_target(call, scope);
        let chain = call
            .child_by_field_name("function")
            .map(|f| self.render(f))
            .unwrap_or_default();
        let span = self.span(call);
        // Only the class of the value: `True`, `False`/`None`, or anything
        // decided at run time.
        let vertexai = (class.family == Family::Genai)
            .then(|| keyword_value(call, |n| self.text(n), |k| k == "vertexai"))
            .flatten()
            .map(|value| match value.kind() {
                "true" => VertexAi::On,
                "false" | "none" => VertexAi::Off,
                _ => VertexAi::Unknown,
            });
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
            vertexai,
        };
        self.constructions
            .insert(call.id(), (self.out.clients.len(), caveat.clone()));
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

    /// For `google.genai.Client(...)`: the option that may send its requests
    /// to Vertex AI or another endpoint instead of the Gemini API, if any.
    /// `vertexai=` (unless `False` or `None`), `http_options=` (unless `None`
    /// or a dict literal or `HttpOptions(...)` call that names no
    /// `base_url` and expands nothing) and a `**` expansion each count.
    fn genai_endpoint_option(
        &self,
        call: Node<'t>,
        class: ClientClass,
        args: &Args<'t>,
    ) -> Option<&'static str> {
        if class.family != Family::Genai {
            return None;
        }
        if args.splat {
            return Some("a `**` expansion");
        }
        let text = |n: Node<'t>| self.text(n);
        if let Some(value) = keyword_value(call, text, |k| k == "vertexai") {
            if !matches!(value.kind(), "false" | "none") {
                return Some("`vertexai=`");
            }
        }
        let options = keyword_value(call, text, |k| k == "http_options")?;
        let names_no_endpoint = match options.kind() {
            "none" => true,
            "dictionary" => dict_entries(options, |n| self.literal(n).ok(), text)
                .iter()
                .all(|entry| entry.as_ref().is_some_and(|(k, _)| k != "base_url")),
            "call" => {
                let last = options
                    .child_by_field_name("function")
                    .and_then(|f| match f.kind() {
                        "identifier" => Some(f),
                        "attribute" => f.child_by_field_name("attribute"),
                        _ => None,
                    });
                last.is_some_and(|n| self.text(n) == "HttpOptions")
                    && options
                        .child_by_field_name("arguments")
                        .is_some_and(|list| {
                            named_children(list).iter().all(|arg| {
                                arg.kind() == "comment"
                                    || (arg.kind() == "keyword_argument"
                                        && arg
                                            .child_by_field_name("name")
                                            .is_some_and(|n| self.text(n) != "base_url"))
                            })
                        })
            }
            _ => false,
        };
        (!names_no_endpoint).then_some("`http_options=`")
    }

    /// Why a client built by `class` may not reach the endpoint its
    /// constructor names: when it names none (`base_url` absent or `None`),
    /// the SDK reads its endpoint variable, which this file sets
    /// ([`ClientCaveat::EndpointEnv`]) or may set through a run-time name or
    /// an alias of `os.environ` ([`ClientCaveat::EnvWrite`]).
    fn endpoint_env(&self, class: ClientClass, base_url: BaseUrl) -> Option<ClientCaveat> {
        if !matches!(
            class.family,
            Family::OpenaiDirect | Family::AnthropicDirect | Family::Genai
        ) || base_url != BaseUrl::Absent
        {
            return None;
        }
        endpoint_env_vars(class.pkg)
            .iter()
            .find(|var| self.env_writes.contains(**var))
            .map(|var| ClientCaveat::EndpointEnv { var })
            .or_else(|| self.env_wild.map(ClientCaveat::EnvWrite))
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
        if let Some(statement) = self.import_cut {
            // A name the file no longer binds could change what any call
            // resolves to: report nothing rather than something wrong.
            self.out.gaps.push(UsageGap {
                kind: GapKind::UnresolvedModel,
                sdk: None,
                file: self.file.clone(),
                span: Some(self.span(statement)),
                symbol: None,
                chain: None,
                key: None,
                reason: "truncated".to_string(),
                detail: format!(
                    "this file binds more than {MAX_FILE_IMPORTS} imported names, so its calls are not analyzed"
                ),
                values: Vec::new(),
                values_complete: false,
            });
            return;
        }
        // Calls inside a scope nested past the cap are not analyzed.
        for (node, scope) in std::mem::take(&mut self.scope_cuts) {
            if self.records() >= MAX_FILE_RECORDS {
                return self.truncate_file(node, scope);
            }
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
                    "scopes here are nested more than {MAX_SCOPE_DEPTH} deep, so the calls inside are not analyzed"
                ),
                values: Vec::new(),
                values_complete: false,
            });
        }
        self.settle_builtin_values();
        self.settle_borrowed_methods();
        // A bare `environ` is the process environment only when imported
        // from `os`.
        let environ_from_os = ["environ", "environb"].iter().any(|name| {
            self.other_imports
                .get(&(0, name.to_string()))
                .is_some_and(|defs| defs.iter().any(|d| &*d.module == "os"))
        });
        if environ_from_os {
            if let Some(feature) = self.bare_environ_wild {
                self.env_wild.get_or_insert(feature);
            }
        }
        self.openai_routing = self.module_openai_routing();
        if let Some(var) = OPENAI_MODULE_ENV
            .iter()
            .find(|var| self.env_writes.contains(**var))
        {
            if self.openai_routing == Routing::OpenaiDirect {
                self.openai_routing = Routing::Unknown;
            }
            self.openai_caveat = Some(ClientCaveat::EndpointEnv { var });
        } else if let Some(feature) = self.env_wild {
            if self.openai_routing == Routing::OpenaiDirect {
                self.openai_routing = Routing::Unknown;
            }
            self.openai_caveat = Some(ClientCaveat::EnvWrite(feature));
        }
        self.note_legacy_configure();
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

    /// Imports, calls, gaps and clients reported so far.
    fn records(&self) -> usize {
        self.out.imports.len() + self.out.calls.len() + self.out.gaps.len() + self.out.clients.len()
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
                "this file reports more than {MAX_FILE_RECORDS} imports, calls, clients and gaps; nothing from this line on is analyzed"
            ),
            values: Vec::new(),
            values_complete: false,
        });
    }

    /// Where the top-of-file setup block ends: the start of the first
    /// module-level statement that is not an import, a string (the
    /// docstring), or an assignment (`=`, annotated or augmented) with no
    /// lambda, comprehension, generator expression, walrus, `yield` or
    /// `await` in it. A `def`, a `class`, a decorator, a bare expression or
    /// call, and any compound statement (`if`, `try`, `for`, `with`...) end
    /// it. Before it this file has defined no function, class or lambda,
    /// so no code of its own can run: a configuration written there holds
    /// for every call this file's functions make. One pass over the
    /// statements of the block, on first use.
    fn setup_end(&mut self) -> usize {
        if let Some(end) = self.setup_end {
            return end;
        }
        let end = self.root.map_or(0, |root| self.setup_block_end(root));
        self.setup_end = Some(end);
        end
    }

    /// [`Ctx::setup_end`], read from the module's statements.
    fn setup_block_end(&self, root: Node<'t>) -> usize {
        let mut cursor = root.walk();
        for statement in root.named_children(&mut cursor) {
            let setup = match statement.kind() {
                "import_statement"
                | "import_from_statement"
                | "future_import_statement"
                | "comment" => true,
                "expression_statement" => match named_children(statement).as_slice() {
                    [only] => match only.kind() {
                        "string" | "concatenated_string" => true,
                        "assignment" | "augmented_assignment" => !self.may_define_code(*only),
                        _ => false,
                    },
                    _ => false,
                },
                _ => false,
            };
            if !setup {
                return statement.start_byte();
            }
        }
        root.end_byte()
    }

    /// Whether `node` holds a lambda, comprehension, generator expression,
    /// walrus, `yield` or `await`: code that defines a scope or binds a name
    /// where it runs.
    fn may_define_code(&self, node: Node<'t>) -> bool {
        let mut cursor = node.walk();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if matches!(
                current.kind(),
                "lambda"
                    | "list_comprehension"
                    | "set_comprehension"
                    | "dictionary_comprehension"
                    | "generator_expression"
                    | "named_expression"
                    | "yield"
                    | "await"
            ) {
                return true;
            }
            stack.extend(current.named_children(&mut cursor));
        }
        false
    }

    /// Routing of the module-level OpenAI API, from `openai.api_type`,
    /// `openai.api_base` / `base_url` and `openai.azure_endpoint`
    /// assignments anywhere in the file. The supported grammar is a
    /// configuration written in the top-of-file setup block (see
    /// [`Ctx::setup_block_end`]): plain `=` statements directly in the
    /// module body, each routing attribute assigned once. Anything else (a
    /// routing setting in a function, a branch, loop, `try`, `with` or class
    /// body, after a `def`, `class`, decorator or other statement, by `+=`,
    /// `setattr` or `del`, assigned twice, or written on an object this file
    /// cannot identify) gives every module-level call a `module_config` gap;
    /// the routing reported is what the assignments together suggest, as a
    /// candidate.
    fn module_openai_routing(&mut self) -> Routing {
        let (mut azure, mut unknown, mut custom) = (false, false, false);
        let mut assigned: HashSet<&'t str> = HashSet::new();
        for assign in self.config_assigns.clone() {
            let target = self.eval(assign.object, assign.scope, 0);
            self.note_sdk_patch(&target, assign);
            let attr = assign.attr;
            let routes = is_module_routing_attr(attr);
            if !matches!(&target, Val::Module { pkg: Pkg::Openai, path } if path.is_empty()) {
                // An object this file cannot identify (an alias chosen at
                // run time, a parameter) may be the module.
                if routes && matches!(target, Val::Untracked(_)) {
                    self.openai_caveat
                        .get_or_insert(ClientCaveat::ModuleSetup(Self::feature(
                            "a routing setting written on an object this file cannot identify, which may be the SDK module",
                            assign.object,
                        )));
                }
                continue;
            }
            if routes && self.openai_last_config.is_none_or(|(at, _)| at < assign.at) {
                let feature = Self::feature("the SDK configured after the call", assign.object);
                self.openai_last_config = Some((assign.at, feature));
            }
            if routes {
                // Set in a function, the configuration holds only once (and
                // if) the function runs; in a branch, loop or `try`, only on
                // that path; after the setup block, perhaps after this
                // file's own code ran; assigned twice, whichever ran last.
                let what = if self.scopes[assign.scope].kind == ScopeKind::Function {
                    Some("the SDK configured inside a function, which may not run first")
                } else if !assign.top {
                    Some("the SDK configured inside a branch, loop, `try`, `with` or class body, which may not run")
                } else if !assign.plain {
                    Some("the SDK configured by `+=`, `setattr` or `del`")
                } else if !self.imported_only(assign.object) {
                    Some("the SDK configured through a name this file also assigns, or a wildcard import may rebind")
                } else if assign.at >= self.setup_end() {
                    Some("the SDK configured after the top-of-file setup block, where this file's own code may already have run")
                } else if !assigned.insert(attr) {
                    Some("an SDK routing setting assigned more than once")
                } else {
                    None
                };
                if let Some(what) = what {
                    self.openai_caveat
                        .get_or_insert(ClientCaveat::ModuleSetup(Self::feature(
                            what,
                            assign.object,
                        )));
                }
            }
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
                "api_base" | "base_url" => {
                    let kind = match assign.value {
                        Some(value) => self.base_url_kind(Pkg::Openai, value, assign.scope),
                        None => BaseUrl::Unknown { env: false },
                    };
                    match kind {
                        BaseUrl::Absent | BaseUrl::Provider => {}
                        BaseUrl::Custom => custom = true,
                        BaseUrl::CustomFallback => {
                            custom = true;
                            self.openai_caveat.get_or_insert(ClientCaveat::BaseUrl {
                                env: true,
                                fallback: true,
                            });
                        }
                        BaseUrl::ProviderFallback => {
                            self.openai_caveat.get_or_insert(ClientCaveat::BaseUrl {
                                env: false,
                                fallback: true,
                            });
                        }
                        BaseUrl::Unknown { env } => {
                            unknown = true;
                            self.openai_caveat.get_or_insert(ClientCaveat::BaseUrl {
                                env,
                                fallback: false,
                            });
                        }
                    }
                }
                _ if attr.starts_with("bedrock_") => unknown = true,
                _ => {}
            }
        }
        // An attribute written with a run-time name may be a routing
        // setting of the module.
        for (object, scope) in self.wild_attr_objects.clone() {
            let target = self.eval(object, scope, 0);
            let module =
                matches!(&target, Val::Module { pkg: Pkg::Openai, path } if path.is_empty());
            if module || matches!(target, Val::Untracked(_)) {
                self.openai_caveat
                    .get_or_insert(ClientCaveat::ModuleSetup(Self::feature(
                        "an attribute written with a run-time name on an object that may be the SDK module",
                        object,
                    )));
                break;
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

    /// Whether `call` passes `client_options=` or a `**` expansion, which
    /// may hold it.
    fn may_pass_client_options(&self, call: Node<'t>) -> bool {
        let Some(args) = call.child_by_field_name("arguments") else {
            return false;
        };
        let mut cursor = args.walk();
        let found = args
            .named_children(&mut cursor)
            .any(|arg| match arg.kind() {
                "dictionary_splat" => true,
                "keyword_argument" => arg
                    .child_by_field_name("name")
                    .is_some_and(|n| self.text(n) == "client_options"),
                _ => false,
            });
        found
    }

    /// `genai.configure(client_options=...)` (the legacy Gemini SDK) may
    /// point every legacy call at another endpoint: they then route
    /// `unknown` with an `endpoint_option` gap. So may `**options`. In a
    /// file that imports the legacy SDK, the callee of every call that may
    /// pass `client_options` is evaluated, so `configure` under another
    /// name (`setup = genai.configure`) counts.
    fn note_legacy_configure(&mut self) {
        if !self.legacy_imported {
            return;
        }
        for (call, scope) in self.calls.clone() {
            if !self.may_pass_client_options(call) {
                continue;
            }
            let Some(function) = call.child_by_field_name("function") else {
                continue;
            };
            let configures = matches!(
                self.eval(function, scope, 0),
                Val::Module { pkg: Pkg::GoogleGenerativeai, ref path }
                    | Val::Symbol { pkg: Pkg::GoogleGenerativeai, ref path }
                    if path.last().is_some_and(|p| p == "configure")
            );
            if !configures {
                continue;
            }
            let args = self.arguments(call);
            let endpoint = keyword_value(call, |n| self.text(n), |k| k == "client_options")
                .is_some_and(|v| v.kind() != "none");
            if endpoint || args.splat {
                self.legacy_caveat
                    .get_or_insert(ClientCaveat::EndpointOption {
                        option: "`configure(client_options=...)`",
                    });
            }
        }
    }

    /// A write that replaces an SDK member rather than configuring it: on a
    /// submodule, a class, an SDK symbol or a client's resource
    /// (`client.chat.completions.create = patched`), or a class or resource
    /// of the module or a client (`openai.OpenAI = Sub`, `client.chat =
    /// wrapper`). Settings (`openai.api_key`, `client.max_retries`) are not.
    fn note_sdk_patch(&mut self, target: &Val, assign: ConfigAssign<'t>) {
        const RESOURCES: &[&str] = &[
            "chat",
            "completions",
            "responses",
            "messages",
            "models",
            "beta",
            "embeddings",
            "images",
            "audio",
            "files",
            "batches",
            "aio",
            "chats",
            "with_options",
            "with_raw_response",
            "with_streaming_response",
            "copy",
            "generate_content",
            "generate_content_async",
        ];
        // The SDK clients and resources a LangChain chat model sends its
        // requests through.
        const LANGCHAIN_RESOURCES: &[&str] = &[
            "client",
            "async_client",
            "root_client",
            "root_async_client",
            "_client",
            "_async_client",
        ];
        let attr = assign.attr;
        let langchain = target.pkg().is_some_and(Pkg::is_langchain);
        let resource = RESOURCES.contains(&attr)
            || attr.starts_with(|c: char| c.is_ascii_uppercase())
            || (langchain && LANGCHAIN_RESOURCES.contains(&attr));
        let pkg = match target {
            Val::Module { pkg, path } if !path.is_empty() || resource => *pkg,
            Val::Class(class) => class.pkg,
            Val::Symbol { pkg, .. } => *pkg,
            Val::Client { client, path } if !path.is_empty() || resource => client.pkg,
            _ => return,
        };
        self.sdk_patched
            .entry(pkg)
            .or_insert_with(|| Self::feature("an SDK member replaced in this file", assign.object));
    }

    /// Clients given a new `base_url` after they are built
    /// (`client.base_url = "https://openrouter.ai/api/v1"`).
    fn note_rerouted_clients(&mut self) {
        let written: Vec<(Node<'t>, usize)> = self
            .config_assigns
            .iter()
            .filter(|assign| BASE_URL_ATTRS.contains(&assign.attr))
            .map(|assign| (assign.object, assign.scope))
            // An attribute named at run time may be the base URL too.
            .chain(self.wild_attr_objects.iter().copied())
            .collect();
        for (object, scope) in written {
            match self.eval(object, scope, 0) {
                Val::Client { client, path } if path.is_empty() => match client.site {
                    Some(site) => {
                        self.rerouted.insert(site);
                    }
                    None => {
                        self.rerouted_pkgs.insert(client.pkg);
                    }
                },
                // An SDK module is routed by `module_openai_routing`; a
                // member of a client, or `self` of a same-file class, is not
                // a client.
                Val::Client { .. }
                | Val::Module { .. }
                | Val::Symbol { .. }
                | Val::Class(_)
                | Val::Result(_)
                | Val::GoogleNs => {}
                // `self`/`cls`, or a same-file class by name: not a client.
                _ if object.kind() == "identifier"
                    && (matches!(self.text(object), "self" | "cls")
                        || self.class_receiver(object, scope).is_some()) => {}
                // An object this file cannot identify may be any client.
                _ => {
                    self.rerouted_any.get_or_insert(Self::feature(
                        "a `base_url` written on an object this file cannot identify",
                        object,
                    ));
                }
            }
        }
    }

    /// `client` with its routing unknown when a `base_url` is assigned to it
    /// after it is built.
    fn reroute(&self, mut client: Client) -> Client {
        // A legacy Gemini model object: `configure(client_options=...)`
        // routes every one of them.
        if client.pkg == Pkg::GoogleGenerativeai && client.caveat.is_none() && !client.ambiguous {
            if let Some(caveat) = &self.legacy_caveat {
                client.routing = Routing::Unknown;
                client.caveat = Some(caveat.clone());
            }
        }
        // A deprecated LangChain OpenAI chat model on the pre-1.0 SDK sends
        // its requests through the module-level API, which this file
        // configures (`openai.api_type = "azure"`, `openai.api_base = ...`).
        let module_configured =
            self.openai_routing != Routing::OpenaiDirect || self.openai_caveat.is_some();
        if client.pkg.reads_module_config()
            && module_configured
            && client.routing == Routing::OpenaiDirect
            && client.caveat.is_none()
            && !client.ambiguous
        {
            client.routing = Routing::Unknown;
            client.caveat = Some(ClientCaveat::LangchainModuleConfig);
        }
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
        } else if let Some(feature) = self.rerouted_any {
            let direct = matches!(
                client.routing,
                Routing::OpenaiDirect | Routing::AnthropicDirect
            );
            if direct && client.caveat.is_none() && !client.ambiguous {
                client.caveat = Some(ClientCaveat::Dynamic(feature));
            }
        }
        if let Some(feature) = self.sdk_patched.get(&client.pkg) {
            if client.caveat.is_none() && !client.ambiguous {
                client.caveat = Some(ClientCaveat::Dynamic(*feature));
            }
        }
        client
    }

    /// Why the module-level API of `pkg` may not route as reported.
    fn module_caveat(&self, pkg: Pkg) -> Option<ClientCaveat> {
        match pkg {
            Pkg::Openai => self.openai_caveat.clone(),
            Pkg::GoogleGenerativeai => self.legacy_caveat.clone(),
            _ => None,
        }
    }

    fn module_routing(&self, pkg: Pkg) -> Routing {
        match pkg {
            Pkg::Openai => self.openai_routing,
            Pkg::Anthropic => Routing::AnthropicDirect,
            Pkg::GoogleGenai => Routing::GoogleGenai,
            Pkg::GoogleGenerativeai if self.legacy_caveat.is_some() => Routing::Unknown,
            Pkg::GoogleGenerativeai => Routing::GoogleGenerativeaiLegacy,
            // Only LangChain's chat-model classes are followed, and each
            // routes by its own construction.
            Pkg::LangchainOpenai
            | Pkg::LangchainAnthropic
            | Pkg::LangchainCommunityOpenai
            | Pkg::LangchainCommunityAnthropic
            | Pkg::LangchainLegacyOpenai
            | Pkg::LangchainLegacyAnthropic => Routing::Unknown,
        }
    }

    /// Routing of a module-level API call (`openai.ChatCompletion.create`)
    /// and why it may not hold: the file's configuration (see
    /// [`Self::module_openai_routing`]), a configuration written after a
    /// call that runs at import, or (with no configuration in the file) an
    /// `engine`/`deployment_id` argument, which another module's Azure
    /// configuration would serve.
    fn module_target(
        &mut self,
        pkg: Pkg,
        call: Node<'t>,
        scope: usize,
    ) -> (Routing, Option<ClientCaveat>) {
        let mut routing = self.module_routing(pkg);
        let mut caveat = self.module_caveat(pkg);
        if caveat.is_none() {
            caveat = self
                .sdk_patched
                .get(&pkg)
                .map(|f| ClientCaveat::Dynamic(*f));
        }
        if pkg != Pkg::Openai || caveat.is_some() {
            return (routing, caveat);
        }
        match self.openai_last_config {
            Some((at, feature)) => {
                // The configuration is in the setup block (else the caveat
                // is set): no function of this file exists before it, but a
                // call that runs at import in the block itself
                // (`R = openai.chat.completions.create(...)`) may come
                // before it.
                if self.runs_at_import(scope) && call.start_byte() < at {
                    caveat = Some(ClientCaveat::ModuleSetup(feature));
                }
            }
            None => {
                if self.passes_engine(call, scope) {
                    routing = Routing::Unknown;
                    caveat = Some(ClientCaveat::ModuleConfig);
                }
            }
        }
        (routing, caveat)
    }

    /// Whether `object` is a name only import statements bind at module
    /// level: never assigned (an alias such as `o = openai if x else y`),
    /// and no wildcard import or namespace write may rebind it.
    fn imported_only(&self, object: Node<'t>) -> bool {
        object.kind() == "identifier"
            && !self
                .assigns
                .contains_key(&(0, self.text(object).to_string()))
            && self.module_stars.is_empty()
            && !self.dynamic_globals
    }

    /// Whether code in `scope` runs when the module is imported: the module
    /// body, a class body in it, or a list, set or dict comprehension there.
    fn runs_at_import(&self, scope: usize) -> bool {
        let mut s = scope;
        while s != 0 {
            let current = &self.scopes[s];
            if current.kind == ScopeKind::Function && !current.immediate {
                return false;
            }
            s = current.parent.unwrap_or(0);
        }
        true
    }

    /// Whether `call` passes `engine` or `deployment_id`, by keyword or
    /// possibly through a `**` expansion. Two lookups per expansion, however
    /// many keys its dict has.
    fn passes_engine(&mut self, call: Node<'t>, scope: usize) -> bool {
        let args = self.arguments(call);
        let azure_key = |k: &str| k == "engine" || k == "deployment_id";
        if args.kwargs.iter().any(|k| azure_key(k)) {
            return true;
        }
        args.splats
            .iter()
            .any(|splat| match self.expand_splat(*splat, scope) {
                Some(expansion) => {
                    let last = &expansion.summary.last;
                    expansion.opaque()
                        || last.contains_key("engine")
                        || last.contains_key("deployment_id")
                }
                None => true,
            })
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
            Val::Class(class) if class.pkg.is_langchain() => {
                // A LangChain chat model is built with the model its
                // requests use, and routes by the client it builds.
                let built = self.construction(call, class);
                let built = self.reroute(built);
                let receiver = if function.kind() == "attribute" {
                    Receiver::Module
                } else {
                    Receiver::Symbol
                };
                let target = CallTarget {
                    pkg: class.pkg,
                    member: class.name.to_string(),
                    receiver,
                    routing: built.routing,
                    client_line: built.line,
                    ambiguous: built.ambiguous,
                    caveat: built.caveat,
                };
                self.record_call(call, function, scope, target);
                // Its requests read the model field when they are sent, so
                // `llm.model_name = ...` after it is built changes it. Any
                // write of that name, or one with a run-time name, on an
                // object other than `self` or a same-file class may be one.
                let rewrite = self.wild_attr_any.or_else(|| {
                    ["model", "model_name"]
                        .iter()
                        .find_map(|attr| self.outside_attr_writes.get(attr).copied())
                });
                if let Some(feature) = rewrite {
                    let chain = self.render(function);
                    self.out.gaps.push(UsageGap {
                        kind: GapKind::UnresolvedModel,
                        sdk: Some(class.pkg.sdk()),
                        file: self.file.clone(),
                        span: Some(self.span(call)),
                        symbol: self.symbol(scope),
                        chain: Some(chain.clone()),
                        key: None,
                        reason: "dynamic_feature".to_string(),
                        detail: format!(
                            "the LangChain chat model `{chain}` builds may be given another model after it is built ({})",
                            feature.describe()
                        ),
                        values: Vec::new(),
                        values_complete: false,
                    });
                }
            }
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
                        routing: self.module_routing(class.pkg),
                        client_line: None,
                        ambiguous: false,
                        caveat: self.module_caveat(class.pkg).or_else(|| {
                            self.sdk_patched
                                .get(&class.pkg)
                                .map(|f| ClientCaveat::Dynamic(*f))
                        }),
                    };
                    self.record_call(call, function, scope, target);
                }
            }
            // LangChain is followed only through its chat-model classes
            // (above) and the objects they build.
            Val::Module { pkg, .. } | Val::Symbol { pkg, .. } if pkg.is_langchain() => {}
            Val::Module { pkg, path } if !path.is_empty() => {
                let (routing, caveat) = self.module_target(pkg, call, scope);
                let target = CallTarget {
                    pkg,
                    member: path.join("."),
                    receiver: Receiver::Module,
                    routing,
                    client_line: None,
                    ambiguous: false,
                    caveat,
                };
                self.record_call(call, function, scope, target);
            }
            Val::Symbol { pkg, path } => {
                let (routing, caveat) = self.module_target(pkg, call, scope);
                let target = CallTarget {
                    pkg,
                    member: path.join("."),
                    receiver: Receiver::Symbol,
                    routing,
                    client_line: None,
                    ambiguous: false,
                    caveat,
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
        // its kwargs, without a gap. Every call recorded for LangChain
        // builds a chat model or calls one.
        let on_sdk_object = matches!(target.receiver, Receiver::Client | Receiver::Module)
            && member.split('.').next() != Some("types");
        let gap_worthy = request || model_bearing || on_sdk_object || target.pkg.is_langchain();
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
        // A keyword repeated with the same expression (a syntax error
        // tree-sitter accepts) is read once.
        let mut read_args: HashSet<(&str, &str)> = HashSet::new();
        for (key, expr) in &args.value_args {
            keyed = true;
            if !read_args.insert((key.as_str(), self.text(*expr))) {
                continue;
            }
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
            let place = NestedPlace {
                prefix: &prefix,
                positional: keyword.is_none(),
                scope,
                overridden: false,
            };
            self.nested_values_in(*expr, place, 0, &mut read);
        }
        let mut nested = read.found;
        let mut nested_overridden = read.overridden;
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
            for (key, index, value) in expansion.summary.expansion.iter() {
                if !is_value_key(key) {
                    continue;
                }
                keyed = true;
                let label = expansion.name.as_ref().map(|n| format!("{n}[\"{key}\"]"));
                let mut result = self
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
                // `{"model": "gpt-4o", **overrides}`: the `**overrides` (or a
                // computed key) written after the entry may replace it, so
                // the entry is a candidate, not the value.
                if expansion.summary.overridable(*index) {
                    let shape = expansion
                        .name
                        .clone()
                        .unwrap_or_else(|| "{...}".to_string());
                    let candidates: Vec<Resolved> = result.ok().into_iter().collect();
                    result =
                        Err(Unresolved::new("splat", shape).with_candidates(candidates, false));
                }
                self.take_value(&site, key, false, result, &mut values);
            }
            let list = self.dict_nested(&expansion);
            nested.extend(list.found.iter().cloned());
            nested_overridden.extend(list.overridden.iter().cloned());
            nested_cut |= list.cut;
        }
        self.take_nested(&site, nested, nested_cut, &mut values);
        if site.gap_worthy && !nested_overridden.is_empty() {
            // One gap per key path, at most [`MAX_NESTED_VALUES`]; past
            // that, one `truncated` gap stands for the rest.
            let mut seen: HashSet<String> = HashSet::new();
            let mut cut = false;
            for path in nested_overridden {
                if seen.contains(&path) {
                    continue;
                }
                if seen.len() >= MAX_NESTED_VALUES {
                    cut = true;
                    break;
                }
                let unresolved = Unresolved::new("splat", "{...}".to_string());
                self.unresolved_gap(&site, &path, &unresolved, None);
                seen.insert(path);
            }
            if cut {
                self.out.gaps.push(UsageGap {
                    kind: GapKind::UnresolvedModel,
                    sdk: Some(site.sdk),
                    file: self.file.clone(),
                    span: Some(span),
                    symbol: symbol.clone(),
                    chain: Some(chain.clone()),
                    key: None,
                    reason: "truncated".to_string(),
                    detail: format!(
                        "`{chain}` passes more nested model values that a `**` expansion may replace than are listed ({MAX_NESTED_VALUES} listed); the rest are not reported"
                    ),
                    values: Vec::new(),
                    values_complete: false,
                });
            }
        }

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
        // A LangChain chat model takes its model as `model` or as
        // `model_name`; given both, which one it keeps is not confirmed.
        let named = |key: &str| args.kwargs.iter().any(|k| k == key);
        if target.pkg.is_langchain() && named("model") && named("model_name") {
            self.out.gaps.push(UsageGap {
                kind: GapKind::UnresolvedModel,
                sdk: Some(sdk),
                file: self.file.clone(),
                span: Some(span),
                symbol: symbol.clone(),
                chain: Some(chain.clone()),
                key: None,
                reason: "reassigned".to_string(),
                detail: format!(
                    "`{chain}` is given its model twice, as `model` and as `model_name`, so which one LangChain uses is not confirmed"
                ),
                values: Vec::new(),
                values_complete: false,
            });
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
                values_complete: false,
            });
        } else if let Some(caveat) = &target.caveat {
            let (reason, detail) = match caveat {
                ClientCaveat::InFileCallers { param } => (
                    "in_file_callers",
                    format!(
                        "the receiver of `{chain}` is parameter `{param}`: every caller in this file passes the same client, but other modules may call the function with another one"
                    ),
                ),
                ClientCaveat::ParameterDefault { param, unseen: false } => (
                    "parameter_default",
                    format!(
                        "the receiver of `{chain}` is the default of parameter `{param}`, but other modules may call the function with another client"
                    ),
                ),
                ClientCaveat::ParameterDefault { param, unseen: true } => (
                    "unseen_callers",
                    format!(
                        "the receiver of `{chain}` is the default of parameter `{param}`, but the function is decorated, passed or stored, so a caller this file does not show may pass another client"
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
                ClientCaveat::EnvWrite(feature) => (
                    "dynamic_feature",
                    format!(
                        "this file may set the SDK's endpoint variable ({}), so `{chain}` may send its requests elsewhere",
                        feature.describe()
                    ),
                ),
                ClientCaveat::BaseUrl {
                    env: true,
                    fallback: true,
                } => (
                    "base_url_env",
                    format!(
                        "the `base_url` of the client behind `{chain}` is read from the environment and falls back to another host, so the deployment may point it anywhere"
                    ),
                ),
                ClientCaveat::EndpointOption { option } => (
                    "endpoint_option",
                    format!(
                        "the client behind `{chain}` is configured with {option}, which may send its requests to Vertex AI or another endpoint"
                    ),
                ),
                ClientCaveat::BaseUrl { env: true, .. } => (
                    "base_url_env",
                    format!(
                        "the `base_url` of the client behind `{chain}` is read from the environment with no fallback, so its endpoint is not known"
                    ),
                ),
                ClientCaveat::BaseUrl { fallback: true, .. } => (
                    "base_url_env",
                    format!(
                        "the `base_url` of the client behind `{chain}` falls back to the provider's own host, but the environment may set another"
                    ),
                ),
                ClientCaveat::BaseUrl { .. } => (
                    "base_url_unresolved",
                    format!(
                        "the `base_url` of the client behind `{chain}` does not resolve to a string in this file, so its endpoint is not known"
                    ),
                ),
                ClientCaveat::Dynamic(feature) => (
                    "dynamic_feature",
                    format!(
                        "the receiver of `{chain}` may be rebound or replaced at run time: {}",
                        feature.describe()
                    ),
                ),
                ClientCaveat::ModuleSetup(feature) => (
                    "module_config",
                    format!(
                        "the module-level API's routing is not configured by a top-of-file setup block ({}), so it may not hold when `{chain}` runs",
                        feature.describe()
                    ),
                ),
                ClientCaveat::ModuleConfig => (
                    "module_config",
                    format!(
                        "`{chain}` passes `engine` or `deployment_id`, which Azure deployments take, but this file does not configure the module-level API, so another module may route it to Azure"
                    ),
                ),
                ClientCaveat::LangchainModuleConfig => (
                    "module_config",
                    format!(
                        "this file configures the module-level OpenAI API, which the LangChain chat model behind `{chain}` sends its requests through when it runs on the pre-1.0 `openai` SDK, so its routing is not confirmed"
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
                values_complete: false,
            });
        } else if let Some(feature) = self.file_dynamic {
            // Anything in this file may be rebound, the receiver and the
            // SDK's configuration included.
            self.out.gaps.push(UsageGap {
                kind: GapKind::AmbiguousClient,
                sdk: Some(sdk),
                file: self.file.clone(),
                span: Some(span),
                symbol: symbol.clone(),
                chain: Some(chain.clone()),
                key: None,
                reason: "dynamic_feature".to_string(),
                detail: format!(
                    "this file may rebind any name or attribute at run time ({}), so the routing of `{chain}` is not confirmed",
                    feature.describe()
                ),
                values: Vec::new(),
                values_complete: false,
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
        // In a file that may rebind anything at run time, only a value
        // written at the call itself is confirmed.
        let result = result.map(|mut resolved| {
            let flagged = resolved
                .caveats
                .iter()
                .any(|c| c.reason == "dynamic_feature");
            if let (Some(feature), false) = (self.file_dynamic, flagged) {
                if !within(site.call, resolved.span) {
                    let name = resolved.name.clone().unwrap_or_else(|| key.to_string());
                    resolved
                        .caveats
                        .push(Unresolved::dynamic(feature).via(&name));
                }
            }
            resolved
        });
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
                values_complete: false,
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
            _ if unresolved.reason == "splat" && shape != "**kwargs" => format!(
                "`{key}` for `{chain}` is set in a dict (`{shape}`) where a `**` expansion or a computed key written after it may replace it"
            ),
            _ if unresolved.reason == "splat" => {
                format!("`{key}` for `{chain}` is not passed by name and may come from `**kwargs`")
            }
            _ if unresolved.reason == "missing_model" => {
                format!("`{key}` for `{chain}` is not passed, so the SDK or server default applies")
            }
            Some(via) if unresolved.reason == "dynamic_feature" => format!(
                "`{key}` for `{chain}` comes from `{via}`, which may change at run time: {shape}"
            ),
            None if unresolved.reason == "dynamic_feature" => {
                format!("`{key}` for `{chain}` may change at run time: {shape}")
            }
            Some(via) if !shape.is_empty() && via != shape => {
                format!("`{key}` for `{chain}` comes from `{via}`, which is {phrase} (`{shape}`)")
            }
            Some(via) => format!("`{key}` for `{chain}` comes from `{via}`, which is {phrase}"),
            None => format!("`{key}` for `{chain}` is `{shape}`, which is {phrase}"),
        };
        let (values, cut) = self.candidate_values(key, &unresolved.candidates);
        // A file that may rebind anything at run time can complete a list
        // only with literals written at the call.
        let rebinding = self.file_dynamic.is_some()
            && !unresolved
                .candidates
                .iter()
                .all(|c| within(site.call, c.span));
        let complete = unresolved.complete() && !cut && !rebinding;
        let listed = if complete {
            "every other value it may take is listed"
        } else {
            "some values it may take are listed, not necessarily all"
        };
        match captured {
            Some(origin) if unresolved.reason == "parameter_overridden" => {
                detail.push_str(&format!(
                    "; its default is captured with origin `{}`",
                    origin.as_str()
                ));
                if complete {
                    detail.push_str(", and every other value in-file callers pass is listed");
                } else if !values.is_empty() {
                    detail
                        .push_str(", and some values callers pass are listed, not necessarily all");
                }
            }
            Some(origin) if unresolved.reason == "unseen_callers" => {
                detail.push_str(&format!(
                    "; its default is captured with origin `{}`",
                    origin.as_str()
                ));
                if !values.is_empty() {
                    detail.push_str(
                        ", and some values in-file calls pass are listed, not necessarily all",
                    );
                }
            }
            Some(origin) if unresolved.reason == "parameter_default" => {
                detail.push_str(&format!(
                    "; its default is captured with origin `{}`",
                    origin.as_str()
                ));
            }
            Some(origin) if unresolved.reason == "dynamic_feature" => {
                detail.push_str(&format!(
                    "; its value is captured with origin `{}`",
                    origin.as_str()
                ));
            }
            Some(_) => {
                detail.push_str("; its literal fallback is captured with origin `env_default`");
                if !values.is_empty() {
                    detail.push_str(&format!(", and {listed}"));
                }
            }
            None if complete => detail.push_str("; every value it may take is listed"),
            None if !values.is_empty() => detail.push_str(&format!("; {listed}")),
            None => {}
        }
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
            values_complete: complete && !values.is_empty(),
            values,
        });
    }

    /// Candidate values listed with a gap for `key`: distinct, at most
    /// [`MAX_CANDIDATES`] per gap and [`MAX_FILE_CANDIDATES`] per file; and
    /// whether a limit cut the list.
    fn candidate_values(
        &mut self,
        key: &str,
        candidates: &[Resolved],
    ) -> (Vec<CapturedValue>, bool) {
        let mut seen = HashSet::new();
        let mut listed = Vec::new();
        for candidate in candidates {
            if !seen.insert(candidate.value.as_str()) {
                continue;
            }
            if listed.len() >= MAX_CANDIDATES || self.candidates_left == 0 {
                return (listed, true);
            }
            self.candidates_left -= 1;
            let mut candidate = candidate.clone();
            candidate.caveats.clear();
            listed.push(captured(key, candidate, false));
        }
        (listed, false)
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
            values_complete: false,
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
        // Every SDK shape has a member: a bare name never matches.
        if function.kind() == "identifier" || sdk_shape(&self.render_links(function, 8)).is_none() {
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
            Origin::Dynamic(feature) => (
                "dynamic_feature",
                format!(
                    "the receiver of `{chain}` may be rebound or replaced at run time: {}",
                    feature.describe()
                ),
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
            values_complete: false,
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
            values_complete: false,
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
        for (key, index, value) in expansion.summary.expansion.iter() {
            if !is_value_key(key) && is_container(*value) {
                let prefix = format!("{key}.");
                let place = NestedPlace {
                    prefix: &prefix,
                    positional: false,
                    scope: expansion.scope,
                    overridden: expansion.summary.overridable(*index),
                };
                self.nested_values_in(*value, place, 0, &mut read);
            }
        }
        let list = Rc::new(NestedList {
            found: read.found,
            overridden: read.overridden,
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
    /// argument may not configure the request at all. A dict keeps the last
    /// entry of each key, as Python does, and an entry that a later `**`
    /// expansion or computed key may replace (or one inside such an entry,
    /// `place.overridden`) is listed in `read.overridden` instead of read.
    /// `read` bounds the entries read (see [`MAX_NESTED_ENTRIES`]).
    fn nested_values_in(
        &mut self,
        node: Node<'t>,
        place: NestedPlace<'_>,
        depth: u32,
        read: &mut NestedRead,
    ) {
        if depth > 4 {
            return;
        }
        let mut cursor = node.walk();
        match node.kind() {
            "dictionary" => {
                // Read once per literal (memoized), like any dict lookup.
                let summary = self.dict_summary(node);
                // Indexes as `dict_entries` counts them: every named child
                // but comments.
                let mut index = 0;
                for pair in node.named_children(&mut cursor) {
                    if pair.kind() == "comment" {
                        continue;
                    }
                    let at = index;
                    index += 1;
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
                    // `{"model": "a", "model": "b"}` sends `b`: an earlier
                    // entry of a key is discarded.
                    if summary.last.get(&key).is_some_and(|(last, _)| *last != at) {
                        continue;
                    }
                    let overridden = place.overridden || summary.overridable(at);
                    let path = format!("{}{key}", place.prefix);
                    if is_value_key(&key) {
                        if overridden {
                            read.overridden.push(path);
                        } else if let Ok(resolved) = self.resolve_value(value, place.scope, 0) {
                            read.found.push(NestedValue {
                                path,
                                positional: place.positional,
                                resolved,
                            });
                        }
                    } else if is_container(value) {
                        let nested = format!("{path}.");
                        let inner = NestedPlace {
                            prefix: &nested,
                            overridden,
                            ..place
                        };
                        self.nested_values_in(value, inner, depth + 1, read);
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
                    self.nested_values_in(item, place, depth + 1, read);
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
                    if let Some(feature) = self.rebound_receiver(object, scope) {
                        return Err(Unresolved::dynamic(feature).via(&rendered));
                    }
                    if let Some((class, through_self)) = self.class_receiver(object, scope) {
                        let lookup = self.attr_lookup(class, &attr, through_self);
                        let assigns = Rc::clone(&lookup.assigns);
                        let dynamic = lookup.dynamic;
                        // Nothing the grammar reads, and a dynamic feature
                        // that may set it.
                        if let (true, Some(feature)) = (assigns.is_empty(), dynamic) {
                            return Err(Unresolved::dynamic(feature).via(&rendered));
                        }
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
                                let mut resolved =
                                    resolved.through(&rendered, ValueOrigin::ClassConstant);
                                if let Some(feature) = dynamic {
                                    resolved
                                        .caveats
                                        .push(Unresolved::dynamic(feature).via(&rendered));
                                } else if through_self && self.may_be_subclassed_elsewhere(class) {
                                    // Other modules may subclass a public
                                    // class (or a private one's public
                                    // subclass) and set the attribute (rule
                                    // 3, like a public function's default).
                                    resolved
                                        .caveats
                                        .push(Unresolved::new("subclass_override", rendered));
                                }
                                Ok(resolved)
                            }
                            Err(unresolved) if unresolved.reason == "reassigned" => {
                                let (candidates, complete) = self.assign_candidates(
                                    &assigns,
                                    ValueOrigin::ClassConstant,
                                    &rendered,
                                    depth,
                                );
                                // Complete only for assignments in one
                                // scope (a class body's branches), with no
                                // dynamic feature (P4).
                                let one_scope = assigns.iter().all(|a| a.scope == assigns[0].scope);
                                Err(unresolved.via(&rendered).with_candidates(
                                    candidates,
                                    complete && one_scope && dynamic.is_none(),
                                ))
                            }
                            Err(unresolved) if dynamic.is_some() => {
                                Err(unresolved.via(&rendered).incomplete())
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
                let unresolved = Unresolved::new(reason, self.render(expr));
                let dict = expr
                    .child_by_field_name("value")
                    .and_then(|base| self.dict_node(base, scope, 0));
                Err(match dict {
                    Some(dict) => {
                        let found = self.dict_candidates(&dict);
                        unresolved.with_candidates(found.values.clone(), found.complete)
                    }
                    None => unresolved,
                })
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
                // `"gpt-4o" if fast else "o1"`: each side is a candidate.
                let (candidates, complete) = self.branch_candidates(expr, scope, depth);
                Err(Unresolved::new(reason, self.render(expr))
                    .with_candidates(candidates, complete))
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
                // The parameter's own values are not listed: partial.
                let origin = constant_origin(self.scopes[home].kind);
                let (candidates, _) = self.name_candidates(name, scope, pos, origin, depth);
                return Err(Unresolved::new("reassigned", name.to_string())
                    .with_candidates(candidates, false));
            }
            let default = self.params.get(&key).and_then(|p| p.default);
            let Some(default) = default else {
                // What in-file callers pass, for triage.
                let passed = self.caller_models(home, name);
                return Err(Unresolved::new("parameter", name.to_string())
                    .with_candidates(passed.values.clone(), passed.complete));
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
                    // default one of several; a function other modules may
                    // call (not private, not in a script) may be passed
                    // anything.
                    if let Some(caveat) = self.overridden(home, name, &resolved.value) {
                        resolved.caveats.push(caveat);
                    } else if !self.is_private(home) {
                        resolved
                            .caveats
                            .push(Unresolved::new("parameter_default", name.to_string()));
                    }
                    Ok(resolved)
                }
                Err(unresolved) if unresolved.reason == "credential" => Err(unresolved.via(name)),
                Err(unresolved) => {
                    let passed = self.caller_models(home, name);
                    let default_complete = unresolved.complete();
                    Err(Unresolved::new("parameter", name.to_string())
                        .with_candidates(unresolved.candidates, default_complete)
                        .with_candidates(passed.values.clone(), passed.complete))
                }
            };
        }
        if defs.assigns > 0 {
            if imported {
                // What the assignments give; the import's value is unknown.
                let origin = constant_origin(self.scopes[home].kind);
                let (candidates, _) = self.name_candidates(name, scope, pos, origin, depth);
                return Err(Unresolved::new("reassigned", name.to_string())
                    .with_candidates(candidates, false));
            }
            let origin = constant_origin(self.scopes[home].kind);
            let only = defs
                .only_assign
                .and_then(|i| self.assigns.get(&key).and_then(|v| v.get(i)).copied());
            return match self.single_constant(only, defs.assigns, origin, depth) {
                Ok(resolved) => Ok(resolved.through(name, origin)),
                // Assigned in more than one branch: what each assigns.
                Err(unresolved) if unresolved.reason == "reassigned" => {
                    let (candidates, complete) =
                        self.name_candidates(name, scope, pos, origin, depth);
                    Err(unresolved.via(name).with_candidates(candidates, complete))
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
        let visible = self.visible_calls(func);
        models.unseen = !matches!(visible, Some((_, true)));
        let mut read = false;
        if let (Some(slot), Some((calls, _))) = (self.param_slot(func, name), visible) {
            read = true;
            let keyword = slot.keyword.then_some(name);
            self.collecting += 1;
            for (i, &(call, call_scope)) in calls.iter().enumerate() {
                if i >= MAX_CALLERS_READ {
                    models.unknown = true;
                    break;
                }
                match argument_for(call, slot.position, keyword, |n| self.text(n)) {
                    ArgFor::Found(expr) => match self.resolve_value(expr, call_scope, 0) {
                        Ok(resolved) if resolved.is_certain() => {
                            if !push_candidate(&mut models.values, resolved) {
                                models.unknown = true;
                            }
                        }
                        Ok(resolved) => {
                            push_candidate(&mut models.values, resolved);
                            models.unknown = true;
                        }
                        Err(_) => models.unknown = true,
                    },
                    ArgFor::Missing => {}
                    ArgFor::Unknown => models.unknown = true,
                }
            }
            self.collecting -= 1;
        }
        models.complete = read && !models.unseen && !models.unknown && self.is_private(func);
        let models = Rc::new(models);
        self.caller_models.insert(key, Rc::clone(&models));
        models
    }

    /// A `parameter_overridden` caveat when an in-file call passes `name`
    /// a value other than its `default` (or one that does not resolve), and
    /// an `unseen_callers` one when some calls of the function cannot be
    /// seen (it is decorated, passed, stored or bound twice).
    fn overridden(&mut self, func: usize, name: &str, default: &str) -> Option<Unresolved> {
        let passed = self.caller_models(func, name);
        let others: Vec<Resolved> = passed
            .values
            .iter()
            .filter(|r| r.value != default)
            .cloned()
            .collect();
        let reason = if passed.unseen {
            "unseen_callers"
        } else if !others.is_empty() || passed.unknown {
            "parameter_overridden"
        } else {
            return None;
        };
        Some(Unresolved::new(reason, name.to_string()).with_candidates(others, passed.complete))
    }

    /// The assignments of `key` among `reach`, at most
    /// [`MAX_CANDIDATE_READS`] (only those are copied, however many the
    /// name has), and whether that is all of them.
    fn reaching_assigns(&self, key: &Key, reach: &Reach) -> (Vec<Assign<'t>>, bool) {
        let Some(assigns) = self.assigns.get(key) else {
            return (Vec::new(), true);
        };
        let mut reaching: Vec<Assign<'t>> = match reach {
            Reach::All => assigns
                .iter()
                .take(MAX_CANDIDATE_READS + 1)
                .copied()
                .collect(),
            Reach::Subset(defs) => defs
                .iter()
                .filter_map(|def| match def {
                    DefRef::Assign(i) => assigns.get(*i).copied(),
                    _ => None,
                })
                .take(MAX_CANDIDATE_READS + 1)
                .collect(),
        };
        let all = reaching.len() <= MAX_CANDIDATE_READS;
        reaching.truncate(MAX_CANDIDATE_READS);
        (reaching, all)
    }

    /// The values the assignments of `name` that reach a use at byte `pos`
    /// of `scope` give it (see [`Self::assign_candidates`]), and whether
    /// they are every value: only when each resolved, there were few
    /// enough to read, and all are in the scope that binds the name (a
    /// `global` or `nonlocal` write from another function is not). When
    /// every definition reaches the use, the list is the same for every
    /// use and is read once.
    fn name_candidates(
        &mut self,
        name: &str,
        scope: usize,
        pos: usize,
        origin: ValueOrigin,
        depth: u32,
    ) -> (Vec<Resolved>, bool) {
        let (key, at) = self.locate(name, scope, pos);
        let reach = self.reaching(&key, at);
        let every_use = matches!(reach, Reach::All);
        let memo_key = (key, origin);
        if every_use {
            if let Some(found) = self.reach_all_memo.get(&memo_key) {
                return (found.0.clone(), found.1);
            }
        }
        let (reaching, all) = self.reaching_assigns(&memo_key.0, &reach);
        let one_scope = reaching.iter().all(|a| a.scope == memo_key.0 .0);
        // A nested lookup collects nothing; its empty answer is not kept.
        let reading = self.collecting == 0;
        let (candidates, complete) = self.assign_candidates(&reaching, origin, name, depth);
        let found = (candidates, complete && all && one_scope);
        if every_use && reading {
            self.reach_all_memo
                .insert(memo_key, Rc::new((found.0.clone(), found.1)));
        }
        found
    }

    /// The values `assigns` give a name, for a `reassigned` gap: each
    /// assignment resolved on its own (without collecting candidates of
    /// its own), a loop target's items, distinct, at most
    /// [`MAX_CANDIDATES`]; and whether every assignment resolved, with no
    /// caveat, into the list. At most [`MAX_CANDIDATE_READS`] values are
    /// read in all, assignments and loop items together, so a use reached
    /// by many loops over a long list costs no more than one.
    fn assign_candidates(
        &mut self,
        assigns: &[Assign<'t>],
        origin: ValueOrigin,
        name: &str,
        depth: u32,
    ) -> (Vec<Resolved>, bool) {
        if self.collecting > 0 {
            return (Vec::new(), false);
        }
        self.collecting += 1;
        let mut found = Vec::new();
        let mut complete = assigns.len() <= MAX_CANDIDATE_READS;
        let mut reads_left = MAX_CANDIDATE_READS;
        for assign in assigns.iter().take(MAX_CANDIDATE_READS) {
            if reads_left == 0 {
                complete = false;
                break;
            }
            let value = match (assign.value, assign.unknowable) {
                (Some(value), false) => value,
                _ => {
                    // A loop target takes each item of a literal list.
                    let Some(iter) = assign.iter else {
                        complete = false;
                        continue;
                    };
                    let items = self.loop_items(iter, assign.scope, origin, depth);
                    complete &= items.1;
                    for item in items.0.iter() {
                        if reads_left == 0 {
                            complete = false;
                            break;
                        }
                        reads_left -= 1;
                        complete &= push_candidate(&mut found, item.clone().through(name, origin));
                    }
                    continue;
                }
            };
            reads_left -= 1;
            match self.resolve_value(value, assign.scope, depth + 1) {
                Ok(mut resolved) => {
                    complete &= resolved.is_certain();
                    if resolved.origin == ValueOrigin::Literal {
                        resolved.origin = origin;
                    }
                    complete &= push_candidate(&mut found, resolved.through(name, origin));
                }
                Err(_) => complete = false,
            }
        }
        self.collecting -= 1;
        (found, complete)
    }

    /// The items of the literal list, tuple or set a loop runs over (or of
    /// one bound once to a name that is never aliased, passed or mutated),
    /// resolved, and whether every item resolved. Called while candidates
    /// are being collected; read once per iterable, and at most
    /// [`MAX_CANDIDATE_READS`] items of it.
    fn loop_items(
        &mut self,
        iter: Node<'t>,
        scope: usize,
        origin: ValueOrigin,
        depth: u32,
    ) -> CandidateList {
        let key = (iter.id(), origin);
        if let Some(found) = self.loop_items_memo.get(&key) {
            return Rc::clone(found);
        }
        let found = Rc::new(self.read_loop_items(iter, scope, origin, depth));
        self.loop_items_memo.insert(key, Rc::clone(&found));
        found
    }

    fn read_loop_items(
        &mut self,
        iter: Node<'t>,
        scope: usize,
        origin: ValueOrigin,
        depth: u32,
    ) -> (Vec<Resolved>, bool) {
        // A comprehension in a class body runs its first iterable in the
        // class scope and the rest in its own: not read.
        let in_class = self.scopes[scope].comprehension
            && self.scopes[scope]
                .parent
                .is_some_and(|p| self.scopes[p].kind == ScopeKind::Class);
        if in_class {
            return (Vec::new(), false);
        }
        let list = match iter.kind() {
            "list" | "tuple" | "set" => Some((iter, scope)),
            "identifier" => {
                let name = self.text(iter);
                let (key, defs) = self.defs_at(name, scope, iter.start_byte());
                let single = !defs.param && !defs.imported && defs.assigns == 1;
                match defs
                    .only_assign
                    .filter(|_| single && !self.escaped.contains(name))
                {
                    Some(i) => self
                        .assigns
                        .get(&key)
                        .and_then(|v| v.get(i))
                        .filter(|a| !a.unknowable)
                        .and_then(|a| a.value.map(|value| (value, a.scope)))
                        .filter(|(value, _)| matches!(value.kind(), "list" | "tuple" | "set")),
                    None => None,
                }
            }
            _ => None,
        };
        let Some((list, list_scope)) = list else {
            return (Vec::new(), false);
        };
        let mut found = Vec::new();
        let mut complete = true;
        let mut read = 0;
        // A cursor stops at the limit: a huge literal is not listed whole.
        let mut cursor = list.walk();
        let mut more = cursor.goto_first_child();
        while more {
            let item = cursor.node();
            more = cursor.goto_next_sibling();
            if !item.is_named() || item.kind() == "comment" {
                continue;
            }
            if read >= MAX_CANDIDATE_READS {
                complete = false;
                break;
            }
            read += 1;
            match self.resolve_value(item, list_scope, depth + 1) {
                Ok(mut resolved) => {
                    complete &= resolved.is_certain();
                    if resolved.origin == ValueOrigin::Literal {
                        resolved.origin = origin;
                    }
                    complete &= push_candidate(&mut found, resolved);
                }
                Err(_) => complete = false,
            }
        }
        (found, complete)
    }

    /// The two sides of `a if cond else b`, each resolved on its own, and
    /// whether both resolved.
    fn branch_candidates(
        &mut self,
        expr: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> (Vec<Resolved>, bool) {
        if self.collecting > 0 {
            return (Vec::new(), false);
        }
        self.collecting += 1;
        let children = named_children(expr);
        let mut found = Vec::new();
        let mut complete = true;
        for side in [children.first(), children.last()].into_iter().flatten() {
            match self.resolve_value(*side, scope, depth + 1) {
                Ok(resolved) => {
                    complete &= resolved.is_certain();
                    complete &= push_candidate(&mut found, resolved);
                }
                Err(unresolved) => {
                    complete &= unresolved.complete();
                    for candidate in unresolved.candidates {
                        complete &= push_candidate(&mut found, candidate);
                    }
                }
            }
        }
        self.collecting -= 1;
        (found, complete)
    }

    /// The values a dict literal's entries hold, for a lookup whose key is
    /// known only at run time: the first [`MAX_CANDIDATE_READS`] entries
    /// (never under a secret-named or credential-shaped key), distinct, at
    /// most [`MAX_CANDIDATES`], and whether they are all of its values.
    /// Read once per dict.
    fn dict_candidates(&mut self, dict: &DictRef<'t>) -> Rc<DictCandidates> {
        let key = (dict.node.id(), dict.name.clone());
        if let Some(found) = self.dict_candidates_memo.get(&key) {
            return Rc::clone(found);
        }
        if self.collecting > 0 {
            return Rc::new(DictCandidates {
                values: Vec::new(),
                complete: false,
            });
        }
        self.collecting += 1;
        let mut found = Vec::new();
        let entries = dict_entries(dict.node, |n| self.literal(n).ok(), |n| self.text(n));
        let mut complete = entries.len() <= MAX_CANDIDATE_READS;
        for entry in entries.into_iter().take(MAX_CANDIDATE_READS) {
            // `**other` or a computed key: anything may be there.
            let Some((entry_key, value)) = entry else {
                complete = false;
                continue;
            };
            // A value under a credential's key is never read.
            if is_secret_key(&entry_key) || is_credential_value(&entry_key) {
                complete = false;
                continue;
            }
            match self.dict_entry(dict, &entry_key, value, 0) {
                Ok(resolved) => {
                    complete &= resolved.is_certain();
                    complete &= push_candidate(&mut found, resolved);
                }
                Err(_) => complete = false,
            }
        }
        self.collecting -= 1;
        let found = Rc::new(DictCandidates {
            values: found,
            complete,
        });
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
                    let unresolved = Unresolved::new("non_literal", String::new());
                    // `for model in ["gpt-4o", "o1"]`: each item is a value.
                    let Some(iter) = assign.iter.filter(|_| self.collecting == 0) else {
                        return Err(unresolved);
                    };
                    self.collecting += 1;
                    let items = self.loop_items(iter, assign.scope, origin, depth);
                    self.collecting -= 1;
                    return Err(unresolved.with_candidates(items.0.clone(), items.1));
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
        let mut candidates_complete = false;
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
                        let found = self.dict_candidates(&dict);
                        candidates = found.values.clone();
                        candidates_complete = found.complete;
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
                Unresolved::new(reason, shape).with_candidates(candidates, candidates_complete)
            ));
        };
        Some(match self.resolve_value(default, scope, depth + 1) {
            Ok(mut resolved) => {
                resolved.origin = ValueOrigin::EnvDefault;
                // With the default captured, the dict's values are every
                // other value only for a dict lookup; an environment or
                // config lookup may return anything.
                let complete = candidates_complete && resolved.caveats.is_empty();
                resolved
                    .caveats
                    .push(Unresolved::new(reason, shape).with_candidates(candidates, complete));
                Ok(resolved)
            }
            Err(unresolved) if unresolved.reason == "credential" => Err(unresolved),
            Err(_) => Err(Unresolved::new(reason, shape).with_candidates(candidates, false)),
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
            Some((index, value)) if !summary.overridable(*index) => Lookup::Found(*value),
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
                if let Some(slot) = summary.expansion.iter_mut().find(|(k, _, _)| *k == key) {
                    slot.1 = index;
                    slot.2 = value;
                } else if summary.expansion.len() < 256 {
                    summary.expansion.push((key.clone(), index, value));
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
/// most [`MAX_CANDIDATES`]; `false` when it was dropped at that limit.
fn push_candidate(values: &mut Vec<Resolved>, mut resolved: Resolved) -> bool {
    if values.iter().any(|v| v.value == resolved.value) {
        return true;
    }
    if values.len() >= MAX_CANDIDATES {
        return false;
    }
    resolved.caveats.clear();
    values.push(resolved);
    true
}

/// Whether `span` lies inside `node`: a literal written at that call.
fn within(node: Node<'_>, span: Span) -> bool {
    node.start_byte() <= span.start_byte as usize && span.end_byte as usize <= node.end_byte()
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

/// Builtins that read or rebind a namespace by name.
fn is_namespace_builtin(name: &str) -> bool {
    matches!(
        name,
        "exec" | "eval" | "compile" | "globals" | "locals" | "vars" | "__import__"
    )
}

/// Whether the node of `frame` is a statement of its own directly in the
/// module body: an expression statement there, not a branch, loop, `try`,
/// `with`, class body or function, nor part of a larger expression.
/// `os.environ` or `os.environb`, which shares its data on POSIX.
fn is_environ_name(name: &str) -> bool {
    matches!(name, "environ" | "environb")
}

/// A module attribute that routes the module-level OpenAI API.
fn is_module_routing_attr(attr: &str) -> bool {
    matches!(
        attr,
        "api_type"
            | "api_base"
            | "base_url"
            | "azure_endpoint"
            | "azure_ad_token"
            | "azure_ad_token_provider"
    ) || attr.starts_with("bedrock_")
}

fn is_top_statement(frame: &Frame<'_>) -> bool {
    frame
        .parent
        .is_some_and(|p| p.kind() == "expression_statement")
        && frame.grand.is_some_and(|g| g.kind() == "module")
}

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

/// `super()`.
fn is_super_call<'t>(node: Node<'t>, text: impl Fn(Node<'t>) -> &'t str) -> bool {
    node.kind() == "call"
        && node
            .child_by_field_name("function")
            .is_some_and(|f| f.kind() == "identifier" && text(f) == "super")
}

/// `globals()`, `locals()` or `vars()` with no argument: the namespace
/// itself, whose keys are binding names.
fn is_namespace_call<'t>(node: Node<'t>, text: impl Fn(Node<'t>) -> &'t str) -> bool {
    node.kind() == "call"
        && node.child_by_field_name("function").is_some_and(|f| {
            f.kind() == "identifier" && matches!(text(f), "globals" | "locals" | "vars")
        })
        && node
            .child_by_field_name("arguments")
            .is_some_and(|a| a.named_child_count() == 0)
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

/// The value of the first keyword argument of `call` whose name `wanted`
/// accepts.
fn keyword_value<'t>(
    call: Node<'t>,
    text: impl Fn(Node<'t>) -> &'t str,
    wanted: impl Fn(&str) -> bool,
) -> Option<Node<'t>> {
    let args = call
        .child_by_field_name("arguments")
        .filter(|a| a.kind() == "argument_list")?;
    named_children(args).into_iter().find_map(|arg| {
        let name = arg.child_by_field_name("name")?;
        (arg.kind() == "keyword_argument" && wanted(text(name)))
            .then(|| arg.child_by_field_name("value"))
            .flatten()
    })
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

/// Combines every value a binding is given. Placeholders (`None`, literals)
/// and self-references are ignored next to a real value. A client mixed with
/// a different client or with an unknown value is ambiguous.
fn combine(vals: Vec<Val>) -> Val {
    // A dynamic feature next to the values: a client keeps them with a
    // `dynamic_feature` caveat; anything else is replaced by it.
    let dynamic = vals.iter().find_map(|v| match v {
        Val::Untracked(Origin::Dynamic(feature)) => Some(*feature),
        _ => None,
    });
    if let Some(feature) = dynamic {
        let rest: Vec<Val> = vals
            .into_iter()
            .filter(|v| !matches!(v, Val::Untracked(Origin::Dynamic(_))))
            .collect();
        if rest.is_empty() {
            return Val::untracked(Origin::Dynamic(feature));
        }
        return match combine(rest) {
            Val::Client { mut client, path } => {
                if client.caveat.is_none() {
                    client.caveat = Some(ClientCaveat::Dynamic(feature));
                }
                Val::Client { client, path }
            }
            _ => Val::untracked(Origin::Dynamic(feature)),
        };
    }
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

    // Constructors that route differently, or a constructor next to
    // something else (a factory, a name from another module, an unknown
    // value): what they build is ambiguous.
    let classes: Vec<ClientClass> = real
        .iter()
        .filter_map(|v| match v {
            Val::Class(class) => Some(*class),
            _ => None,
        })
        .collect();
    if let Some(first) = classes.first() {
        if classes.len() < real.len()
            || classes
                .iter()
                .any(|c| c.pkg != first.pkg || c.family != first.family)
        {
            return Val::untracked(Origin::MixedClasses(first.pkg));
        }
    }

    // No client or constructor: a tracked module beats a fallback;
    // otherwise keep the most informative origin.
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
