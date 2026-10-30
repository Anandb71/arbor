//! TypeScript and JavaScript SDK usage extraction.
//!
//! # Soundness policy
//!
//! The contract is the one [`super::python`] keeps: a value or route
//! reported with no gap is confirmed, so a consumer acts on it without
//! asking. This module resolves a small **supported grammar** exactly and
//! turns everything else into a gap. `docs/SDK_USAGES.md` states the grammar
//! in full; in short, a value or route has no gap only when it is:
//!
//! - a string literal, or a template literal with no substitution, written
//!   at the call (never downgraded);
//! - a `const` binding of such a value, or a `let` or `var` binding the file
//!   assigns only in its declaration, read in the scope that binds it;
//! - a property of a same-file object literal bound that way and only ever
//!   read (by a static key, a spread or a destructuring), or a member of a
//!   same-file `enum`;
//! - a class property assigned once (by its field declaration or one
//!   `this.x = ...`), read through `this` in the class's own methods or by
//!   class name for a static one, in a class with no decorator, no
//!   computed or reflective write of its properties, at most one same-file
//!   level of inheritance and at most 16 direct same-file subclasses (a
//!   model value read through `this` in a class another module may extend
//!   keeps a `subclass_override` gap);
//! - a parameter default of a function whose calls are all in this file (it
//!   is not exported and is only ever called directly, or it is a `private`
//!   or `#private` method, or the constructor of such a class), none of
//!   which passes another value;
//! - a client from a tracked constructor with no `baseURL`, or one that
//!   resolves to a string (the provider's own host is direct, another host
//!   is `custom_base_url`), and for Gemini no `vertexai`/`enterprise`
//!   option, endpoint `httpOptions` or spread, in a file that does not set
//!   the SDK's endpoint variable, replace an SDK member or mock the SDK.
//!
//! JavaScript scopes names lexically, and nothing but an assignment in the
//! file can rebind one: `eval` and `with` are the exceptions, and either
//! makes every value and route of the file a `dynamic_feature` gap. A
//! binding is therefore read the same everywhere in its scope (hoisting and
//! the temporal dead zone make it visible in all of it), and resolution is
//! flow-insensitive: `const` bindings and imports are never written, and a
//! `let` or `var` binding written more than once is a `reassigned` gap that
//! lists what it may hold.
//!
//! # Passes
//!
//! 1. **Collect.** One iterative walk records scopes (the module, each
//!    function, each block), declarations (including destructuring
//!    patterns and their defaults), imports, every write of a name or a
//!    property, every use of a name with what the use does (called,
//!    constructed, read as an object, written into, exported, passed...),
//!    classes with their fields and `this` writes, functions with their
//!    parameters, every call and `new`, and the graph's name for each
//!    function, so a record's symbol is the one `get_callers` knows.
//!    Parents travel down the walk; no node is asked for its parent.
//! 2. **Analyze.** Names are bound to declarations, then each call's callee
//!    is evaluated. A chain rooted at a tracked SDK binding becomes a
//!    construction or a call; an SDK-shaped chain on an unknown receiver, a
//!    computed member on a tracked binding, an SDK method used as a value
//!    and an unresolved model value become typed gaps.
//!
//! Every lookup does a bounded amount of work (the `MAX_*` constants), so
//! the pass is linear in the file.

use super::{
    is_capturable_value, is_credential_value, is_identifier_key, is_secret_key, is_value_key,
    model_family, BindingKind, CallArgs, CapturedValue, ClientConstruction, ClientScope,
    FileUsages, GapKind, Receiver, Routing, Sdk, SdkCall, SdkImport, Span, UsageGap, ValueOrigin,
    VertexAi,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use tree_sitter::{Node, Tree};

/// A file mentioning none of these cannot report anything, so it is neither
/// parsed nor walked.
const PREFILTER: &[&str] = &[
    "openai",
    "anthropic",
    "@google/genai",
    "@google/generative-ai",
    "GoogleGenAI",
    "GoogleGenerativeAI",
    "completions.",
    "responses.",
    "messages.",
    "embeddings.",
    "generateContent",
    "embedContent",
    "createChatCompletion",
    "createCompletion",
];

/// Whether `source` mentions anything the extractor tracks. A file that does
/// not reports nothing, so callers may skip parsing it.
pub fn may_use_sdk(source: &str) -> bool {
    PREFILTER.iter().any(|needle| source.contains(needle))
        || MODEL_BEARING.iter().any(|needle| source.contains(needle))
}

/// The language a file's records carry: `javascript` for `.js`, `.jsx`,
/// `.mjs` and `.cjs`, `typescript` for the rest.
pub fn language_of(file_path: &str) -> &'static str {
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ["js", "jsx", "mjs", "cjs"]
        .iter()
        .any(|e| e.eq_ignore_ascii_case(extension))
    {
        "javascript"
    } else {
        "typescript"
    }
}

/// Last member names that send a request to the provider. A call ending in
/// one is an [`SdkCall::request`]; storing or passing one as a value hides
/// the eventual request.
const REQUEST_METHODS: &[&str] = &[
    "create",
    "stream",
    "parse",
    "retrieve",
    "list",
    "delete",
    "del",
    "update",
    "cancel",
    "pause",
    "resume",
    "upload",
    "download",
    "content",
    "listEvents",
    "compact",
    "count",
    "runTools",
    "runFunctions",
    "createAndPoll",
    "createAndStream",
    "createAndRun",
    "createAndRunPoll",
    "createAndRunStream",
    "submitToolOutputs",
    "submitToolOutputsAndPoll",
    "submitToolOutputsStream",
    "poll",
    "uploadAndPoll",
    "generate",
    "edit",
    "createVariation",
    "connect",
    "countTokens",
    "computeTokens",
    "toolRunner",
    "generateContent",
    "generateContentStream",
    "embedContent",
    "batchEmbedContents",
    "generateImages",
    "editImage",
    "upscaleImage",
    "recontextImage",
    "segmentImage",
    "generateVideos",
    "createEmbeddings",
    "sendMessage",
    "sendMessageStream",
    "tune",
    "get",
    "uploadFile",
    "getFile",
    "listFiles",
    "deleteFile",
    // openai v3 (`OpenAIApi`).
    "createChatCompletion",
    "createCompletion",
    "createEdit",
    "createEmbedding",
    "createModeration",
    "createImage",
    "createImageEdit",
    "createImageVariation",
    "createFineTune",
    "createAnswer",
    "createClassification",
    "createSearch",
    "createTranscription",
    "createTranslation",
    "createFile",
    "listModels",
    "retrieveModel",
    "deleteModel",
    "listEngines",
    "retrieveEngine",
    "retrieveFile",
    "downloadFile",
    "listFineTunes",
    "retrieveFineTune",
    "cancelFineTune",
    "listFineTuneEvents",
];

/// Legacy Gemini model and chat methods that take request options as their
/// second argument, which may name a `baseUrl`.
const LEGACY_REQUEST_OPTIONS: &[&str] = &[
    "generateContent",
    "generateContentStream",
    "countTokens",
    "embedContent",
    "batchEmbedContents",
    "sendMessage",
    "sendMessageStream",
];

/// Whether a call's member path ends in a request method.
fn is_request(member: &str) -> bool {
    let last = member.rsplit('.').next().unwrap_or(member);
    REQUEST_METHODS.contains(&last)
}

/// Members that take a model, matched whole or as a `.`-suffix. Like a
/// request, an unresolved model value here is an `unresolved_model` gap, a
/// spread or an options object built elsewhere with no explicit model key
/// is a `splat` gap, and passing no model is a `missing_model` gap.
const MODEL_BEARING: &[&str] = &[
    "chat.completions.create",
    "chat.completions.parse",
    "chat.completions.stream",
    "chat.completions.runTools",
    "chat.completions.runFunctions",
    "completions.create",
    "responses.create",
    "responses.parse",
    "responses.stream",
    "responses.compact",
    "embeddings.create",
    "images.generate",
    "images.edit",
    "images.createVariation",
    "audio.transcriptions.create",
    "audio.translations.create",
    "audio.speech.create",
    "moderations.create",
    "fineTuning.jobs.create",
    "assistants.create",
    "runs.create",
    "runs.createAndPoll",
    "runs.createAndStream",
    "runs.stream",
    "threads.createAndRun",
    "threads.createAndRunPoll",
    "threads.createAndRunStream",
    "videos.create",
    "messages.create",
    "messages.stream",
    "messages.parse",
    "messages.countTokens",
    "messages.toolRunner",
    "models.generateContent",
    "models.generateContentStream",
    "models.embedContent",
    "models.countTokens",
    "models.computeTokens",
    "models.generateImages",
    "models.editImage",
    "models.upscaleImage",
    "models.recontextImage",
    "models.segmentImage",
    "models.generateVideos",
    "chats.create",
    "live.connect",
    "caches.create",
    "getGenerativeModel",
    // openai v3 (`OpenAIApi`).
    "createChatCompletion",
    "createCompletion",
    "createEdit",
    "createEmbedding",
    "createModeration",
    "createFineTune",
    "createTranscription",
    "createTranslation",
];

/// OpenAI methods whose first argument is a path parameter and whose second
/// is the request body (`runs.create(threadId, { model })`).
const PATH_PARAM_METHODS: &[&str] = &[
    "runs.create",
    "runs.createAndPoll",
    "runs.createAndStream",
    "runs.stream",
    "assistants.update",
];

/// Whether `member` is `name` or ends in `.name`.
fn member_is(member: &str, name: &str) -> bool {
    member == name
        || member
            .strip_suffix(name)
            .is_some_and(|head| head.ends_with('.'))
}

/// Whether a call's member path takes a model.
fn is_model_bearing(member: &str) -> bool {
    MODEL_BEARING.iter().any(|m| {
        member == *m
            || member
                .strip_suffix(m)
                .is_some_and(|head| head.ends_with('.'))
    })
}

/// openai v3 methods whose model is a positional argument
/// (`createTranscription(file, "whisper-1")`), and its index.
const V3_POSITIONAL_MODEL: &[(&str, usize)] =
    &[("createTranscription", 1), ("createTranslation", 1)];

/// Maximum expression depth within one evaluation; deeper chains are
/// treated as unresolved. Every name resolution starts a fresh budget.
const MAX_DEPTH: u32 = 64;
/// Maximum nesting of name, property and caller resolutions.
const MAX_NAME_DEPTH: u32 = 64;
/// Maximum nesting of scopes (functions, blocks, classes) the collection
/// pass follows. A deeper scope is not analyzed: it is a `truncated` gap,
/// and a dynamic feature of the whole file, since what it binds or writes
/// is not seen.
const MAX_SCOPE_DEPTH: u32 = 64;
/// Direct same-file subclasses a `this.x` read looks at.
const MAX_SUBCLASSES: usize = 16;
/// Writes of one name or property read per lookup. More is a gap anyway
/// (`reassigned`), so the rest are not read.
const MAX_WRITES_READ: usize = 64;
/// Distinct paths read off one name (`CONFIG.a`, `CONFIG.b.c`...) that are
/// checked for a container they may hand out; past that every read of the
/// name is a gap.
const MAX_HANDED_OUT: usize = 4096;
/// Longest string literal read, in source bytes. A captured value is at
/// most 256 bytes, so a longer literal is refused without being copied.
const MAX_LITERAL_BYTES: usize = 4096;
/// Object and array entries a search for nested model keys reads per call.
const MAX_NESTED_ENTRIES: usize = 256;
/// Spreads expanded per object literal; past it, a spread is opaque.
const MAX_SPREADS: usize = 64;
/// Items of a `middleware` array checked for the SDK's own refusal
/// fallback; a longer array keeps its gap.
const MAX_MIDDLEWARE: usize = 16;
/// Literals below an option's value a search for nested model keys
/// descends, as in Python (`requests: [{ params: { model } }]` is 2).
const MAX_NESTED_DEPTH: usize = 4;
/// Nested model values listed per call and per file. Past either, the call
/// gets a `truncated` gap instead.
const MAX_NESTED_VALUES: usize = 32;
const MAX_FILE_NESTED_VALUES: usize = 4096;
/// Candidate values listed with one gap, and per file.
const MAX_CANDIDATES: usize = 32;
const MAX_FILE_CANDIDATES: usize = 4096;
/// In-file calls read for the values a parameter is given. A function or
/// method called more often is treated as having callers this file does
/// not show.
const MAX_CALLERS_READ: usize = 256;
/// Longest name, chain or expression shape reported, in bytes: a longer one
/// keeps its start and end around `...`.
const MAX_TEXT: usize = 200;
/// Longest member path followed off a tracked binding.
const MAX_PATH: usize = 32;
/// Links of a chain rendered before the rest is shown as `<expr>`.
const MAX_LINKS: usize = 64;
/// Imports, calls, gaps and clients reported per file. Past it, the rest of
/// the file is one `truncated` gap.
const MAX_FILE_RECORDS: usize = 20_000;
/// Names bound by import statements per file. A file that binds more is not
/// analyzed past its imports (one `truncated` gap).
const MAX_FILE_IMPORTS: usize = MAX_FILE_RECORDS;

/// Extracts SDK usages from a parsed TypeScript or JavaScript file.
pub fn extract(tree: &Tree, source: &str, file_path: &str) -> FileUsages {
    let language = language_of(file_path);
    if !may_use_sdk(source) {
        return FileUsages::new(file_path, language);
    }
    let mut cx = Ctx::new(source, file_path, language);
    cx.collect(tree.root_node());
    cx.bind_names();
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
    AnthropicBedrock,
    AnthropicVertex,
    AnthropicFoundry,
    AnthropicAws,
    GoogleGenai,
    GoogleGenerativeai,
}

impl Pkg {
    fn sdk(self) -> Sdk {
        match self {
            Pkg::Openai => Sdk::Openai,
            Pkg::Anthropic
            | Pkg::AnthropicBedrock
            | Pkg::AnthropicVertex
            | Pkg::AnthropicFoundry
            | Pkg::AnthropicAws => Sdk::Anthropic,
            Pkg::GoogleGenai | Pkg::GoogleGenerativeai => Sdk::Google,
        }
    }

    /// The npm package name.
    fn package(self) -> &'static str {
        match self {
            Pkg::Openai => "openai",
            Pkg::Anthropic => "@anthropic-ai/sdk",
            Pkg::AnthropicBedrock => "@anthropic-ai/bedrock-sdk",
            Pkg::AnthropicVertex => "@anthropic-ai/vertex-sdk",
            Pkg::AnthropicFoundry => "@anthropic-ai/foundry-sdk",
            Pkg::AnthropicAws => "@anthropic-ai/aws-sdk",
            Pkg::GoogleGenai => "@google/genai",
            Pkg::GoogleGenerativeai => "@google/generative-ai",
        }
    }

    /// The class a default import (or `require`) of the package's entry
    /// point is. `@google/genai` and `@google/generative-ai` have none.
    fn default_class(self) -> Option<ClientClass> {
        let name = match self {
            Pkg::Openai => "OpenAI",
            Pkg::Anthropic => "Anthropic",
            Pkg::AnthropicBedrock => "AnthropicBedrock",
            Pkg::AnthropicVertex => "AnthropicVertex",
            Pkg::AnthropicFoundry => "AnthropicFoundry",
            Pkg::AnthropicAws => "AnthropicAws",
            Pkg::GoogleGenai | Pkg::GoogleGenerativeai => return None,
        };
        client_class(self, name)
    }

    /// The package an import specifier names, and whether it is an entry
    /// point that exports the package's clients (`openai`, `openai/index`,
    /// `@google/genai/node`) rather than a module of types or helpers
    /// (`openai/resources/chat`). Deno's `npm:` and `jsr:` specifiers,
    /// versions (`openai@4`) and CDN URLs (`https://esm.sh/openai@4`) are
    /// read too.
    fn of_source(source: &str) -> Option<(Pkg, bool)> {
        let spec = source.trim();
        let spec = spec.split(['?', '#']).next().unwrap_or(spec);
        if let Some(rest) = spec.strip_prefix("jsr:") {
            let (name, sub) = split_package(rest.trim_start_matches('/'))?;
            return (name == "@openai/openai").then(|| (Pkg::Openai, Pkg::Openai.is_entry(sub)));
        }
        let spec = spec.strip_prefix("npm:").unwrap_or(spec);
        let spec = match spec
            .strip_prefix("https://")
            .or_else(|| spec.strip_prefix("http://"))
        {
            Some(url) => {
                let (host, path) = url.split_once('/')?;
                match host {
                    // `https://esm.sh/v135/openai@4` pins a build version.
                    "esm.sh" => {
                        let path = path.trim_start_matches('/');
                        match path.split_once('/') {
                            Some((build, rest))
                                if build.len() > 1
                                    && build.starts_with('v')
                                    && build[1..].bytes().all(|b| b.is_ascii_digit()) =>
                            {
                                rest
                            }
                            _ => path,
                        }
                    }
                    "cdn.skypack.dev" | "esm.run" | "unpkg.com" => path.trim_start_matches('/'),
                    // `https://cdn.jsdelivr.net/npm/openai@4/+esm`
                    "cdn.jsdelivr.net" => {
                        let path = path.strip_prefix("npm/")?;
                        path.strip_suffix("/+esm").unwrap_or(path)
                    }
                    "deno.land" => {
                        let rest = path.strip_prefix("x/")?;
                        let (name, sub) = rest.split_once('/').unwrap_or((rest, ""));
                        let name = name.split('@').next().unwrap_or(name);
                        return (name == "openai")
                            .then(|| (Pkg::Openai, sub.is_empty() || sub == "mod.ts"));
                    }
                    _ => return None,
                }
            }
            None => spec,
        };
        let (name, sub) = split_package(spec)?;
        let pkg = match name {
            "openai" => Pkg::Openai,
            "@anthropic-ai/sdk" => Pkg::Anthropic,
            "@anthropic-ai/bedrock-sdk" => Pkg::AnthropicBedrock,
            "@anthropic-ai/vertex-sdk" => Pkg::AnthropicVertex,
            "@anthropic-ai/foundry-sdk" => Pkg::AnthropicFoundry,
            "@anthropic-ai/aws-sdk" => Pkg::AnthropicAws,
            "@google/genai" => Pkg::GoogleGenai,
            "@google/generative-ai" => Pkg::GoogleGenerativeai,
            _ => return None,
        };
        Some((pkg, pkg.is_entry(sub)))
    }

    /// Whether `sub` (the path after the package name, without a leading
    /// `/`) is an entry point that exports the package's clients.
    fn is_entry(self, sub: &str) -> bool {
        let sub = sub.trim_end_matches('/');
        let sub = sub
            .strip_suffix(".js")
            .or_else(|| sub.strip_suffix(".mjs"))
            .or_else(|| sub.strip_suffix(".cjs"))
            .or_else(|| sub.strip_suffix(".ts"))
            .unwrap_or(sub);
        match self {
            Pkg::Openai => matches!(sub, "" | "index" | "client" | "azure" | "bedrock" | "mod"),
            Pkg::GoogleGenai => matches!(sub, "" | "node" | "web" | "index"),
            _ => matches!(sub, "" | "index" | "client"),
        }
    }
}

/// An npm specifier split into the package name (version removed) and the
/// subpath after it.
fn split_package(spec: &str) -> Option<(&str, &str)> {
    let (scope_len, rest) = match spec.strip_prefix('@') {
        Some(rest) => {
            let (scope, rest) = rest.split_once('/')?;
            (scope.len() + 2, rest)
        }
        None => (0, spec),
    };
    let name_end = rest.find('/').unwrap_or(rest.len());
    let name = &rest[..name_end];
    let name = name.split('@').next().unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    let full = &spec[..scope_len + name.len()];
    let sub = rest[name_end..].trim_start_matches('/');
    Some((full, sub))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    OpenaiDirect,
    /// `OpenAIApi` from openai v3: routed by its `Configuration`.
    OpenaiV3,
    Azure,
    AnthropicDirect,
    Bedrock,
    Vertex,
    Genai,
    /// `GoogleGenerativeAI`: its models (`getGenerativeModel`) make the
    /// requests.
    LegacyRoot,
    /// A model from `getGenerativeModel`, or its chat.
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
    class(Pkg::Openai, "AzureOpenAI", Family::Azure),
    class(Pkg::Openai, "BedrockOpenAI", Family::Platform),
    class(Pkg::Openai, V3_API, Family::OpenaiV3),
    class(Pkg::Anthropic, "Anthropic", Family::AnthropicDirect),
    class(Pkg::AnthropicBedrock, "AnthropicBedrock", Family::Bedrock),
    class(
        Pkg::AnthropicBedrock,
        "AnthropicBedrockMantle",
        Family::Platform,
    ),
    class(Pkg::AnthropicVertex, "AnthropicVertex", Family::Vertex),
    class(Pkg::AnthropicFoundry, "AnthropicFoundry", Family::Platform),
    class(Pkg::AnthropicAws, "AnthropicAws", Family::Platform),
    class(Pkg::GoogleGenai, "GoogleGenAI", Family::Genai),
    class(
        Pkg::GoogleGenerativeai,
        "GoogleGenerativeAI",
        Family::LegacyRoot,
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

/// openai v3's `Configuration`: `new OpenAIApi(new Configuration({ basePath }))`.
const V3_CONFIGURATION: &str = "Configuration";
/// openai v3's client, which sends its requests through axios.
const V3_API: &str = "OpenAIApi";

impl ClientClass {
    /// Routing from the constructor, its option names and what its
    /// `baseURL` is.
    fn routing(self, kwargs: &[String], splat: bool, base_url: BaseUrl) -> Routing {
        // `new OpenAI({ provider: bedrock(...) })` hands requests to a
        // platform adapter.
        let adapter = kwargs.iter().any(|k| k == "provider");
        let direct = matches!(
            self.family,
            Family::OpenaiDirect | Family::OpenaiV3 | Family::AnthropicDirect
        );
        match self.family {
            _ if direct && adapter => Routing::Unknown,
            _ if direct && matches!(base_url, BaseUrl::Custom | BaseUrl::CustomFallback) => {
                Routing::CustomBaseUrl
            }
            _ if direct && matches!(base_url, BaseUrl::Unknown { .. }) => Routing::Unknown,
            _ if direct && splat && base_url == BaseUrl::Absent => Routing::Unknown,
            Family::OpenaiDirect | Family::OpenaiV3 => Routing::OpenaiDirect,
            Family::AnthropicDirect => Routing::AnthropicDirect,
            Family::Azure => Routing::AzureOpenai,
            Family::Bedrock => Routing::AnthropicBedrock,
            Family::Vertex => Routing::AnthropicVertex,
            Family::Genai => Routing::GoogleGenai,
            Family::LegacyRoot | Family::LegacyModel => Routing::GoogleGenerativeaiLegacy,
            Family::Platform => Routing::Unknown,
        }
    }

    /// Routing known from a type annotation alone: a direct client may
    /// still have been given a `baseURL` (a Gemini client `vertexai`), so
    /// it is `unknown`.
    fn typed_routing(self) -> Routing {
        match self.family {
            Family::OpenaiDirect
            | Family::OpenaiV3
            | Family::AnthropicDirect
            | Family::Genai
            | Family::LegacyRoot
            | Family::LegacyModel => Routing::Unknown,
            _ => self.routing(&[], false, BaseUrl::Absent),
        }
    }
}

/// The option that sets a client's endpoint, for the families that have
/// one: `baseURL` for OpenAI and Anthropic, `basePath` for openai v3's
/// `Configuration`.
fn base_url_key(family: Family) -> Option<&'static str> {
    match family {
        Family::OpenaiDirect | Family::AnthropicDirect => Some("baseURL"),
        Family::OpenaiV3 => Some("basePath"),
        _ => None,
    }
}

/// Whether a client sends its requests to a provider's own API through the
/// global `fetch`, as every SDK does unless the client is given its own:
/// not openai v3's `OpenAIApi` (`v3`), which sends through axios.
fn through_global_fetch(routing: Routing, v3: bool) -> bool {
    !v3 && matches!(
        routing,
        Routing::OpenaiDirect
            | Routing::AnthropicDirect
            | Routing::GoogleGenai
            | Routing::GoogleGenerativeaiLegacy
    )
}

/// The provider an SDK-shaped chain on an unknown receiver most likely
/// belongs to, and whether the shape alone is distinctive enough to report.
fn sdk_shape(chain: &str) -> Option<(Sdk, bool)> {
    const GOOGLE: &[&str] = &[
        ".models.generateContent",
        ".models.generateContentStream",
        ".models.embedContent",
        ".generateContent",
        ".generateContentStream",
    ];
    const OPENAI: &[&str] = &[
        ".chat.completions.create",
        ".chat.completions.parse",
        ".chat.completions.stream",
        ".chat.completions.runTools",
        ".responses.create",
        ".responses.parse",
        ".responses.stream",
        ".completions.create",
        ".embeddings.create",
        ".images.generate",
        ".images.edit",
        ".images.createVariation",
        ".audio.transcriptions.create",
        ".audio.translations.create",
        ".audio.speech.create",
        ".createChatCompletion",
        ".createCompletion",
    ];
    // `messages.create` is also Twilio's and Slack's shape, so it counts only
    // where the file imports an Anthropic SDK.
    const ANTHROPIC: &[&str] = &[
        ".messages.create",
        ".messages.stream",
        ".messages.parse",
        ".messages.countTokens",
    ];
    let chain = chain.replace("?.", ".").replace('!', "");
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

/// The SDK a chain's model-bearing member belongs to, and whether the shape
/// is its own (`messages.create` is also Twilio's), for a receiver Arbor
/// could not resolve.
fn model_bearing_shape(chain: &str) -> Option<(Sdk, bool)> {
    let chain = chain.replace("?.", ".").replace('!', "");
    let (_, member) = chain.split_once('.')?;
    if !is_model_bearing(member) {
        return None;
    }
    const GOOGLE: &[&str] = &[
        "models.",
        "chats.create",
        "live.connect",
        "caches.create",
        "getGenerativeModel",
        "generateContent",
        "generateContentStream",
        "embedContent",
    ];
    if member.contains("messages.") {
        let generic = ["messages.create", "messages.stream", "messages.parse"]
            .iter()
            .any(|m| member_is(member, m));
        Some((Sdk::Anthropic, !generic))
    } else if GOOGLE.iter().any(|g| member.contains(g)) {
        Some((Sdk::Google, true))
    } else {
        Some((Sdk::Openai, true))
    }
}

/// Environment variables an SDK reads its endpoint from when the
/// constructor is not given one. An Anthropic profile (`ANTHROPIC_PROFILE`,
/// `ANTHROPIC_CONFIG_DIR`) may name a `base_url` too.
const ENDPOINT_ENV: &[(Pkg, &str)] = &[
    (Pkg::Openai, "OPENAI_BASE_URL"),
    (Pkg::Anthropic, "ANTHROPIC_BASE_URL"),
    (Pkg::Anthropic, "ANTHROPIC_PROFILE"),
    (Pkg::Anthropic, "ANTHROPIC_CONFIG_DIR"),
    (Pkg::GoogleGenai, "GOOGLE_GENAI_USE_VERTEXAI"),
    (Pkg::GoogleGenai, "GOOGLE_GENAI_USE_ENTERPRISE"),
    (Pkg::GoogleGenai, "GOOGLE_GEMINI_BASE_URL"),
    (Pkg::GoogleGenai, "GOOGLE_VERTEX_BASE_URL"),
];

/// Client properties that re-route a built client when assigned.
const BASE_URL_PROPS: &[&str] = &["baseURL", "_baseURL", "baseUrl"];

/// Client properties a file may set without changing where requests go.
const SETTING_PROPS: &[&str] = &[
    "apiKey",
    "authToken",
    "organization",
    "project",
    "maxRetries",
    "timeout",
    "logLevel",
    "logger",
    "webhookSecret",
];

/// Test-runner calls that replace a module (`jest.mock("openai")`).
const MODULE_MOCKS: &[&str] = &[
    "jest.mock",
    "jest.doMock",
    "jest.unstable_mockModule",
    "jest.setMock",
    "vi.mock",
    "vi.doMock",
    "vitest.mock",
    "mock.module",
];

/// Test-runner calls that replace an object's method (`vi.spyOn(client.chat.completions, "create")`).
const METHOD_MOCKS: &[&str] = &[
    "jest.spyOn",
    "vi.spyOn",
    "sinon.stub",
    "sinon.replace",
    "mock.method",
];

/// Builtins that write properties of their first argument under names known
/// only at run time.
const REFLECTIVE_WRITES: &[&str] = &[
    "Object.assign",
    "Object.defineProperty",
    "Object.defineProperties",
    "Object.setPrototypeOf",
    "Reflect.set",
    "Reflect.defineProperty",
    "Reflect.deleteProperty",
    "Reflect.setPrototypeOf",
];

/// Test-runner functions whose callback is a test (mirrors the graph's
/// `TEST_BLOCKS` in `languages/typescript.rs`, so records name tests the
/// way the graph does).
const TEST_BLOCKS: &[&str] = &[
    "it",
    "test",
    "specify",
    "bench",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "before",
    "after",
];

// ============================================================================
// Values
// ============================================================================

/// A resolved client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Client {
    pkg: Pkg,
    family: Family,
    routing: Routing,
    /// Line of the same-file construction, when there is exactly one.
    line: Option<u32>,
    /// The construction (its node id), when there is exactly one.
    site: Option<usize>,
    /// Assigned more than one kind of client, or a client and something else.
    ambiguous: bool,
    /// Why the routing may not hold at a call, reported there as an
    /// `ambiguous_client` gap.
    caveat: Option<ClientCaveat>,
}

/// Why a resolved client's routing may not be the one a request uses.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientCaveat {
    /// Every in-file caller of a function passes this client for parameter
    /// `param`, but other modules may call it with another.
    InFileCallers { param: String },
    /// Known only from parameter `param`'s type annotation, not from where
    /// it is built.
    Annotation { param: String },
    /// `client.baseURL = ...` after the client is built.
    BaseUrlAssigned,
    /// The file sets an environment variable the SDK reads its endpoint
    /// from (`process.env.OPENAI_BASE_URL = ...`).
    EndpointEnv { var: &'static str },
    /// The file writes the environment with a name known only at run time,
    /// which may set the endpoint variable.
    EnvWrite(Feature),
    /// The `baseURL` given to the constructor is read from the environment
    /// with no fallback (`env`), falls back to the provider's own host
    /// (`fallback`), or does not resolve to a string.
    BaseUrl { env: bool, fallback: bool },
    /// A dynamic feature of the file may rebind or replace the client.
    Dynamic(Feature),
    /// A client built with an option that may send its requests to another
    /// endpoint (Gemini's `vertexai`, `enterprise` or `httpOptions`,
    /// OpenAI's `dataResidency`, a spread), or a legacy Gemini model given
    /// `requestOptions` with a `baseUrl`.
    EndpointOption { option: &'static str },
}

/// A JavaScript feature that can change what a name, property or the
/// environment holds at run time, and the line it is on.
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

/// How the `baseURL` given to a direct client routes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseUrl {
    /// None given, or `undefined`/`null`: the SDK's default host or its
    /// endpoint variable.
    Absent,
    /// The provider's own host (`https://api.openai.com/v1`).
    Provider,
    /// Another host: a gateway, a proxy or a local server.
    Custom,
    /// The provider's own host as the fallback of an environment lookup.
    ProviderFallback,
    /// Another host as the fallback of an environment lookup.
    CustomFallback,
    /// Read from the environment with no fallback (`env`), or not a string
    /// this file resolves.
    Unknown { env: bool },
}

/// The provider's own API host, for [`BaseUrl::Provider`].
fn provider_host(pkg: Pkg) -> &'static str {
    match pkg {
        Pkg::Openai => "api.openai.com",
        Pkg::GoogleGenai | Pkg::GoogleGenerativeai => "generativelanguage.googleapis.com",
        _ => "api.anthropic.com",
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
    // The SDKs build `baseURL || "https://api.openai.com/v1"`.
    if url.is_empty() || url_host(url) == provider_host(pkg) {
        BaseUrl::Provider
    } else {
        BaseUrl::Custom
    }
}

/// Where an untracked value came from, for gap details.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    /// No binding in this file.
    Unbound,
    /// `undefined`, `null` or a literal: ignored next to a real value.
    Placeholder,
    /// The result of calling or constructing something that is not a
    /// tracked constructor: the callee (by node id, rendered only when a
    /// gap names it).
    Factory {
        callee: usize,
    },
    /// A parameter with no tracked annotation and no in-file caller that
    /// passes a known client.
    Parameter {
        name: String,
    },
    /// A name imported from a module that is not a tracked SDK.
    Imported {
        module: String,
    },
    /// `this.x` with no assignment in the class.
    UnassignedAttribute {
        name: String,
    },
    /// The value returned by an SDK call that is not a known constructor.
    SdkResult {
        pkg: Option<Pkg>,
    },
    /// A dynamic feature of the file may rebind or replace the value.
    Dynamic(Feature),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Val {
    /// A tracked package's module object: an ESM namespace, the value of
    /// `require(...)`, `await import(...)` or a default import of a package
    /// with no default export. `entry` is false for a module of types or
    /// helpers (`openai/resources`).
    Module {
        pkg: Pkg,
        entry: bool,
    },
    /// A client constructor.
    Class(ClientClass),
    /// openai v3's `Configuration` class.
    V3ConfigClass,
    /// A `new Configuration(...)` (openai v3): where its `basePath` routes.
    V3Config {
        base_url: BaseUrl,
        splat: bool,
        kwargs: Vec<String>,
    },
    /// Another SDK name, plus the member path after its binding (`toFile`,
    /// `APIError`, `OpenAIRealtimeWS`).
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
    fn is_tracked(&self) -> bool {
        !matches!(self, Val::Untracked(_) | Val::Result(_))
    }

    fn pkg(&self) -> Option<Pkg> {
        match self {
            Val::Module { pkg, .. } | Val::Symbol { pkg, .. } => Some(*pkg),
            Val::Class(class) => Some(class.pkg),
            Val::V3ConfigClass | Val::V3Config { .. } => Some(Pkg::Openai),
            Val::Client { client, .. } => Some(client.pkg),
            Val::Result(pkg) => *pkg,
            Val::Untracked(_) => None,
        }
    }

    /// The member path after the tracked root.
    fn path(&self) -> &[String] {
        match self {
            Val::Symbol { path, .. } | Val::Client { path, .. } => path,
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
    /// parameter, the entries of an object read with a run-time key, the
    /// assignments of a name, the items a loop runs over, the sides of a
    /// conditional. Listed with the gap for triage, never as the call's
    /// value.
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

    /// Adds the values one source says it may take, and whether that source
    /// listed all of its values.
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
            "f_string" => "built with a template literal",
            "function_call" => "returned by a function call",
            "attribute" => "a property of another object",
            "subscript" => "looked up with a key known only at run time",
            "conditional" => "chosen at run time",
            "parameter" => "a parameter whose default is not a resolvable string",
            "reassigned" => "assigned more than once",
            "imported_name" => "imported from another module",
            "unbound" => "not defined in this file",
            "splat" => "possibly passed through a spread or an options object built elsewhere",
            "parameter_overridden" => "a parameter that in-file callers set to other values",
            "unseen_callers" => {
                "a parameter of a function that is passed, stored or exported in a way this file does not show, so callers may set it"
            }
            "credential" => {
                "credential-shaped (a secret-like name or value), so it is never captured"
            }
            "parameter_default" => {
                "a parameter of a function other modules may call with another value"
            }
            "dynamic_feature" => "something this file may change at run time",
            "subclass_override" => {
                "a property read through `this` in a class another module may extend, which a subclass may override"
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
    /// [`ValueOrigin::EnvDefault`] fallback, in-file callers that pass
    /// their own value for a parameter default, or a subclass that may
    /// override a property.
    caveats: Vec<Unresolved>,
}

impl Resolved {
    /// Whether the value is the only one its source gives: no caveat, and
    /// not a parameter default.
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

    /// Relabels a literal reached through a constant with the constant's
    /// origin.
    fn constant(mut self, name: &str, origin: ValueOrigin) -> Self {
        if self.origin == ValueOrigin::Literal {
            self.origin = origin;
        }
        self.through(name, origin)
    }
}

/// Adds `resolved` to `values` unless a candidate with that value is already
/// listed. Returns false when the list is full and the candidate was left
/// out.
fn push_candidate(values: &mut Vec<Resolved>, resolved: Resolved) -> bool {
    if values.iter().any(|v| v.value == resolved.value) {
        return true;
    }
    if values.len() >= MAX_CANDIDATES {
        return false;
    }
    values.push(resolved);
    true
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

/// The value's origin for a constant bound in a scope of `kind`.
fn constant_origin(kind: ScopeKind) -> ValueOrigin {
    match kind {
        ScopeKind::Module => ValueOrigin::ModuleConstant,
        ScopeKind::Function | ScopeKind::Block => ValueOrigin::LocalConstant,
    }
}

fn captured(key: &str, resolved: Resolved, positional: bool) -> CapturedValue {
    let family = model_family(&resolved.value);
    CapturedValue {
        key: key.to_string(),
        value: resolved.value,
        origin: resolved.origin,
        name: resolved.name,
        positional,
        span: resolved.span,
        model_family: family,
    }
}

/// Whether `span` lies inside `node`.
fn within(node: Node<'_>, span: Span) -> bool {
    span.start_byte as usize >= node.start_byte() && span.end_byte as usize <= node.end_byte()
}

// ============================================================================
// Collected program structure
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Module,
    Function,
    Block,
}

/// What `this` is in a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum This {
    /// Not a class: module level, a plain function, an object-literal
    /// method.
    Other,
    /// An instance of the class: a method, the constructor, an instance
    /// field initializer, and the arrow functions inside them.
    Instance(usize),
    /// The class itself: a static method, a static field initializer or a
    /// static block.
    Static(usize),
}

struct Scope {
    kind: ScopeKind,
    parent: Option<usize>,
    /// The nearest function or module scope: `var` declarations go there.
    var_scope: usize,
    this: This,
    /// The graph's name for the code here (an index into `symbols`).
    symbol: Option<usize>,
    depth: u32,
    /// An arrow function's scope: it has no `this` or `arguments` of its own.
    arrow: bool,
    /// The class method (an index into `funcs`) whose `this` the code here
    /// sees: its own scope, and the blocks and arrow functions inside it.
    method: Option<usize>,
}

/// How a name is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decl {
    Const,
    Let,
    Var,
    Param,
    Function,
    Class,
    Enum,
    Import,
    Catch,
    Namespace,
}

/// One step into a destructured value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Step<'t> {
    Key(&'t str),
    Index(usize),
    /// A rest element or a computed key: the value is not one property.
    Other,
}

/// A write of a name after its declaration.
#[derive(Debug, Clone, Copy)]
struct Write<'t> {
    /// The assigned expression; `None` when the write is not one expression
    /// (`+=`, `++`, a destructuring or loop target, a function declaration
    /// of the name).
    value: Option<Node<'t>>,
    scope: usize,
}

/// An import binding.
#[derive(Debug, Clone)]
struct ImportDef {
    /// The tracked package, and whether the specifier is one of its entry
    /// points; `None` for any other module.
    pkg: Option<(Pkg, bool)>,
    /// The specifier as written, clipped.
    module: String,
    kind: ImportKind,
    /// `import type`: no value exists at run time.
    type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImportKind {
    /// `import X from`, `import { default as X } from`.
    Default,
    /// `import { name as X } from`.
    Named(String),
    /// `import * as X from`, `import X = require(...)`.
    Namespace,
}

/// A path read off a binding, and the call (with its scope) the value read
/// was passed to, if it was an argument.
type HandedOut<'t> = (Rc<[Step<'t>]>, Option<(Node<'t>, usize)>);

/// An enum's members by name, and the value each is given.
type EnumMembers<'t> = Rc<HashMap<&'t str, Option<Node<'t>>>>;

/// An object literal, the scope it is written in, and the name it was
/// reached through with the origin a value read from it has.
type ObjectRef<'t> = (Node<'t>, usize, Option<(String, ValueOrigin)>);

/// What uses of a name do.
#[derive(Debug, Default, Clone)]
struct Uses<'t> {
    /// Direct calls (`f(...)`), with the scope each is in. At most
    /// [`MAX_CALLERS_READ`] + 1 are kept.
    calls: Vec<(Node<'t>, usize)>,
    /// `new X(...)`, the same way.
    news: Vec<(Node<'t>, usize)>,
    /// Uses that let the value reach code this file does not analyze:
    /// stored, returned, captured by another expression...
    escapes: bool,
    /// Calls the value is passed to as an argument. Passing an object to an
    /// SDK request or a read-only builtin (`JSON.stringify`) hands it to
    /// nothing that writes it; anything else is an escape. At most
    /// [`MAX_WRITES_READ`] are kept; `passed_cut` is set past that.
    passed_to: Vec<(Node<'t>, usize)>,
    passed_cut: bool,
    /// A property is written or deleted (`x.k = v`, `delete x.k`,
    /// `Object.assign(x, ...)`).
    mutated: bool,
    /// A member is called (`x.k(...)`), which may write into it through
    /// `this`.
    member_calls: bool,
    /// Paths read in a way that may hand out what they hold (`f(x.nested)`):
    /// a nested container there escapes, unless the read is an argument of
    /// a call (kept with it) that writes nothing. At most
    /// [`MAX_WRITES_READ`] are kept; `handed_out_cut` is set past that.
    handed_out: Vec<HandedOut<'t>>,
    handed_out_cut: bool,
    /// `class Sub extends X`.
    extended: bool,
    exported: bool,
}

struct Binding<'t> {
    name: &'t str,
    scope: usize,
    decl: Decl,
    /// The declaring name.
    node: Node<'t>,
    /// The expression the declaration gives the name: a declarator's
    /// initializer, a parameter's default, the iterable of a loop variable.
    init: Option<Node<'t>>,
    /// The scope `init` is evaluated in.
    init_scope: usize,
    /// How a destructured name is reached from `init` (or from the argument,
    /// for a parameter).
    path: Rc<[Step<'t>]>,
    /// A default written in the destructuring pattern (`{ model = "x" }`).
    fallback: Option<Node<'t>>,
    /// A loop variable: `init` is what the loop runs over.
    each: bool,
    import: Option<Rc<ImportDef>>,
    /// The function a parameter belongs to and its position.
    param: Option<(usize, usize)>,
    /// A parameter's type annotation.
    annotation: Option<Node<'t>>,
    /// The function a declaration (or `const f = () => ...`) names.
    func: Option<usize>,
    /// The class a class declaration names.
    class: Option<usize>,
    writes: Vec<Write<'t>>,
    /// More writes than [`MAX_WRITES_READ`] were seen.
    writes_cut: bool,
    uses: Uses<'t>,
}

/// A use of a name, bound to its declaration after the walk.
struct NameRef<'t> {
    scope: usize,
    name: &'t str,
    kind: RefKind<'t>,
}

#[derive(Clone)]
enum RefKind<'t> {
    /// The name is called: `f(...)`.
    Call(Node<'t>),
    /// The name is constructed: `new C(...)`.
    New(Node<'t>),
    /// The root of a member chain (`x.a.b`, `x["k"]`): the path (a
    /// computed key is [`Step::Other`]) and what the chain does.
    Member {
        path: Rc<[Step<'t>]>,
        does: ChainUse<'t>,
    },
    /// `export { x }`, `export default x`, `module.exports = x`.
    Export,
    /// `class Sub extends X`.
    Extends,
    /// `{ ...x }` in an object or array literal: a copy of what it holds,
    /// and the call the literal is an argument of, if any.
    Spread(Option<Node<'t>>),
    /// `typeof x`, `x instanceof C`, `x === y`: nothing escapes.
    Harmless,
    /// An argument of a call (`f(x)`).
    Arg(Node<'t>),
    /// A plain assignment of the name (or one that is not one expression).
    Write(Write<'t>),
    /// Anything else: passed, stored, returned...
    Escape,
}

/// What the top of a member chain does.
#[derive(Debug, Clone, Copy)]
enum ChainUse<'t> {
    /// The chain's value is read.
    Read,
    /// The chain's value is an argument of a call (`f(x.a)`).
    Arg(Node<'t>),
    /// The chain's value is spread into an object or array literal, and
    /// the call the literal is an argument of, if any.
    Spread(Option<Node<'t>>),
    /// The chain is called: `x.a.b(...)`.
    Call,
    /// The chain is written (`x.a = v`; `value` is `v` for a plain `=`),
    /// updated, deleted or destructured into.
    Write {
        value: Option<Node<'t>>,
        scope: usize,
    },
}

/// A function, arrow function, method or constructor.
struct Func<'t> {
    /// Its parameters, in order.
    params: Vec<Node<'t>>,
    /// The binding that names it: a declaration, `const f = () => ...`, or
    /// `None` for a method, a callback or an anonymous function.
    binding: Option<usize>,
    /// A function expression's own name, visible only inside it
    /// (`function retry() { ... retry() }`): its calls are calls too.
    inner: Option<usize>,
    method: Option<Method<'t>>,
}

/// A class member function.
#[derive(Debug, Clone, Copy)]
struct Method<'t> {
    class: usize,
    name: &'t str,
    ctor: bool,
    /// Callable only inside its class: `private` or `#name`.
    private: bool,
}

struct Class<'t> {
    name: Option<&'t str>,
    binding: Option<usize>,
    /// The `extends` expression and the scope it is evaluated in.
    base: Option<(Node<'t>, usize)>,
    /// A same-file base class, bound after the walk.
    base_class: Option<usize>,
    subclasses: Vec<usize>,
    /// A decorator on the class or one of its members.
    decorated: Option<Feature>,
    fields: Vec<PropWrite<'t>>,
    static_fields: Vec<PropWrite<'t>>,
    /// `this.x = ...` in the class's instance code, `this.x = ...` in its
    /// static code and `Class.x = ...` by name.
    writes: Vec<PropWrite<'t>>,
    static_writes: Vec<PropWrite<'t>>,
    /// Method, getter and setter names: a property with one of these names
    /// is not a plain value.
    members: HashSet<&'t str>,
    static_members: HashSet<&'t str>,
    /// A write of the class's properties under a name known only at run
    /// time (`this[k] = v`, `Object.assign(this, ...)`), or a computed
    /// member name.
    wild: Option<Feature>,
    static_wild: Option<Feature>,
    /// An instance built without naming the class (`new this()`,
    /// `new this.constructor()`): its constructor's calls are not all seen.
    self_constructed: Option<Feature>,
    ctor: Option<usize>,
    /// A class expression's own name, visible only inside it.
    inner: Option<usize>,
}

/// A write of a class property: a field declaration (`value` is its
/// initializer, `None` for `x;` with none), a `this.x = v`, or a
/// constructor parameter property (`value` is the parameter's name).
#[derive(Debug, Clone, Copy)]
struct PropWrite<'t> {
    name: &'t str,
    value: Option<Node<'t>>,
    /// Declared without an initializer (`model?: string;`).
    bare: bool,
    scope: usize,
}

/// A member write found in the walk, decided once names are bound.
struct MemberWrite<'t> {
    scope: usize,
    root: MemberRoot<'t>,
    path: Rc<[Step<'t>]>,
    value: Option<Node<'t>>,
    line: u32,
}

#[derive(Debug, Clone, Copy)]
enum MemberRoot<'t> {
    Name(&'t str),
    This,
    /// `require("process")`.
    Process,
    /// Any other expression (`getObj().x = v`).
    Other,
}

/// A node waiting to be visited, with the context the walk carries down to
/// it.
#[derive(Debug, Clone, Copy)]
struct Frame<'t> {
    node: Node<'t>,
    scope: usize,
    parent: Option<Node<'t>>,
    field: Option<&'static str>,
    /// The parent's parent, for contexts two levels up (the call whose
    /// arguments a node is in).
    grand: Option<Node<'t>>,
    /// A statement block that is a function's body: it opens no scope of
    /// its own.
    body: bool,
    /// The declaration is exported (`export const`, `export class`).
    exported: bool,
    /// The declaration carries a decorator written before `export`.
    decorated: bool,
    /// The graph's parent name for methods and functions here (a class
    /// name, as an index into `symbols`).
    owner: Option<usize>,
    /// The class whose body this node is a member of.
    class: Option<usize>,
    /// Inside wrappers that keep its value (`(x)`, `x!`, `x as T`, `x
    /// satisfies T`, `<T>x`): where the outermost one's value goes.
    wrapped: Option<Context<'t>>,
}

/// Where a node's value goes: its parent, the field it is in there, and
/// the parent's parent.
#[derive(Debug, Clone, Copy)]
struct Context<'t> {
    parent: Option<Node<'t>>,
    field: Option<&'static str>,
    grand: Option<Node<'t>>,
}

impl<'t> Frame<'t> {
    /// Where this node's value goes, past the wrappers around it.
    fn context(&self) -> Context<'t> {
        self.wrapped.unwrap_or(Context {
            parent: self.parent,
            field: self.field,
            grand: self.grand,
        })
    }
}

struct Ctx<'t> {
    src: &'t str,
    file: String,
    ascii: bool,
    cont_blocks: Vec<u32>,

    scopes: Vec<Scope>,
    bindings: Vec<Binding<'t>>,
    by_name: HashMap<(usize, &'t str), usize>,
    refs: Vec<NameRef<'t>>,
    funcs: Vec<Func<'t>>,
    classes: Vec<Class<'t>>,
    /// The graph's names for code: functions, methods, classes and tests.
    symbols: Vec<String>,
    /// Test blocks, as (symbol, line): a title repeated in the file gets
    /// its line, as the graph names it.
    test_symbols: Vec<(usize, u32)>,
    /// Declarator values (by node id) that the graph names as functions
    /// (`const ask = async () => ...`), with that name.
    arrow_symbols: HashMap<usize, usize>,
    /// Declarator values (by node id) and the binding they initialize.
    declared: HashMap<usize, usize>,
    /// Test callbacks (by node id) and the graph's name for them.
    test_callbacks: HashMap<usize, usize>,
    /// Functions stored in class fields (by node id): they are methods.
    field_methods: HashMap<usize, Method<'t>>,
    /// Where an expression's value is stored (by node id): a declared name,
    /// `this.x` or a static property, and how it is held.
    targets: HashMap<usize, (String, ClientScope)>,
    /// Calls and `new` expressions in source order, with their scope.
    calls: Vec<(Node<'t>, usize)>,
    /// Member chains used as values, for SDK methods passed or stored.
    member_values: Vec<(Node<'t>, usize)>,
    /// Member writes, decided once names are bound.
    member_writes: Vec<MemberWrite<'t>>,
    /// Calls by member name (`x.ask(...)`), for private methods' callers.
    method_calls: HashMap<&'t str, Vec<(Node<'t>, usize)>>,
    /// Member names read without being called (`x.ask` passed or stored,
    /// or a key of an object pattern: `const { ask } = bot`).
    method_refs: HashSet<&'t str>,
    /// A method may be called under a name known only at run time
    /// (`this[kind](...)`, as Python's `getattr(self, kind)(...)`): no
    /// method's calls are all seen.
    method_calls_wild: bool,
    /// A method may be read under a name known only at run time and run
    /// with another `this` (`this[m].call(other)`, `const { [m]: f } =
    /// this`): as if every method name were in `method_refs`.
    method_refs_wild: bool,
    /// SDK imports found in the walk: the binding, written specifier, the
    /// imported name and its span, recorded once names are bound.
    pending_imports: Vec<(usize, String, Option<String>, Node<'t>)>,

    /// Properties written on an object other than a class's own `this`, by
    /// name: a class property of that name is not read through `this`.
    outside_writes: HashMap<&'t str, Feature>,
    /// A property written on such an object under a name known only at
    /// run time (`Object.assign(bot, overrides)`).
    outside_wild: Option<Feature>,
    /// Scopes whose function may write `arguments`, which aliases the
    /// parameters of a sloppy-mode function with a simple parameter list.
    arguments_scopes: Vec<usize>,
    /// A `.constructor` read as a value outside its own class: any class
    /// may be constructed without being named.
    any_constructed: Option<Feature>,
    /// Namespace bodies, by scope, and the binding that names the
    /// namespace (`None` for a dotted name, `namespace A.B`).
    namespace_of: HashMap<usize, Option<usize>>,
    /// Scopes of object-literal methods, and the literal (by node id) that
    /// is their `this`.
    object_methods: HashMap<usize, usize>,
    /// Object literals (by node id) whose own methods write `this`.
    this_written_objects: HashSet<usize>,
    /// Object and array literals (by node id) that are, or are nested in,
    /// a call's arguments, and the call.
    literal_calls: HashMap<usize, Node<'t>>,
    /// Calls (by node id) that are a statement of their own: their result
    /// is dropped.
    dropped_calls: HashSet<usize>,
    /// Paths already handed out, by binding, path and call (by node id).
    handed_seen: HashSet<(usize, Rc<[Step<'t>]>, Option<usize>)>,
    /// Object literals and member chains read inside one another: past
    /// [`MAX_NAME_DEPTH`] a spread is opaque and a chain is not an object.
    object_depth: u32,
    /// Reads cut short by that bound: a result computed while this grew
    /// is not memoized.
    object_cuts: u32,
    /// [`Ctx::readonly_object`], by binding.
    readonly_memo: HashMap<usize, Option<(Node<'t>, usize)>>,
    /// [`Ctx::holds_any_container`], by object literal (node id).
    container_memo: HashMap<usize, bool>,
    /// [`Ctx::enum_members`], by binding.
    enum_memo: HashMap<usize, EnumMembers<'t>>,
    /// Environment variables written by a literal name.
    env_writes: HashSet<String>,
    /// The environment written under a name known only at run time.
    env_wild: Option<Feature>,
    /// A feature that may rebind anything in the file (`eval`, `with`,
    /// scopes nested too deep).
    file_dynamic: Option<Feature>,
    /// Packages whose module the file mocks or whose members it replaces.
    sdk_patched: HashMap<Pkg, Feature>,
    /// Constructions (by node id) whose client is given a `baseURL` after it
    /// is built.
    rerouted: HashSet<usize>,
    /// A `baseURL` written on an object the file cannot identify.
    rerouted_any: Option<Feature>,
    /// A `fetch` written on an object the file cannot identify, which may be
    /// the global one every SDK request goes through (`globalThis.fetch =
    /// ...`, `fetch = ...`): a direct client may send its requests
    /// elsewhere.
    global_fetch: bool,
    /// `setDefaultBaseUrls(...)` from `@google/genai`.
    genai_base_urls: Option<Feature>,
    /// A legacy Gemini request, or an `@google/genai` one, passes options that
    /// may name another endpoint: the models and chats its model was given
    /// to may be routed elsewhere.
    legacy_request_endpoint: Option<Feature>,
    genai_request_endpoint: Option<Feature>,
    sdk_imported: HashSet<Sdk>,
    import_bindings: usize,
    import_cut: Option<Node<'t>>,
    scope_cuts: Vec<Node<'t>>,

    // Memoized evaluations.
    val_memo: HashMap<usize, Val>,
    value_memo: HashMap<usize, Result<Resolved, Unresolved>>,
    prop_val_memo: HashMap<(usize, bool, &'t str), Val>,
    visiting: HashSet<(u8, usize)>,
    name_depth: u32,
    /// Resolutions cut off by [`MAX_NAME_DEPTH`] or [`MAX_DEPTH`]: a result
    /// computed while this grew is not memoized.
    cutoffs: u32,
    /// Constructions already recorded (by node id): the index in
    /// `out.clients` and the client.
    constructions: HashMap<usize, Client>,
    nested_left: usize,
    candidates_left: usize,
    /// Callees of untracked calls, by node id, for gap details.
    factory_nodes: HashMap<usize, Node<'t>>,
    /// Object literals read (by node id), and those being read.
    object_memo: HashMap<usize, Rc<ObjectRead<'t>>>,
    reading_objects: HashSet<usize>,
    /// Child buffer reused by the walk.
    kids: Vec<(Node<'t>, Option<&'static str>)>,

    out: FileUsages,
}

// ============================================================================
// Pass 1: collect
// ============================================================================

impl<'t> Ctx<'t> {
    fn new(src: &'t str, file: &str, language: &'static str) -> Self {
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
            scopes: vec![Scope {
                kind: ScopeKind::Module,
                parent: None,
                var_scope: 0,
                this: This::Other,
                symbol: None,
                depth: 0,
                arrow: false,
                method: None,
            }],
            bindings: Vec::new(),
            by_name: HashMap::new(),
            refs: Vec::new(),
            funcs: Vec::new(),
            classes: Vec::new(),
            symbols: Vec::new(),
            test_symbols: Vec::new(),
            arrow_symbols: HashMap::new(),
            declared: HashMap::new(),
            test_callbacks: HashMap::new(),
            field_methods: HashMap::new(),
            targets: HashMap::new(),
            calls: Vec::new(),
            member_values: Vec::new(),
            member_writes: Vec::new(),
            method_calls: HashMap::new(),
            method_refs: HashSet::new(),
            method_calls_wild: false,
            method_refs_wild: false,
            pending_imports: Vec::new(),
            outside_writes: HashMap::new(),
            outside_wild: None,
            arguments_scopes: Vec::new(),
            any_constructed: None,
            namespace_of: HashMap::new(),
            object_methods: HashMap::new(),
            this_written_objects: HashSet::new(),
            literal_calls: HashMap::new(),
            dropped_calls: HashSet::new(),
            handed_seen: HashSet::new(),
            object_depth: 0,
            object_cuts: 0,
            readonly_memo: HashMap::new(),
            container_memo: HashMap::new(),
            enum_memo: HashMap::new(),
            env_writes: HashSet::new(),
            env_wild: None,
            file_dynamic: None,
            sdk_patched: HashMap::new(),
            rerouted: HashSet::new(),
            rerouted_any: None,
            global_fetch: false,
            genai_base_urls: None,
            legacy_request_endpoint: None,
            genai_request_endpoint: None,
            sdk_imported: HashSet::new(),
            import_bindings: 0,
            import_cut: None,
            scope_cuts: Vec::new(),
            val_memo: HashMap::new(),
            value_memo: HashMap::new(),
            prop_val_memo: HashMap::new(),
            visiting: HashSet::new(),
            name_depth: 0,
            cutoffs: 0,
            constructions: HashMap::new(),
            nested_left: MAX_FILE_NESTED_VALUES,
            candidates_left: MAX_FILE_CANDIDATES,
            factory_nodes: HashMap::new(),
            object_memo: HashMap::new(),
            reading_objects: HashSet::new(),
            kids: Vec::new(),
            out: FileUsages::new(file, language),
        }
    }

    fn text(&self, node: Node<'t>) -> &'t str {
        self.src.get(node.byte_range()).unwrap_or("")
    }

    fn line(&self, node: Node<'t>) -> u32 {
        node.start_position().row as u32 + 1
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

    /// The graph's name for the code in `scope`.
    fn symbol(&self, scope: usize) -> Option<String> {
        self.scopes[scope].symbol.map(|s| self.symbols[s].clone())
    }

    fn new_symbol(&mut self, name: String) -> usize {
        self.symbols.push(clip_owned(name));
        self.symbols.len() - 1
    }

    /// `Owner.name` inside a class (the graph qualifies methods, and
    /// functions declared in them, with the class name), else `name`.
    fn qualified(&mut self, owner: Option<usize>, name: &str) -> usize {
        let full = match owner {
            Some(owner) => format!("{}.{}", self.symbols[owner], name),
            None => name.to_string(),
        };
        self.new_symbol(full)
    }

    /// A new scope under `parent`, or `None` past [`MAX_SCOPE_DEPTH`].
    fn new_scope(
        &mut self,
        kind: ScopeKind,
        parent: usize,
        this: This,
        symbol: Option<usize>,
    ) -> Option<usize> {
        let depth = self.scopes[parent].depth + 1;
        if depth > MAX_SCOPE_DEPTH {
            return None;
        }
        let id = self.scopes.len();
        let (var_scope, method) = match kind {
            ScopeKind::Block => (self.scopes[parent].var_scope, self.scopes[parent].method),
            ScopeKind::Function | ScopeKind::Module => (id, None),
        };
        self.scopes.push(Scope {
            kind,
            parent: Some(parent),
            var_scope,
            this,
            symbol,
            depth,
            arrow: false,
            method,
        });
        Some(id)
    }

    /// A block scope under `parent`, inheriting its `this` and symbol.
    fn block_scope(&mut self, parent: usize) -> Option<usize> {
        let (this, symbol) = (self.scopes[parent].this, self.scopes[parent].symbol);
        self.new_scope(ScopeKind::Block, parent, this, symbol)
    }

    /// A scope nested past [`MAX_SCOPE_DEPTH`]: not analyzed, a `truncated`
    /// gap, and a dynamic feature of the whole file.
    fn cut(&mut self, node: Node<'t>) {
        self.scope_cuts.push(node);
        self.note_file_dynamic("scopes nested more than 64 deep", node);
    }

    fn note_file_dynamic(&mut self, what: &'static str, node: Node<'t>) {
        let line = self.line(node);
        self.file_dynamic.get_or_insert(Feature { what, line });
    }

    fn collect(&mut self, root: Node<'t>) {
        let mut stack = vec![Frame {
            node: root,
            scope: 0,
            parent: None,
            field: None,
            grand: None,
            body: true,
            exported: false,
            decorated: false,
            owner: None,
            class: None,
            wrapped: None,
        }];
        while let Some(frame) = stack.pop() {
            self.visit(frame, &mut stack);
        }
        // A test title repeated in the file gets its line, as the graph
        // names it (`distinguish_repeated_tests`).
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (symbol, _) in &self.test_symbols {
            *seen.entry(self.symbols[*symbol].clone()).or_default() += 1;
        }
        for (symbol, line) in self.test_symbols.clone() {
            if seen
                .get(&self.symbols[symbol])
                .is_some_and(|count| *count > 1)
            {
                let named = format!("{} (line {line})", self.symbols[symbol]);
                self.symbols[symbol] = clip_owned(named);
            }
        }
    }

    /// A child frame of `f` for `node`.
    fn child(
        &self,
        f: &Frame<'t>,
        node: Node<'t>,
        scope: usize,
        field: Option<&'static str>,
    ) -> Frame<'t> {
        Frame {
            node,
            scope,
            parent: Some(f.node),
            field,
            grand: f.parent,
            body: false,
            exported: false,
            decorated: false,
            owner: f.owner,
            class: None,
            // One step per frame: a wrapper's own context, already past
            // the wrappers around it.
            wrapped: keeps_value(f.node.kind()).then(|| f.context()),
        }
    }

    /// Pushes the named children of `f`'s node, in `scope`.
    fn push_children(&mut self, stack: &mut Vec<Frame<'t>>, f: &Frame<'t>, scope: usize) {
        let mut kids = std::mem::take(&mut self.kids);
        kids.clear();
        let mut cursor = f.node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.is_named() {
                    kids.push((child, cursor.field_name()));
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        for (child, field) in kids.drain(..).rev() {
            stack.push(self.child(f, child, scope, field));
        }
        self.kids = kids;
    }

    /// Records the object and array literals a call's arguments are, and
    /// those nested in them, so that a spread in one knows the call it is
    /// handed to.
    fn mark_call_literals(&mut self, call: Node<'t>) {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return;
        };
        let mut work: Vec<(Node<'t>, usize)> = Vec::new();
        let mut cursor = arguments.walk();
        for arg in arguments.named_children(&mut cursor) {
            work.push((unwrap_expression(arg), 0));
        }
        while let Some((node, depth)) = work.pop() {
            if depth > MAX_NESTED_DEPTH * 2 || !matches!(node.kind(), "object" | "array") {
                continue;
            }
            self.literal_calls.insert(node.id(), call);
            let mut cursor = node.walk();
            for entry in node.named_children(&mut cursor) {
                let value = match entry.kind() {
                    "pair" => entry.child_by_field_name("value"),
                    "object"
                    | "array"
                    | "parenthesized_expression"
                    | "as_expression"
                    | "satisfies_expression" => Some(entry),
                    _ => None,
                };
                if let Some(value) = value {
                    work.push((unwrap_expression(value), depth + 1));
                }
            }
        }
    }

    fn add_ref(&mut self, scope: usize, node: Node<'t>, kind: RefKind<'t>) {
        let name = self.text(node);
        self.refs.push(NameRef { scope, name, kind });
    }

    fn visit(&mut self, f: Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        match node.kind() {
            // Leaves and types hold no values.
            "comment"
            | "hash_bang_line"
            | "string"
            | "number"
            | "regex"
            | "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "super"
            | "import"
            | "meta_property"
            | "property_identifier"
            | "private_property_identifier"
            | "type_identifier"
            | "statement_identifier"
            | "jsx_text"
            | "type_annotation"
            | "type_arguments"
            | "type_parameters"
            | "interface_declaration"
            | "type_alias_declaration"
            | "implements_clause"
            | "ambient_declaration"
            | "abstract_method_signature"
            | "method_signature"
            | "index_signature"
            | "property_signature"
            | "function_signature"
            | "accessibility_modifier"
            | "override_modifier"
            | "import_alias" => {}
            "import_statement" => self.import_statement(node, f.scope),
            "export_statement" => self.export_statement(&f, stack),
            "lexical_declaration" | "variable_declaration" => self.declaration(&f, stack),
            "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function" => self.function(&f, stack),
            "method_definition" => self.method(&f, stack),
            "class_declaration" | "abstract_class_declaration" | "class" => self.class(&f, stack),
            "statement_block" if f.body => self.push_children(stack, &f, f.scope),
            "statement_block" | "for_statement" | "switch_body" => {
                match self.block_scope(f.scope) {
                    Some(scope) => self.push_children(stack, &f, scope),
                    None => self.cut(node),
                }
            }
            "for_in_statement" => self.for_in(&f, stack),
            "catch_clause" => self.catch_clause(&f, stack),
            "internal_module" | "module" => self.namespace(&f, stack),
            "enum_declaration" => self.enumeration(&f, stack),
            "with_statement" => {
                self.note_file_dynamic("`with`", node);
                self.push_children(stack, &f, f.scope);
            }
            "assignment_expression" => self.assignment(&f, stack),
            "augmented_assignment_expression" | "update_expression" => self.update(&f, stack),
            "unary_expression" if is_delete(node) => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    stack.push(self.child(&f, argument, f.scope, Some("argument")));
                }
            }
            "call_expression" => {
                self.calls.push((node, f.scope));
                self.test_block(node);
                self.mark_call_literals(node);
                // `(f)(...)` calls `f` itself, and `(eval)(...)` is a direct
                // eval.
                let callee = node
                    .child_by_field_name("function")
                    .filter(|c| c.kind() == "parenthesized_expression")
                    .map(unwrap_parens)
                    .filter(|c| c.kind() == "identifier");
                match callee {
                    Some(callee) => {
                        self.add_ref(f.scope, callee, RefKind::Call(node));
                        let mut cursor = node.walk();
                        let rest: Vec<Node<'t>> = node
                            .named_children(&mut cursor)
                            .filter(|c| node.child_by_field_name("function") != Some(*c))
                            .collect();
                        for child in rest.into_iter().rev() {
                            stack.push(self.child(&f, child, f.scope, None));
                        }
                    }
                    None => self.push_children(stack, &f, f.scope),
                }
            }
            "new_expression" => {
                self.calls.push((node, f.scope));
                self.note_self_construction(node, f.scope);
                self.mark_call_literals(node);
                self.push_children(stack, &f, f.scope);
            }
            // A call whose result is dropped hands nothing back.
            "expression_statement" => {
                if let Some(expr) = node.named_child(0).map(unwrap_parens) {
                    if expr.kind() == "call_expression" {
                        self.dropped_calls.insert(expr.id());
                    }
                }
                self.push_children(stack, &f, f.scope);
            }
            "member_expression" | "subscript_expression" => self.chain(&f, stack),
            "identifier" => self.name_use(&f),
            "shorthand_property_identifier" => self.add_ref(f.scope, node, RefKind::Escape),
            // Only the expression of `x as T`, `x satisfies T`, `<T>x`.
            "as_expression" | "satisfies_expression" => {
                if let Some(expr) = node.named_child(0) {
                    stack.push(self.child(&f, expr, f.scope, None));
                }
            }
            "type_assertion" => {
                let count = node.named_child_count();
                if let Some(expr) = count.checked_sub(1).and_then(|i| node.named_child(i)) {
                    stack.push(self.child(&f, expr, f.scope, None));
                }
            }
            _ => self.push_children(stack, &f, f.scope),
        }
    }

    // ------------------------------------------------------------------
    // Names
    // ------------------------------------------------------------------

    /// Declares `name_node` in `scope`. A second declaration of a name in
    /// one scope (`var x` twice, a function and a `var`) is the same
    /// binding written again.
    #[allow(clippy::too_many_arguments)]
    fn declare(
        &mut self,
        scope: usize,
        name_node: Node<'t>,
        decl: Decl,
        init: Option<Node<'t>>,
        init_scope: usize,
        path: Rc<[Step<'t>]>,
        fallback: Option<Node<'t>>,
        exported: bool,
    ) -> usize {
        let name = self.text(name_node);
        if let Some(&existing) = self.by_name.get(&(scope, name)) {
            let binding = &mut self.bindings[existing];
            binding.uses.exported |= exported;
            let value = match decl {
                Decl::Function | Decl::Class | Decl::Enum | Decl::Namespace => Some(None),
                // `var x;` again leaves the value as it was.
                _ if init.is_none() && path.is_empty() && fallback.is_none() => None,
                _ if path.is_empty() && fallback.is_none() => Some(init),
                _ => Some(None),
            };
            if let Some(value) = value {
                push_write(
                    binding,
                    Write {
                        value,
                        scope: init_scope,
                    },
                );
            }
            return existing;
        }
        let id = self.bindings.len();
        self.bindings.push(Binding {
            name,
            scope,
            decl,
            node: name_node,
            init,
            init_scope,
            path,
            fallback,
            each: false,
            import: None,
            param: None,
            annotation: None,
            func: None,
            class: None,
            writes: Vec::new(),
            writes_cut: false,
            uses: Uses {
                exported,
                ..Uses::default()
            },
        });
        self.by_name.insert((scope, name), id);
        id
    }

    /// Declares every name a binding pattern binds (`a`, `{ a, b: [c] }`,
    /// `[x = 1, ...rest]`). Defaults written in the pattern are walked as
    /// expressions. Returns the bindings declared.
    #[allow(clippy::too_many_arguments)]
    fn declare_pattern(
        &mut self,
        pattern: Node<'t>,
        scope: usize,
        decl: Decl,
        init: Option<Node<'t>>,
        init_scope: usize,
        exported: bool,
        f: &Frame<'t>,
        stack: &mut Vec<Frame<'t>>,
    ) -> Vec<usize> {
        let mut declared = Vec::new();
        // (pattern, path, fallback)
        let mut work: Vec<(Node<'t>, Vec<Step<'t>>, Option<Node<'t>>)> =
            vec![(pattern, Vec::new(), None)];
        while let Some((node, path, fallback)) = work.pop() {
            match node.kind() {
                "identifier" => declared.push(self.declare(
                    scope,
                    node,
                    decl,
                    init,
                    init_scope,
                    path.into(),
                    fallback,
                    exported,
                )),
                "shorthand_property_identifier_pattern" => {
                    self.pattern_key(node);
                    let mut path = path;
                    path.push(Step::Key(self.text(node)));
                    declared.push(self.declare(
                        scope,
                        node,
                        decl,
                        init,
                        init_scope,
                        path.into(),
                        fallback,
                        exported,
                    ));
                }
                "object_pattern" => {
                    let mut cursor = node.walk();
                    let entries: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                    for child in entries {
                        match child.kind() {
                            "pair_pattern" => {
                                let key = child.child_by_field_name("key");
                                if let Some(key) = key {
                                    self.pattern_key(key);
                                }
                                let step = key.map_or(Step::Other, |k| self.key_step(k));
                                if let Some(key) =
                                    key.filter(|k| k.kind() == "computed_property_name")
                                {
                                    stack.push(self.child(f, key, init_scope, None));
                                }
                                if let Some(value) = child.child_by_field_name("value") {
                                    let mut path = path.clone();
                                    path.push(step);
                                    work.push((value, path, None));
                                }
                            }
                            "shorthand_property_identifier_pattern" => {
                                work.push((child, path.clone(), None));
                            }
                            "object_assignment_pattern" => {
                                let right = child.child_by_field_name("right");
                                if let Some(right) = right {
                                    stack.push(self.child(f, right, init_scope, Some("right")));
                                }
                                if let Some(left) = child.child_by_field_name("left") {
                                    work.push((left, path.clone(), right));
                                }
                            }
                            "rest_pattern" => {
                                if let Some(inner) = child.named_child(0) {
                                    let mut path = path.clone();
                                    path.push(Step::Other);
                                    work.push((inner, path, None));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "array_pattern" => {
                    let mut index = 0;
                    let mut cursor = node.walk();
                    let children: Vec<Node<'t>> = node.children(&mut cursor).collect();
                    for child in children {
                        if child.kind() == "," {
                            index += 1;
                            continue;
                        }
                        if !child.is_named() || child.kind() == "comment" {
                            continue;
                        }
                        let mut path = path.clone();
                        path.push(if child.kind() == "rest_pattern" {
                            Step::Other
                        } else {
                            Step::Index(index)
                        });
                        work.push((child, path, None));
                    }
                }
                "assignment_pattern" => {
                    let right = node.child_by_field_name("right");
                    if let Some(right) = right {
                        stack.push(self.child(f, right, init_scope, Some("right")));
                    }
                    if let Some(left) = node.child_by_field_name("left") {
                        work.push((left, path, right));
                    }
                }
                "rest_pattern" => {
                    if let Some(inner) = node.named_child(0) {
                        work.push((inner, path, None));
                    }
                }
                _ => {}
            }
        }
        declared
    }

    /// The step a destructuring key takes.
    fn key_step(&self, key: Node<'t>) -> Step<'t> {
        match key.kind() {
            "property_identifier" | "number" => Step::Key(self.text(key)),
            "string" => string_content(key, self.src).map_or(Step::Other, Step::Key),
            _ => Step::Other,
        }
    }

    /// A key of an object pattern reads that member without calling it
    /// (`const { ask } = this` is `this.ask` read): the method may then be
    /// called from anywhere, with any `this`. A computed key that is not a
    /// literal may read any member.
    fn pattern_key(&mut self, key: Node<'t>) {
        let name = match key.kind() {
            "shorthand_property_identifier_pattern" => Some(self.text(key)),
            "computed_property_name" => key.named_child(0).and_then(|k| static_key(k, self.src)),
            _ => match self.key_step(key) {
                Step::Key(name) => Some(name),
                _ => None,
            },
        };
        match name {
            Some(name) => {
                self.method_refs.insert(name);
            }
            None => self.method_refs_wild = true,
        }
    }

    /// Records the writes of an assignment pattern's targets
    /// (`[a, b] = ...`, `({ x: this.y } = ...)`).
    fn pattern_targets(&mut self, pattern: Node<'t>, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let mut work = vec![pattern];
        while let Some(node) = work.pop() {
            match node.kind() {
                "identifier" | "shorthand_property_identifier_pattern" => {
                    if node.kind() == "shorthand_property_identifier_pattern" {
                        self.pattern_key(node);
                    }
                    self.add_ref(
                        f.scope,
                        node,
                        RefKind::Write(Write {
                            value: None,
                            scope: f.scope,
                        }),
                    )
                }
                "member_expression" | "subscript_expression" | "non_null_expression" => {
                    let mut target = self.child(f, node, f.scope, None);
                    target.parent = Some(pattern);
                    stack.push(target);
                }
                "object_pattern" | "array_pattern" | "rest_pattern" => {
                    let mut cursor = node.walk();
                    let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                    work.extend(children);
                }
                "pair_pattern" => {
                    if let Some(key) = node.child_by_field_name("key") {
                        self.pattern_key(key);
                        if key.kind() == "computed_property_name" {
                            stack.push(self.child(f, key, f.scope, None));
                        }
                    }
                    if let Some(value) = node.child_by_field_name("value") {
                        work.push(value);
                    }
                }
                "assignment_pattern" | "object_assignment_pattern" => {
                    if let Some(right) = node.child_by_field_name("right") {
                        stack.push(self.child(f, right, f.scope, Some("right")));
                    }
                    if let Some(left) = node.child_by_field_name("left") {
                        work.push(left);
                    }
                }
                _ => {}
            }
        }
    }

    /// Records where the value of `expr` is stored, for each expression its
    /// value may come from (`client ?? new OpenAI()`, `await make()`).
    fn set_target(&mut self, expr: Node<'t>, name: String, held: ClientScope) {
        let name = clip_owned(name);
        let mut work = vec![(expr, 0)];
        while let Some((node, depth)) = work.pop() {
            if depth > 16 {
                continue;
            }
            self.targets.insert(node.id(), (name.clone(), held));
            match node.kind() {
                "parenthesized_expression"
                | "await_expression"
                | "non_null_expression"
                | "as_expression"
                | "satisfies_expression" => {
                    if let Some(inner) = node.named_child(0) {
                        work.push((inner, depth + 1));
                    }
                }
                "binary_expression" if matches!(operator(node), Some("??" | "||")) => {
                    for field in ["left", "right"] {
                        if let Some(side) = node.child_by_field_name(field) {
                            work.push((side, depth + 1));
                        }
                    }
                }
                "ternary_expression" => {
                    for field in ["consequence", "alternative"] {
                        if let Some(side) = node.child_by_field_name(field) {
                            work.push((side, depth + 1));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// An identifier used as a value.
    fn name_use(&mut self, f: &Frame<'t>) {
        let node = f.node;
        let parent = f.parent;
        let grand = f.grand.map(|g| g.kind());
        let kind = match (parent.map(|p| p.kind()), f.field) {
            (Some("call_expression"), Some("function")) => RefKind::Call(parent.unwrap_or(node)),
            (Some("new_expression"), Some("constructor")) => RefKind::New(parent.unwrap_or(node)),
            (Some("arguments"), _)
                if matches!(grand, Some("call_expression" | "new_expression")) =>
            {
                f.grand.map_or(RefKind::Escape, RefKind::Arg)
            }
            (Some("spread_element"), _) if matches!(grand, Some("object" | "array")) => {
                RefKind::Spread(
                    f.grand
                        .and_then(|g| self.literal_calls.get(&g.id()).copied()),
                )
            }
            (Some("extends_clause"), _) => RefKind::Extends,
            (Some("export_statement"), _) | (Some("export_specifier"), _) => RefKind::Export,
            (Some("unary_expression"), _) => RefKind::Harmless,
            (Some("binary_expression"), _)
                if !matches!(parent.and_then(operator), Some("??" | "||" | "&&")) =>
            {
                RefKind::Harmless
            }
            (Some("ternary_expression"), Some("condition")) => RefKind::Harmless,
            _ => RefKind::Escape,
        };
        self.add_ref(f.scope, node, kind);
    }

    // ------------------------------------------------------------------
    // Imports and exports
    // ------------------------------------------------------------------

    fn import_statement(&mut self, node: Node<'t>, scope: usize) {
        let type_only = has_token(node, "type") || has_token(node, "typeof");
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in children {
            match child.kind() {
                "import_clause" => {
                    let Some(source) = node.child_by_field_name("source") else {
                        continue;
                    };
                    let module = string_content(source, self.src).unwrap_or_default();
                    let mut inner = child.walk();
                    let parts: Vec<Node<'t>> = child.named_children(&mut inner).collect();
                    for part in parts {
                        match part.kind() {
                            "identifier" => self.bind_import(
                                scope,
                                part,
                                module,
                                ImportKind::Default,
                                type_only,
                            ),
                            "namespace_import" => {
                                if let Some(local) = part.named_child(0) {
                                    self.bind_import(
                                        scope,
                                        local,
                                        module,
                                        ImportKind::Namespace,
                                        type_only,
                                    );
                                }
                            }
                            "named_imports" => {
                                let mut specs = part.walk();
                                let specs: Vec<Node<'t>> =
                                    part.named_children(&mut specs).collect();
                                for spec in specs {
                                    if spec.kind() != "import_specifier" {
                                        continue;
                                    }
                                    let Some(name) = spec.child_by_field_name("name") else {
                                        continue;
                                    };
                                    let imported = match name.kind() {
                                        "string" => {
                                            string_content(name, self.src).unwrap_or_default()
                                        }
                                        _ => self.text(name),
                                    };
                                    let local = spec.child_by_field_name("alias").unwrap_or(name);
                                    if local.kind() != "identifier" {
                                        continue;
                                    }
                                    let kind = if imported == "default" {
                                        ImportKind::Default
                                    } else {
                                        ImportKind::Named(imported.to_string())
                                    };
                                    let type_only = type_only
                                        || has_token(spec, "type")
                                        || has_token(spec, "typeof");
                                    self.bind_import(scope, local, module, kind, type_only);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // `import OpenAI = require("openai")`
                "import_require_clause" => {
                    let (Some(local), Some(source)) =
                        (child.named_child(0), child.child_by_field_name("source"))
                    else {
                        continue;
                    };
                    let module = string_content(source, self.src).unwrap_or_default();
                    self.bind_import(scope, local, module, ImportKind::Namespace, type_only);
                }
                _ => {}
            }
        }
    }

    fn bind_import(
        &mut self,
        scope: usize,
        local: Node<'t>,
        module: &'t str,
        kind: ImportKind,
        type_only: bool,
    ) {
        if self.import_cut.is_some() {
            return;
        }
        self.import_bindings += 1;
        if self.import_bindings > MAX_FILE_IMPORTS {
            self.import_cut = Some(local);
            return;
        }
        let pkg = Pkg::of_source(module);
        let def = Rc::new(ImportDef {
            pkg,
            module: clip(module),
            kind: kind.clone(),
            type_only,
        });
        let id = self.declare(
            scope,
            local,
            Decl::Import,
            None,
            scope,
            Rc::from([]),
            None,
            false,
        );
        if self.bindings[id].import.is_some() || self.bindings[id].decl != Decl::Import {
            // A name bound twice: the second binding replaces it.
            push_write(&mut self.bindings[id], Write { value: None, scope });
            return;
        }
        self.bindings[id].import = Some(Rc::clone(&def));
        let Some((pkg, _)) = pkg else {
            return;
        };
        self.sdk_imported.insert(pkg.sdk());
        let name = match &kind {
            ImportKind::Default => Some("default".to_string()),
            ImportKind::Named(name) => Some(clip(name)),
            ImportKind::Namespace => None,
        };
        let binding_kind = match import_val(&def) {
            Val::Module { .. } => BindingKind::Module,
            Val::Class(_) => BindingKind::ClientClass,
            _ => BindingKind::Symbol,
        };
        self.out.imports.push(SdkImport {
            sdk: pkg.sdk(),
            package: pkg.package().to_string(),
            module: clip(module),
            name,
            local: clip(self.text(local)),
            kind: binding_kind,
            file: self.file.clone(),
            span: self.span(local),
        });
    }

    fn export_statement(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let decorated = has_field(node, "decorator");
        let reexport = node.child_by_field_name("source").is_some();
        let mut cursor = node.walk();
        let mut children = Vec::new();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.is_named() {
                    children.push((child, cursor.field_name()));
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        for (child, field) in children.into_iter().rev() {
            match (field, child.kind()) {
                (Some("declaration"), _) => {
                    let mut frame = self.child(f, child, f.scope, field);
                    frame.exported = true;
                    frame.decorated = decorated;
                    stack.push(frame);
                }
                (Some("source"), _) => {}
                (_, "export_clause") if !reexport => {
                    let mut specs = child.walk();
                    let specs: Vec<Node<'t>> = child.named_children(&mut specs).collect();
                    for spec in specs {
                        if let Some(name) = spec.child_by_field_name("name") {
                            if name.kind() == "identifier" {
                                self.add_ref(f.scope, name, RefKind::Export);
                            }
                        }
                    }
                }
                (_, "export_clause" | "namespace_export" | "string") => {}
                // `export default x`, `export = x`, decorators.
                _ => stack.push(self.child(f, child, f.scope, field)),
            }
        }
    }

    /// `module.exports = x`, `module.exports = { a, b: c }`,
    /// `exports.a = x`: the names exported are not only used here.
    fn export_values(&mut self, value: Node<'t>, scope: usize) {
        match value.kind() {
            "identifier" => self.add_ref(scope, value, RefKind::Export),
            "object" => {
                let mut cursor = value.walk();
                let entries: Vec<Node<'t>> = value.named_children(&mut cursor).collect();
                for entry in entries {
                    match entry.kind() {
                        "shorthand_property_identifier" => {
                            self.add_ref(scope, entry, RefKind::Export)
                        }
                        "pair" => {
                            if let Some(v) = entry.child_by_field_name("value") {
                                if v.kind() == "identifier" {
                                    self.add_ref(scope, v, RefKind::Export);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Declarations
    // ------------------------------------------------------------------

    fn declaration(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let decl = match node.kind() {
            "variable_declaration" => Decl::Var,
            _ => match node.child_by_field_name("kind").map(|k| k.kind()) {
                Some("const") => Decl::Const,
                _ => Decl::Let,
            },
        };
        let scope = match decl {
            Decl::Var => self.scopes[f.scope].var_scope,
            _ => f.scope,
        };
        let mut cursor = node.walk();
        let declarators: Vec<Node<'t>> = node
            .named_children(&mut cursor)
            .filter(|n| n.kind() == "variable_declarator")
            .collect();
        // The graph names the first arrow function of a declaration after
        // its declarator, and stops at a declarator with no value
        // (`extract_arrow_function`).
        for declarator in &declarators {
            let (Some(name), Some(value)) = (
                declarator.child_by_field_name("name"),
                declarator.child_by_field_name("value"),
            ) else {
                break;
            };
            if value.kind() == "arrow_function" {
                let symbol = self.new_symbol(self.text(name).to_string());
                self.arrow_symbols.insert(value.id(), symbol);
                break;
            }
        }
        for declarator in declarators.into_iter().rev() {
            let Some(name) = declarator.child_by_field_name("name") else {
                continue;
            };
            let value = declarator.child_by_field_name("value");
            let frame = self.child(f, declarator, f.scope, None);
            let declared =
                self.declare_pattern(name, scope, decl, value, f.scope, f.exported, &frame, stack);
            if let Some(value) = value {
                if name.kind() == "identifier" {
                    if let Some(&binding) = declared.first() {
                        self.declared.insert(value.id(), binding);
                        let held = match self.scopes[scope].kind {
                            ScopeKind::Module => ClientScope::Module,
                            _ => ClientScope::Local,
                        };
                        self.set_target(value, self.text(name).to_string(), held);
                    }
                }
                if let Some((module, member)) = self.required_module(value) {
                    for binding in &declared {
                        let imported = match self.bindings[*binding].path.first() {
                            Some(Step::Key(key)) => Some(key.to_string()),
                            Some(_) => None,
                            None => member.map(str::to_string),
                        };
                        let node = self.bindings[*binding].node;
                        self.pending_imports
                            .push((*binding, module.to_string(), imported, node));
                    }
                }
                stack.push(self.child(&frame, value, f.scope, Some("value")));
            }
        }
    }

    /// `require("x")`, `await import("x")`, `require("x").default`: the
    /// specifier, and the member read off it.
    fn required_module(&self, value: Node<'t>) -> Option<(&'t str, Option<&'t str>)> {
        let mut node = value;
        let mut member = None;
        for _ in 0..8 {
            match node.kind() {
                "parenthesized_expression"
                | "await_expression"
                | "non_null_expression"
                | "as_expression" => node = node.named_child(0)?,
                "member_expression" if member.is_none() => {
                    member = node.child_by_field_name("property").map(|p| self.text(p));
                    node = node.child_by_field_name("object")?;
                }
                "call_expression" => {
                    let function = node.child_by_field_name("function")?;
                    let is_loader = function.kind() == "import"
                        || (function.kind() == "identifier" && self.text(function) == "require");
                    if !is_loader {
                        return None;
                    }
                    let first = first_argument(node)?;
                    return Some((string_content(first, self.src)?, member));
                }
                _ => return None,
            }
        }
        None
    }

    fn function(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let kind = node.kind();
        let name = node.child_by_field_name("name");
        let declaration = matches!(
            kind,
            "function_declaration" | "generator_function_declaration"
        );
        let symbol = if let Some(symbol) = self.test_callbacks.get(&node.id()) {
            Some(*symbol)
        } else if let (true, Some(name)) = (kind == "function_declaration", name) {
            let text = self.text(name);
            Some(self.qualified(f.owner, text))
        } else if let Some(symbol) = self.arrow_symbols.get(&node.id()) {
            Some(*symbol)
        } else {
            self.scopes[f.scope].symbol
        };
        let this = if kind == "arrow_function" {
            self.scopes[f.scope].this
        } else {
            This::Other
        };
        let func = self.funcs.len();
        self.funcs.push(Func {
            params: Vec::new(),
            binding: None,
            inner: None,
            method: self.field_methods.get(&node.id()).copied(),
        });
        if declaration {
            if let Some(name) = name {
                let binding = self.declare(
                    f.scope,
                    name,
                    Decl::Function,
                    None,
                    f.scope,
                    Rc::from([]),
                    None,
                    f.exported,
                );
                self.bindings[binding].func = Some(func);
                self.funcs[func].binding = Some(binding);
            }
        }
        let Some(scope) = self.new_scope(ScopeKind::Function, f.scope, this, symbol) else {
            self.cut(node);
            return;
        };
        if kind == "arrow_function" {
            self.scopes[scope].arrow = true;
            self.scopes[scope].method = self.scopes[f.scope].method;
        }
        // A function value of an object literal: its `this` is the object.
        if kind == "function_expression" && f.parent.is_some_and(|p| p.kind() == "pair") {
            if let Some(object) = f.grand.filter(|o| o.kind() == "object") {
                self.object_methods.insert(scope, object.id());
            }
        }
        if !declaration {
            if let Some(name) = name {
                // A named function expression sees its own name.
                let binding = self.declare(
                    scope,
                    name,
                    Decl::Function,
                    None,
                    scope,
                    Rc::from([]),
                    None,
                    false,
                );
                self.bindings[binding].func = Some(func);
                self.funcs[func].inner = Some(binding);
            }
            if let Some(&binding) = self.declared.get(&node.id()) {
                self.funcs[func].binding = Some(binding);
                self.bindings[binding].func = Some(func);
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut frame = self.child(f, body, scope, Some("body"));
            frame.body = true;
            stack.push(frame);
        }
        self.parameters(node, func, scope, f, stack);
    }

    /// Declares a function's parameters in its scope. Returns the
    /// parameter nodes.
    fn parameters(
        &mut self,
        node: Node<'t>,
        func: usize,
        scope: usize,
        f: &Frame<'t>,
        stack: &mut Vec<Frame<'t>>,
    ) -> Vec<Node<'t>> {
        let mut params = Vec::new();
        if let Some(list) = node.child_by_field_name("parameters") {
            let mut cursor = list.walk();
            for param in list.named_children(&mut cursor) {
                if param.kind() == "comment" {
                    continue;
                }
                // TypeScript's `this` parameter is not passed by position.
                if param
                    .child_by_field_name("pattern")
                    .is_some_and(|p| p.kind() == "this")
                {
                    continue;
                }
                params.push(param);
            }
        } else if let Some(param) = node.child_by_field_name("parameter") {
            params.push(param);
        }
        let frame = self.child(f, node, scope, None);
        for (index, param) in params.iter().enumerate() {
            let (pattern, default, annotation) = match param.kind() {
                "required_parameter" | "optional_parameter" => (
                    param.child_by_field_name("pattern"),
                    param.child_by_field_name("value"),
                    param.child_by_field_name("type"),
                ),
                "assignment_pattern" => (
                    param.child_by_field_name("left"),
                    param.child_by_field_name("right"),
                    None,
                ),
                _ => (Some(*param), None, None),
            };
            if let Some(default) = default {
                stack.push(self.child(&frame, default, scope, Some("value")));
            }
            let Some(pattern) = pattern else {
                continue;
            };
            let declared = self.declare_pattern(
                pattern,
                scope,
                Decl::Param,
                default,
                scope,
                false,
                &frame,
                stack,
            );
            for binding in declared {
                let b = &mut self.bindings[binding];
                if b.decl == Decl::Param && b.param.is_none() {
                    b.param = Some((func, index));
                    b.annotation = annotation;
                }
            }
        }
        self.funcs[func].params = params.clone();
        params
    }

    fn method(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let name_node = node.child_by_field_name("name");
        let name = name_node.and_then(|n| self.member_name(n));
        let is_static = has_token(node, "static");
        let private =
            has_private_modifier(node, self.src) || name.is_some_and(|n| n.starts_with('#'));
        // The graph names a method `Owner.name`, from its name as written.
        let symbol = match name_node {
            Some(n) => {
                let text = self.text(n);
                Some(self.qualified(f.owner, text))
            }
            None => self.scopes[f.scope].symbol,
        };
        let func = self.funcs.len();
        let (this, method) = match f.class {
            Some(class) => {
                let ctor = !is_static && name == Some("constructor");
                match (name, ctor) {
                    (_, true) => self.classes[class].ctor = Some(func),
                    (Some(n), false) if is_static => {
                        self.classes[class].static_members.insert(n);
                    }
                    (Some(n), false) => {
                        self.classes[class].members.insert(n);
                    }
                    (None, false) => {
                        let line = self.line(node);
                        let wild = if is_static {
                            &mut self.classes[class].static_wild
                        } else {
                            &mut self.classes[class].wild
                        };
                        wild.get_or_insert(Feature {
                            what: "a computed member name",
                            line,
                        });
                    }
                }
                let this = if is_static {
                    This::Static(class)
                } else {
                    This::Instance(class)
                };
                (
                    this,
                    Some(Method {
                        class,
                        name: name.unwrap_or(""),
                        ctor,
                        private,
                    }),
                )
            }
            None => (This::Other, None),
        };
        self.funcs.push(Func {
            params: Vec::new(),
            binding: None,
            inner: None,
            method,
        });
        let Some(scope) = self.new_scope(ScopeKind::Function, f.scope, this, symbol) else {
            self.cut(node);
            return;
        };
        if method.is_some() {
            self.scopes[scope].method = Some(func);
        } else if let Some(object) = f.parent.filter(|p| p.kind() == "object") {
            // An object literal's method: its `this` is the object.
            self.object_methods.insert(scope, object.id());
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut frame = self.child(f, body, scope, Some("body"));
            frame.body = true;
            stack.push(frame);
        }
        let params = self.parameters(node, func, scope, f, stack);
        // `constructor(private client: OpenAI, readonly model = "x")`: each
        // parameter property is a write of `this.<name>` with the
        // parameter's value.
        if let Some(Method {
            class, ctor: true, ..
        }) = method
        {
            for param in params {
                let mut cursor = param.walk();
                let property = param
                    .named_children(&mut cursor)
                    .any(|c| c.kind() == "accessibility_modifier")
                    || has_token(param, "readonly");
                let pattern = param.child_by_field_name("pattern");
                if let (true, Some(pattern)) = (property, pattern) {
                    if pattern.kind() == "identifier" {
                        let name = self.text(pattern);
                        self.classes[class].writes.push(PropWrite {
                            name,
                            value: Some(pattern),
                            bare: false,
                            scope,
                        });
                    }
                }
            }
        }
    }

    /// A member's name: an identifier, `#private`, or a string key.
    /// `None` for a computed name.
    fn member_name(&self, node: Node<'t>) -> Option<&'t str> {
        match node.kind() {
            "property_identifier" | "private_property_identifier" | "number" => {
                Some(self.text(node))
            }
            "string" => string_content(node, self.src),
            _ => None,
        }
    }

    fn class(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let name = node.child_by_field_name("name");
        let id = self.classes.len();
        let decorated = (f.decorated || has_field(node, "decorator")).then(|| Feature {
            what: "a class decorator",
            line: self.line(node),
        });
        self.classes.push(Class {
            name: name.map(|n| self.text(n)),
            binding: None,
            base: None,
            base_class: None,
            subclasses: Vec::new(),
            decorated,
            fields: Vec::new(),
            static_fields: Vec::new(),
            writes: Vec::new(),
            static_writes: Vec::new(),
            members: HashSet::new(),
            static_members: HashSet::new(),
            wild: None,
            static_wild: None,
            self_constructed: None,
            ctor: None,
            inner: None,
        });
        let mut inner = f.scope;
        if node.kind() == "class" {
            // A class expression's own name is visible inside it only.
            if let Some(name) = name {
                let Some(scope) = self.block_scope(f.scope) else {
                    self.cut(node);
                    return;
                };
                inner = scope;
                let binding = self.declare(
                    scope,
                    name,
                    Decl::Class,
                    None,
                    scope,
                    Rc::from([]),
                    None,
                    false,
                );
                self.bindings[binding].class = Some(id);
                self.classes[id].inner = Some(binding);
            }
            if let Some(&binding) = self.declared.get(&node.id()) {
                self.bindings[binding].class = Some(id);
                self.classes[id].binding = Some(binding);
            }
        } else if let Some(name) = name {
            let binding = self.declare(
                f.scope,
                name,
                Decl::Class,
                None,
                f.scope,
                Rc::from([]),
                None,
                f.exported,
            );
            self.bindings[binding].class = Some(id);
            self.classes[id].binding = Some(binding);
        }
        // The graph names a named class, and qualifies its methods with
        // that name.
        let (symbol, owner) = match name {
            Some(name) => {
                let symbol = self.new_symbol(self.text(name).to_string());
                (Some(symbol), Some(symbol))
            }
            None => (self.scopes[f.scope].symbol, f.owner),
        };
        if let Some(body) = node.child_by_field_name("body") {
            match self.new_scope(ScopeKind::Block, inner, This::Other, symbol) {
                Some(class_scope) => {
                    let body_frame = Frame {
                        owner,
                        ..self.child(f, body, class_scope, Some("body"))
                    };
                    self.class_body(body, id, class_scope, symbol, &body_frame, stack);
                }
                None => self.cut(node),
            }
        }
        // Decorators and `extends` run where the class is written.
        let mut cursor = node.walk();
        let mut children = Vec::new();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.is_named() {
                    children.push((child, cursor.field_name()));
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        for (child, field) in children {
            match child.kind() {
                "decorator" => stack.push(self.child(f, child, f.scope, field)),
                "class_heritage" => {
                    let heritage = self.child(f, child, f.scope, None);
                    let mut clauses = child.walk();
                    let clauses: Vec<Node<'t>> = child.named_children(&mut clauses).collect();
                    for clause in clauses {
                        if clause.kind() != "extends_clause" {
                            continue;
                        }
                        let base = clause
                            .child_by_field_name("value")
                            .or_else(|| clause.named_child(0));
                        if let Some(base) = base {
                            self.classes[id].base = Some((base, f.scope));
                            let clause_frame = self.child(&heritage, clause, f.scope, None);
                            stack.push(self.child(&clause_frame, base, f.scope, Some("value")));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn class_body(
        &mut self,
        body: Node<'t>,
        class: usize,
        class_scope: usize,
        symbol: Option<usize>,
        body_frame: &Frame<'t>,
        stack: &mut Vec<Frame<'t>>,
    ) {
        let mut cursor = body.walk();
        let members: Vec<Node<'t>> = body.named_children(&mut cursor).collect();
        for member in members.into_iter().rev() {
            match member.kind() {
                "decorator" => {
                    let line = self.line(member);
                    self.classes[class].decorated.get_or_insert(Feature {
                        what: "a member decorator",
                        line,
                    });
                    stack.push(self.child(body_frame, member, class_scope, None));
                }
                "method_definition" => {
                    let mut frame = self.child(body_frame, member, class_scope, None);
                    frame.class = Some(class);
                    stack.push(frame);
                }
                "public_field_definition" => {
                    self.field(member, class, class_scope, body_frame, stack)
                }
                "class_static_block" => {
                    let Some(scope) = self.new_scope(
                        ScopeKind::Function,
                        class_scope,
                        This::Static(class),
                        symbol,
                    ) else {
                        self.cut(member);
                        continue;
                    };
                    if let Some(block) = member.child_by_field_name("body") {
                        let mut frame = self.child(body_frame, block, scope, Some("body"));
                        frame.body = true;
                        stack.push(frame);
                    }
                }
                _ => {}
            }
        }
    }

    fn field(
        &mut self,
        member: Node<'t>,
        class: usize,
        class_scope: usize,
        body: &Frame<'t>,
        stack: &mut Vec<Frame<'t>>,
    ) {
        let line = self.line(member);
        if has_field(member, "decorator") {
            self.classes[class].decorated.get_or_insert(Feature {
                what: "a field decorator",
                line,
            });
        }
        // `declare model: string;` only types a property: it emits nothing.
        if has_token(member, "declare") {
            return;
        }
        let is_static = has_token(member, "static");
        let this = if is_static {
            This::Static(class)
        } else {
            This::Instance(class)
        };
        let symbol = self.scopes[class_scope].symbol;
        let Some(scope) = self.new_scope(ScopeKind::Function, class_scope, this, symbol) else {
            self.cut(member);
            return;
        };
        let value = member.child_by_field_name("value");
        if let Some(value) = value {
            stack.push(self.child(body, value, scope, Some("value")));
        }
        let Some(name) = member
            .child_by_field_name("name")
            .and_then(|n| self.member_name(n))
        else {
            let wild = if is_static {
                &mut self.classes[class].static_wild
            } else {
                &mut self.classes[class].wild
            };
            wild.get_or_insert(Feature {
                what: "a computed member name",
                line,
            });
            return;
        };
        // `accessor x = ...` defines a getter and a setter.
        if has_token(member, "accessor") {
            if is_static {
                self.classes[class].static_members.insert(name);
            } else {
                self.classes[class].members.insert(name);
            }
        }
        let write = PropWrite {
            name,
            value,
            bare: value.is_none(),
            scope,
        };
        if is_static {
            self.classes[class].static_fields.push(write);
        } else {
            self.classes[class].fields.push(write);
        }
        if let Some(value) = value {
            if matches!(value.kind(), "arrow_function" | "function_expression") {
                let private = has_private_modifier(member, self.src) || name.starts_with('#');
                self.field_methods.insert(
                    value.id(),
                    Method {
                        class,
                        name,
                        ctor: false,
                        private,
                    },
                );
            }
            let (target, held) = if is_static {
                let owner = self.classes[class].name.unwrap_or("<class>");
                (format!("{owner}.{name}"), ClientScope::Class)
            } else {
                (format!("this.{name}"), ClientScope::Attribute)
            };
            self.set_target(value, target, held);
        }
    }

    fn for_in(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let Some(scope) = self.block_scope(f.scope) else {
            self.cut(node);
            return;
        };
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        let of = node
            .child_by_field_name("operator")
            .is_some_and(|op| op.kind() == "of");
        if let Some(body) = node.child_by_field_name("body") {
            stack.push(self.child(f, body, scope, Some("body")));
        }
        let frame = self.child(f, node, scope, None);
        match (node.child_by_field_name("kind").map(|k| k.kind()), left) {
            (Some(kind), Some(left)) => {
                let decl = match kind {
                    "const" => Decl::Const,
                    "let" => Decl::Let,
                    _ => Decl::Var,
                };
                let target = match decl {
                    Decl::Var => self.scopes[scope].var_scope,
                    _ => scope,
                };
                // `for (const m of MODELS)` takes each item of `MODELS`; a
                // `for ... in` loop takes its keys, never a model.
                let init = if of { right } else { None };
                let declared =
                    self.declare_pattern(left, target, decl, init, f.scope, false, &frame, stack);
                for binding in declared {
                    self.bindings[binding].each = true;
                }
            }
            (None, Some(left)) => match left.kind() {
                "identifier" => {
                    self.add_ref(scope, left, RefKind::Write(Write { value: None, scope }))
                }
                "object_pattern" | "array_pattern" => self.pattern_targets(left, &frame, stack),
                _ => stack.push(self.child(f, left, scope, Some("left"))),
            },
            _ => {}
        }
        if let Some(right) = right {
            stack.push(self.child(f, right, f.scope, Some("right")));
        }
    }

    fn catch_clause(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let Some(scope) = self.block_scope(f.scope) else {
            self.cut(node);
            return;
        };
        if let Some(body) = node.child_by_field_name("body") {
            let mut frame = self.child(f, body, scope, Some("body"));
            frame.body = true;
            stack.push(frame);
        }
        if let Some(param) = node.child_by_field_name("parameter") {
            let frame = self.child(f, node, scope, None);
            self.declare_pattern(param, scope, Decl::Catch, None, scope, false, &frame, stack);
        }
    }

    /// `namespace NS { ... }`: its body is a scope of its own.
    fn namespace(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let mut binding = None;
        if let Some(name) = node.child_by_field_name("name") {
            if name.kind() == "identifier" {
                binding = Some(self.declare(
                    f.scope,
                    name,
                    Decl::Namespace,
                    None,
                    f.scope,
                    Rc::from([]),
                    None,
                    f.exported,
                ));
            }
        }
        let symbol = self.scopes[f.scope].symbol;
        let Some(scope) = self.new_scope(ScopeKind::Function, f.scope, This::Other, symbol) else {
            self.cut(node);
            return;
        };
        self.namespace_of.insert(scope, binding);
        if let Some(body) = node.child_by_field_name("body") {
            let mut frame = self.child(f, body, scope, Some("body"));
            frame.body = true;
            stack.push(frame);
        }
    }

    fn enumeration(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let body = node.child_by_field_name("body");
        if let Some(name) = node.child_by_field_name("name") {
            self.declare(
                f.scope,
                name,
                Decl::Enum,
                body,
                f.scope,
                Rc::from([]),
                None,
                f.exported,
            );
        }
        if let Some(body) = body {
            let mut cursor = body.walk();
            let members: Vec<Node<'t>> = body.named_children(&mut cursor).collect();
            for member in members {
                if let Some(value) = member.child_by_field_name("value") {
                    stack.push(self.child(f, value, f.scope, Some("value")));
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Writes and member chains
    // ------------------------------------------------------------------

    fn assignment(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        if let Some(right) = right {
            stack.push(self.child(f, right, f.scope, Some("right")));
        }
        let Some(left) = left else {
            return;
        };
        match left.kind() {
            "identifier" => {
                self.add_ref(
                    f.scope,
                    left,
                    RefKind::Write(Write {
                        value: right,
                        scope: f.scope,
                    }),
                );
                if let Some(right) = right {
                    let held = match self.scopes[f.scope].kind {
                        ScopeKind::Module => ClientScope::Module,
                        _ => ClientScope::Local,
                    };
                    self.set_target(right, self.text(left).to_string(), held);
                }
            }
            "member_expression" | "subscript_expression" | "non_null_expression" => {
                if let Some(right) = right {
                    let rendered = self.render(left);
                    if rendered == "module.exports"
                        || rendered.starts_with("module.exports.")
                        || rendered.starts_with("exports.")
                    {
                        self.export_values(right, f.scope);
                    }
                    let held = if self.chain_root_is_class(left, f.scope) {
                        ClientScope::Class
                    } else {
                        ClientScope::Attribute
                    };
                    self.set_target(right, rendered, held);
                }
                stack.push(self.child(f, left, f.scope, Some("left")));
            }
            "object_pattern" | "array_pattern" => self.pattern_targets(left, f, stack),
            _ => stack.push(self.child(f, left, f.scope, Some("left"))),
        }
    }

    /// Whether a member chain is rooted at a class declared in an enclosing
    /// scope, as far as the walk has seen.
    fn chain_root_is_class(&self, mut node: Node<'t>, scope: usize) -> bool {
        loop {
            let next = match node.kind() {
                "member_expression" | "subscript_expression" => node.child_by_field_name("object"),
                "non_null_expression" => node.named_child(0),
                _ => break,
            };
            match next {
                Some(next) => node = next,
                None => return false,
            }
        }
        node.kind() == "identifier"
            && self
                .lookup(scope, self.text(node))
                .is_some_and(|b| self.bindings[b].class.is_some())
    }

    fn update(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let node = f.node;
        let field = if node.kind() == "update_expression" {
            "argument"
        } else {
            "left"
        };
        if let Some(right) = node.child_by_field_name("right") {
            stack.push(self.child(f, right, f.scope, Some("right")));
        }
        let Some(target) = node.child_by_field_name(field) else {
            return;
        };
        match target.kind() {
            "identifier" => self.add_ref(
                f.scope,
                target,
                RefKind::Write(Write {
                    value: None,
                    scope: f.scope,
                }),
            ),
            _ => stack.push(self.child(f, target, f.scope, Some(field))),
        }
    }

    /// What the top of a member chain at `f` does.
    fn chain_use(&self, f: &Frame<'t>) -> ChainUse<'t> {
        // `(x.a as any)(...)`, `x.a! = v`: a wrapper keeps the chain's value,
        // so its use is the wrapper's.
        let context = f.context();
        let Some(parent) = context.parent else {
            return ChainUse::Read;
        };
        let write = |value| ChainUse::Write {
            value,
            scope: f.scope,
        };
        let grand = context.grand.map(|g| g.kind());
        match (parent.kind(), context.field) {
            ("assignment_expression", Some("left")) => write(parent.child_by_field_name("right")),
            ("augmented_assignment_expression", Some("left"))
            | ("update_expression", _)
            | ("for_in_statement", Some("left")) => write(None),
            ("unary_expression", _) if is_delete(parent) => write(None),
            ("call_expression", Some("function")) => ChainUse::Call,
            ("arguments", _) if matches!(grand, Some("call_expression" | "new_expression")) => {
                context.grand.map_or(ChainUse::Read, ChainUse::Arg)
            }
            ("spread_element", _) if matches!(grand, Some("object" | "array")) => ChainUse::Spread(
                context
                    .grand
                    .and_then(|g| self.literal_calls.get(&g.id()).copied()),
            ),
            ("assignment_pattern" | "object_assignment_pattern", Some("left"))
            | ("object_pattern" | "array_pattern" | "rest_pattern", _)
            | ("pair_pattern", Some("value")) => write(None),
            _ => ChainUse::Read,
        }
    }

    /// The top of a member chain (`a.b.c`, `a["k"].d`, `this.x`): records
    /// what its root does with the static path, and walks the computed keys
    /// and a root that is not a name.
    fn chain(&mut self, f: &Frame<'t>, stack: &mut Vec<Frame<'t>>) {
        let top = f.node;
        let does = self.chain_use(f);
        let mut links: Vec<Step<'t>> = Vec::new();
        let mut current = top;
        let mut innermost = top;
        // The last member or subscript link: past any `!` below it, the next
        // node down is its object.
        let mut object_of = top;
        loop {
            match current.kind() {
                "member_expression" => {
                    links.push(
                        current
                            .child_by_field_name("property")
                            .map_or(Step::Other, |p| Step::Key(self.text(p))),
                    );
                }
                "subscript_expression" => {
                    let index = current.child_by_field_name("index");
                    match index.and_then(|i| static_key(i, self.src)) {
                        Some(key) => links.push(Step::Key(key)),
                        None => {
                            links.push(Step::Other);
                            if let Some(index) = index {
                                let link = self.child(f, current, f.scope, None);
                                stack.push(self.child(&link, index, f.scope, Some("index")));
                            }
                        }
                    }
                }
                "non_null_expression" => {}
                _ => break,
            }
            if current.kind() != "non_null_expression" {
                object_of = current;
            }
            innermost = current;
            let next = match current.kind() {
                "non_null_expression" => current.named_child(0),
                _ => current.child_by_field_name("object"),
            };
            match next {
                Some(next) => current = next,
                None => break,
            }
        }
        links.reverse();
        let path: Rc<[Step<'t>]> = links.into();
        let root = current;
        let line = self.line(top);
        // `this.constructor` or `x.constructor` used as a value may build an
        // instance without naming its class.
        let context = f.context();
        if path.last() == Some(&Step::Key("constructor"))
            && !matches!(does, ChainUse::Write { .. })
            && context.parent.map(|p| p.kind()) != Some("binary_expression")
        {
            let feature = Feature {
                what: "an instance built through its `constructor` property",
                line,
            };
            match (root.kind(), self.scopes[f.scope].this, path.len()) {
                ("this", This::Instance(class), 1) => {
                    self.classes[class].self_constructed.get_or_insert(feature);
                }
                // A static method's `this.constructor` is `Function`.
                ("this", This::Static(_), 1) => {}
                _ => {
                    self.any_constructed.get_or_insert(feature);
                }
            }
        }
        match root.kind() {
            "identifier" => self.add_ref(
                f.scope,
                root,
                RefKind::Member {
                    path: Rc::clone(&path),
                    does,
                },
            ),
            "this" | "meta_property" => {}
            _ if root.id() != top.id() => {
                let link = self.child(f, innermost, f.scope, None);
                let mut object = self.child(&link, root, f.scope, Some("object"));
                // `(this[m])!.call(...)`: the root is the object of the link
                // above the `!`.
                if innermost.kind() == "non_null_expression" {
                    object.wrapped = Some(Context {
                        parent: Some(object_of),
                        field: Some("object"),
                        grand: None,
                    });
                }
                stack.push(object);
            }
            _ => {}
        }
        // `x.ask.call(other)`, `.apply(...)`, `.bind(...)`: `ask` runs with
        // another `this`. Under a key known only at run time
        // (`this[m].call(other)`, `(this[m] as any).call(other)`), any
        // method may; called directly (`this[kind](...)`), any method may
        // be called.
        let rebound = f.wrapped.is_some()
            && context.field == Some("object")
            && context.parent.is_some_and(|p| self.rebinds(p));
        match path.as_ref() {
            [.., Step::Key(method), Step::Key("call" | "apply" | "bind")] => {
                self.method_refs.insert(method);
            }
            [.., Step::Other, Step::Key("call" | "apply" | "bind")] => {
                self.method_refs_wild = true;
            }
            [.., Step::Other] if rebound => self.method_refs_wild = true,
            [.., Step::Other] if matches!(does, ChainUse::Call) => {
                self.method_calls_wild = true;
            }
            _ => {}
        }
        if let Some(Step::Key(last)) = path.last() {
            match does {
                ChainUse::Call => {
                    if let Some(call) = context.parent {
                        let sites = self.method_calls.entry(last).or_default();
                        if sites.len() <= MAX_CALLERS_READ {
                            sites.push((call, f.scope));
                        }
                    }
                }
                ChainUse::Read | ChainUse::Arg(_) | ChainUse::Spread(_) => {
                    self.method_refs.insert(last);
                }
                ChainUse::Write { .. } => {}
            }
        }
        let rooted = matches!(root.kind(), "identifier" | "this");
        if rooted && !path.is_empty() && matches!(does, ChainUse::Read | ChainUse::Arg(_)) {
            self.member_values.push((top, f.scope));
        }
        if let ChainUse::Write { value, scope } = does {
            let root = match root.kind() {
                "identifier" => MemberRoot::Name(self.text(root)),
                "this" => MemberRoot::This,
                "call_expression" if self.process_object(root, f.scope) => MemberRoot::Process,
                _ => MemberRoot::Other,
            };
            self.member_writes.push(MemberWrite {
                scope,
                root,
                path,
                value,
                line,
            });
        }
    }

    /// Whether `member` reads `call`, `apply` or `bind` off its object.
    fn rebinds(&self, member: Node<'t>) -> bool {
        let key = match member.kind() {
            "member_expression" => member.child_by_field_name("property").map(|p| self.text(p)),
            "subscript_expression" => member
                .child_by_field_name("index")
                .and_then(|i| static_key(i, self.src)),
            _ => None,
        };
        matches!(key, Some("call" | "apply" | "bind"))
    }

    /// `new this()` or `new this.constructor()`: an instance built without
    /// naming its class.
    fn note_self_construction(&mut self, node: Node<'t>, scope: usize) {
        let Some(constructor) = node.child_by_field_name("constructor") else {
            return;
        };
        let class = match (constructor.kind(), self.scopes[scope].this) {
            ("this", This::Static(class)) => class,
            ("member_expression", This::Instance(class))
                if self.text(constructor) == "this.constructor" =>
            {
                class
            }
            _ => return,
        };
        let line = self.line(node);
        self.classes[class].self_constructed.get_or_insert(Feature {
            what: "an instance built without naming its class",
            line,
        });
    }

    /// `it("does x", () => { ... })`: the graph names the callback
    /// `it: does x` (mirrors `extract_test_block`).
    fn test_block(&mut self, call: Node<'t>) {
        let Some(callee) = call.child_by_field_name("function") else {
            return;
        };
        let Some(block) = test_callee(callee, self.src) else {
            return;
        };
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return;
        };
        let mut title = None;
        let mut callback = None;
        let mut cursor = arguments.walk();
        for argument in arguments.named_children(&mut cursor) {
            match argument.kind() {
                "string" | "template_string" if title.is_none() => {
                    let text = self.text(argument);
                    let trimmed = text.trim_matches(|c| matches!(c, '"' | '\'' | '`'));
                    title = Some(trimmed.split_whitespace().collect::<Vec<_>>().join(" "));
                }
                "arrow_function" | "function_expression" => callback = Some(argument),
                _ => {}
            }
        }
        let Some(callback) = callback else {
            return;
        };
        let label = match title.filter(|title| !title.is_empty()) {
            Some(title) => {
                let title: String = title.chars().take(80).collect();
                format!("{block}: {title}")
            }
            None => block.to_string(),
        };
        let symbol = self.new_symbol(label);
        self.test_symbols.push((symbol, self.line(call)));
        self.test_callbacks.insert(callback.id(), symbol);
    }

    fn lookup(&self, mut scope: usize, name: &str) -> Option<usize> {
        loop {
            if let Some(&binding) = self.by_name.get(&(scope, name)) {
                return Some(binding);
            }
            scope = self.scopes[scope].parent?;
        }
    }
}

/// Records a write of `binding`, at most [`MAX_WRITES_READ`] of them.
fn push_write<'t>(binding: &mut Binding<'t>, write: Write<'t>) {
    if binding.writes.len() >= MAX_WRITES_READ {
        binding.writes_cut = true;
    } else {
        binding.writes.push(write);
    }
}

/// The value an import binds, from the tables alone.
fn import_val(def: &ImportDef) -> Val {
    let Some((pkg, entry)) = def.pkg else {
        return Val::Untracked(Origin::Imported {
            module: def.module.clone(),
        });
    };
    let name = match &def.kind {
        ImportKind::Default => "default",
        ImportKind::Named(name) => name.as_str(),
        ImportKind::Namespace => return Val::Module { pkg, entry },
    };
    if def.type_only {
        return Val::Symbol {
            pkg,
            path: vec![clip(name)],
        };
    }
    module_member(pkg, entry, name)
}

/// A named export of a tracked package (`default` for the default export).
fn module_member(pkg: Pkg, entry: bool, name: &str) -> Val {
    if entry {
        if name == "default" {
            return match pkg.default_class() {
                Some(class) => Val::Class(class),
                // No default export: a CommonJS default import is the
                // module object.
                None => Val::Module { pkg, entry },
            };
        }
        if let Some(class) = client_class(pkg, name) {
            return Val::Class(class);
        }
        if pkg == Pkg::Openai && name == V3_CONFIGURATION {
            return Val::V3ConfigClass;
        }
    } else if let Some(class) = client_class(pkg, name) {
        // `import { AzureOpenAI } from "openai/azure"`.
        return Val::Class(class);
    }
    Val::Symbol {
        pkg,
        path: vec![clip(name)],
    }
}

/// Whether `node` has the anonymous token `token` among its children.
fn has_token(node: Node<'_>, token: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| !child.is_named() && child.kind() == token);
    found
}

/// Whether `node` has a child in field `field`.
fn has_field(node: Node<'_>, field: &str) -> bool {
    node.child_by_field_name(field).is_some()
}

/// Expressions whose value is the one inside them: parentheses, and
/// TypeScript's assertions, which compile to nothing.
fn keeps_value(kind: &str) -> bool {
    matches!(
        kind,
        "parenthesized_expression"
            | "non_null_expression"
            | "as_expression"
            | "satisfies_expression"
            | "type_assertion"
    )
}

/// `expr` without the parentheses around it.
fn unwrap_parens(mut node: Node<'_>) -> Node<'_> {
    for _ in 0..16 {
        match node.named_child(0) {
            Some(inner) if node.kind() == "parenthesized_expression" => node = inner,
            _ => break,
        }
    }
    node
}

/// `process` or `node:process`.
fn is_process_module(module: &str) -> bool {
    matches!(module, "process" | "node:process")
}

/// Whether a member has the `private` accessibility modifier.
fn has_private_modifier(node: Node<'_>, src: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).any(|child| {
        child.kind() == "accessibility_modifier" && src.get(child.byte_range()) == Some("private")
    });
    found
}

/// Whether a `unary_expression` is `delete x`.
fn is_delete(node: Node<'_>) -> bool {
    node.child_by_field_name("operator")
        .is_some_and(|op| op.kind() == "delete")
}

/// The operator of a binary expression.
fn operator(node: Node<'_>) -> Option<&'static str> {
    node.child_by_field_name("operator").map(|op| op.kind())
}

/// The text of a string literal with no escape sequence, or of a template
/// literal with no substitution and no escape.
fn string_content<'t>(node: Node<'t>, src: &'t str) -> Option<&'t str> {
    if !matches!(node.kind(), "string" | "template_string") {
        return None;
    }
    let mut cursor = node.walk();
    let mut fragment = None;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "string_fragment" if fragment.is_none() => fragment = Some(child),
            _ => return None,
        }
    }
    match fragment {
        Some(fragment) => src.get(fragment.byte_range()),
        None => Some(""),
    }
}

/// A computed key that is a plain string or number literal (`x["model"]`,
/// `x[0]`).
fn static_key<'t>(index: Node<'t>, src: &'t str) -> Option<&'t str> {
    match index.kind() {
        "string" | "template_string" => string_content(index, src),
        "number" => src.get(index.byte_range()),
        _ => None,
    }
}

/// The first argument of a call, when it is not a spread.
fn first_argument(call: Node<'_>) -> Option<Node<'_>> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let first = arguments
        .named_children(&mut cursor)
        .find(|n| n.kind() != "comment");
    first.filter(|n| n.kind() != "spread_element")
}

/// The test-runner function a callee goes through: `it`, `it.only`,
/// `test.skip`, `test.each(rows)` (mirrors the graph's `test_callee`).
fn test_callee(callee: Node<'_>, src: &str) -> Option<&'static str> {
    let mut current = callee;
    // Runner chains are short (`test.each(rows)`, `it.only.each`): a long
    // chain is not a test, and is not walked to its root.
    for _ in 0..16 {
        match current.kind() {
            "identifier" => {
                let name = src.get(current.byte_range())?;
                return TEST_BLOCKS.iter().find(|block| **block == name).copied();
            }
            "member_expression" => current = current.child_by_field_name("object")?,
            "call_expression" => current = current.child_by_field_name("function")?,
            _ => return None,
        }
    }
    None
}

// ============================================================================
// Binding names
// ============================================================================

impl<'t> Ctx<'t> {
    /// Binds every recorded use of a name to its declaration, and each
    /// class to its same-file base.
    fn bind_names(&mut self) {
        let refs = std::mem::take(&mut self.refs);
        for r in refs {
            let Some(b) = self.lookup(r.scope, r.name) else {
                // Only a direct call of the global `eval` can rebind the
                // names of the file.
                if let RefKind::Call(call) = r.kind {
                    if r.name == "eval" {
                        self.note_file_dynamic("`eval`", call);
                    }
                }
                // `fetch = custom`: the global `fetch` replaced.
                if r.name == "fetch" && matches!(r.kind, RefKind::Write(_)) {
                    self.global_fetch = true;
                }
                // `arguments[0] = x`, or `arguments` passed or stored.
                if r.name == "arguments"
                    && matches!(
                        r.kind,
                        RefKind::Arg(_)
                            | RefKind::Escape
                            | RefKind::Member {
                                does: ChainUse::Write { .. },
                                ..
                            }
                    )
                {
                    self.arguments_scopes.push(r.scope);
                }
                continue;
            };
            match r.kind {
                RefKind::Write(write) => push_write(&mut self.bindings[b], write),
                RefKind::Call(call) => {
                    let calls = &mut self.bindings[b].uses.calls;
                    if calls.len() <= MAX_CALLERS_READ {
                        calls.push((call, r.scope));
                    }
                }
                RefKind::New(call) => {
                    let news = &mut self.bindings[b].uses.news;
                    if news.len() <= MAX_CALLERS_READ {
                        news.push((call, r.scope));
                    }
                }
                RefKind::Arg(call) => {
                    let uses = &mut self.bindings[b].uses;
                    if uses.passed_to.len() < MAX_WRITES_READ {
                        uses.passed_to.push((call, r.scope));
                    } else {
                        uses.passed_cut = true;
                    }
                }
                RefKind::Member { path, does, .. } => {
                    let uses = &mut self.bindings[b].uses;
                    let handed: Option<HandedOut<'t>> = match does {
                        ChainUse::Call => {
                            uses.member_calls = true;
                            None
                        }
                        ChainUse::Write { .. } => {
                            uses.mutated = true;
                            None
                        }
                        // `{ ...CONFIG.openai }` copies the entries, and
                        // shares the containers they hold.
                        ChainUse::Spread(call) => {
                            let mut entries = path.to_vec();
                            entries.push(Step::Other);
                            Some((entries.into(), call.map(|c| (c, r.scope))))
                        }
                        ChainUse::Read => Some((path, None)),
                        ChainUse::Arg(call) => Some((path, Some((call, r.scope)))),
                    };
                    if let Some(handed) = handed {
                        self.hand_out(b, handed);
                    }
                }
                RefKind::Export => self.bindings[b].uses.exported = true,
                RefKind::Extends => self.bindings[b].uses.extended = true,
                // `{ ...CONFIG }` copies the entries, and shares the
                // containers they hold.
                RefKind::Spread(call) => {
                    self.hand_out(b, (Rc::from([Step::Other]), call.map(|c| (c, r.scope))));
                }
                RefKind::Harmless => {}
                RefKind::Escape => self.bindings[b].uses.escapes = true,
            }
        }
        for class in 0..self.classes.len() {
            let Some((base, scope)) = self.classes[class].base else {
                continue;
            };
            let base = unwrap_expression(base);
            // A same-file class is the base. An imported or global one is
            // outside the file; anything else may be any class.
            let known = match base.kind() {
                "identifier" => match self.lookup(scope, self.text(base)) {
                    None => true,
                    Some(binding) => match self.bindings[binding].class {
                        Some(base_class) => {
                            if base_class != class {
                                self.classes[class].base_class = Some(base_class);
                                self.classes[base_class].subclasses.push(class);
                            }
                            true
                        }
                        None => self.bindings[binding].decl == Decl::Import,
                    },
                },
                "member_expression" => {
                    let mut root = base;
                    while let Some(object) = root.child_by_field_name("object") {
                        root = unwrap_expression(object);
                    }
                    root.kind() == "identifier"
                        && self
                            .lookup(scope, self.text(root))
                            .is_none_or(|b| self.bindings[b].decl == Decl::Import)
                }
                _ => false,
            };
            if !known {
                let feature = Feature {
                    what: "a base class Arbor cannot identify",
                    line: self.line(base),
                };
                self.classes[class].wild.get_or_insert(feature);
                self.classes[class].static_wild.get_or_insert(feature);
            }
        }
        self.alias_arguments();
    }

    /// Records a path read off binding `b` that may hand out a container.
    fn hand_out(&mut self, b: usize, handed: HandedOut<'t>) {
        let key = (b, Rc::clone(&handed.0), handed.1.map(|(call, _)| call.id()));
        if !self.handed_seen.insert(key) {
            return;
        }
        let uses = &mut self.bindings[b].uses;
        if uses.handed_out.len() >= MAX_HANDED_OUT {
            uses.handed_out_cut = true;
        } else {
            uses.handed_out.push(handed);
        }
    }

    /// A sloppy-mode function with a simple parameter list whose
    /// `arguments` may be written: each parameter may be rewritten through
    /// it (`arguments[0] = x`).
    fn alias_arguments(&mut self) {
        let mut owners = HashSet::new();
        for &scope in &self.arguments_scopes {
            // `arguments` is the nearest non-arrow function's.
            let mut s = self.scopes[scope].var_scope;
            loop {
                let here = &self.scopes[s];
                if here.kind == ScopeKind::Module {
                    break;
                }
                if !here.arrow {
                    owners.insert(s);
                    break;
                }
                match here.parent {
                    Some(parent) => s = self.scopes[parent].var_scope,
                    None => break,
                }
            }
        }
        if owners.is_empty() {
            return;
        }
        for b in 0..self.bindings.len() {
            let binding = &self.bindings[b];
            let Some((func, _)) = binding.param else {
                continue;
            };
            if !owners.contains(&binding.scope) {
                continue;
            }
            let simple = self.funcs[func].params.iter().all(|p| match p.kind() {
                "identifier" => true,
                "required_parameter" | "optional_parameter" => {
                    p.child_by_field_name("value").is_none()
                        && p.child_by_field_name("pattern")
                            .is_some_and(|q| q.kind() == "identifier")
                }
                _ => false,
            });
            if simple {
                self.bindings[b].writes_cut = true;
            }
        }
    }

    /// The object literal a binding holds, when it is bound once to one
    /// (`const MODELS = { fast: "..." } as const`), with the scope it is
    /// written in.
    fn object_literal_of(&self, b: usize) -> Option<(Node<'t>, usize)> {
        let binding = &self.bindings[b];
        if !matches!(binding.decl, Decl::Const | Decl::Let | Decl::Var)
            || binding.each
            || !binding.path.is_empty()
            || !binding.writes.is_empty()
            || binding.writes_cut
        {
            return None;
        }
        let init = unwrap_expression(binding.init?);
        (init.kind() == "object").then_some((init, binding.init_scope))
    }

    /// Whether a binding holds the process environment (`const env =
    /// process.env`, `const { env } = process`, `import { env } from
    /// "node:process"`).
    fn is_env_object(&self, b: usize) -> bool {
        self.binding_is_env(b, &mut Vec::new())
    }

    /// Whether `node` is the process environment: `process.env`,
    /// `process["env"]`, `import.meta.env`, `Bun.env`,
    /// `Deno.env.toObject()`, the `env` of a name bound to `process`
    /// (`import process from "node:process"`, `globalThis.process`,
    /// `require("process")`), or a name bound to one of them.
    fn env_object(&self, node: Node<'t>, scope: usize) -> bool {
        self.env_object_at(node, scope, &mut Vec::new())
    }

    /// Whether `node` is the `process` object.
    fn process_object(&self, node: Node<'t>, scope: usize) -> bool {
        self.process_object_at(node, scope, &mut Vec::new())
    }

    /// The bindings followed so far are in `seen`: a name bound to itself
    /// through others holds `undefined`, and a longer chain than
    /// [`MAX_NAME_DEPTH`] is taken to be the environment, so that no write
    /// through it is missed.
    fn binding_is_env(&self, b: usize, seen: &mut Vec<usize>) -> bool {
        if seen.contains(&b) {
            return false;
        }
        if seen.len() >= MAX_NAME_DEPTH as usize {
            return true;
        }
        seen.push(b);
        let binding = &self.bindings[b];
        let env = match binding.decl {
            Decl::Import => binding.import.as_ref().is_some_and(|def| {
                is_process_module(&def.module) && def.kind == ImportKind::Named("env".to_string())
            }),
            Decl::Const | Decl::Let | Decl::Var
                if binding.writes.is_empty() && !binding.writes_cut && !binding.each =>
            {
                match (binding.init, binding.path.as_ref()) {
                    (Some(init), []) => self.env_object_at(init, binding.init_scope, seen),
                    (Some(init), [Step::Key("env")]) => {
                        self.process_object_at(init, binding.init_scope, seen)
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        seen.pop();
        env
    }

    fn binding_is_process(&self, b: usize, seen: &mut Vec<usize>) -> bool {
        if seen.contains(&b) {
            return false;
        }
        if seen.len() >= MAX_NAME_DEPTH as usize {
            return true;
        }
        seen.push(b);
        let binding = &self.bindings[b];
        let process = match binding.decl {
            Decl::Import => binding.import.as_ref().is_some_and(|def| {
                is_process_module(&def.module)
                    && matches!(def.kind, ImportKind::Default | ImportKind::Namespace)
            }),
            Decl::Const | Decl::Let | Decl::Var
                if binding.writes.is_empty()
                    && !binding.writes_cut
                    && !binding.each
                    && binding.path.is_empty() =>
            {
                binding
                    .init
                    .is_some_and(|init| self.process_object_at(init, binding.init_scope, seen))
            }
            _ => false,
        };
        seen.pop();
        process
    }

    fn process_object_at(&self, node: Node<'t>, scope: usize, seen: &mut Vec<usize>) -> bool {
        let node = unwrap_expression(node);
        match node.kind() {
            "identifier" => match self.lookup(scope, self.text(node)) {
                None => self.text(node) == "process",
                Some(b) => self.binding_is_process(b, seen),
            },
            // `globalThis.process`, `global.process`, `window.process`
            "member_expression" => {
                let (Some(object), Some(property)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("property"),
                ) else {
                    return false;
                };
                self.text(property) == "process"
                    && object.kind() == "identifier"
                    && matches!(
                        self.text(object),
                        "globalThis" | "global" | "window" | "self"
                    )
                    && self.lookup(scope, self.text(object)).is_none()
            }
            // `require("process")`, `require("node:process")`
            "call_expression" => {
                let required = node.child_by_field_name("function").is_some_and(|f| {
                    f.kind() == "identifier"
                        && self.text(f) == "require"
                        && self.lookup(scope, "require").is_none()
                });
                required
                    && first_argument(node)
                        .and_then(|a| string_content(a, self.src))
                        .is_some_and(is_process_module)
            }
            _ => false,
        }
    }

    fn env_object_at(&self, node: Node<'t>, scope: usize, seen: &mut Vec<usize>) -> bool {
        let node = unwrap_expression(node);
        match node.kind() {
            "member_expression" | "subscript_expression" => {
                let Some(object) = node.child_by_field_name("object") else {
                    return false;
                };
                let key = match node.kind() {
                    "member_expression" => {
                        node.child_by_field_name("property").map(|p| self.text(p))
                    }
                    _ => node
                        .child_by_field_name("index")
                        .and_then(|i| static_key(i, self.src)),
                };
                if key != Some("env") {
                    return false;
                }
                let object = unwrap_expression(object);
                match object.kind() {
                    "identifier" if self.text(object) == "Bun" => {
                        self.lookup(scope, "Bun").is_none()
                    }
                    // `import.meta`
                    "member_expression"
                        if object
                            .child_by_field_name("object")
                            .is_some_and(|o| o.kind() == "import") =>
                    {
                        object
                            .child_by_field_name("property")
                            .is_some_and(|p| self.text(p) == "meta")
                    }
                    _ => self.process_object_at(object, scope, seen),
                }
            }
            // `Deno.env.toObject()`
            "call_expression" => {
                node.child_by_field_name("function")
                    .is_some_and(|f| self.render(f) == "Deno.env.toObject")
                    && self.lookup(scope, "Deno").is_none()
            }
            "identifier" => self
                .lookup(scope, self.text(node))
                .is_some_and(|b| self.binding_is_env(b, seen)),
            _ => false,
        }
    }

    /// Whether `init` destructured along `path` reads an environment
    /// variable: `const { MODEL } = process.env`, `const { env: { MODEL } }
    /// = process`.
    fn env_path(&self, init: Node<'t>, scope: usize, path: &[Step<'t>]) -> bool {
        match path {
            [] => false,
            [Step::Key("env"), _, ..] if self.process_object(init, scope) => true,
            _ => self.env_object(init, scope),
        }
    }

    // ------------------------------------------------------------------
    // Writes the walk could not decide
    // ------------------------------------------------------------------

    /// Member writes that need names bound: environment writes, class
    /// properties written through `this` or by class name, and properties
    /// written on other objects.
    fn settle_member_writes(&mut self) {
        let writes = std::mem::take(&mut self.member_writes);
        for w in &writes {
            let feature = |what| Feature { what, line: w.line };
            let first = w.path.first().copied();
            match w.root {
                MemberRoot::This => {
                    let (class, is_static) = match self.scopes[w.scope].this {
                        This::Instance(class) => (class, false),
                        This::Static(class) => (class, true),
                        This::Other => {
                            if let Some(object) = self.this_object(w.scope) {
                                self.this_written_objects.insert(object);
                            }
                            self.note_property_write(w);
                            continue;
                        }
                    };
                    match first {
                        Some(Step::Key(name)) => {
                            let write = PropWrite {
                                name,
                                value: if w.path.len() == 1 { w.value } else { None },
                                bare: false,
                                scope: w.scope,
                            };
                            if is_static {
                                self.classes[class].static_writes.push(write);
                            } else {
                                self.classes[class].writes.push(write);
                            }
                        }
                        _ => {
                            let wild = if is_static {
                                &mut self.classes[class].static_wild
                            } else {
                                &mut self.classes[class].wild
                            };
                            wild.get_or_insert(feature(
                                "a write of `this` under a name known only at run time",
                            ));
                        }
                    }
                    // `this.config.model = ...` also writes a property of
                    // another object.
                    if w.path.len() > 1 {
                        self.note_property_write(w);
                    }
                }
                MemberRoot::Name(name) => match self.lookup(w.scope, name) {
                    None if matches!(name, "process" | "Bun")
                        && first == Some(Step::Key("env")) =>
                    {
                        self.env_write(w, 1)
                    }
                    // `globalThis.process.env.X = ...`
                    None if matches!(name, "globalThis" | "global" | "window" | "self")
                        && w.path.get(..2) == Some(&[Step::Key("process"), Step::Key("env")]) =>
                    {
                        self.env_write(w, 2)
                    }
                    Some(b) if self.is_env_object(b) => self.env_write(w, 0),
                    Some(b)
                        if first == Some(Step::Key("env"))
                            && self.binding_is_process(b, &mut Vec::new()) =>
                    {
                        self.env_write(w, 1)
                    }
                    Some(b) => {
                        if let (Some(class), 1, Some(Step::Key(prop))) =
                            (self.bindings[b].class, w.path.len(), first)
                        {
                            self.classes[class].static_writes.push(PropWrite {
                                name: prop,
                                value: w.value,
                                bare: false,
                                scope: w.scope,
                            });
                            continue;
                        }
                        // A property of an object literal is not a class
                        // instance's.
                        if w.path.len() == 1 && self.object_literal_of(b).is_some() {
                            continue;
                        }
                        self.note_property_write(w);
                    }
                    None => self.note_property_write(w),
                },
                MemberRoot::Process => {
                    if first == Some(Step::Key("env")) {
                        self.env_write(w, 1);
                    } else {
                        self.note_property_write(w);
                    }
                }
                MemberRoot::Other => self.note_property_write(w),
            }
        }
        self.member_writes = writes;
    }

    /// The object literal (by node id) whose method's `this` the code in
    /// `scope` sees, if any.
    fn this_object(&self, mut scope: usize) -> Option<usize> {
        loop {
            if let Some(object) = self.object_methods.get(&scope) {
                return Some(*object);
            }
            let here = &self.scopes[scope];
            if here.kind == ScopeKind::Function && !here.arrow {
                return None;
            }
            scope = here.parent?;
        }
    }

    /// The property names a reflective write sets (`Object.assign(x, { a })`
    /// sets `a`; `Reflect.set(x, "a", v)` sets `a`), or `None` when they are
    /// known only at run time.
    fn reflective_keys(&self, call: Node<'t>, callee: &str) -> Option<Vec<&'t str>> {
        let arguments = call.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        let args: Vec<Node<'t>> = arguments
            .named_children(&mut cursor)
            .filter(|a| a.kind() != "comment")
            .collect();
        if args.iter().any(|a| a.kind() == "spread_element") {
            return None;
        }
        let object_keys = |object: Node<'t>| -> Option<Vec<&'t str>> {
            let object = unwrap_expression(object);
            if object.kind() != "object" {
                return None;
            }
            let mut keys = Vec::new();
            let mut cursor = object.walk();
            for entry in object.named_children(&mut cursor) {
                let key = match entry.kind() {
                    "pair" => entry
                        .child_by_field_name("key")
                        .and_then(|k| match k.kind() {
                            "computed_property_name" => {
                                k.named_child(0).and_then(|i| static_key(i, self.src))
                            }
                            _ => self.member_name(k),
                        }),
                    "shorthand_property_identifier" => Some(self.text(entry)),
                    "method_definition" => entry
                        .child_by_field_name("name")
                        .and_then(|n| self.member_name(n)),
                    "comment" => continue,
                    _ => None,
                };
                keys.push(key?);
            }
            Some(keys)
        };
        match callee {
            "Object.assign" => {
                let mut keys = Vec::new();
                for source in args.iter().skip(1) {
                    keys.extend(object_keys(*source)?);
                }
                Some(keys)
            }
            "Object.defineProperties" => object_keys(*args.get(1)?),
            "Object.defineProperty"
            | "Reflect.defineProperty"
            | "Reflect.set"
            | "Reflect.deleteProperty" => {
                let key = unwrap_expression(*args.get(1)?);
                static_key(key, self.src).map(|k| vec![k])
            }
            _ => None,
        }
    }

    /// A reflective write of an object that is not a class's own `this`:
    /// no class property it may set is read through `this` with no gap.
    fn note_reflective_write(&mut self, keys: &Option<Vec<&'t str>>, line: u32) {
        let feature = Feature {
            what: "a reflective write of a property of that name on another object",
            line,
        };
        match keys {
            Some(keys) => {
                for key in keys {
                    self.outside_writes.entry(key).or_insert(feature);
                }
            }
            None => {
                self.outside_wild.get_or_insert(Feature {
                    what: "a reflective write of another object under names known only at run time",
                    line,
                });
            }
        }
    }

    /// A write of the environment: `process.env.X = ...` names `X`;
    /// anything else may write any variable.
    fn env_write(&mut self, w: &MemberWrite<'t>, at: usize) {
        match (w.path.get(at), w.path.len() == at + 1) {
            (Some(Step::Key(var)), true) => {
                self.env_writes.insert(var.to_string());
            }
            _ => {
                self.env_wild.get_or_insert(Feature {
                    what: "a write of the environment under a name known only at run time",
                    line: w.line,
                });
            }
        }
    }

    /// A property written on an object that is not a class's own `this`:
    /// no class property of that name is read through `this` with no gap.
    fn note_property_write(&mut self, w: &MemberWrite<'t>) {
        let Some(Step::Key(name)) = w.path.last().copied() else {
            return;
        };
        self.outside_writes.entry(name).or_insert(Feature {
            what: "a write of a property of that name on another object",
            line: w.line,
        });
        if BASE_URL_PROPS.contains(&name) {
            self.rerouted_any.get_or_insert(Feature {
                what: "a `baseURL` written on an object Arbor cannot identify",
                line: w.line,
            });
        }
        // `globalThis.fetch = ...`, `(window as any).fetch = ...`.
        if name == "fetch" {
            self.global_fetch = true;
        }
    }

    /// Calls that write what other lookups read: reflective writes
    /// (`Object.assign(this, ...)`), test mocks of an SDK module, and
    /// `setDefaultBaseUrls`. None of them needs a client evaluated.
    fn settle_special_calls(&mut self) {
        let calls = std::mem::take(&mut self.calls);
        for (call, scope) in &calls {
            if call.kind() != "call_expression" {
                continue;
            }
            let Some(function) = call.child_by_field_name("function") else {
                continue;
            };
            // `setup(process.env)`: the environment handed to code that may
            // write it.
            if self.env_writes_unknown_through(*call, function, *scope) {
                let line = self.line(*call);
                self.env_wild.get_or_insert(Feature {
                    what: "the environment passed to code that may write it",
                    line,
                });
            }
            let Some(callee) = self.global_callee(function, *scope) else {
                if self.is_genai_symbol(function, *scope, "setDefaultBaseUrls") {
                    let line = self.line(*call);
                    self.genai_base_urls.get_or_insert(Feature {
                        what: "`setDefaultBaseUrls`",
                        line,
                    });
                }
                continue;
            };
            let line = self.line(*call);
            if REFLECTIVE_WRITES.contains(&callee.as_str()) {
                let Some(target) = first_argument(*call) else {
                    continue;
                };
                let target = unwrap_expression(target);
                let keys = self.reflective_keys(*call, &callee);
                match target.kind() {
                    "this" => match self.scopes[*scope].this {
                        This::Instance(class) => {
                            self.classes[class].wild.get_or_insert(Feature {
                                what: "a reflective write of `this`",
                                line,
                            });
                        }
                        This::Static(class) => {
                            self.classes[class].static_wild.get_or_insert(Feature {
                                what: "a reflective write of `this`",
                                line,
                            });
                        }
                        This::Other => {
                            if let Some(object) = self.this_object(*scope) {
                                self.this_written_objects.insert(object);
                            }
                            self.note_reflective_write(&keys, line);
                        }
                    },
                    _ if self.env_object(target, *scope) => {
                        self.env_wild.get_or_insert(Feature {
                            what: "a reflective write of the environment",
                            line,
                        });
                    }
                    "identifier" => match self.lookup(*scope, self.text(target)) {
                        Some(b) => {
                            self.bindings[b].uses.mutated = true;
                            if let Some(class) = self.bindings[b].class {
                                self.classes[class].static_wild.get_or_insert(Feature {
                                    what: "a reflective write of the class",
                                    line,
                                });
                            } else if self.object_literal_of(b).is_none() {
                                // It may be a class instance.
                                self.note_reflective_write(&keys, line);
                            }
                        }
                        None => self.note_reflective_write(&keys, line),
                    },
                    "member_expression" => {
                        // `Object.assign(Bot.prototype, ...)`
                        let prototype_of = target
                            .child_by_field_name("object")
                            .filter(|o| o.kind() == "identifier")
                            .filter(|_| {
                                target
                                    .child_by_field_name("property")
                                    .is_some_and(|p| self.text(p) == "prototype")
                            })
                            .and_then(|o| self.lookup(*scope, self.text(o)))
                            .and_then(|b| self.bindings[b].class);
                        match prototype_of {
                            Some(class) => {
                                self.classes[class].wild.get_or_insert(Feature {
                                    what: "a reflective write of the class's prototype",
                                    line,
                                });
                            }
                            None => self.note_reflective_write(&keys, line),
                        }
                    }
                    // A fresh literal (`Object.assign({}, a, b)`) is no
                    // instance.
                    "object" | "array" => {}
                    _ => self.note_reflective_write(&keys, line),
                }
            } else if matches!(callee.as_str(), "Deno.env.set" | "Deno.env.delete") {
                match first_argument(*call).and_then(|a| string_content(a, self.src)) {
                    Some(var) => {
                        self.env_writes.insert(var.to_string());
                    }
                    None => {
                        self.env_wild.get_or_insert(Feature {
                            what: "a write of the environment under a name known only at run time",
                            line,
                        });
                    }
                }
            } else if MODULE_MOCKS.contains(&callee.as_str()) {
                // `vi.mock("openai")`, `vi.mock(import("openai"))`
                let mocked = first_argument(*call)
                    .map(unwrap_expression)
                    .and_then(|a| match a.kind() {
                        "call_expression"
                            if a.child_by_field_name("function")
                                .is_some_and(|f| f.kind() == "import") =>
                        {
                            first_argument(a)
                        }
                        _ => Some(a),
                    })
                    .and_then(|a| string_content(a, self.src))
                    .and_then(Pkg::of_source);
                if let Some((pkg, _)) = mocked {
                    self.sdk_patched.entry(pkg).or_insert(Feature {
                        what: "a test mock of the SDK module",
                        line,
                    });
                }
            }
        }
        self.calls = calls;
    }

    /// Whether a call passes the environment object to code that may write
    /// it: anything but a read-only builtin or a reflective write (which
    /// is decided on its own).
    fn env_writes_unknown_through(&self, call: Node<'t>, function: Node<'t>, scope: usize) -> bool {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return false;
        };
        let mut cursor = arguments.walk();
        let passes_env = arguments.named_children(&mut cursor).any(|a| {
            let a = match a.kind() {
                "spread_element" => a.named_child(0).unwrap_or(a),
                _ => a,
            };
            self.env_object(a, scope)
        });
        if !passes_env {
            return false;
        }
        !self.global_callee(function, scope).is_some_and(|callee| {
            READ_ONLY_CALLS.contains(&callee.as_str())
                || REFLECTIVE_WRITES.contains(&callee.as_str())
        })
    }

    /// Calls that need a value evaluated: a member of an SDK client or
    /// module replaced (`client.chat = ...`), a client given a `baseURL`
    /// after it is built, and a test spy on an SDK object.
    fn settle_sdk_writes(&mut self) {
        // `genai.setDefaultBaseUrls(...)` through `require`, or under
        // another name.
        if self.genai_base_urls.is_none() {
            let calls = std::mem::take(&mut self.calls);
            for (call, scope) in &calls {
                let Some(function) = call
                    .child_by_field_name("function")
                    .filter(|_| call.kind() == "call_expression")
                else {
                    continue;
                };
                let candidate = match function.kind() {
                    "identifier" => self.lookup(*scope, self.text(function)).is_some(),
                    "member_expression" => function
                        .child_by_field_name("property")
                        .is_some_and(|p| self.text(p) == "setDefaultBaseUrls"),
                    _ => false,
                };
                if !candidate {
                    continue;
                }
                if let Val::Symbol {
                    pkg: Pkg::GoogleGenai,
                    path,
                } = self.eval(function, *scope, 0)
                {
                    if path.last().is_some_and(|m| m == "setDefaultBaseUrls") {
                        let line = self.line(*call);
                        self.genai_base_urls.get_or_insert(Feature {
                            what: "`setDefaultBaseUrls`",
                            line,
                        });
                        break;
                    }
                }
            }
            self.calls = calls;
        }
        let writes = std::mem::take(&mut self.member_writes);
        for w in &writes {
            let MemberRoot::Name(name) = w.root else {
                continue;
            };
            let Some(b) = self.lookup(w.scope, name) else {
                continue;
            };
            if self.bindings[b].class.is_some() || self.object_literal_of(b).is_some() {
                continue;
            }
            let val = self.eval_binding(b);
            let Some(pkg) = val.pkg().filter(|_| val.is_tracked()) else {
                continue;
            };
            match (&val, w.path.as_ref()) {
                (Val::Client { path, client }, [Step::Key(prop)])
                    if path.is_empty() && BASE_URL_PROPS.contains(prop) =>
                {
                    match client.site {
                        Some(site) => {
                            self.rerouted.insert(site);
                        }
                        None => {
                            self.rerouted_any.get_or_insert(Feature {
                                what:
                                    "a `baseURL` written on a client built in more than one place",
                                line: w.line,
                            });
                        }
                    }
                }
                (Val::Client { path, .. }, [Step::Key(prop)])
                    if path.is_empty() && SETTING_PROPS.contains(prop) => {}
                _ => {
                    self.sdk_patched.entry(pkg).or_insert(Feature {
                        what: "a member of the SDK replaced",
                        line: w.line,
                    });
                }
            }
        }
        self.member_writes = writes;
        let calls = std::mem::take(&mut self.calls);
        for (call, scope) in &calls {
            if call.kind() != "call_expression" {
                continue;
            }
            let Some(function) = call.child_by_field_name("function") else {
                continue;
            };
            let Some(callee) = self.global_callee(function, *scope) else {
                continue;
            };
            let spy = METHOD_MOCKS.contains(&callee.as_str());
            if !spy && !REFLECTIVE_WRITES.contains(&callee.as_str()) {
                continue;
            }
            let Some(target) = first_argument(*call) else {
                continue;
            };
            let val = self.eval(target, *scope, 0);
            let line = self.line(*call);
            let (true, Some(pkg)) = (val.is_tracked(), val.pkg()) else {
                // A `baseURL` written on an object that may be a client.
                let keys = (!spy)
                    .then(|| self.reflective_keys(*call, &callee))
                    .flatten()
                    .filter(|_| !matches!(val, Val::Untracked(Origin::Placeholder)))
                    .unwrap_or_default();
                if keys.iter().any(|k| BASE_URL_PROPS.contains(k)) {
                    self.rerouted_any.get_or_insert(Feature {
                        what: "a `baseURL` written on an object Arbor cannot identify",
                        line,
                    });
                }
                // `Object.assign(globalThis, { fetch })`.
                if keys.contains(&"fetch") {
                    self.global_fetch = true;
                }
                continue;
            };
            if spy {
                self.sdk_patched.entry(pkg).or_insert(Feature {
                    what: "a test spy on an SDK object",
                    line,
                });
                continue;
            }
            // `Object.assign(client, { baseURL })` re-routes the client;
            // settings change nothing; anything else replaces a member.
            let keys = self.reflective_keys(*call, &callee);
            match (&val, &keys) {
                (Val::Client { path, client }, Some(keys))
                    if path.is_empty()
                        && keys
                            .iter()
                            .all(|k| BASE_URL_PROPS.contains(k) || SETTING_PROPS.contains(k)) =>
                {
                    if keys.iter().any(|k| BASE_URL_PROPS.contains(k)) {
                        match client.site {
                            Some(site) => {
                                self.rerouted.insert(site);
                            }
                            None => {
                                self.rerouted_any.get_or_insert(Feature {
                                    what: "a `baseURL` written on a client built in more than one place",
                                    line,
                                });
                            }
                        }
                    }
                }
                _ => {
                    self.sdk_patched.entry(pkg).or_insert(Feature {
                        what: "a reflective write of an SDK object",
                        line,
                    });
                }
            }
        }
        self.calls = calls;
    }

    /// `Object.assign`, `jest.mock`... for a callee whose root is a global
    /// (or an import of a test runner): its dotted name.
    fn global_callee(&self, function: Node<'t>, scope: usize) -> Option<String> {
        let mut parts = Vec::new();
        let mut node = function;
        for _ in 0..4 {
            match node.kind() {
                "member_expression" => {
                    parts.push(self.text(node.child_by_field_name("property")?));
                    node = node.child_by_field_name("object")?;
                }
                "identifier" => {
                    let name = self.text(node);
                    let free = match self.lookup(scope, name) {
                        None => true,
                        // `import { vi } from "vitest"`, `import { jest } from "@jest/globals"`.
                        Some(b) => self.bindings[b]
                            .import
                            .as_ref()
                            .is_some_and(|def| def.pkg.is_none()),
                    };
                    if !free {
                        return None;
                    }
                    parts.push(name);
                    parts.reverse();
                    return Some(parts.join("."));
                }
                _ => return None,
            }
        }
        None
    }

    /// Whether `function` names `@google/genai`'s export `name`.
    fn is_genai_symbol(&self, function: Node<'t>, scope: usize, name: &str) -> bool {
        let (root, member) = match function.kind() {
            "identifier" => (function, None),
            "member_expression" => match (
                function.child_by_field_name("object"),
                function.child_by_field_name("property"),
            ) {
                (Some(object), Some(property)) if object.kind() == "identifier" => {
                    (object, Some(self.text(property)))
                }
                _ => return false,
            },
            _ => return false,
        };
        let Some(def) = self
            .lookup(scope, self.text(root))
            .and_then(|b| self.bindings[b].import.clone())
        else {
            return false;
        };
        match (def.pkg, &def.kind, member) {
            (Some((Pkg::GoogleGenai, _)), ImportKind::Named(imported), None) => imported == name,
            (Some((Pkg::GoogleGenai, _)), ImportKind::Namespace, Some(member)) => member == name,
            _ => false,
        }
    }

    /// SDK imports through `require(...)` and `import(...)`: recorded once
    /// the bindings can be evaluated.
    fn record_required_imports(&mut self) {
        let pending = std::mem::take(&mut self.pending_imports);
        for (binding, module, name, node) in pending {
            let val = self.eval_binding(binding);
            let Some(pkg) = val.pkg().filter(|_| val.is_tracked()) else {
                continue;
            };
            self.sdk_imported.insert(pkg.sdk());
            let kind = match val {
                Val::Module { .. } => BindingKind::Module,
                Val::Class(_) => BindingKind::ClientClass,
                _ => BindingKind::Symbol,
            };
            self.out.imports.push(SdkImport {
                sdk: pkg.sdk(),
                package: pkg.package().to_string(),
                module: clip(&module),
                name: name.map(|n| clip(&n)),
                local: clip(self.bindings[binding].name),
                kind,
                file: self.file.clone(),
                span: self.span(node),
            });
        }
    }

    // ------------------------------------------------------------------
    // Receivers
    // ------------------------------------------------------------------

    fn eval(&mut self, node: Node<'t>, scope: usize, depth: u32) -> Val {
        stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
            self.eval_inner(node, scope, depth)
        })
    }

    fn eval_inner(&mut self, node: Node<'t>, scope: usize, depth: u32) -> Val {
        if depth > MAX_DEPTH {
            self.cutoffs += 1;
            return Val::Untracked(Origin::Other);
        }
        match node.kind() {
            "parenthesized_expression"
            | "non_null_expression"
            | "await_expression"
            | "as_expression"
            | "satisfies_expression" => match node.named_child(0) {
                Some(inner) => self.eval(inner, scope, depth + 1),
                None => Val::Untracked(Origin::Other),
            },
            "type_assertion" => match node
                .named_child_count()
                .checked_sub(1)
                .and_then(|i| node.named_child(i))
            {
                Some(inner) => self.eval(inner, scope, depth + 1),
                None => Val::Untracked(Origin::Other),
            },
            "identifier" => match self.lookup(scope, self.text(node)) {
                Some(b) => self.eval_binding(b),
                None if self.text(node) == "undefined" => Val::Untracked(Origin::Placeholder),
                None => Val::Untracked(Origin::Unbound),
            },
            "undefined" | "null" | "string" | "template_string" | "number" | "true" | "false"
            | "object" | "array" => Val::Untracked(Origin::Placeholder),
            "member_expression" => {
                let (Some(object), Some(property)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("property"),
                ) else {
                    return Val::Untracked(Origin::Other);
                };
                let name = self.text(property);
                if object.kind() == "this" {
                    return self.this_val(scope, name);
                }
                let base = self.eval(object, scope, depth + 1);
                member_val(base, name)
            }
            "subscript_expression" => {
                let Some(object) = node.child_by_field_name("object") else {
                    return Val::Untracked(Origin::Other);
                };
                let key = node
                    .child_by_field_name("index")
                    .and_then(|i| static_key(i, self.src));
                match key {
                    Some(key) if object.kind() == "this" => self.this_val(scope, key),
                    Some(key) => {
                        let base = self.eval(object, scope, depth + 1);
                        member_val(base, key)
                    }
                    // `client[name]` is reported where it is called.
                    None => Val::Untracked(Origin::Other),
                }
            }
            "call_expression" => self.eval_call(node, scope, depth),
            "new_expression" => self.eval_new(node, scope, depth),
            "binary_expression" => match operator(node) {
                Some("??" | "||") => {
                    let left = node
                        .child_by_field_name("left")
                        .map(|l| self.eval(l, scope, depth + 1));
                    let right = node
                        .child_by_field_name("right")
                        .map(|r| self.eval(r, scope, depth + 1));
                    combine(left.into_iter().chain(right).collect())
                }
                Some("&&") => match node.child_by_field_name("right") {
                    Some(right) => self.eval(right, scope, depth + 1),
                    None => Val::Untracked(Origin::Other),
                },
                _ => Val::Untracked(Origin::Other),
            },
            "ternary_expression" => {
                let sides: Vec<Val> = ["consequence", "alternative"]
                    .iter()
                    .filter_map(|f| node.child_by_field_name(f))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|side| self.eval(side, scope, depth + 1))
                    .collect();
                combine(sides)
            }
            "sequence_expression" => {
                let count = node.named_child_count();
                match count.checked_sub(1).and_then(|i| node.named_child(i)) {
                    Some(last) => self.eval(last, scope, depth + 1),
                    None => Val::Untracked(Origin::Other),
                }
            }
            "assignment_expression" => match node.child_by_field_name("right") {
                Some(right) => self.eval(right, scope, depth + 1),
                None => Val::Untracked(Origin::Other),
            },
            _ => Val::Untracked(Origin::Other),
        }
    }

    /// What a binding holds, memoized.
    fn eval_binding(&mut self, b: usize) -> Val {
        if let Some(val) = self.val_memo.get(&b) {
            return val.clone();
        }
        if self.name_depth >= MAX_NAME_DEPTH {
            self.cutoffs += 1;
            return Val::Untracked(Origin::Other);
        }
        if !self.visiting.insert((0, b)) {
            return Val::Untracked(Origin::Other);
        }
        let cutoffs = self.cutoffs;
        self.name_depth += 1;
        let val = self.eval_binding_inner(b);
        self.name_depth -= 1;
        self.visiting.remove(&(0, b));
        if self.cutoffs == cutoffs {
            self.val_memo.insert(b, val.clone());
        }
        val
    }

    fn eval_binding_inner(&mut self, b: usize) -> Val {
        let binding = &self.bindings[b];
        let (decl, init, init_scope, path, fallback, each) = (
            binding.decl,
            binding.init,
            binding.init_scope,
            Rc::clone(&binding.path),
            binding.fallback,
            binding.each,
        );
        let writes = binding.writes.clone();
        let writes_cut = binding.writes_cut;
        let import = binding.import.clone();
        let mut vals = match decl {
            Decl::Import => vec![match import {
                Some(def) => import_val(&def),
                None => Val::Untracked(Origin::Other),
            }],
            Decl::Const | Decl::Let | Decl::Var if each => vec![Val::Untracked(Origin::Other)],
            Decl::Const | Decl::Let | Decl::Var => vec![match init {
                Some(init) => self.eval_path(init, init_scope, &path, fallback),
                None => Val::Untracked(Origin::Placeholder),
            }],
            Decl::Param => vec![self.param_client(b)],
            Decl::Function | Decl::Class | Decl::Enum | Decl::Namespace | Decl::Catch => {
                vec![Val::Untracked(Origin::Other)]
            }
        };
        for write in writes {
            vals.push(match write.value {
                Some(value) => self.eval(value, write.scope, 0),
                None => Val::Untracked(Origin::Other),
            });
        }
        if writes_cut || self.namespace_export_written(b) {
            vals.push(Val::Untracked(Origin::Other));
        }
        combine(vals)
    }

    /// Whether a namespace's export may be written as a property of the
    /// namespace: TypeScript compiles `export let model` in
    /// `namespace Cfg` to `Cfg.model`, which `Cfg.model = ...`, a cast of
    /// `Cfg` or another `namespace Cfg` block writes.
    fn namespace_export_written(&self, b: usize) -> bool {
        let binding = &self.bindings[b];
        if !binding.uses.exported {
            return false;
        }
        let Some(namespace) = self.namespace_of.get(&binding.scope) else {
            return false;
        };
        let Some(namespace) = *namespace else {
            return true;
        };
        let namespace = &self.bindings[namespace];
        let uses = &namespace.uses;
        !namespace.writes.is_empty()
            || namespace.writes_cut
            || uses.mutated
            || uses.escapes
            || !uses.passed_to.is_empty()
            || uses.passed_cut
    }

    /// What `init`, destructured along `path`, holds.
    fn eval_path(
        &mut self,
        init: Node<'t>,
        scope: usize,
        path: &[Step<'t>],
        fallback: Option<Node<'t>>,
    ) -> Val {
        let mut val = self.eval(init, scope, 0);
        for step in path {
            val = match step {
                Step::Key(key) => member_val(val, key),
                Step::Index(_) | Step::Other => match val {
                    Val::Untracked(origin) => Val::Untracked(origin),
                    _ => Val::Untracked(Origin::Other),
                },
            };
        }
        match fallback {
            // A default applies only when the property is `undefined`,
            // which a tracked export never is.
            Some(fallback) if !val.is_tracked() => {
                let other = self.eval(fallback, scope, 0);
                combine(vec![val, other])
            }
            _ => val,
        }
    }

    /// `this.<name>` in `scope`.
    fn this_val(&mut self, scope: usize, name: &'t str) -> Val {
        let val = match self.scopes[scope].this {
            This::Instance(class) => self.prop_val(class, false, name),
            This::Static(class) => self.prop_val(class, true, name),
            This::Other => return Val::Untracked(Origin::Other),
        };
        if self.this_rebound(scope) {
            return combine(vec![val, Val::Untracked(Origin::Other)]);
        }
        val
    }

    /// Whether the method whose `this` the code in `scope` sees may run with
    /// another object as `this`: its name is read without being called
    /// (`setTimeout(bot.ask)`, `{ ask: bot.ask }`, `const { ask } = bot`),
    /// it is called through `call`, `apply` or `bind`, or the file does
    /// either under a name known only at run time (`this[m].call(other)`),
    /// which never reaches a `#name`.
    fn this_rebound(&self, scope: usize) -> bool {
        self.scopes[scope]
            .method
            .and_then(|func| self.funcs[func].method)
            .is_some_and(|m| {
                !m.ctor
                    && ((self.method_refs_wild && !m.name.starts_with('#'))
                        || (!m.name.is_empty() && self.method_refs.contains(m.name)))
            })
    }

    /// What a class property holds: the one or more writes of it, combined.
    fn prop_val(&mut self, class: usize, is_static: bool, name: &'t str) -> Val {
        let key = (class, is_static, name);
        if let Some(val) = self.prop_val_memo.get(&key) {
            return val.clone();
        }
        let id = class * 2 + usize::from(is_static);
        if self.name_depth >= MAX_NAME_DEPTH || !self.visiting.insert((1, id)) {
            self.cutoffs += 1;
            return Val::Untracked(Origin::Other);
        }
        let cutoffs = self.cutoffs;
        self.name_depth += 1;
        let lookup = self.prop_lookup(class, is_static, true, name);
        let val = match lookup {
            Err(feature) => Val::Untracked(Origin::Dynamic(feature)),
            Ok(writes) if writes.is_empty() => Val::Untracked(Origin::UnassignedAttribute {
                name: format!("this.{name}"),
            }),
            Ok(writes) => {
                let mut vals = Vec::new();
                for write in writes {
                    vals.push(match write.value {
                        Some(value) => self.eval(value, write.scope, 0),
                        None if write.bare => Val::Untracked(Origin::Placeholder),
                        None => Val::Untracked(Origin::Other),
                    });
                }
                combine(vals)
            }
        };
        self.name_depth -= 1;
        self.visiting.remove(&(1, id));
        if self.cutoffs == cutoffs {
            self.prop_val_memo.insert(key, val.clone());
        }
        val
    }

    /// Every write of property `name` an instance of `class` (or the class
    /// itself, for a static one) may see, or the feature that makes them
    /// unknowable. The classes involved are the class, its same-file base
    /// and its direct same-file subclasses; a static property read by class
    /// name is the class's own, and read through `this` it may be a
    /// subclass's.
    fn prop_lookup(
        &self,
        class: usize,
        is_static: bool,
        through_this: bool,
        name: &str,
    ) -> Result<Vec<PropWrite<'t>>, Feature> {
        let line = |c: usize| {
            self.classes[c]
                .binding
                .map_or(0, |b| self.line(self.bindings[b].node))
        };
        let mut family = vec![class];
        if let Some(base) = self.classes[class].base_class {
            if self.classes[base].base_class.is_some() {
                return Err(Feature {
                    what: "inheritance more than one same-file level deep",
                    line: line(class),
                });
            }
            family.push(base);
        }
        let subclasses = &self.classes[class].subclasses;
        if subclasses.len() > MAX_SUBCLASSES {
            return Err(Feature {
                what: "more than 16 same-file subclasses",
                line: line(class),
            });
        }
        for &sub in subclasses {
            if !self.classes[sub].subclasses.is_empty() {
                return Err(Feature {
                    what: "inheritance more than one same-file level deep",
                    line: line(sub),
                });
            }
            if !is_static || through_this {
                family.push(sub);
            }
        }
        if let Some(feature) = self.outside_writes.get(name).or(self.outside_wild.as_ref()) {
            return Err(*feature);
        }
        let mut writes = Vec::new();
        for &member in &family {
            let c = &self.classes[member];
            if let Some(feature) = c.decorated {
                return Err(feature);
            }
            let (wild, members, fields, own) = if is_static {
                (
                    c.static_wild,
                    &c.static_members,
                    &c.static_fields,
                    &c.static_writes,
                )
            } else {
                (c.wild, &c.members, &c.fields, &c.writes)
            };
            if let Some(feature) = wild {
                return Err(feature);
            }
            if members.contains(name) {
                return Err(Feature {
                    what: "a method or accessor of that name",
                    line: line(member),
                });
            }
            // A field declared with no value in a subclass resets the
            // property to `undefined` after the base constructor ran.
            let subclass = member != class && self.classes[member].base_class == Some(class);
            for field in fields.iter().filter(|f| f.name == name) {
                if !field.bare || subclass {
                    writes.push(*field);
                }
            }
            writes.extend(own.iter().filter(|w| w.name == name).copied());
            if writes.len() > MAX_WRITES_READ {
                writes.truncate(MAX_WRITES_READ);
            }
        }
        Ok(writes)
    }

    /// Whether another module may extend `class` (or replace its properties
    /// through an instance): it is exported, anonymous, or passed or stored
    /// somewhere this file does not follow.
    fn class_public(&self, class: usize) -> bool {
        let Some(b) = self.classes[class].binding else {
            return true;
        };
        let uses = &self.bindings[b].uses;
        uses.exported || uses.escapes || !uses.passed_to.is_empty() || uses.passed_cut
    }

    fn eval_call(&mut self, node: Node<'t>, scope: usize, depth: u32) -> Val {
        let Some(function) = node.child_by_field_name("function") else {
            return Val::Untracked(Origin::Other);
        };
        if let Some(module) = self.module_call(node, scope) {
            return module;
        }
        let target = self.eval(function, scope, depth + 1);
        match target {
            Val::Client { client, path } => {
                let path: Vec<&str> = path.iter().map(String::as_str).collect();
                match (client.family, path.as_slice()) {
                    (_, ["withOptions"]) => self.copied_client(client, node, scope),
                    // A chat keeps its client's routing.
                    (Family::Genai, ["chats", "create"]) | (Family::LegacyModel, ["startChat"]) => {
                        Val::Client {
                            client,
                            path: Vec::new(),
                        }
                    }
                    (
                        Family::LegacyRoot,
                        ["getGenerativeModel"] | ["getGenerativeModelFromCachedContent"],
                    ) => self.legacy_model(client, node, scope),
                    _ => Val::Result(Some(client.pkg)),
                }
            }
            Val::Untracked(_) => {
                self.factory_nodes.insert(function.id(), function);
                Val::Untracked(Origin::Factory {
                    callee: function.id(),
                })
            }
            other => Val::Result(other.pkg()),
        }
    }

    /// `require("x")`, `import("x")`: the module, when `x` is tracked.
    fn module_call(&self, node: Node<'t>, scope: usize) -> Option<Val> {
        let function = node.child_by_field_name("function")?;
        let loader = function.kind() == "import"
            || (function.kind() == "identifier"
                && self.text(function) == "require"
                && self.lookup(scope, "require").is_none());
        if !loader {
            return None;
        }
        let module = first_argument(node).and_then(|a| string_content(a, self.src))?;
        Some(match Pkg::of_source(module) {
            Some((pkg, entry)) => Val::Module { pkg, entry },
            None => Val::Untracked(Origin::Imported {
                module: clip(module),
            }),
        })
    }

    fn eval_new(&mut self, node: Node<'t>, scope: usize, depth: u32) -> Val {
        let Some(constructor) = node.child_by_field_name("constructor") else {
            return Val::Untracked(Origin::Other);
        };
        let target = self.eval(constructor, scope, depth + 1);
        match target {
            Val::Class(class) => Val::Client {
                client: self.construction(node, class, scope),
                path: Vec::new(),
            },
            // `new (require("openai"))()`, `new ns.default()`
            Val::Module { pkg, entry: true } => match pkg.default_class() {
                Some(class) => Val::Client {
                    client: self.construction(node, class, scope),
                    path: Vec::new(),
                },
                None => Val::Result(Some(pkg)),
            },
            Val::V3ConfigClass => self.v3_config(node, scope),
            Val::Untracked(_) => {
                self.factory_nodes.insert(node.id(), node);
                Val::Untracked(Origin::Factory { callee: node.id() })
            }
            other => Val::Result(other.pkg()),
        }
    }

    /// `client.withOptions({...})`: a copy of the client, re-routed when it
    /// is given a `baseURL` (or options that may hold one).
    fn copied_client(&mut self, mut client: Client, call: Node<'t>, scope: usize) -> Val {
        let class = ClientClass {
            pkg: client.pkg,
            name: "",
            family: client.family,
        };
        if let Some(key) = base_url_key(client.family) {
            let options = self.options(call, scope, 0);
            let base_url = match options.lookup(key) {
                // A spread written after it may replace it.
                Lookup::Found(_, true, _) | Lookup::Opaque => BaseUrl::Unknown { env: false },
                Lookup::Found(value, false, value_scope) => {
                    self.base_url_kind(client.pkg, value, value_scope)
                }
                Lookup::Missing => BaseUrl::Absent,
            };
            if base_url != BaseUrl::Absent {
                let routing = class.routing(&options.keys, options.opaque, base_url);
                client.routing = routing;
                client.caveat = base_url_caveat(base_url, options.opaque).or(client.caveat);
                if client.caveat.is_some() && !matches!(base_url, BaseUrl::ProviderFallback) {
                    client.routing = Routing::Unknown;
                }
            } else if options.opaque {
                client.routing = Routing::Unknown;
                client.caveat = Some(ClientCaveat::BaseUrl {
                    env: false,
                    fallback: false,
                });
            }
            if let Some(option) = self.request_hook(&options, client.family) {
                client.routing = Routing::Unknown;
                client.caveat = Some(ClientCaveat::EndpointOption { option });
            }
        }
        Val::Client {
            client,
            path: Vec::new(),
        }
    }

    /// The option of a direct client that sees every request it sends and
    /// may send it elsewhere: a custom `fetch`, which receives each
    /// request's URL, or Anthropic's `middleware`, which may change a
    /// request before passing it on or answer it itself. A `middleware`
    /// array whose items are all calls of the SDK's own
    /// `betaRefusalFallbackMiddleware` keeps the route: that one only sends
    /// the request again with a fallback model.
    fn request_hook(&mut self, options: &Options<'t>, family: Family) -> Option<&'static str> {
        if let Lookup::Found(..) = options.lookup("fetch") {
            return Some("a custom `fetch`");
        }
        if family != Family::AnthropicDirect {
            return None;
        }
        match options.lookup("middleware") {
            Lookup::Found(value, false, scope) if self.refusal_fallbacks_only(value, scope) => None,
            Lookup::Found(..) => Some("`middleware`"),
            Lookup::Missing | Lookup::Opaque => None,
        }
    }

    /// Whether `value` is an array literal whose items are all calls of the
    /// Anthropic SDK's own `betaRefusalFallbackMiddleware`, imported from
    /// the package: a same-named function of the file or another module's
    /// is not it.
    fn refusal_fallbacks_only(&mut self, value: Node<'t>, scope: usize) -> bool {
        let array = unwrap_expression(value);
        if array.kind() != "array" {
            return false;
        }
        let mut cursor = array.walk();
        let items: Vec<Node<'t>> = array
            .named_children(&mut cursor)
            .filter(|n| n.kind() != "comment")
            .take(MAX_MIDDLEWARE + 1)
            .collect();
        if items.len() > MAX_MIDDLEWARE {
            return false;
        }
        for item in items {
            let item = unwrap_expression(item);
            let Some(function) = item
                .child_by_field_name("function")
                .filter(|_| item.kind() == "call_expression")
            else {
                return false;
            };
            let sdk_own = matches!(
                self.eval(function, scope, 0),
                Val::Symbol { pkg: Pkg::Anthropic, path }
                    if matches!(path.as_slice(), [name] if name == "betaRefusalFallbackMiddleware")
            );
            if !sdk_own {
                return false;
            }
        }
        true
    }

    /// `genAI.getGenerativeModel({ model }, { baseUrl })`: a model object
    /// routed like its client, unless the request options name an endpoint.
    fn legacy_model(&mut self, mut client: Client, call: Node<'t>, scope: usize) -> Val {
        client.family = Family::LegacyModel;
        let request_options = nth_argument(call, 1);
        let endpoint = match request_options.map(unwrap_expression) {
            None => false,
            Some(options) if options.kind() == "object" => {
                let read = self.object_entries(options, scope);
                !matches!(read.lookup("baseUrl"), Lookup::Missing)
            }
            Some(_) => true,
        };
        if endpoint {
            client.routing = Routing::Unknown;
            client.caveat = Some(ClientCaveat::EndpointOption {
                option: "request options with a `baseUrl`",
            });
        }
        client.line = Some(self.line(call));
        client.site = Some(call.id());
        Val::Client {
            client,
            path: Vec::new(),
        }
    }

    /// `new Configuration({ basePath })` from openai v3.
    fn v3_config(&mut self, node: Node<'t>, scope: usize) -> Val {
        let options = self.options(node, scope, 0);
        let base_url = match options.lookup("basePath") {
            Lookup::Found(_, true, _) | Lookup::Opaque => BaseUrl::Unknown { env: false },
            Lookup::Found(value, false, value_scope) => {
                self.base_url_kind(Pkg::Openai, value, value_scope)
            }
            Lookup::Missing => BaseUrl::Absent,
        };
        Val::V3Config {
            base_url,
            splat: options.opaque,
            kwargs: options.keys,
        }
    }

    /// The client built by `node` (a `new` expression), recorded once.
    fn construction(&mut self, node: Node<'t>, class: ClientClass, scope: usize) -> Client {
        if let Some(client) = self.constructions.get(&node.id()) {
            return client.clone();
        }
        let direct = matches!(
            class.family,
            Family::OpenaiDirect | Family::AnthropicDirect | Family::OpenaiV3
        );
        let (kwargs, splat, base_url) = match class.family {
            Family::OpenaiV3 => {
                // `new OpenAIApi(configuration, basePath?)`
                let config = nth_argument(node, 0).map(|c| self.eval(c, scope, 0));
                let (mut base_url, splat, kwargs) = match config {
                    Some(Val::V3Config {
                        base_url,
                        splat,
                        kwargs,
                    }) => (base_url, splat, kwargs),
                    None => (BaseUrl::Absent, false, Vec::new()),
                    Some(_) => (BaseUrl::Unknown { env: false }, false, Vec::new()),
                };
                if let Some(path) = nth_argument(node, 1) {
                    base_url = self.base_url_kind(Pkg::Openai, path, scope);
                }
                (kwargs, splat, base_url)
            }
            _ => {
                let options = self.options(node, scope, 0);
                let base_url = match base_url_key(class.family) {
                    Some(key) => match options.lookup(key) {
                        // A spread or computed key written after it may
                        // replace it.
                        Lookup::Found(_, true, _) => BaseUrl::Unknown { env: false },
                        Lookup::Found(value, false, value_scope) => {
                            self.base_url_kind(class.pkg, value, value_scope)
                        }
                        Lookup::Missing | Lookup::Opaque => BaseUrl::Absent,
                    },
                    None => BaseUrl::Absent,
                };
                (options.keys, options.opaque, base_url)
            }
        };
        let mut routing = class.routing(&kwargs, splat, base_url);
        let (option, vertexai) = self.endpoint_option(node, class, scope);
        let mut caveat = match option {
            Some(option) => Some(ClientCaveat::EndpointOption { option }),
            None => self.endpoint_env(class, base_url),
        };
        if caveat.is_some() {
            routing = Routing::Unknown;
        }
        if direct {
            if let Some(base) = base_url_caveat(base_url, splat) {
                caveat.get_or_insert(base);
            }
        }
        let (binding, held) = match self.targets.get(&node.id()) {
            Some((name, held)) => (Some(name.clone()), *held),
            None => (None, ClientScope::Inline),
        };
        let chain = node
            .child_by_field_name("constructor")
            .map(|c| self.render(c))
            .unwrap_or_default();
        let span = self.span(node);
        self.out.clients.push(ClientConstruction {
            sdk: class.pkg.sdk(),
            package: class.pkg.package().to_string(),
            constructor: format!("{}.{}", class.pkg.package(), class.name),
            chain,
            binding,
            scope: held,
            routing,
            is_async: false,
            kwargs,
            splat,
            file: self.file.clone(),
            span,
            symbol: self.symbol(scope),
            vertexai,
        });
        let client = Client {
            pkg: class.pkg,
            family: class.family,
            routing,
            line: Some(span.line),
            site: Some(node.id()),
            ambiguous: false,
            caveat,
        };
        self.constructions.insert(node.id(), client.clone());
        client
    }

    /// For a Gemini or OpenAI client: the option that may send its requests
    /// to another endpoint, and for Gemini how `vertexai`/`enterprise`
    /// reads.
    fn endpoint_option(
        &mut self,
        node: Node<'t>,
        class: ClientClass,
        scope: usize,
    ) -> (Option<&'static str>, Option<VertexAi>) {
        match class.family {
            Family::OpenaiDirect | Family::AnthropicDirect => {
                let options = self.options(node, scope, 0);
                let residency = class.family == Family::OpenaiDirect
                    && options.keys.iter().any(|k| k == "dataResidency");
                let option = if residency {
                    Some("`dataResidency`")
                } else {
                    self.request_hook(&options, class.family)
                };
                (option, None)
            }
            Family::Genai => {
                let options = self.options(node, scope, 0);
                let mut vertexai = None;
                let mut option = None;
                for key in ["vertexai", "enterprise"] {
                    match options.lookup(key) {
                        Lookup::Found(value, overridden, _) => {
                            let read = match unwrap_expression(value).kind() {
                                "true" if !overridden => VertexAi::On,
                                "false" | "null" | "undefined" if !overridden => VertexAi::Off,
                                _ => VertexAi::Unknown,
                            };
                            if read != VertexAi::Off {
                                option.get_or_insert(if key == "vertexai" {
                                    "`vertexai`"
                                } else {
                                    "`enterprise`"
                                });
                            }
                            vertexai = Some(match (vertexai, read) {
                                (None, read) | (Some(VertexAi::Off), read) => read,
                                (Some(VertexAi::On), VertexAi::Off) => VertexAi::On,
                                (Some(first), _) => first,
                            });
                        }
                        Lookup::Missing | Lookup::Opaque => {}
                    }
                }
                if options.opaque {
                    option.get_or_insert("a spread or an options object built elsewhere");
                }
                if let Lookup::Found(value, _, value_scope) = options.lookup("httpOptions") {
                    let value = unwrap_expression(value);
                    let names_endpoint = match value.kind() {
                        "object" => {
                            let read = self.object_entries(value, value_scope);
                            read.opaque || !matches!(read.lookup("baseUrl"), Lookup::Missing)
                        }
                        "null" | "undefined" => false,
                        _ => true,
                    };
                    if names_endpoint {
                        option.get_or_insert("`httpOptions` that may name a `baseUrl`");
                    }
                }
                if let Some(feature) = self.genai_base_urls {
                    let _ = feature;
                    option.get_or_insert("`setDefaultBaseUrls`, which this file calls");
                }
                (option, vertexai)
            }
            _ => (None, None),
        }
    }

    /// Why a client may not reach the endpoint its constructor names: when
    /// it names none, the SDK reads its endpoint variable, which this file
    /// sets or may set under a run-time name.
    fn endpoint_env(&self, class: ClientClass, base_url: BaseUrl) -> Option<ClientCaveat> {
        if !matches!(
            class.family,
            Family::OpenaiDirect | Family::AnthropicDirect | Family::Genai
        ) || base_url != BaseUrl::Absent
        {
            return None;
        }
        ENDPOINT_ENV
            .iter()
            .find(|(pkg, var)| *pkg == class.pkg && self.env_writes.contains(*var))
            .map(|(_, var)| ClientCaveat::EndpointEnv { var })
            .or_else(|| self.env_wild.map(ClientCaveat::EnvWrite))
    }

    /// How a `baseURL` value routes a direct client: the provider's own host
    /// is direct, any other host a custom route, the provider's host as an
    /// environment fallback direct with a gap, and an environment lookup
    /// with no fallback, or anything that does not resolve to a string,
    /// unknown with a gap. The URL is never reported.
    fn base_url_kind(&mut self, pkg: Pkg, expr: Node<'t>, scope: usize) -> BaseUrl {
        let expr = unwrap_expression(expr);
        if matches!(expr.kind(), "null" | "undefined")
            || (expr.kind() == "identifier"
                && self.text(expr) == "undefined"
                && self.lookup(scope, "undefined").is_none())
        {
            return BaseUrl::Absent;
        }
        if let Some(url) = string_content(expr, self.src) {
            return host_kind(pkg, url);
        }
        match self.resolve_value(expr, scope, 0) {
            // The fallback of an environment lookup; a property or parameter
            // with a fallback may be anything its caller sets.
            Ok(resolved)
                if resolved.origin == ValueOrigin::EnvDefault
                    && resolved.caveats.iter().all(|c| c.reason == "env_var") =>
            {
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

    /// A parameter used as a client: what every in-file caller passes, when
    /// they all pass the same one, else its type annotation.
    fn param_client(&mut self, b: usize) -> Val {
        let binding = &self.bindings[b];
        let name = binding.name.to_string();
        let Some((func, index)) = binding.param else {
            return Val::Untracked(Origin::Parameter { name });
        };
        let (path_empty, default, init_scope, annotation) = (
            binding.path.is_empty(),
            binding.init,
            binding.init_scope,
            binding.annotation,
        );
        let typed = annotation.and_then(|a| self.annotated_class(a, init_scope));
        let typed_val = |class: Option<ClientClass>, name: &str| match class {
            Some(class) => {
                let routing = class.typed_routing();
                Val::Client {
                    client: Client {
                        pkg: class.pkg,
                        family: class.family,
                        routing,
                        line: None,
                        site: None,
                        ambiguous: false,
                        caveat: (routing == Routing::Unknown).then(|| ClientCaveat::Annotation {
                            param: name.to_string(),
                        }),
                    },
                    path: Vec::new(),
                }
            }
            None => Val::Untracked(Origin::Parameter {
                name: name.to_string(),
            }),
        };
        if !path_empty {
            return typed_val(typed, &name);
        }
        let callers = self.callers(func);
        if callers.sites.is_empty() || !(callers.complete || callers.exported) {
            return typed_val(typed, &name);
        }
        let mut vals = Vec::new();
        for (call, scope) in callers.sites.clone() {
            match self.argument_at(call, scope, index) {
                Argument::Missing => vals.push(match default {
                    Some(default) => self.eval(default, init_scope, 0),
                    None => Val::Untracked(Origin::Placeholder),
                }),
                Argument::Unknown => vals.push(Val::Untracked(Origin::Other)),
                Argument::Expr(expr, expr_scope) => vals.push(self.eval(expr, expr_scope, 0)),
            }
        }
        match combine(vals) {
            Val::Client { mut client, path } if path.is_empty() && !client.ambiguous => {
                if !callers.complete {
                    client.caveat.get_or_insert(ClientCaveat::InFileCallers {
                        param: name.clone(),
                    });
                }
                Val::Client { client, path }
            }
            _ => typed_val(typed, &name),
        }
    }

    /// The tracked client class a type annotation names (`client: OpenAI`,
    /// `client: Anthropic | null`).
    fn annotated_class(&mut self, annotation: Node<'t>, scope: usize) -> Option<ClientClass> {
        let mut found = None;
        let mut work = vec![annotation];
        let mut seen = 0;
        while let Some(node) = work.pop() {
            seen += 1;
            if seen > 64 {
                return None;
            }
            match node.kind() {
                "type_annotation" | "union_type" | "parenthesized_type" => {
                    let mut cursor = node.walk();
                    let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                    work.extend(children);
                }
                "type_identifier" => {
                    let name = self.text(node);
                    let class = self
                        .lookup(scope, name)
                        .and_then(|b| match self.eval_binding(b) {
                            Val::Class(class) => Some(class),
                            _ => None,
                        });
                    match (found, class) {
                        (_, None) => return None,
                        (None, class) => found = class,
                        (Some(a), Some(b)) if a == b => {}
                        _ => return None,
                    }
                }
                "nested_type_identifier" => {
                    // `OpenAI.OpenAI`, `Anthropic.Anthropic`
                    return None;
                }
                "predefined_type" | "literal_type" => {}
                _ => return None,
            }
        }
        found
    }
}

/// A member of a tracked value.
fn member_val(base: Val, name: &str) -> Val {
    match base {
        Val::Module { pkg, entry } => module_member(pkg, entry, name),
        // A class's static members: openai v4 sets `OpenAI.OpenAI` and
        // `OpenAI.AzureOpenAI`, and a CommonJS `require` of a package is
        // its default class with the named exports on it.
        Val::Class(class) => match name {
            "default" => class.pkg.default_class().map_or(
                Val::Symbol {
                    pkg: class.pkg,
                    path: vec![clip(name)],
                },
                Val::Class,
            ),
            _ => client_class(class.pkg, name).map_or(
                Val::Symbol {
                    pkg: class.pkg,
                    path: vec![clip(name)],
                },
                Val::Class,
            ),
        },
        Val::V3ConfigClass | Val::V3Config { .. } => Val::Symbol {
            pkg: Pkg::Openai,
            path: vec![clip(name)],
        },
        Val::Symbol { pkg, mut path } => {
            if path.len() >= MAX_PATH {
                return Val::Untracked(Origin::Other);
            }
            path.push(clip(name));
            Val::Symbol { pkg, path }
        }
        Val::Client { client, mut path } => {
            if path.len() >= MAX_PATH {
                return Val::Untracked(Origin::Other);
            }
            path.push(clip(name));
            Val::Client { client, path }
        }
        Val::Result(pkg) => Val::Result(pkg),
        Val::Untracked(origin) => Val::Untracked(origin),
    }
}

/// One value from several (the assignments of a name, the sides of a
/// conditional): `undefined` and literals are ignored next to a real value;
/// clients of one kind stay one client; anything else is ambiguous.
fn combine(vals: Vec<Val>) -> Val {
    let mut real: Vec<Val> = vals
        .into_iter()
        .filter(|v| !matches!(v, Val::Untracked(Origin::Placeholder)))
        .collect();
    if real.is_empty() {
        return Val::Untracked(Origin::Placeholder);
    }
    if real.len() == 1 {
        return real.pop().unwrap_or(Val::Untracked(Origin::Other));
    }
    let first = real[0].clone();
    if real.iter().all(|v| *v == first) {
        return first;
    }
    let clients: Vec<&Client> = real
        .iter()
        .filter_map(|v| match v {
            Val::Client { client, path } if path.is_empty() => Some(client),
            _ => None,
        })
        .collect();
    if let Some(head) = clients.first() {
        let mut client = (*head).clone();
        let alike = clients.len() == real.len()
            && clients.iter().all(|c| {
                c.pkg == head.pkg
                    && c.routing == head.routing
                    && c.caveat == head.caveat
                    && !c.ambiguous
            });
        if alike {
            if clients.iter().any(|c| c.line != head.line) {
                client.line = None;
                client.site = None;
            }
        } else {
            client.ambiguous = true;
        }
        return Val::Client {
            client,
            path: Vec::new(),
        };
    }
    // A mix of untracked values: keep the first reason.
    match real.into_iter().find(|v| matches!(v, Val::Untracked(_))) {
        Some(untracked) => untracked,
        None => Val::Untracked(Origin::Other),
    }
}

/// The caveat a direct client's `baseURL` leaves.
fn base_url_caveat(base_url: BaseUrl, splat: bool) -> Option<ClientCaveat> {
    match base_url {
        BaseUrl::CustomFallback => Some(ClientCaveat::BaseUrl {
            env: true,
            fallback: true,
        }),
        BaseUrl::Unknown { env } => Some(ClientCaveat::BaseUrl {
            env,
            fallback: false,
        }),
        BaseUrl::ProviderFallback => Some(ClientCaveat::BaseUrl {
            env: false,
            fallback: true,
        }),
        // `new OpenAI({ ...options })`: the spread may hold a `baseURL`.
        BaseUrl::Absent if splat => Some(ClientCaveat::BaseUrl {
            env: false,
            fallback: false,
        }),
        _ => None,
    }
}

/// `expr` without the parentheses, `as`, `satisfies`, `!` and `<T>` around it.
fn unwrap_expression(mut node: Node<'_>) -> Node<'_> {
    for _ in 0..16 {
        let inner = match node.kind() {
            "parenthesized_expression"
            | "non_null_expression"
            | "as_expression"
            | "satisfies_expression" => node.named_child(0),
            "type_assertion" => node
                .named_child_count()
                .checked_sub(1)
                .and_then(|i| node.named_child(i)),
            _ => None,
        };
        match inner {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

/// The `n`th argument of a call (0-based), when every argument before it
/// and it are not spreads.
fn nth_argument(call: Node<'_>, n: usize) -> Option<Node<'_>> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let args: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|a| a.kind() != "comment")
        .take(n + 1)
        .collect();
    if args.iter().any(|a| a.kind() == "spread_element") {
        return None;
    }
    args.get(n).copied()
}

// ============================================================================
// Options objects, arguments and callers
// ============================================================================

/// One entry of an object literal as read.
#[derive(Debug, Clone)]
enum Entry<'t> {
    /// A property with its value and the scope the value is written in.
    Pair {
        key: &'t str,
        value: Node<'t>,
        scope: usize,
    },
    /// A spread or a computed key this file cannot read: it may set any
    /// property.
    Opaque,
    /// `...OTHER` where `OTHER` is a same-file object literal that is only
    /// read: its entries (read once), and the name it is reached by.
    Spread(Rc<ObjectRead<'t>>, Option<(String, ValueOrigin)>),
}

/// An object literal's entries, read once per literal.
#[derive(Debug, Default)]
struct ObjectRead<'t> {
    entries: Vec<Entry<'t>>,
    /// The index of the last entry of each key written in this literal.
    last: HashMap<&'t str, usize>,
    /// The indexes of the spreads that were expanded.
    spreads: Vec<usize>,
    /// The index of the last opaque entry.
    last_opaque: Option<usize>,
    /// An entry here or in an expanded spread is opaque.
    opaque: bool,
    /// A spread is written in it.
    spread: bool,
    /// `__proto__: P` is written in it: a key it does not set may be
    /// inherited.
    proto: bool,
}

/// A key an object literal sets, with its last entry.
#[derive(Debug, Clone)]
struct FlatEntry<'t> {
    key: &'t str,
    value: Node<'t>,
    scope: usize,
    /// The object it came from when that is not the literal itself.
    via: Option<(String, ValueOrigin)>,
    /// An opaque entry written after it may replace it.
    overridden: bool,
}

/// What sets a key of an object literal.
#[derive(Debug, Clone)]
struct Found<'t> {
    /// The entry's value, its scope, and the name of the object it came
    /// from when that is not the literal itself (a spread of `PARAMS`).
    pair: Option<ObjectRef<'t>>,
    /// A spread or computed key written after it (or inside the spread that
    /// sets it) may replace it.
    overridden: bool,
    /// Not set by a known entry, but an opaque entry may set it.
    maybe: bool,
}

impl<'t> ObjectRead<'t> {
    /// The entry that sets `key`: a key written twice counts by its last
    /// entry, and a spread written after an entry replaces it when the
    /// spread's object sets the key.
    fn find(&self, key: &str) -> Found<'t> {
        let direct = self.last.get(key).copied();
        let mut overridden = false;
        for &at in self.spreads.iter().rev() {
            if direct.is_some_and(|d| d > at) {
                break;
            }
            let Entry::Spread(layer, via) = &self.entries[at] else {
                continue;
            };
            let inner = layer.find(key);
            let after = self.last_opaque.is_some_and(|o| o > at);
            match inner.pair {
                Some((value, scope, inner_via)) => {
                    return Found {
                        pair: Some((value, scope, inner_via.or_else(|| via.clone()))),
                        overridden: overridden || inner.overridden || after,
                        maybe: false,
                    }
                }
                // This spread may set it.
                None if inner.maybe => overridden = true,
                None => {}
            }
        }
        match direct.map(|d| (d, &self.entries[d])) {
            Some((d, Entry::Pair { value, scope, .. })) => Found {
                pair: Some((*value, *scope, None)),
                overridden: overridden || self.last_opaque.is_some_and(|o| o > d),
                maybe: false,
            },
            _ => Found {
                pair: None,
                overridden: false,
                maybe: overridden || self.last_opaque.is_some() || self.proto,
            },
        }
    }

    fn lookup(&self, key: &str) -> Lookup<'t> {
        let found = self.find(key);
        match found.pair {
            Some((value, scope, _)) => Lookup::Found(value, found.overridden, scope),
            None if found.maybe => Lookup::Opaque,
            None => Lookup::Missing,
        }
    }

    /// Every key the literal sets with its last entry, spreads expanded, in
    /// source order, at most `limit` of them; whether the limit cut the
    /// list; and whether an opaque entry may set keys not listed.
    fn flat(&self, limit: usize) -> (Vec<FlatEntry<'t>>, bool, bool) {
        let mut found = Vec::new();
        let mut seen: HashSet<&'t str> = HashSet::new();
        let mut opaque = false;
        let mut complete = true;
        self.flat_into(
            None,
            limit,
            &mut found,
            &mut seen,
            &mut opaque,
            &mut complete,
            0,
        );
        found.reverse();
        (found, !complete, opaque || self.proto)
    }

    #[allow(clippy::too_many_arguments)]
    fn flat_into(
        &self,
        via: Option<&(String, ValueOrigin)>,
        limit: usize,
        found: &mut Vec<FlatEntry<'t>>,
        seen: &mut HashSet<&'t str>,
        opaque: &mut bool,
        complete: &mut bool,
        depth: usize,
    ) {
        for entry in self.entries.iter().rev() {
            if found.len() >= limit {
                *complete = false;
                return;
            }
            match entry {
                Entry::Pair { key, value, scope } => {
                    if seen.insert(key) {
                        found.push(FlatEntry {
                            key,
                            value: *value,
                            scope: *scope,
                            via: via.cloned(),
                            overridden: *opaque,
                        });
                    }
                }
                Entry::Opaque => *opaque = true,
                Entry::Spread(layer, layer_via) if depth < 4 => {
                    let inner = layer_via.as_ref().or(via);
                    layer.flat_into(inner, limit, found, seen, opaque, complete, depth + 1);
                }
                Entry::Spread(..) => *opaque = true,
            }
        }
    }
}

/// Records an entry that may set any property.
fn push_opaque(read: &mut ObjectRead<'_>) {
    read.last_opaque = Some(read.entries.len());
    read.opaque = true;
    read.entries.push(Entry::Opaque);
}

/// An options object as a call passes it.
#[derive(Debug, Default)]
struct Options<'t> {
    read: Rc<ObjectRead<'t>>,
    /// Property names written at the call, in source order.
    keys: Vec<String>,
    /// A spread is written in it, or it is not written at the call
    /// ([`SdkCall::splat`], [`ClientConstruction::splat`]).
    spread: bool,
    /// Some property is not known: a spread or computed key that did not
    /// resolve, or an options object this file does not show.
    opaque: bool,
    /// The argument is the options object (not a prompt or a file).
    present: bool,
    /// It is an object literal written at the call.
    literal: bool,
    /// How the object reads when it is shown in a gap.
    shape: String,
    /// The name the object is reached by when it is not written at the
    /// call (`create(PARAMS)`).
    via: Option<(String, ValueOrigin)>,
}

impl<'t> Options<'t> {
    /// Some property it sends may not be written at the call: a spread, a
    /// computed key, a prototype, or an object passed by name. The
    /// retirement check then never reads its property names as every
    /// argument the call sends ([`SdkCall::splat`]).
    fn unwritten(&self) -> bool {
        self.spread || self.opaque || self.read.proto
    }

    /// What sets `key`, with the name of the object it came from.
    fn find(&self, key: &str) -> Found<'t> {
        let mut found = self.read.find(key);
        if let Some((_, _, via)) = &mut found.pair {
            if via.is_none() {
                *via = self.via.clone();
            }
        }
        if found.pair.is_none() && self.opaque && !self.literal && self.read.entries.is_empty() {
            found.maybe = true;
        }
        found
    }

    fn lookup(&self, key: &str) -> Lookup<'t> {
        let found = self.find(key);
        match found.pair {
            Some((value, scope, _)) => Lookup::Found(value, found.overridden, scope),
            None if found.maybe => Lookup::Opaque,
            None => Lookup::Missing,
        }
    }
}

/// A property looked up in an object.
#[derive(Debug, Clone, Copy)]
enum Lookup<'t> {
    /// Its value and scope, and whether a spread or computed key written
    /// after it may replace it.
    Found(Node<'t>, bool, usize),
    /// Not set, and nothing may set it.
    Missing,
    /// Not written, but a spread or computed key may set it.
    Opaque,
}

/// An argument a call passes for a parameter.
#[derive(Debug, Clone, Copy)]
enum Argument<'t> {
    /// Nothing (or `undefined`): the parameter's default applies.
    Missing,
    /// Not known: a spread before or at it, or an object this file does
    /// not show.
    Unknown,
    /// This expression, in this scope.
    Expr(Node<'t>, usize),
}

/// The calls of a function this file shows.
#[derive(Debug, Clone)]
struct Callers<'t> {
    sites: Vec<(Node<'t>, usize)>,
    /// Every call of the function is in `sites`.
    complete: bool,
    /// Other modules may call it: it is exported, or a public method.
    exported: bool,
}

/// What a read-only call returns of the object it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandBack {
    /// Nothing that shares its containers: a string, a copy, a count.
    Nothing,
    /// Its entries (`Object.values`, `Object.entries`).
    Entries,
    /// The object itself (`Object.freeze`).
    Itself,
}

/// Calls that read their arguments without writing into them.
const READ_ONLY_CALLS: &[&str] = &[
    "JSON.stringify",
    "console.log",
    "console.info",
    "console.warn",
    "console.error",
    "console.debug",
    "Object.keys",
    "Object.values",
    "Object.entries",
    "Object.freeze",
    "Object.isFrozen",
    "structuredClone",
    "String",
    "Number",
    "Boolean",
    "Array.isArray",
];

impl<'t> Ctx<'t> {
    /// The entries of an object literal, read once per literal. A spread of
    /// a same-file object literal that is only read is expanded; any other
    /// spread, and a computed key that is not a literal, is opaque.
    fn object_entries(&mut self, object: Node<'t>, scope: usize) -> Rc<ObjectRead<'t>> {
        if let Some(read) = self.object_memo.get(&object.id()) {
            return Rc::clone(read);
        }
        // A literal spread into itself (through names), or nested past the
        // bound, is read as opaque.
        let deep = self.object_depth >= MAX_NAME_DEPTH;
        if deep || !self.reading_objects.insert(object.id()) {
            if deep {
                self.object_cuts += 1;
            }
            return Rc::new(ObjectRead {
                opaque: true,
                last_opaque: Some(0),
                entries: vec![Entry::Opaque],
                ..ObjectRead::default()
            });
        }
        self.object_depth += 1;
        let mut read = ObjectRead::default();
        let mut cursor = object.walk();
        let entries: Vec<Node<'t>> = object.named_children(&mut cursor).collect();
        for entry in entries {
            let pair = match entry.kind() {
                "pair" => {
                    let (Some(key), Some(value)) = (
                        entry.child_by_field_name("key"),
                        entry.child_by_field_name("value"),
                    ) else {
                        continue;
                    };
                    let computed = key.kind() == "computed_property_name";
                    let key = match key.kind() {
                        "property_identifier" | "number" => Some(self.text(key)),
                        "string" => string_content(key, self.src),
                        "computed_property_name" => {
                            key.named_child(0).and_then(|k| static_key(k, self.src))
                        }
                        _ => None,
                    };
                    // `__proto__: P` sets the prototype: a property the
                    // literal does not set may come from `P`.
                    if key == Some("__proto__") && !computed {
                        read.proto = true;
                        continue;
                    }
                    match key {
                        Some(key) => Some((key, value)),
                        None => {
                            push_opaque(&mut read);
                            continue;
                        }
                    }
                }
                "shorthand_property_identifier" => Some((self.text(entry), entry)),
                "method_definition" => entry
                    .child_by_field_name("name")
                    .and_then(|n| self.member_name(n))
                    .map(|name| (name, entry)),
                "spread_element" => {
                    read.spread = true;
                    let expanded = (read.spreads.len() < MAX_SPREADS)
                        .then(|| entry.named_child(0))
                        .flatten()
                        .and_then(|target| self.object_node(target, scope));
                    match expanded {
                        Some((inner, inner_scope, name)) => {
                            let layer = self.object_entries(inner, inner_scope);
                            read.opaque |= layer.opaque;
                            read.spreads.push(read.entries.len());
                            read.entries.push(Entry::Spread(layer, name));
                        }
                        None => push_opaque(&mut read),
                    }
                    None
                }
                _ => None,
            };
            if let Some((key, value)) = pair {
                read.last.insert(key, read.entries.len());
                read.entries.push(Entry::Pair { key, value, scope });
            }
        }
        // Its own methods write `this`: any property may change.
        if self.this_written_objects.contains(&object.id()) {
            push_opaque(&mut read);
        }
        self.object_depth -= 1;
        self.reading_objects.remove(&object.id());
        let read = Rc::new(read);
        self.object_memo.insert(object.id(), Rc::clone(&read));
        read
    }

    /// The property names written in an object literal, in source order.
    fn written_keys(read: &ObjectRead<'t>) -> Vec<String> {
        read.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Pair { key, .. } => Some(clip(key)),
                _ => None,
            })
            .collect()
    }

    /// The options object a call passes at position `index`.
    fn options(&mut self, call: Node<'t>, scope: usize, index: usize) -> Options<'t> {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return Options::default();
        };
        let mut cursor = arguments.walk();
        let args: Vec<Node<'t>> = arguments
            .named_children(&mut cursor)
            .filter(|a| a.kind() != "comment")
            .take(index + 1)
            .collect();
        if args.iter().any(|a| a.kind() == "spread_element") {
            return Options {
                spread: true,
                opaque: true,
                present: true,
                shape: "...".to_string(),
                ..Options::default()
            };
        }
        let Some(arg) = args.get(index).copied() else {
            return Options::default();
        };
        let expr = unwrap_expression(arg);
        match expr.kind() {
            "object" => {
                let read = self.object_entries(expr, scope);
                Options {
                    keys: Self::written_keys(&read),
                    spread: read.spread,
                    opaque: read.opaque,
                    present: true,
                    literal: true,
                    shape: "{...}".to_string(),
                    via: None,
                    read,
                }
            }
            // A prompt, a file or a callback, not an options object.
            "string"
            | "template_string"
            | "number"
            | "array"
            | "arrow_function"
            | "function_expression"
            | "true"
            | "false"
            | "null"
            | "undefined" => Options::default(),
            _ => match self.object_node(expr, scope) {
                Some((object, object_scope, via)) => {
                    let read = self.object_entries(object, object_scope);
                    Options {
                        spread: true,
                        opaque: read.opaque,
                        present: true,
                        literal: false,
                        shape: self.render(expr),
                        via,
                        keys: Vec::new(),
                        read,
                    }
                }
                None => Options {
                    spread: true,
                    opaque: true,
                    present: true,
                    shape: self.render(expr),
                    ..Options::default()
                },
            },
        }
    }

    /// The same-file object literal `expr` names, when it is only ever read:
    /// the literal itself, a name bound to one, or a nested literal under
    /// one (`CONFIG.openai`). Also the name it was reached through, with the
    /// origin of a value read from it.
    fn object_node(&mut self, expr: Node<'t>, scope: usize) -> Option<ObjectRef<'t>> {
        let expr = unwrap_expression(expr);
        match expr.kind() {
            "object" => Some((expr, scope, None)),
            "identifier" => {
                let b = self.lookup(scope, self.text(expr))?;
                let (object, object_scope) = self.readonly_object(b)?;
                let origin = constant_origin(self.scopes[self.bindings[b].scope].kind);
                Some((object, object_scope, Some((clip(self.text(expr)), origin))))
            }
            "member_expression" | "subscript_expression" => {
                let object = expr.child_by_field_name("object")?;
                let key = match expr.kind() {
                    "member_expression" => self.text(expr.child_by_field_name("property")?),
                    _ => static_key(expr.child_by_field_name("index")?, self.src)?,
                };
                if self.object_depth >= MAX_NAME_DEPTH {
                    self.object_cuts += 1;
                    return None;
                }
                self.object_depth += 1;
                let parent = self.object_node(object, scope);
                self.object_depth -= 1;
                let (parent, parent_scope, via) = parent?;
                let read = self.object_entries(parent, parent_scope);
                match read.lookup(key) {
                    Lookup::Found(value, false, value_scope) => {
                        let value = unwrap_expression(value);
                        (value.kind() == "object").then(|| {
                            let via =
                                via.map(|(name, origin)| (clip(&format!("{name}.{key}")), origin));
                            (value, value_scope, via)
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The object literal a binding holds, when every use of the name only
    /// reads it: static or computed member reads, spreads, destructuring,
    /// and passing it to an SDK request or a read-only builtin. Any write
    /// into it, a method call on it, or passing it (or a nested container
    /// of it) anywhere else means another value may be read.
    fn readonly_object(&mut self, b: usize) -> Option<(Node<'t>, usize)> {
        if let Some(found) = self.readonly_memo.get(&b) {
            return *found;
        }
        let cuts = self.object_cuts;
        let found = self.readonly_object_inner(b);
        // A read cut short by the nesting bound is not kept.
        if self.object_cuts == cuts {
            self.readonly_memo.insert(b, found);
        }
        found
    }

    fn readonly_object_inner(&mut self, b: usize) -> Option<(Node<'t>, usize)> {
        let (object, scope) = self.object_literal_of(b)?;
        let uses = self.bindings[b].uses.clone();
        if uses.escapes
            || uses.mutated
            || uses.member_calls
            || uses.handed_out_cut
            || uses.passed_cut
        {
            return None;
        }
        for (call, call_scope) in &uses.passed_to {
            if !self.harmless_call(*call, *call_scope) {
                return None;
            }
            let handed = match self.hands_back(*call, *call_scope) {
                HandBack::Nothing => false,
                HandBack::Entries => self.holds_container(object, scope, &[Step::Other]),
                HandBack::Itself => true,
            };
            if handed {
                return None;
            }
        }
        for (path, call) in &uses.handed_out {
            let harmless = match call {
                Some((call, call_scope)) if self.harmless_call(*call, *call_scope) => {
                    match self.hands_back(*call, *call_scope) {
                        HandBack::Nothing => true,
                        HandBack::Entries => {
                            let mut entries = path.to_vec();
                            entries.push(Step::Other);
                            !self.holds_container(object, scope, &entries)
                        }
                        HandBack::Itself => false,
                    }
                }
                _ => false,
            };
            if !harmless && self.holds_container(object, scope, path) {
                return None;
            }
        }
        Some((object, scope))
    }

    /// What a read-only call hands back of the argument it reads:
    /// `Object.values(x)` its entries, `Object.freeze(x)` the argument
    /// itself, unless the result is dropped.
    fn hands_back(&self, call: Node<'t>, scope: usize) -> HandBack {
        if self.dropped_calls.contains(&call.id()) {
            return HandBack::Nothing;
        }
        let callee = call
            .child_by_field_name("function")
            .and_then(|f| self.global_callee(f, scope));
        match callee.as_deref() {
            Some("Object.values" | "Object.entries") => HandBack::Entries,
            Some("Object.freeze") => HandBack::Itself,
            _ => HandBack::Nothing,
        }
    }

    /// Whether `path` in the object literal reaches a nested object or
    /// array literal (or something this file does not show).
    fn holds_container(&mut self, object: Node<'t>, scope: usize, path: &[Step<'t>]) -> bool {
        let mut current = object;
        let mut current_scope = scope;
        for (at, step) in path.iter().enumerate() {
            if unwrap_expression(current).kind() != "object" {
                return true;
            }
            let Step::Key(key) = step else {
                // `MODELS[tier]` hands out an entry: a container only if
                // one of the entries is.
                if at + 1 < path.len() {
                    return true;
                }
                return self.holds_any_container(unwrap_expression(current), current_scope);
            };
            let read = self.object_entries(unwrap_expression(current), current_scope);
            match read.lookup(key) {
                Lookup::Found(value, _, value_scope) => {
                    current = value;
                    current_scope = value_scope;
                }
                Lookup::Missing => return false,
                Lookup::Opaque => return true,
            }
        }
        matches!(
            unwrap_expression(current).kind(),
            "object" | "array" | "identifier" | "member_expression" | "call_expression"
        )
    }

    /// Whether an entry of the object literal, spreads expanded, is anything
    /// but a primitive literal (or may be: an opaque entry). Read once per
    /// literal.
    fn holds_any_container(&mut self, object: Node<'t>, scope: usize) -> bool {
        if let Some(holds) = self.container_memo.get(&object.id()) {
            return *holds;
        }
        let read = self.object_entries(object, scope);
        let (entries, cut, opaque) = read.flat(usize::MAX);
        let holds = cut
            || opaque
            || entries.iter().any(|entry| {
                !matches!(
                    unwrap_expression(entry.value).kind(),
                    "string" | "template_string" | "number" | "true" | "false" | "null"
                )
            });
        self.container_memo.insert(object.id(), holds);
        holds
    }

    /// Whether a call writes nothing into its arguments: a read-only
    /// builtin, or a request to a tracked SDK client.
    fn harmless_call(&mut self, call: Node<'t>, scope: usize) -> bool {
        let Some(function) = call.child_by_field_name("function") else {
            return false;
        };
        if self
            .global_callee(function, scope)
            .is_some_and(|callee| READ_ONLY_CALLS.contains(&callee.as_str()))
        {
            return true;
        }
        matches!(self.eval(function, scope, 0), Val::Client { path, .. } if !path.is_empty())
    }

    /// What a call passes at position `index`.
    fn argument_at(&self, call: Node<'t>, scope: usize, index: usize) -> Argument<'t> {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return Argument::Missing;
        };
        if arguments.kind() != "arguments" {
            return Argument::Unknown;
        }
        let mut cursor = arguments.walk();
        let args: Vec<Node<'t>> = arguments
            .named_children(&mut cursor)
            .filter(|a| a.kind() != "comment")
            .take(index + 1)
            .collect();
        if args.iter().any(|a| a.kind() == "spread_element") {
            return Argument::Unknown;
        }
        match args.get(index) {
            None => Argument::Missing,
            Some(arg) if self.is_undefined(*arg, scope) => Argument::Missing,
            Some(arg) => Argument::Expr(*arg, scope),
        }
    }

    fn is_undefined(&self, node: Node<'t>, scope: usize) -> bool {
        let node = unwrap_expression(node);
        node.kind() == "undefined"
            || (node.kind() == "identifier"
                && self.text(node) == "undefined"
                && self.lookup(scope, "undefined").is_none())
    }

    /// What a call passes for a destructured parameter's property
    /// (`f({ model: "x" })` for `function f({ model })`).
    fn param_argument(
        &mut self,
        call: Node<'t>,
        scope: usize,
        index: usize,
        path: &[Step<'t>],
    ) -> Argument<'t> {
        let argument = self.argument_at(call, scope, index);
        let Argument::Expr(mut expr, mut expr_scope) = argument else {
            return argument;
        };
        for step in path {
            let Step::Key(key) = step else {
                return Argument::Unknown;
            };
            if self.is_undefined(expr, expr_scope) {
                return Argument::Missing;
            }
            let Some((object, object_scope, _)) = self.object_node(expr, expr_scope) else {
                return Argument::Unknown;
            };
            let read = self.object_entries(object, object_scope);
            match read.lookup(key) {
                Lookup::Found(value, false, value_scope) => {
                    expr = value;
                    expr_scope = value_scope;
                }
                Lookup::Found(_, true, _) | Lookup::Opaque => return Argument::Unknown,
                Lookup::Missing => return Argument::Missing,
            }
        }
        if self.is_undefined(expr, expr_scope) {
            Argument::Missing
        } else {
            Argument::Expr(expr, expr_scope)
        }
    }

    /// The calls of `func` this file shows, and whether they are all of
    /// them. A function's calls are all seen when it is not exported and
    /// its name is only ever called directly; a constructor's when its class
    /// is not exported and only ever constructed by name; a `private` or
    /// `#private` method's always (every call of its name in the file is
    /// listed). Other modules may call an exported function or a public
    /// method.
    fn callers(&self, func: usize) -> Callers<'t> {
        let unseen = Callers {
            sites: Vec::new(),
            complete: false,
            exported: false,
        };
        if let Some(method) = self.funcs[func].method {
            let class = &self.classes[method.class];
            if method.ctor {
                let Some(b) = class.binding else {
                    return Callers {
                        exported: true,
                        ..unseen
                    };
                };
                let mut sites = Vec::new();
                let mut complete = class.self_constructed.is_none()
                    && class.decorated.is_none()
                    && self.any_constructed.is_none();
                // `new Bot(...)`, and `new Inner(...)` through a class
                // expression's own name.
                for binding in std::iter::once(b).chain(class.inner) {
                    let binding = &self.bindings[binding];
                    let uses = &binding.uses;
                    complete &= !uses.exported
                        && !uses.escapes
                        && uses.passed_to.is_empty()
                        && !uses.passed_cut
                        && !uses.extended
                        && binding.writes.is_empty();
                    sites.extend(uses.news.iter().copied());
                }
                complete &= sites.len() <= MAX_CALLERS_READ;
                return Callers {
                    sites,
                    complete,
                    exported: self.bindings[b].uses.exported,
                };
            }
            if method.name.is_empty() {
                return unseen;
            }
            let sites = self
                .method_calls
                .get(method.name)
                .cloned()
                .unwrap_or_default();
            if method.private {
                // A key known only at run time never reaches a `#name`.
                let by_key = !method.name.starts_with('#')
                    && (self.method_refs_wild || self.method_calls_wild);
                let complete = !self.method_refs.contains(method.name)
                    && !by_key
                    && sites.len() <= MAX_CALLERS_READ
                    && class.decorated.is_none();
                return Callers {
                    sites,
                    complete,
                    exported: false,
                };
            }
            return Callers {
                sites,
                complete: false,
                exported: true,
            };
        }
        let Some(b) = self.funcs[func].binding else {
            return unseen;
        };
        let mut sites = Vec::new();
        let mut complete = true;
        // `ask(...)`, and `retry(...)` through a function expression's own
        // name.
        for binding in std::iter::once(b).chain(self.funcs[func].inner) {
            let binding = &self.bindings[binding];
            let uses = &binding.uses;
            complete &= !uses.exported
                && !uses.escapes
                && uses.passed_to.is_empty()
                && !uses.passed_cut
                && uses.news.is_empty()
                && !uses.member_calls
                && !uses.mutated
                && binding.writes.is_empty()
                && !binding.writes_cut;
            sites.extend(uses.calls.iter().copied());
        }
        complete &= sites.len() <= MAX_CALLERS_READ;
        Callers {
            sites,
            complete,
            exported: self.bindings[b].uses.exported,
        }
    }

    // ------------------------------------------------------------------
    // Values
    // ------------------------------------------------------------------

    fn resolve_value(
        &mut self,
        node: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || {
            self.resolve_value_inner(node, scope, depth)
        })
    }

    fn resolve_value_inner(
        &mut self,
        node: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        if depth > 16 {
            self.cutoffs += 1;
            return Err(Unresolved::new("non_literal", self.render(node)));
        }
        match node.kind() {
            "parenthesized_expression"
            | "non_null_expression"
            | "as_expression"
            | "satisfies_expression"
            | "type_assertion" => {
                let inner = unwrap_expression(node);
                if inner.id() == node.id() {
                    return Err(Unresolved::new("non_literal", self.render(node)));
                }
                self.resolve_value(inner, scope, depth + 1)
            }
            "string" | "template_string" => self.literal(node),
            "identifier" | "shorthand_property_identifier" => {
                let name = self.text(node);
                match self.lookup(scope, name) {
                    Some(b) => self.binding_value(b, depth),
                    None if name == "undefined" => {
                        Err(Unresolved::new("non_literal", name.to_string()))
                    }
                    None => Err(Unresolved::new("unbound", name.to_string())),
                }
            }
            "member_expression" | "subscript_expression" => self.member_value(node, scope, depth),
            "binary_expression" => self.binary_value(node, scope, depth),
            "ternary_expression" => {
                let mut found = Vec::new();
                let mut complete = true;
                for field in ["consequence", "alternative"] {
                    let side = node.child_by_field_name(field);
                    match side.map(|s| self.resolve_value(s, scope, depth + 1)) {
                        Some(Ok(resolved)) => {
                            complete &= resolved.is_certain();
                            complete &= push_candidate(&mut found, resolved);
                        }
                        Some(Err(unresolved)) if unresolved.reason == "credential" => {
                            return Err(unresolved)
                        }
                        _ => complete = false,
                    }
                }
                Err(Unresolved::new("conditional", self.render(node))
                    .with_candidates(found, complete))
            }
            "call_expression" => match self.env_lookup(node, scope) {
                Some(shape) => Err(Unresolved::new("env_var", shape)),
                None => Err(Unresolved::new("function_call", self.render(node))),
            },
            "await_expression" => match node.named_child(0) {
                Some(inner) if inner.kind() == "call_expression" => {
                    Err(Unresolved::new("function_call", self.render(node)))
                }
                Some(inner) => self.resolve_value(inner, scope, depth + 1),
                None => Err(Unresolved::new("non_literal", self.render(node))),
            },
            _ => Err(Unresolved::new("non_literal", self.render(node))),
        }
    }

    /// A plain string literal, or a template literal with no substitution.
    fn literal(&self, node: Node<'t>) -> Result<Resolved, Unresolved> {
        let shape = match node.kind() {
            "template_string" => "`...`",
            _ => "\"...\"",
        };
        if node.end_byte() - node.start_byte() > MAX_LITERAL_BYTES {
            return Err(Unresolved::new("non_literal", shape.to_string()));
        }
        let mut cursor = node.walk();
        let substituted = node
            .named_children(&mut cursor)
            .any(|c| c.kind() == "template_substitution");
        if substituted {
            return Err(Unresolved::new("f_string", "`...${...}`".to_string()));
        }
        match string_content(node, self.src) {
            Some(value) if is_capturable_value(value) => Ok(Resolved {
                value: value.to_string(),
                origin: ValueOrigin::Literal,
                name: None,
                span: self.span(node),
                caveats: Vec::new(),
            }),
            Some(value) if is_credential_value(value) => {
                Err(Unresolved::new("credential", shape.to_string()))
            }
            _ => Err(Unresolved::new("non_literal", shape.to_string())),
        }
    }

    /// The value a name holds, memoized.
    fn binding_value(&mut self, b: usize, depth: u32) -> Result<Resolved, Unresolved> {
        if let Some(result) = self.value_memo.get(&b) {
            return result.clone();
        }
        let name = self.bindings[b].name;
        if self.name_depth >= MAX_NAME_DEPTH || !self.visiting.insert((2, b)) {
            self.cutoffs += 1;
            return Err(Unresolved::new("non_literal", name.to_string()));
        }
        let cutoffs = self.cutoffs;
        self.name_depth += 1;
        let result = self.binding_value_inner(b, depth);
        self.name_depth -= 1;
        self.visiting.remove(&(2, b));
        if self.cutoffs == cutoffs {
            self.value_memo.insert(b, result.clone());
        }
        result
    }

    fn binding_value_inner(&mut self, b: usize, depth: u32) -> Result<Resolved, Unresolved> {
        let binding = &self.bindings[b];
        let name = binding.name;
        // No value flows through a secret-named binding.
        if is_secret_key(name) {
            return Err(Unresolved::new("credential", name.to_string()));
        }
        let origin = constant_origin(self.scopes[binding.scope].kind);
        let (decl, init, init_scope, each, fallback) = (
            binding.decl,
            binding.init,
            binding.init_scope,
            binding.each,
            binding.fallback,
        );
        let path = Rc::clone(&binding.path);
        let writes = binding.writes.clone();
        let writes_cut = binding.writes_cut;
        let imported = binding.import.as_ref().map(|def| def.pkg.is_none());
        if self.namespace_export_written(b) {
            return Err(Unresolved::new("reassigned", name.to_string()));
        }
        match decl {
            Decl::Import if imported == Some(true) => {
                Err(Unresolved::new("imported_name", name.to_string()))
            }
            Decl::Const | Decl::Let | Decl::Var if each => {
                // A loop variable takes each item of what the loop runs
                // over.
                let (items, complete) = match init {
                    Some(iterable) if path.is_empty() => {
                        self.array_items(iterable, init_scope, depth)
                    }
                    _ => (Vec::new(), false),
                };
                let items = items
                    .into_iter()
                    .map(|item| item.constant(name, origin))
                    .collect();
                Err(Unresolved::new("reassigned", name.to_string())
                    .with_candidates(items, complete))
            }
            Decl::Const | Decl::Let | Decl::Var if writes.is_empty() && !writes_cut => match init {
                Some(init) => {
                    // A name destructured from the environment is the
                    // variable's name, which is never reported.
                    if !path.is_empty() && self.env_path(init, init_scope, &path) {
                        return self.path_value(init, init_scope, &path, fallback, depth + 1);
                    }
                    let result = if path.is_empty() {
                        self.resolve_value(init, init_scope, depth + 1)
                    } else {
                        self.path_value(init, init_scope, &path, fallback, depth + 1)
                    };
                    result
                        .map(|resolved| resolved.constant(name, origin))
                        .map_err(|unresolved| unresolved.via(name))
                }
                None => Err(Unresolved::new("non_literal", name.to_string())),
            },
            Decl::Const | Decl::Let | Decl::Var => {
                // Several assignments: list what each one gives.
                let mut found = Vec::new();
                let mut complete = !writes_cut;
                let first = match init {
                    Some(init) if path.is_empty() => {
                        Some(self.resolve_value(init, init_scope, depth + 1))
                    }
                    Some(init) => {
                        Some(self.path_value(init, init_scope, &path, fallback, depth + 1))
                    }
                    None => None,
                };
                let rest: Vec<Option<Result<Resolved, Unresolved>>> = writes
                    .iter()
                    .map(|write| write.value.map(|v| (v, write.scope)))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|value| value.map(|(v, s)| self.resolve_value(v, s, depth + 1)))
                    .collect();
                if first.is_none() {
                    // `let model;` holds `undefined` until it is assigned.
                    complete = false;
                }
                for result in first.into_iter().map(Some).chain(rest) {
                    match result {
                        Some(Ok(resolved)) => {
                            complete &= resolved.is_certain();
                            complete &= push_candidate(&mut found, resolved.constant(name, origin));
                        }
                        Some(Err(unresolved)) if unresolved.reason == "credential" => {
                            return Err(unresolved.via(name))
                        }
                        _ => complete = false,
                    }
                }
                Err(Unresolved::new("reassigned", name.to_string())
                    .with_candidates(found, complete))
            }
            Decl::Param => self.param_value(b, depth),
            _ => Err(Unresolved::new("non_literal", name.to_string())),
        }
    }

    /// The items of the array literal a loop runs over (or of one bound to
    /// a name that is only read), resolved, and whether every item did.
    fn array_items(
        &mut self,
        iterable: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> (Vec<Resolved>, bool) {
        let mut array = unwrap_expression(iterable);
        let mut array_scope = scope;
        if array.kind() == "identifier" {
            let Some(b) = self.lookup(scope, self.text(array)) else {
                return (Vec::new(), false);
            };
            let binding = &self.bindings[b];
            let uses = &binding.uses;
            let read_only = matches!(binding.decl, Decl::Const | Decl::Let | Decl::Var)
                && binding.writes.is_empty()
                && binding.path.is_empty()
                && !binding.each
                && !uses.escapes
                && !uses.mutated
                && !uses.member_calls
                && uses.passed_to.is_empty();
            match (read_only, binding.init) {
                (true, Some(init)) => {
                    array = unwrap_expression(init);
                    array_scope = binding.init_scope;
                }
                _ => return (Vec::new(), false),
            }
        }
        if array.kind() != "array" {
            return (Vec::new(), false);
        }
        let mut found = Vec::new();
        let mut complete = true;
        let mut cursor = array.walk();
        let items: Vec<Node<'t>> = array.named_children(&mut cursor).collect();
        for item in items.into_iter().take(MAX_CANDIDATES + 1) {
            if item.kind() == "comment" {
                continue;
            }
            match self.resolve_value(item, array_scope, depth + 1) {
                Ok(resolved) => {
                    complete &= resolved.is_certain();
                    complete &= push_candidate(&mut found, resolved);
                }
                Err(_) => complete = false,
            }
        }
        (found, complete)
    }

    /// The value of `init` destructured along `path` (`const { model } =
    /// CONFIG`), or the pattern's default when the property is not set.
    fn path_value(
        &mut self,
        init: Node<'t>,
        scope: usize,
        path: &[Step<'t>],
        fallback: Option<Node<'t>>,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        // `const { MODEL = "gpt-4o" } = process.env`: an environment lookup,
        // with the pattern's default as its fallback. The variable's name is
        // never reported.
        if self.env_path(init, scope, path) {
            let env = Unresolved::new("env_var", "process.env[...]".to_string());
            return match fallback.map(|f| self.resolve_value(f, scope, depth + 1)) {
                Some(Ok(mut resolved)) => {
                    resolved.origin = ValueOrigin::EnvDefault;
                    resolved.caveats.push(env);
                    Ok(resolved)
                }
                Some(Err(unresolved)) if unresolved.reason == "credential" => Err(unresolved),
                _ => Err(env),
            };
        }
        let shape = self.render(init);
        let found = match self.object_node(init, scope) {
            Some((object, object_scope, via)) => {
                self.object_path_value(object, object_scope, path, via, depth)
            }
            None => Some(Err(Unresolved::new("attribute", shape.clone()))),
        };
        match (found, fallback) {
            // Not set: the pattern's default applies.
            (None, Some(fallback)) => self.resolve_value(fallback, scope, depth + 1),
            (None, None) => Err(Unresolved::new("attribute", shape)),
            (Some(Err(unresolved)), Some(fallback)) if unresolved.reason != "credential" => {
                // `const { model = "gpt-4o" } = options`: the default of a
                // lookup this file cannot read.
                match self.resolve_value(fallback, scope, depth + 1) {
                    Ok(mut resolved) => {
                        resolved.origin = ValueOrigin::EnvDefault;
                        resolved.caveats.push(unresolved);
                        Ok(resolved)
                    }
                    Err(_) => Err(unresolved),
                }
            }
            (Some(result), _) => result,
        }
    }

    /// The value at `path` in an object literal, or `None` when the
    /// property is not set (a default may apply).
    fn object_path_value(
        &mut self,
        object: Node<'t>,
        scope: usize,
        path: &[Step<'t>],
        via: Option<(String, ValueOrigin)>,
        depth: u32,
    ) -> Option<Result<Resolved, Unresolved>> {
        let mut current = object;
        let mut current_scope = scope;
        let mut label = via.as_ref().map(|(name, _)| name.clone());
        let origin = via
            .as_ref()
            .map_or(ValueOrigin::Literal, |(_, origin)| *origin);
        for (at, step) in path.iter().enumerate() {
            let last = at + 1 == path.len();
            let shape = label.clone().unwrap_or_else(|| "{...}".to_string());
            let read = self.object_entries(current, current_scope);
            let key = match step {
                Step::Key(key) => *key,
                Step::Index(_) | Step::Other => {
                    return Some(Err(Unresolved::new("subscript", shape)));
                }
            };
            if is_secret_key(key) {
                return Some(Err(Unresolved::new("credential", shape)));
            }
            label = label.map(|l| clip(&format!("{l}.{key}")));
            let found = read.find(key);
            let overridden = found.overridden;
            let (value, value_scope, entry_via) = match found.pair {
                Some(pair) => pair,
                None if found.maybe => return Some(Err(Unresolved::new("splat", shape))),
                None => return None,
            };
            if let Some((name, _)) = &entry_via {
                label = Some(clip(&format!("{name}.{key}")));
            }
            if overridden {
                let candidate = self.resolve_value(value, value_scope, depth + 1).ok();
                return Some(Err(Unresolved::new("splat", shape)
                    .with_candidates(candidate.into_iter().collect(), false)));
            }
            if last {
                let result = self.resolve_value(value, value_scope, depth + 1);
                return Some(match &label {
                    Some(label) => result
                        .map(|r| r.constant(label, origin))
                        .map_err(|u| u.via(label)),
                    None => result,
                });
            }
            let inner = unwrap_expression(value);
            match self.object_node(inner, value_scope) {
                Some((nested, nested_scope, _)) => {
                    current = nested;
                    current_scope = nested_scope;
                }
                None => return Some(Err(Unresolved::new("attribute", self.render(value)))),
            }
        }
        Some(Err(Unresolved::new("attribute", label.unwrap_or_default())))
    }

    /// `process.env.MODEL`, `import.meta.env.MODEL`, `Bun.env.MODEL`,
    /// `Deno.env.get("MODEL")`, or a name destructured from one of them: the
    /// shape to report, with the variable's name left out.
    fn env_lookup(&self, node: Node<'t>, scope: usize) -> Option<String> {
        let node = unwrap_expression(node);
        match node.kind() {
            "member_expression" | "subscript_expression" => {
                let object = node.child_by_field_name("object")?;
                self.env_object(object, scope)
                    .then(|| format!("{}[...]", self.env_shape(object)))
            }
            "call_expression" => {
                let function = node.child_by_field_name("function")?;
                let deno = function.kind() == "member_expression"
                    && self.render(function) == "Deno.env.get"
                    && self.lookup(scope, "Deno").is_none();
                deno.then(|| "Deno.env.get(...)".to_string())
            }
            "identifier" => {
                let b = self.lookup(scope, self.text(node))?;
                let binding = &self.bindings[b];
                (binding.writes.is_empty()
                    && !binding.writes_cut
                    && binding
                        .init
                        .is_some_and(|init| self.env_path(init, binding.init_scope, &binding.path)))
                .then(|| "process.env[...]".to_string())
            }
            _ => None,
        }
    }

    /// How an environment object reads in a gap.
    fn env_shape(&self, object: Node<'t>) -> String {
        let object = unwrap_expression(object);
        match object.kind() {
            "member_expression" | "subscript_expression" => {
                match object.child_by_field_name("object").map(unwrap_expression) {
                    Some(root)
                        if root.kind() == "member_expression"
                            && root
                                .child_by_field_name("object")
                                .is_some_and(|o| o.kind() == "import") =>
                    {
                        "import.meta.env".to_string()
                    }
                    Some(root) if root.kind() == "identifier" && self.text(root) == "Bun" => {
                        "Bun.env".to_string()
                    }
                    _ => "process.env".to_string(),
                }
            }
            "call_expression" => "Deno.env.toObject()".to_string(),
            _ => clip(self.text(object)),
        }
    }

    /// `MODELS.fast`, `MODELS["fast"]`, `MODELS[key]`, `Model.Fast`,
    /// `this.model`, `Bot.MODEL`.
    fn member_value(
        &mut self,
        node: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        if let Some(shape) = self.env_lookup(node, scope) {
            return Err(Unresolved::new("env_var", shape));
        }
        let shape = self.render(node);
        // The static path from the root, and the computed key, if any.
        let mut steps = Vec::new();
        let mut current = node;
        loop {
            match current.kind() {
                "member_expression" => {
                    let property = current.child_by_field_name("property");
                    steps.push(property.map_or(Step::Other, |p| Step::Key(self.text(p))));
                    current = match current.child_by_field_name("object") {
                        Some(object) => object,
                        None => break,
                    };
                }
                "subscript_expression" => {
                    let key = current
                        .child_by_field_name("index")
                        .and_then(|i| static_key(i, self.src));
                    steps.push(key.map_or(Step::Other, Step::Key));
                    current = match current.child_by_field_name("object") {
                        Some(object) => object,
                        None => break,
                    };
                }
                "non_null_expression" | "parenthesized_expression" => {
                    current = match current.named_child(0) {
                        Some(inner) => inner,
                        None => break,
                    };
                }
                _ => break,
            }
            if steps.len() > MAX_PATH {
                return Err(Unresolved::new("attribute", shape));
            }
        }
        steps.reverse();
        if steps
            .iter()
            .any(|s| matches!(s, Step::Key(key) if is_secret_key(key)))
        {
            return Err(Unresolved::new("credential", shape));
        }
        let root = current;
        match (root.kind(), steps.as_slice()) {
            ("this", [Step::Key(name)]) => {
                let found = match self.scopes[scope].this {
                    This::Instance(class) => self.prop_value(class, false, name, true, depth),
                    This::Static(class) => self.prop_value(class, true, name, true, depth),
                    This::Other => return Err(Unresolved::new("attribute", shape)),
                };
                match found {
                    // Called with another `this`, the method reads another
                    // object's property.
                    Ok(mut resolved) if self.this_rebound(scope) => {
                        resolved.caveats.push(Unresolved::new("attribute", shape));
                        Ok(resolved)
                    }
                    found => found,
                }
            }
            ("identifier", _) => {
                let Some(b) = self.lookup(scope, self.text(root)) else {
                    return Err(Unresolved::new("attribute", shape));
                };
                if let (Some(class), [Step::Key(name)]) = (self.bindings[b].class, steps.as_slice())
                {
                    return self.prop_value(class, true, name, false, depth);
                }
                if self.bindings[b].decl == Decl::Enum {
                    if let [Step::Key(member)] = steps.as_slice() {
                        return self.enum_value(b, member, depth);
                    }
                }
                if let Some((object, object_scope)) = self.readonly_object(b) {
                    let origin = constant_origin(self.scopes[self.bindings[b].scope].kind);
                    let via = Some((clip(self.text(root)), origin));
                    if let Some(at) = steps.iter().position(|s| *s == Step::Other) {
                        return self.computed_entry(
                            object,
                            object_scope,
                            &steps[..at],
                            via,
                            &shape,
                            depth,
                        );
                    }
                    return self
                        .object_path_value(object, object_scope, &steps, via, depth)
                        .unwrap_or_else(|| Err(Unresolved::new("attribute", shape)));
                }
                Err(Unresolved::new("attribute", shape))
            }
            _ => Err(Unresolved::new("attribute", shape)),
        }
    }

    /// `MODELS[key]`: a `subscript` gap listing the entries the key may pick.
    fn computed_entry(
        &mut self,
        object: Node<'t>,
        scope: usize,
        prefix: &[Step<'t>],
        via: Option<(String, ValueOrigin)>,
        shape: &str,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        let mut current = object;
        let mut current_scope = scope;
        for step in prefix {
            let Step::Key(key) = step else {
                return Err(Unresolved::new("subscript", shape.to_string()));
            };
            let read = self.object_entries(current, current_scope);
            let Lookup::Found(value, false, value_scope) = read.lookup(key) else {
                return Err(Unresolved::new("subscript", shape.to_string()));
            };
            match self.object_node(value, value_scope) {
                Some((nested, nested_scope, _)) => {
                    current = nested;
                    current_scope = nested_scope;
                }
                None => return Err(Unresolved::new("subscript", shape.to_string())),
            }
        }
        let read = self.object_entries(current, current_scope);
        // A key written twice counts by its last entry.
        let (entries, cut, opaque) = read.flat(MAX_CANDIDATES + 1);
        let mut found = Vec::new();
        let mut complete = !cut && !opaque && entries.len() <= MAX_CANDIDATES;
        for entry in entries.into_iter().take(MAX_CANDIDATES) {
            match self.resolve_value(entry.value, entry.scope, depth + 1) {
                Ok(resolved) => {
                    complete &= resolved.is_certain();
                    let name = entry.via.as_ref().or(via.as_ref());
                    let resolved = match name {
                        Some((name, origin)) => resolved.constant(&format!("{name}[...]"), *origin),
                        None => resolved,
                    };
                    complete &= push_candidate(&mut found, resolved);
                }
                Err(_) => complete = false,
            }
        }
        Err(Unresolved::new("subscript", shape.to_string()).with_candidates(found, complete))
    }

    /// A member of a same-file `enum` with a string value.
    fn enum_value(&mut self, b: usize, member: &str, depth: u32) -> Result<Resolved, Unresolved> {
        let binding = &self.bindings[b];
        let name = format!("{}.{member}", binding.name);
        let origin = constant_origin(self.scopes[binding.scope].kind);
        let uses = binding.uses.clone();
        // An enum is an object: a write into it, or handing it to code that
        // may write it (`(Model as any).Chat = ...`, `mutate(Model)`).
        if !binding.writes.is_empty()
            || binding.writes_cut
            || uses.mutated
            || uses.escapes
            || uses.member_calls
            || uses.passed_cut
        {
            return Err(Unresolved::new("reassigned", name));
        }
        for (call, call_scope) in &uses.passed_to {
            if !self.harmless_call(*call, *call_scope) {
                return Err(Unresolved::new("reassigned", name));
            }
        }
        let init_scope = self.bindings[b].init_scope;
        let members = self.enum_members(b);
        let Some(value) = members.get(member).copied().flatten() else {
            return Err(Unresolved::new("non_literal", name));
        };
        // Inside the enum, its members shadow outer names: `Default = Fast`
        // is the member `Fast`.
        let inner = unwrap_expression(value);
        if inner.kind() == "identifier" && members.contains_key(self.text(inner)) {
            if depth > MAX_DEPTH {
                return Err(Unresolved::new("non_literal", name));
            }
            let other = self.text(inner);
            return self
                .enum_value(b, other, depth + 1)
                .map(|r| r.constant(&name, origin))
                .map_err(|u| u.via(&name));
        }
        if self.names_any(value, &members) {
            return Err(Unresolved::new("non_literal", name));
        }
        self.resolve_value(value, init_scope, depth + 1)
            .map(|r| r.constant(&name, origin))
            .map_err(|u| u.via(&name))
    }

    /// An enum's members, by name, with the value each is given: read once
    /// per enum.
    fn enum_members(&mut self, b: usize) -> EnumMembers<'t> {
        if let Some(members) = self.enum_memo.get(&b) {
            return Rc::clone(members);
        }
        let mut members = HashMap::new();
        if let Some(body) = self.bindings[b].init {
            let mut cursor = body.walk();
            for m in body.named_children(&mut cursor) {
                let (key, value) = match m.kind() {
                    "enum_assignment" => (
                        m.child_by_field_name("name"),
                        m.child_by_field_name("value"),
                    ),
                    "property_identifier" => (Some(m), None),
                    _ => continue,
                };
                if let Some(key) = key.and_then(|k| self.member_name(k)) {
                    members.insert(key, value);
                }
            }
        }
        let members = Rc::new(members);
        self.enum_memo.insert(b, Rc::clone(&members));
        members
    }

    /// Whether an identifier in `node` is one of `names`.
    fn names_any(&self, node: Node<'t>, names: &HashMap<&'t str, Option<Node<'t>>>) -> bool {
        let mut work = vec![node];
        let mut seen = 0;
        while let Some(n) = work.pop() {
            seen += 1;
            if seen > MAX_NESTED_ENTRIES {
                return true;
            }
            if n.kind() == "identifier" && names.contains_key(self.text(n)) {
                return true;
            }
            let mut cursor = n.walk();
            work.extend(n.named_children(&mut cursor));
        }
        false
    }

    /// `left ?? right`, `left || right`: the right side is the fallback of
    /// an environment or property lookup on the left.
    fn binary_value(
        &mut self,
        node: Node<'t>,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        let shape = self.render(node);
        let op = operator(node);
        let (Some(left), Some(right), true) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
            matches!(op, Some("??" | "||")),
        ) else {
            return Err(Unresolved::new("non_literal", shape));
        };
        if let Some(env) = self.env_lookup(left, scope) {
            return match self.resolve_value(right, scope, depth + 1) {
                Ok(mut resolved) => {
                    resolved.origin = ValueOrigin::EnvDefault;
                    resolved.caveats.push(Unresolved::new("env_var", env));
                    Ok(resolved)
                }
                Err(unresolved) if unresolved.reason == "credential" => Err(unresolved),
                Err(_) => Err(Unresolved::new("env_var", env)),
            };
        }
        match self.resolve_value(left, scope, depth + 1) {
            // A string on the left: `??` keeps it; `||` keeps it unless empty.
            Ok(resolved) if resolved.caveats.is_empty() => {
                if op == Some("??") || !resolved.value.is_empty() {
                    Ok(resolved)
                } else {
                    self.resolve_value(right, scope, depth + 1)
                }
            }
            Ok(resolved) => {
                let mut found = vec![resolved];
                if let Ok(other) = self.resolve_value(right, scope, depth + 1) {
                    push_candidate(&mut found, other);
                }
                Err(Unresolved::new("conditional", shape).with_candidates(found, false))
            }
            Err(unresolved) if unresolved.reason == "credential" => Err(unresolved),
            Err(unresolved) => {
                // `options.model ?? "gpt-4o"`, `model || "gpt-4o"`: the
                // literal fallback of a lookup.
                let lookup = matches!(
                    unwrap_expression(left).kind(),
                    "member_expression" | "subscript_expression"
                ) || unresolved.reason == "parameter";
                match self.resolve_value(right, scope, depth + 1) {
                    Ok(mut resolved) if lookup => {
                        resolved.origin = ValueOrigin::EnvDefault;
                        resolved.caveats.push(unresolved);
                        Ok(resolved)
                    }
                    Ok(resolved) => Err(Unresolved::new("conditional", shape)
                        .with_candidates(vec![resolved], false)),
                    Err(other) if other.reason == "credential" => Err(other),
                    Err(_) => Err(Unresolved::new("conditional", shape)),
                }
            }
        }
    }

    /// A class property's value: its one write, resolved where it is
    /// written. Read through `this` in a class another module may extend,
    /// it keeps a `subclass_override` gap.
    fn prop_value(
        &mut self,
        class: usize,
        is_static: bool,
        name: &'t str,
        through_this: bool,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        let shape = if through_this {
            format!("this.{name}")
        } else {
            format!("{}.{name}", self.classes[class].name.unwrap_or("<class>"))
        };
        if is_secret_key(name) {
            return Err(Unresolved::new("credential", shape));
        }
        let writes = match self.prop_lookup(class, is_static, through_this, name) {
            Err(feature) => return Err(Unresolved::dynamic(feature).via(&shape)),
            Ok(writes) => writes,
        };
        match writes.as_slice() {
            [] => Err(Unresolved::new("attribute", shape)),
            [PropWrite {
                value: Some(value),
                scope: write_scope,
                ..
            }] => {
                let mut resolved = self
                    .resolve_value(*value, *write_scope, depth + 1)
                    .map(|r| r.constant(&shape, ValueOrigin::ClassConstant))
                    .map_err(|u| u.via(&shape))?;
                if through_this && self.class_public(class) {
                    resolved
                        .caveats
                        .push(Unresolved::new("subclass_override", shape.clone()).via(&shape));
                }
                Ok(resolved)
            }
            _ => {
                let mut found = Vec::new();
                let mut complete = writes.len() < MAX_WRITES_READ;
                for write in &writes {
                    let Some(value) = write.value else {
                        complete = false;
                        continue;
                    };
                    match self.resolve_value(value, write.scope, depth + 1) {
                        Ok(resolved) => {
                            complete &= resolved.is_certain();
                            complete &= push_candidate(
                                &mut found,
                                resolved.constant(&shape, ValueOrigin::ClassConstant),
                            );
                        }
                        Err(unresolved) if unresolved.reason == "credential" => {
                            return Err(unresolved.via(&shape))
                        }
                        Err(_) => complete = false,
                    }
                }
                Err(Unresolved::new("reassigned", shape).with_candidates(found, complete))
            }
        }
    }

    /// A parameter's value: its default, confirmed only when every call of
    /// its function is in this file and none passes another value; else the
    /// default with a gap, or a `parameter` gap listing what callers pass.
    fn param_value(&mut self, b: usize, depth: u32) -> Result<Resolved, Unresolved> {
        let binding = &self.bindings[b];
        let name = binding.name;
        let Some((func, index)) = binding.param else {
            return Err(Unresolved::new("parameter", name.to_string()));
        };
        let path = Rc::clone(&binding.path);
        let (default, fallback, init_scope) = (binding.init, binding.fallback, binding.init_scope);
        let (writes, writes_cut) = (binding.writes.clone(), binding.writes_cut);
        // The value when a caller passes nothing: the parameter's default,
        // read along the destructuring path, else the pattern's default.
        let default_value = if path.is_empty() {
            default.map(|d| self.resolve_value(d, init_scope, depth + 1))
        } else {
            let from_object = match default.and_then(|d| self.object_node(d, init_scope)) {
                Some((object, object_scope, _)) => {
                    self.object_path_value(object, object_scope, &path, None, depth + 1)
                }
                None => None,
            };
            from_object.or_else(|| fallback.map(|f| self.resolve_value(f, init_scope, depth + 1)))
        };
        let callers = self.callers(func);
        let mut overrides = Vec::new();
        let mut complete = callers.sites.len() <= MAX_CALLERS_READ;
        let mut omitted = false;
        for (call, scope) in callers.sites.iter().take(MAX_CALLERS_READ) {
            match self.param_argument(*call, *scope, index, &path) {
                Argument::Missing => omitted = true,
                Argument::Unknown => complete = false,
                Argument::Expr(expr, expr_scope) => {
                    match self.resolve_value(expr, expr_scope, depth + 1) {
                        Ok(resolved) => {
                            complete &= resolved.is_certain();
                            complete &= push_candidate(&mut overrides, resolved);
                        }
                        Err(unresolved) if unresolved.reason == "credential" => {
                            return Err(unresolved.via(name))
                        }
                        Err(_) => complete = false,
                    }
                }
            }
        }
        // Written in the function (`model = "x"`, `var model = "x"`,
        // `arguments[0] = x` in a sloppy function): what it held on entry,
        // and each write.
        if !writes.is_empty() || writes_cut {
            let mut found = Vec::new();
            let mut complete = callers.complete && complete && !writes_cut;
            match default_value {
                Some(Ok(resolved)) => {
                    complete &= resolved.is_certain();
                    complete &= push_candidate(&mut found, resolved);
                }
                Some(Err(unresolved)) if unresolved.reason == "credential" => {
                    return Err(unresolved.via(name))
                }
                Some(Err(_)) => complete = false,
                None => complete &= !omitted,
            }
            for resolved in overrides {
                complete &= push_candidate(&mut found, resolved);
            }
            for write in &writes {
                let Some(value) = write.value else {
                    complete = false;
                    continue;
                };
                match self.resolve_value(value, write.scope, depth + 1) {
                    Ok(resolved) => {
                        complete &= resolved.is_certain();
                        complete &= push_candidate(&mut found, resolved);
                    }
                    Err(unresolved) if unresolved.reason == "credential" => {
                        return Err(unresolved.via(name))
                    }
                    Err(_) => complete = false,
                }
            }
            return Err(
                Unresolved::new("reassigned", name.to_string()).with_candidates(found, complete)
            );
        }
        match default_value {
            Some(Ok(mut resolved)) => {
                resolved.origin = ValueOrigin::ParameterDefault;
                resolved.name = Some(clip(name));
                if callers.complete {
                    if !overrides.is_empty() || !complete {
                        resolved.caveats.push(
                            Unresolved::new("parameter_overridden", name.to_string())
                                .with_candidates(overrides, complete),
                        );
                    }
                } else if callers.exported {
                    resolved.caveats.push(
                        Unresolved::new("parameter_default", name.to_string())
                            .with_candidates(overrides, false),
                    );
                } else {
                    resolved.caveats.push(
                        Unresolved::new("unseen_callers", name.to_string())
                            .with_candidates(overrides, false),
                    );
                }
                Ok(resolved)
            }
            Some(Err(unresolved)) => Err(unresolved.via(name)),
            None => Err(Unresolved::new("parameter", name.to_string())
                .with_candidates(overrides, callers.complete && complete && !omitted)),
        }
    }
}

// ============================================================================
// Pass 2: analyze
// ============================================================================

/// What a recorded call goes through.
struct Target {
    pkg: Pkg,
    member: String,
    receiver: Receiver,
    client: Option<Client>,
}

/// The call a value or gap is reported at.
#[derive(Clone, Copy)]
struct Site<'a, 't> {
    sdk: Sdk,
    call: Node<'t>,
    chain: &'a str,
    scope: usize,
    /// A gap for an unresolved value is reported here: a request, a
    /// model-bearing call, or any call on a client or SDK module.
    gap_worthy: bool,
}

/// A model value found inside a nested object or array of the options.
struct NestedValue {
    path: String,
    /// Found in a literal passed by position, not in the options object.
    positional: bool,
    resolved: Resolved,
}

/// A container literal a search for nested model keys still has to read.
struct NestedWork<'t> {
    node: Node<'t>,
    scope: usize,
    /// The key path so far (`requests.params.`).
    prefix: String,
    /// A spread or computed key after an enclosing entry may replace it.
    replaced: bool,
    depth: usize,
    positional: bool,
}

impl<'t> Ctx<'t> {
    fn analyze(&mut self) {
        if let Some(node) = self.import_cut {
            self.truncate(
                node,
                0,
                format!(
                    "this file binds more than {MAX_FILE_IMPORTS} imported names; its calls are not analyzed"
                ),
            );
            return;
        }
        for node in self.scope_cuts.clone() {
            self.truncate(
                node,
                0,
                "code nested more than 64 scopes deep is not analyzed".to_string(),
            );
        }
        self.settle_member_writes();
        self.settle_special_calls();
        self.settle_sdk_writes();
        self.record_required_imports();
        let calls = std::mem::take(&mut self.calls);
        let mut cut = false;
        for (call, scope) in &calls {
            if self.records() >= MAX_FILE_RECORDS {
                self.truncate_file(*call, *scope);
                cut = true;
                break;
            }
            self.analyze_call(*call, *scope);
        }
        self.calls = calls;
        if cut {
            return;
        }
        let values = std::mem::take(&mut self.member_values);
        for (reference, scope) in &values {
            if self.records() >= MAX_FILE_RECORDS {
                self.truncate_file(*reference, *scope);
                break;
            }
            self.analyze_reference(*reference, *scope);
        }
        self.member_values = values;
    }

    /// Imports, calls, gaps and clients reported so far.
    fn records(&self) -> usize {
        self.out.imports.len() + self.out.calls.len() + self.out.gaps.len() + self.out.clients.len()
    }

    /// Stops the analysis at `node`: the rest of the file is one gap.
    fn truncate_file(&mut self, node: Node<'t>, scope: usize) {
        self.truncate(
            node,
            scope,
            format!(
                "this file reports more than {MAX_FILE_RECORDS} imports, calls, clients and gaps; nothing from this line on is analyzed"
            ),
        );
    }

    /// A `truncated` gap standing for part of a file that is not analyzed.
    fn truncate(&mut self, node: Node<'t>, scope: usize, detail: String) {
        self.out.gaps.push(UsageGap {
            kind: GapKind::UnresolvedModel,
            sdk: None,
            file: self.file.clone(),
            span: Some(self.span(node)),
            symbol: self.symbol(scope),
            chain: None,
            key: None,
            reason: "truncated".to_string(),
            detail,
            values: Vec::new(),
            values_complete: false,
            args: None,
        });
    }

    fn analyze_call(&mut self, call: Node<'t>, scope: usize) {
        if call.kind() == "new_expression" {
            let Some(constructor) = call.child_by_field_name("constructor") else {
                return;
            };
            match self.eval(constructor, scope, 0) {
                // Records the construction.
                Val::Class(_) | Val::Module { entry: true, .. } => {
                    self.eval(call, scope, 0);
                }
                // openai v3's `new Configuration({ basePath })`: an SDK
                // object (it routes the `OpenAIApi` built from it).
                Val::V3ConfigClass => {
                    let target = Target {
                        pkg: Pkg::Openai,
                        member: V3_CONFIGURATION.to_string(),
                        receiver: Receiver::Symbol,
                        client: None,
                    };
                    self.record_call(call, constructor, scope, target);
                }
                // `new OpenAIRealtimeWS({ model })`: an SDK object built from
                // a model.
                Val::Symbol { pkg, path } => {
                    let target = Target {
                        pkg,
                        member: path.join("."),
                        receiver: Receiver::Symbol,
                        client: None,
                    };
                    self.record_call(call, constructor, scope, target);
                }
                _ => {}
            }
            return;
        }
        let Some(function) = call.child_by_field_name("function") else {
            return;
        };
        if function.kind() == "import" || self.module_call(call, scope).is_some() {
            return;
        }
        // `client[name](...)`: an SDK method reached under a run-time name.
        if function.kind() == "subscript_expression"
            && function
                .child_by_field_name("index")
                .and_then(|i| static_key(i, self.src))
                .is_none()
        {
            if let Some(object) = function.child_by_field_name("object") {
                let base = self.eval(object, scope, 0);
                if base.is_tracked() {
                    let rendered = self.render(function);
                    self.out.gaps.push(UsageGap {
                        kind: GapKind::DynamicDispatch,
                        sdk: base.pkg().map(Pkg::sdk),
                        file: self.file.clone(),
                        span: Some(self.span(call)),
                        symbol: self.symbol(scope),
                        chain: Some(rendered.clone()),
                        key: None,
                        reason: "computed_member".to_string(),
                        detail: format!(
                            "`{rendered}` reaches an SDK member under a name known only at run time; calls through it are not tracked"
                        ),
                        values: Vec::new(),
                        values_complete: false,
                        args: None,
                    });
                    return;
                }
            }
        }
        match self.eval(function, scope, 0) {
            Val::Client { client, path } if !path.is_empty() => {
                // `create.bind(...)`, `create.call(...)`: the request method
                // used as a value.
                if let [.., method, last] = path.as_slice() {
                    if matches!(last.as_str(), "bind" | "call" | "apply")
                        && REQUEST_METHODS.contains(&method.as_str())
                    {
                        if let Some(object) = function.child_by_field_name("object") {
                            self.method_reference(object, scope, Some(client.pkg));
                        }
                        return;
                    }
                }
                let mut client = client;
                // `genAI.getGenerativeModel({ model }, { baseUrl })` builds a
                // model routed by its request options.
                if client.family == Family::LegacyRoot
                    && matches!(
                        path.as_slice(),
                        [m] if m == "getGenerativeModel" || m == "getGenerativeModelFromCachedContent"
                    )
                {
                    if let Val::Client { client: model, .. } =
                        self.legacy_model(client.clone(), call, scope)
                    {
                        client = model;
                    }
                }
                if let Some(option) = self.request_endpoint(call, scope, &client, &path) {
                    // The model or chat it is sent through was reported where
                    // it was built.
                    let feature = Feature {
                        what: "request options that may name another endpoint",
                        line: self.line(call),
                    };
                    match client.family {
                        Family::LegacyModel => self.legacy_request_endpoint.get_or_insert(feature),
                        _ => self.genai_request_endpoint.get_or_insert(feature),
                    };
                    client.routing = Routing::Unknown;
                    client.caveat = Some(ClientCaveat::EndpointOption { option });
                }
                let target = Target {
                    pkg: client.pkg,
                    member: path.join("."),
                    receiver: Receiver::Client,
                    client: Some(client),
                };
                self.record_call(call, function, scope, target);
            }
            Val::Symbol { pkg, path } => {
                let receiver = match function.child_by_field_name("object") {
                    Some(object)
                        if function.kind() == "member_expression"
                            && matches!(self.eval(object, scope, 0), Val::Module { .. }) =>
                    {
                        Receiver::Module
                    }
                    _ => Receiver::Symbol,
                };
                let target = Target {
                    pkg,
                    member: path.join("."),
                    receiver,
                    client: None,
                };
                self.record_call(call, function, scope, target);
            }
            Val::Result(pkg) => {
                self.check_unknown_receiver(call, function, scope, Origin::SdkResult { pkg })
            }
            Val::Untracked(origin) => self.check_unknown_receiver(call, function, scope, origin),
            _ => {}
        }
    }

    /// An option of one request that may send it to another endpoint: the
    /// legacy Gemini SDK's request options with a `baseUrl`
    /// (`model.generateContent(request, { baseUrl })`), and
    /// `@google/genai`'s `config.httpOptions`.
    fn request_endpoint(
        &mut self,
        call: Node<'t>,
        scope: usize,
        client: &Client,
        path: &[String],
    ) -> Option<&'static str> {
        let last = path.last()?.as_str();
        // Whether an object (written at the call or bound to a name) may set
        // `key`.
        let may_set = |cx: &mut Self, value: Node<'t>, value_scope: usize, key: &str| {
            let value = unwrap_expression(value);
            if matches!(value.kind(), "null" | "undefined") {
                return false;
            }
            match cx.object_node(value, value_scope) {
                Some((object, object_scope, _)) => {
                    let read = cx.object_entries(object, object_scope);
                    !matches!(read.lookup(key), Lookup::Missing)
                }
                None => true,
            }
        };
        match client.family {
            Family::LegacyModel if LEGACY_REQUEST_OPTIONS.contains(&last) => {
                let options = nth_argument(call, 1)?;
                may_set(self, options, scope, "baseUrl")
                    .then_some("request options with a `baseUrl`")
            }
            Family::Genai if is_request(last) => {
                let options = self.options(call, scope, 0);
                if !options.present {
                    return None;
                }
                let (config, config_scope) = match options.lookup("config") {
                    Lookup::Missing => return None,
                    Lookup::Opaque => return Some("a request `config` built elsewhere"),
                    Lookup::Found(value, _, value_scope) => (value, value_scope),
                };
                let config = unwrap_expression(config);
                if matches!(config.kind(), "null" | "undefined") {
                    return None;
                }
                let Some((object, object_scope, _)) = self.object_node(config, config_scope) else {
                    return Some("a request `config` built elsewhere");
                };
                let read = self.object_entries(object, object_scope);
                match read.lookup("httpOptions") {
                    Lookup::Missing => None,
                    Lookup::Opaque => Some("a request `config` built elsewhere"),
                    Lookup::Found(http, _, http_scope) => {
                        may_set(self, http, http_scope, "baseUrl")
                            .then_some("a request's `httpOptions` that may name a `baseUrl`")
                    }
                }
            }
            _ => None,
        }
    }

    /// The client's routing as a call sees it: a member of the SDK replaced
    /// or mocked, or a `baseURL` assigned after the client was built, makes
    /// it unknown.
    fn reroute(&self, client: &Client) -> Client {
        let mut client = client.clone();
        let direct = matches!(
            client.family,
            Family::OpenaiDirect | Family::AnthropicDirect | Family::OpenaiV3
        );
        if let Some(feature) = self.sdk_patched.get(&client.pkg) {
            client.routing = Routing::Unknown;
            client.caveat = Some(ClientCaveat::Dynamic(*feature));
        } else if direct && client.site.is_some_and(|s| self.rerouted.contains(&s)) {
            client.routing = Routing::Unknown;
            client.caveat = Some(ClientCaveat::BaseUrlAssigned);
        } else if let (true, Some(feature)) = (direct, self.rerouted_any) {
            client.routing = Routing::Unknown;
            client.caveat.get_or_insert(ClientCaveat::Dynamic(feature));
        } else if client.family == Family::Genai && self.genai_base_urls.is_some() {
            // `setDefaultBaseUrls` found after the client was evaluated.
            client.routing = Routing::Unknown;
            client.caveat.get_or_insert(ClientCaveat::EndpointOption {
                option: "`setDefaultBaseUrls`, which this file calls",
            });
        }
        if self.global_fetch
            && through_global_fetch(client.routing, client.family == Family::OpenaiV3)
        {
            client.routing = Routing::Unknown;
            client.caveat.get_or_insert(ClientCaveat::EndpointOption {
                option: "the global `fetch`, which this file may replace",
            });
        }
        client
    }

    fn record_call(&mut self, call: Node<'t>, function: Node<'t>, scope: usize, target: Target) {
        let chain = self.render(function);
        let span = self.span(call);
        let symbol = self.symbol(scope);
        let sdk = target.pkg.sdk();
        let member = clip_owned(target.member);
        let request = is_request(&member);
        let model_bearing = is_model_bearing(&member);
        let on_sdk_object = matches!(target.receiver, Receiver::Client | Receiver::Module);
        let site = Site {
            sdk,
            call,
            chain: &chain,
            scope,
            gap_worthy: request || model_bearing || on_sdk_object,
        };
        let gaps_before = self.out.gaps.len();
        let mut values = Vec::new();
        let mut keyed = false;

        // openai v3 `createTranscription(file, "whisper-1")`: the model is
        // positional.
        let positional_model = V3_POSITIONAL_MODEL
            .iter()
            .find(|(name, _)| {
                member == *name
                    || member
                        .strip_suffix(name)
                        .is_some_and(|head| head.ends_with('.'))
            })
            .map(|(_, index)| *index)
            .filter(|_| target.pkg == Pkg::Openai);
        let options_index = usize::from(
            target.pkg == Pkg::Openai
                && PATH_PARAM_METHODS.iter().any(|m| member_is(&member, m))
                && nth_argument(call, 1).is_some(),
        );
        let options = match positional_model {
            Some(_) => Options::default(),
            None => {
                let options = self.options(call, scope, options_index);
                // A call that takes no model (`model.generateContent(prompt)`,
                // `chat.sendMessage(text)`) is passed content, not options,
                // unless an object is written at the call.
                if !model_bearing && options.opaque && !options.literal {
                    Options::default()
                } else {
                    options
                }
            }
        };
        let mut opaque = options.opaque;
        if let Some(index) = positional_model {
            match self.argument_at(call, scope, index) {
                Argument::Expr(expr, expr_scope) => {
                    keyed = true;
                    let result = self.resolve_value(expr, expr_scope, 0);
                    let site = Site {
                        gap_worthy: true,
                        ..site
                    };
                    self.take_value(&site, "model", true, result, &mut values);
                }
                Argument::Unknown => opaque = true,
                Argument::Missing => {}
            }
        }

        for key in super::VALUE_KEYS {
            let found = options.find(key);
            let overridden = found.overridden;
            let Some((value, value_scope, via)) = found.pair else {
                continue;
            };
            if !is_value_key(key) {
                continue;
            }
            keyed = true;
            let label = via
                .as_ref()
                .map(|(name, origin)| (clip(&format!("{name}.{key}")), *origin));
            let mut result = self.resolve_value(value, value_scope, 0);
            if let Some((label, origin)) = &label {
                result = result
                    .map(|r| r.constant(label, *origin))
                    .map_err(|u| u.via(label));
            }
            // `{ model: "gpt-4o", ...overrides }`: the spread (or a computed
            // key) written after the entry may replace it, so the entry is
            // a candidate, not the value.
            if overridden {
                let candidates: Vec<Resolved> = result.ok().into_iter().collect();
                result = Err(Unresolved::new("splat", options.shape.clone())
                    .with_candidates(candidates, false));
            }
            self.take_value(&site, key, false, result, &mut values);
        }

        let positional_literals = self.positional_literals(call, &options, options_index);
        let (nested, nested_overridden, nested_cut) =
            self.nested_values(&options, &positional_literals, scope);
        self.take_nested(&site, nested, nested_cut, &mut values);
        if site.gap_worthy {
            let mut seen = HashSet::new();
            for path in nested_overridden.into_iter().take(MAX_NESTED_VALUES) {
                if seen.insert(path.clone()) {
                    let unresolved = Unresolved::new("splat", "{...}".to_string());
                    self.unresolved_gap(&site, &path, &unresolved, None);
                }
            }
        }

        let spread_args = self.has_spread_argument(call);
        if !keyed && (opaque || spread_args) && model_bearing {
            let unresolved = Unresolved::new("splat", "...".to_string());
            self.unresolved_gap(&site, "model", &unresolved, None);
        } else if !keyed && model_bearing {
            // No model at all: the SDK's or the server's default applies,
            // and it may be one that retires.
            let unresolved = Unresolved::new("missing_model", String::new());
            self.unresolved_gap(&site, "model", &unresolved, None);
        }

        let client = target.client.as_ref().map(|c| self.reroute(c));
        let routing = client.as_ref().map_or(Routing::Unknown, |c| c.routing);
        if let Some(client) = &client {
            self.routing_gap(&site, span, client);
        }

        // The same value or gap reached twice (one object spread twice) is
        // listed once.
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

        let positional = self.positional_count(call, options.present);
        self.out.calls.push(SdkCall {
            sdk,
            package: target.pkg.package().to_string(),
            chain,
            member,
            request,
            receiver: target.receiver,
            routing,
            client_line: client.as_ref().and_then(|c| c.line),
            file: self.file.clone(),
            span,
            symbol,
            splat: options.unwritten() || spread_args,
            kwargs: options.keys,
            positional,
            // The extractor does not record what each spread holds:
            // `splat` alone says that some arguments are unread.
            spreads: Vec::new(),
            values,
        });
    }

    /// Arguments passed by position, the options object and spreads
    /// excluded.
    fn positional_count(&self, call: Node<'t>, options: bool) -> u32 {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return 0;
        };
        let mut cursor = arguments.walk();
        let count = arguments
            .named_children(&mut cursor)
            .filter(|a| !matches!(a.kind(), "comment" | "spread_element"))
            .count() as u32;
        count.saturating_sub(u32::from(options))
    }

    fn has_spread_argument(&self, call: Node<'t>) -> bool {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return false;
        };
        let mut cursor = arguments.walk();
        let found = arguments
            .named_children(&mut cursor)
            .any(|a| a.kind() == "spread_element");
        found
    }

    /// The `ambiguous_client` gap a call's client leaves, if any.
    fn routing_gap(&mut self, site: &Site<'_, 't>, span: Span, client: &Client) {
        let chain = site.chain;
        let (reason, detail) = if client.ambiguous {
            (
                "multiple_assignments",
                format!(
                    "the receiver of `{chain}` is assigned more than one kind of client or a non-client value, so its routing depends on a runtime branch"
                ),
            )
        } else if let Some(caveat) = &client.caveat {
            match caveat {
                ClientCaveat::InFileCallers { param } => (
                    "in_file_callers",
                    format!(
                        "the receiver of `{chain}` is parameter `{param}`: every caller in this file passes the same client, but other modules may call the function with another one"
                    ),
                ),
                ClientCaveat::Annotation { param } => (
                    "annotation",
                    format!(
                        "the receiver of `{chain}` is parameter `{param}`, known only from its type annotation, so the client's options (a `baseURL`, Vertex AI) are not seen"
                    ),
                ),
                ClientCaveat::BaseUrlAssigned => (
                    "base_url_assigned",
                    format!(
                        "the client behind `{chain}` is given a `baseURL` after it is built, so its requests may go to another endpoint"
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
                        "the `baseURL` of the client behind `{chain}` is read from the environment and falls back to another host, so the deployment may point it anywhere"
                    ),
                ),
                ClientCaveat::BaseUrl { env: true, .. } => (
                    "base_url_env",
                    format!(
                        "the `baseURL` of the client behind `{chain}` is read from the environment with no fallback, so its endpoint is not known"
                    ),
                ),
                ClientCaveat::BaseUrl { fallback: true, .. } => (
                    "base_url_env",
                    format!(
                        "the `baseURL` of the client behind `{chain}` falls back to the provider's own host, but the environment may set another"
                    ),
                ),
                ClientCaveat::BaseUrl { .. } => (
                    "base_url_unresolved",
                    format!(
                        "the `baseURL` of the client behind `{chain}` does not resolve to a string in this file, so its endpoint is not known"
                    ),
                ),
                ClientCaveat::EndpointOption { option } => (
                    "endpoint_option",
                    format!(
                        "the client behind `{chain}` is configured with {option}, which may send its requests to Vertex AI or another endpoint"
                    ),
                ),
                ClientCaveat::Dynamic(feature) => (
                    "dynamic_feature",
                    format!(
                        "the receiver of `{chain}` may be rebound or replaced at run time: {}",
                        feature.describe()
                    ),
                ),
            }
        } else if let Some(feature) = self.file_dynamic {
            (
                "dynamic_feature",
                format!(
                    "this file may rebind any name or property at run time ({}), so the routing of `{chain}` is not confirmed",
                    feature.describe()
                ),
            )
        } else {
            return;
        };
        self.out.gaps.push(UsageGap {
            kind: GapKind::AmbiguousClient,
            sdk: Some(site.sdk),
            file: self.file.clone(),
            span: Some(span),
            symbol: self.symbol(site.scope),
            chain: Some(chain.to_string()),
            key: None,
            reason: reason.to_string(),
            detail,
            values: Vec::new(),
            values_complete: false,
            args: None,
        });
    }

    /// Model values nested in the options' object and array literals
    /// (`requests: [{ params: { model } }]`) and in object and array
    /// literals passed by position (`fallback([{ model }])`): the values,
    /// the key paths a later spread or computed key may replace, and whether
    /// a limit cut the read.
    fn nested_values(
        &mut self,
        options: &Options<'t>,
        positional: &[Node<'t>],
        scope: usize,
    ) -> (Vec<NestedValue>, Vec<String>, bool) {
        let mut found = Vec::new();
        let mut overridden = Vec::new();
        let mut left = MAX_NESTED_ENTRIES;
        let mut cut = false;
        let mut work: Vec<NestedWork<'t>> = Vec::new();
        let (entries, read_cut, _) = options.read.flat(left);
        cut |= read_cut;
        left -= entries.len();
        for entry in entries.into_iter().rev() {
            if !is_identifier_key(entry.key) || is_value_key(entry.key) {
                continue;
            }
            if matches!(unwrap_expression(entry.value).kind(), "object" | "array") {
                let replaced = entry.overridden || options.read.find(entry.key).overridden;
                work.push(NestedWork {
                    node: entry.value,
                    scope: entry.scope,
                    prefix: format!("{}.", entry.key),
                    replaced,
                    // As in Python, an option's value is depth 0.
                    depth: 0,
                    positional: false,
                });
            }
        }
        for node in positional.iter().rev() {
            work.push(NestedWork {
                node: *node,
                scope,
                prefix: String::new(),
                replaced: false,
                depth: 0,
                positional: true,
            });
        }
        while let Some(item) = work.pop() {
            if item.depth > MAX_NESTED_DEPTH {
                continue;
            }
            let node = unwrap_expression(item.node);
            match node.kind() {
                "object" => {
                    let read = self.object_entries(node, item.scope);
                    // A key written twice counts by its last entry.
                    let (entries, read_cut, _) = read.flat(left);
                    cut |= read_cut;
                    left -= entries.len();
                    for entry in entries {
                        // Nothing under a secret-shaped key is read.
                        if !is_identifier_key(entry.key) {
                            continue;
                        }
                        let after = entry.overridden || read.find(entry.key).overridden;
                        let path = format!("{}{}", item.prefix, entry.key);
                        if is_value_key(entry.key) {
                            if item.replaced || after {
                                overridden.push(path);
                            } else if let Ok(resolved) =
                                self.resolve_value(entry.value, entry.scope, 0)
                            {
                                found.push(NestedValue {
                                    path,
                                    positional: item.positional,
                                    resolved,
                                });
                            }
                        } else if matches!(
                            unwrap_expression(entry.value).kind(),
                            "object" | "array"
                        ) {
                            work.push(NestedWork {
                                node: entry.value,
                                scope: entry.scope,
                                prefix: format!("{path}."),
                                replaced: item.replaced || after,
                                depth: item.depth + 1,
                                positional: item.positional,
                            });
                        }
                    }
                }
                "array" => {
                    let mut cursor = node.walk();
                    let items: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                    for child in items {
                        if left == 0 {
                            cut = true;
                            break;
                        }
                        left -= 1;
                        if matches!(child.kind(), "object" | "array") {
                            work.push(NestedWork {
                                node: child,
                                scope: item.scope,
                                prefix: item.prefix.clone(),
                                replaced: item.replaced,
                                depth: item.depth + 1,
                                positional: item.positional,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        found.sort_by_key(|n| n.resolved.span.start_byte);
        (found, overridden, cut)
    }

    /// Object and array literals a call passes by position, other than its
    /// options object.
    fn positional_literals(
        &self,
        call: Node<'t>,
        options: &Options<'t>,
        options_index: usize,
    ) -> Vec<Node<'t>> {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return Vec::new();
        };
        let mut cursor = arguments.walk();
        let args: Vec<Node<'t>> = arguments
            .named_children(&mut cursor)
            .filter(|a| a.kind() != "comment")
            .collect();
        args.into_iter()
            .enumerate()
            .filter(|(index, arg)| {
                let literal = matches!(unwrap_expression(*arg).kind(), "object" | "array");
                literal && !(*index == options_index && options.literal)
            })
            .map(|(_, arg)| arg)
            .collect()
    }

    /// Attaches a resolved value to `key`, or reports why it did not
    /// resolve. A fallback captured from an environment or property lookup
    /// is both a value and a gap.
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
            // A value whose gap cannot be reported here is not captured: it
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
    /// and [`MAX_FILE_NESTED_VALUES`] per file; past either, or when the
    /// read was cut short, a `truncated` gap.
    fn take_nested(
        &mut self,
        site: &Site<'_, 't>,
        nested: Vec<NestedValue>,
        read_cut: bool,
        values: &mut Vec<CapturedValue>,
    ) {
        if nested.is_empty() && !read_cut {
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
                args: None,
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
            _ if unresolved.reason == "splat" && shape != "..." => format!(
                "`{key}` for `{chain}` is set in an object (`{shape}`) where a spread or a computed key written after it may replace it"
            ),
            _ if unresolved.reason == "splat" => format!(
                "`{key}` for `{chain}` is not written at the call and may come from a spread or an options object built elsewhere"
            ),
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
            Some(origin)
                if matches!(unresolved.reason, "dynamic_feature" | "subclass_override") =>
            {
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
            args: None,
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

    /// An SDK-shaped call on a receiver that is not a tracked client.
    fn check_unknown_receiver(
        &mut self,
        call: Node<'t>,
        function: Node<'t>,
        scope: usize,
        origin: Origin,
    ) {
        // Every SDK shape has a member: a bare name never matches.
        if function.kind() == "identifier" {
            return;
        }
        let links = self.render_links(function, 8);
        // A request on an object an SDK call returned
        // (`cacheManager.create({ model })` on `new GoogleAICacheManager()`).
        let sdk_result = match origin {
            Origin::SdkResult { pkg: Some(pkg) } => {
                let last = links.rsplit('.').next().unwrap_or(&links);
                is_request(last).then_some((pkg.sdk(), true))
            }
            _ => None,
        };
        if sdk_shape(&links).is_none()
            && model_bearing_shape(&links).is_none()
            && sdk_result.is_none()
        {
            return;
        }
        let chain = self.render(function);
        // A member that takes a model, called with one written at the call
        // (`genAI.getGenerativeModel({ model })` on an imported client).
        let shape = sdk_shape(&chain).or_else(|| {
            model_bearing_shape(&chain).or(sdk_result).filter(|_| {
                let options = self.options(call, scope, 0);
                super::VALUE_KEYS
                    .iter()
                    .any(|key| matches!(options.lookup(key), Lookup::Found(..)))
            })
        });
        let Some((shape_sdk, strong)) = shape else {
            return;
        };
        if !strong && !self.sdk_imported.contains(&shape_sdk) {
            return;
        }
        let root = chain_root(&chain);
        let (reason, detail, sdk) = match &origin {
            Origin::Factory { callee } => {
                let callee = self
                    .factory_nodes
                    .get(callee)
                    .map(|node| self.render(*node))
                    .unwrap_or_default();
                (
                    "factory_call",
                    format!(
                        "`{root}` comes from `{callee}`, which is not a tracked SDK constructor"
                    ),
                    shape_sdk,
                )
            }
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
        let options = self.options(call, scope, 0);
        let mut values = Vec::new();
        for key in super::VALUE_KEYS {
            if let Lookup::Found(value, _, value_scope) = options.lookup(key) {
                if let Ok(resolved) = self.resolve_value(value, value_scope, 0) {
                    values.push(captured(key, resolved, false));
                }
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
            // The arguments are known even when the client is not: the
            // retirement check applies its argument rules to them.
            args: Some(CallArgs {
                splat: options.unwritten() || self.has_spread_argument(call),
                kwargs: options.keys,
                spreads: Vec::new(),
            }),
        });
    }

    /// An SDK request method used as a value (`const create =
    /// client.chat.completions.create`).
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
        self.method_reference(reference, scope, val.pkg());
    }

    fn method_reference(&mut self, reference: Node<'t>, scope: usize, pkg: Option<Pkg>) {
        let rendered = self.render(reference);
        self.out.gaps.push(UsageGap {
            kind: GapKind::DynamicDispatch,
            sdk: pkg.map(Pkg::sdk),
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
            args: None,
        });
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    /// A callee or expression as written, arguments elided: identifiers
    /// and member names only, `(...)` for arguments, `"..."` for literals,
    /// and no environment variable's name.
    fn render(&self, node: Node<'t>) -> String {
        clip_owned(self.render_links(node, MAX_LINKS))
    }

    fn render_links(&self, node: Node<'t>, budget: usize) -> String {
        let mut segments: Vec<String> = Vec::new();
        let mut current = node;
        let mut links = 0;
        let base = loop {
            if links >= budget {
                break "<expr>".to_string();
            }
            links += 1;
            let next = match current.kind() {
                "member_expression" => {
                    let dot = if has_field(current, "optional_chain") {
                        "?."
                    } else {
                        "."
                    };
                    let property = current
                        .child_by_field_name("property")
                        .map_or("", |p| self.text(p));
                    segments.push(format!("{dot}{property}"));
                    current.child_by_field_name("object")
                }
                "subscript_expression" => {
                    let dot = if has_field(current, "optional_chain") {
                        "?."
                    } else {
                        ""
                    };
                    segments.push(format!("{dot}[...]"));
                    current.child_by_field_name("object")
                }
                "call_expression" => {
                    let arguments = current.child_by_field_name("arguments");
                    let args = match arguments {
                        Some(a) if a.kind() == "template_string" => "`...`",
                        Some(a) if a.named_child_count() > 0 => "(...)",
                        _ => "()",
                    };
                    let optional = if has_token(current, "?.") { "?." } else { "" };
                    segments.push(format!("{optional}{args}"));
                    current.child_by_field_name("function")
                }
                "non_null_expression" => {
                    segments.push("!".to_string());
                    current.named_child(0)
                }
                "parenthesized_expression"
                | "await_expression"
                | "as_expression"
                | "satisfies_expression" => current.named_child(0),
                "new_expression" => {
                    let constructor = current.child_by_field_name("constructor").map_or_else(
                        || "<expr>".to_string(),
                        |c| self.render_links(c, budget.saturating_sub(links)),
                    );
                    let args = current
                        .child_by_field_name("arguments")
                        .is_some_and(|a| a.named_child_count() > 0);
                    break format!("new {constructor}{}", if args { "(...)" } else { "()" });
                }
                "identifier" | "this" | "super" | "import" => break self.text(current).to_string(),
                "meta_property" => break "import.meta".to_string(),
                "ternary_expression" => break "<conditional>".to_string(),
                "binary_expression" if matches!(operator(current), Some("??" | "||" | "&&")) => {
                    break "<logical>".to_string()
                }
                "string" => break "\"...\"".to_string(),
                "template_string" => break "`...`".to_string(),
                _ => break "<expr>".to_string(),
            };
            match next {
                Some(next) => current = next,
                None => break "<expr>".to_string(),
            }
        };
        segments.reverse();
        // `process.env.SECRET_NAME`: the variable's name is left out.
        let env_at = match base.as_str() {
            "process" | "Bun" => Some(0),
            "import" if segments.first().is_some_and(|s| s == ".meta") => Some(1),
            _ => None,
        };
        if let Some(at) = env_at {
            if segments.get(at).is_some_and(|s| s == ".env") && segments.len() > at + 1 {
                segments[at + 1] = "[...]".to_string();
            }
        }
        let mut out = base;
        for segment in segments {
            out.push_str(&segment);
        }
        out
    }

    fn finish(mut self) -> FileUsages {
        // A client built from a mocked or replaced SDK is not the SDK's,
        // `setDefaultBaseUrls` re-routes every Gemini client, and a replaced
        // global `fetch` every direct one.
        let v3 = format!("{}.{V3_API}", Pkg::Openai.package());
        for client in &mut self.out.clients {
            if self
                .sdk_patched
                .keys()
                .any(|pkg| pkg.package() == client.package)
                || (self.genai_base_urls.is_some() && client.package == Pkg::GoogleGenai.package())
                || (self.global_fetch
                    && through_global_fetch(client.routing, client.constructor == v3))
            {
                client.routing = Routing::Unknown;
            }
        }
        // A request that names another endpoint is sent with the model of
        // the model object or chat it goes through: where that model was
        // given is not routed with no gap either.
        let shapes: [(Option<Feature>, Pkg, &[&str]); 2] = [
            (
                self.legacy_request_endpoint,
                Pkg::GoogleGenerativeai,
                &[
                    "getGenerativeModel",
                    "getGenerativeModelFromCachedContent",
                    "startChat",
                ],
            ),
            (
                self.genai_request_endpoint,
                Pkg::GoogleGenai,
                &["chats.create"],
            ),
        ];
        for (feature, pkg, members) in shapes {
            let Some(feature) = feature else {
                continue;
            };
            let mut added = Vec::new();
            for call in &mut self.out.calls {
                if call.package != pkg.package()
                    || call.routing == Routing::Unknown
                    || !members.iter().any(|m| member_is(&call.member, m))
                {
                    continue;
                }
                call.routing = Routing::Unknown;
                added.push(UsageGap {
                    kind: GapKind::AmbiguousClient,
                    sdk: Some(call.sdk),
                    file: call.file.clone(),
                    span: Some(call.span),
                    symbol: call.symbol.clone(),
                    chain: Some(call.chain.clone()),
                    key: None,
                    reason: "endpoint_option".to_string(),
                    detail: format!(
                        "a request in this file passes {} (line {}), so where `{}` sends its model is not confirmed",
                        feature.what,
                        feature.line,
                        call.chain
                    ),
                    values: Vec::new(),
                    values_complete: false,
                    args: None,
                });
            }
            self.out.gaps.extend(added);
        }
        self.out.imports.sort_by_key(|i| i.span.start_byte);
        self.out
            .clients
            .sort_by_key(|c| (c.span.start_byte, c.span.end_byte));
        // A call chained on another (`model.startChat().sendMessage()`)
        // starts at the same byte; the inner, shorter one comes first.
        self.out
            .calls
            .sort_by_key(|c| (c.span.start_byte, c.span.end_byte));
        self.out
            .gaps
            .sort_by_key(|g| (g.span.map(|s| s.start_byte).unwrap_or(0), g.kind));
        self.out
    }
}

/// The name a chain starts with (`this.client` for `this.client.chat...`).
fn chain_root(chain: &str) -> String {
    let mut parts = chain.split(['.', '(', '[', '?', '!']);
    let first = parts.next().unwrap_or_default();
    if first == "this" {
        if let Some(second) = chain
            .strip_prefix("this.")
            .and_then(|rest| rest.split(['.', '(', '[', '?', '!']).next())
        {
            return format!("this.{second}");
        }
    }
    first.to_string()
}

#[cfg(test)]
mod tests;
