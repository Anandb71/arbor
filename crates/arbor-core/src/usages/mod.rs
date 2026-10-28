//! SDK usage extraction: where a file talks to an LLM provider's SDK.
//!
//! The call graph drops calls whose target is not defined in the project, so
//! it cannot answer "which lines call OpenAI, and with which model?". This
//! module answers that per file, from the same tree-sitter tree
//! [`crate::parse_source`] builds. It is separate from graph extraction: it
//! adds no nodes or edges and does not change the cached node schema.
//!
//! # What a file reports
//!
//! [`FileUsages`] holds four lists:
//!
//! - [`SdkImport`]: each local name bound to a tracked SDK
//!   (`import openai as oa`, `from anthropic import AsyncAnthropic`,
//!   `from google import genai`).
//! - [`ClientConstruction`]: each client built from a tracked constructor,
//!   with its [`Routing`] decided from the constructor and its keyword
//!   arguments (`AzureOpenAI(...)` is `azure_openai`; `OpenAI(base_url=...)`
//!   is `custom_base_url` unless the URL is the provider's own host).
//! - [`SdkCall`]: each call whose member chain is rooted at a tracked binding
//!   (`client.chat.completions.create`, `openai.ChatCompletion.create`,
//!   `self.client.messages.create`, `genai.GenerativeModel(...)`). A call
//!   records keyword-argument *names* only. A *value* is captured only for
//!   the keys in [`VALUE_KEYS`] (as keyword arguments, as keys of a dict
//!   literal passed to the call or expanded with `**`), only when it is a
//!   string literal or a same-file constant that resolves to one.
//! - [`UsageGap`]: everything the extractor saw but could not resolve. A gap
//!   is never a silent drop: a model passed from the environment, a client
//!   from an unknown factory, `getattr` dispatch, or an ambiguous client each
//!   get a typed record. A model read from the environment or a config
//!   lookup with a literal fallback (`os.getenv("MODEL", "gpt-4o")`) is both:
//!   the fallback is captured with origin [`ValueOrigin::EnvDefault`] and the
//!   call keeps its gap, because the deployed value may differ.
//!
//! A value or route with no gap is confirmed only inside a small supported
//! grammar; everything outside it, and anything a dynamic feature of the file
//! (`exec`, `globals()`, `setattr` with a run-time name, a class decorator,
//! ...) may change, keeps a gap. `docs/SDK_USAGES.md` states the grammar, the
//! dynamic features and the bounds.
//!
//! # Credentials
//!
//! No credential value is ever captured. A value is refused when any name it
//! flows through matches [`is_secret_key`] (the keyword, a constant, a
//! parameter, a `self.attr`, any name in an attribute chain such as
//! `ApiKeys.openai`, a dict key or an environment variable name), when the
//! value itself is credential-shaped ([`is_credential_value`]: `sk-`,
//! `sk-ant-`, `AIza`, `ghp_`, `xox...`, URLs with user information, long
//! random tokens), and dict
//! key paths are built only from identifier-shaped keys
//! ([`is_identifier_key`]). A refused value at a request is an
//! `unresolved_model` gap with reason `credential`; the gap names the
//! binding, never the value.
//!
//! Positions are [`Span`]s: 1-based lines and 1-based columns counted in
//! Unicode scalar values, plus 0-based UTF-8 byte offsets for rewriting. A
//! consumer that needs another column unit (UTF-16 for JS editors) should
//! derive it from the byte offsets.
//!
//! # Scope of this version
//!
//! Python only ([`python`]): `.py` files and extensionless scripts with a
//! Python shebang. LangChain's OpenAI, Anthropic and Bedrock chat models
//! (`ChatOpenAI(model=...)`, `ChatBedrock(model_id=...)`) are recorded as a
//! construction and a call that binds the model, like
//! `genai.GenerativeModel(...)`. A `boto3` `bedrock-runtime` client's
//! requests (`converse(modelId=...)`) route `bedrock` for any model, and
//! `litellm.completion(model=...)` routes `litellm`. A raw HTTP request,
//! `requests.post(...)` or `httpx.post(...)` to a listed `api.openai.com` or
//! `api.anthropic.com` endpoint, is a direct request whose model is read from
//! its JSON body. The TypeScript/ESM slice will implement
//! [`crate::LanguageParser::extract_usages`] for its parsers and reuse these
//! types. Resolution is per file: a client or model constant imported from
//! another module is a gap, not a usage. The one exception is a call to a
//! wrapper defined in another file of a repository scan: a module-level
//! function or class whose parameter the extractor confirms is the model
//! of a request in its own file ([`ModelParam`]). The scan's second pass
//! records each call that passes it, in the calling file, as an
//! [`SdkCall`] with a [`SdkCall::wrapper`]; the value's span stays in that
//! file.

pub mod python;

use serde::{Deserialize, Serialize};

/// A provider whose SDK is tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sdk {
    /// `openai` (v0 module API and v1 clients).
    Openai,
    /// `anthropic`.
    Anthropic,
    /// `google.genai` and the legacy `google.generativeai`.
    Google,
    /// The AWS SDK (`boto3`), followed only to its Amazon Bedrock runtime
    /// client, and LangChain's Bedrock chat models built on it.
    Aws,
    /// LiteLLM (`litellm.completion`), which sends a request to whichever
    /// provider its model string names.
    Litellm,
}

impl Sdk {
    /// Every tracked provider, in display order.
    pub const ALL: [Sdk; 5] = [
        Sdk::Openai,
        Sdk::Anthropic,
        Sdk::Google,
        Sdk::Aws,
        Sdk::Litellm,
    ];

    /// The lowercase name used in JSON and on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Sdk::Openai => "openai",
            Sdk::Anthropic => "anthropic",
            Sdk::Google => "google",
            Sdk::Aws => "aws",
            Sdk::Litellm => "litellm",
        }
    }

    /// Parses `openai`, `anthropic`, `google`, `aws` or `litellm`
    /// (case-insensitive; `gemini`, `bedrock` and `boto3` are accepted too).
    pub fn parse(name: &str) -> Option<Sdk> {
        match name.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Sdk::Openai),
            "anthropic" => Some(Sdk::Anthropic),
            "google" | "gemini" => Some(Sdk::Google),
            "aws" | "bedrock" | "boto3" => Some(Sdk::Aws),
            "litellm" => Some(Sdk::Litellm),
            _ => None,
        }
    }
}

impl std::fmt::Display for Sdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a client's requests go, decided from its constructor and keyword
/// argument names. A provider's model retirement date applies only to the
/// `*_direct` routes and Google's own APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    /// `OpenAI()` / `AsyncOpenAI()` with no `base_url`, the module-level API,
    /// or LangChain's `ChatOpenAI` built the same way.
    OpenaiDirect,
    /// `AzureOpenAI` / `AsyncAzureOpenAI`, `openai.api_type = "azure"`, or
    /// LangChain's `AzureChatOpenAI`.
    AzureOpenai,
    /// A client given a `base_url` / `api_base` whose host is not the
    /// provider's own (OpenRouter, LiteLLM, a local server), or a
    /// module-level `openai.api_base` / `base_url` set that way. A `base_url`
    /// of `api.openai.com` or `api.anthropic.com` is direct. A Bedrock
    /// client given an `endpoint_url` outside AWS is routed this way too.
    CustomBaseUrl,
    /// `Anthropic()` / `AsyncAnthropic()` with no `base_url`, or LangChain's
    /// `ChatAnthropic` built the same way.
    AnthropicDirect,
    /// `AnthropicBedrock` / `AsyncAnthropicBedrock`.
    AnthropicBedrock,
    /// `AnthropicVertex` / `AsyncAnthropicVertex`.
    AnthropicVertex,
    /// `google.genai.Client`.
    GoogleGenai,
    /// `google.generativeai` (the deprecated SDK).
    GoogleGenerativeaiLegacy,
    /// Amazon Bedrock, for any model it serves: a `boto3` `bedrock-runtime`
    /// client, or LangChain's `ChatBedrock` / `ChatBedrockConverse`.
    Bedrock,
    /// A call through LiteLLM (`litellm.completion(...)`), which picks the
    /// provider and endpoint from the model string and its configuration.
    Litellm,
    /// Not decided by this version: `**kwargs` passed to a direct
    /// constructor, a client typed only by annotation, a binding assigned
    /// more than one kind of client, a direct client given a `base_url`
    /// after it is built or built in a file that sets `OPENAI_BASE_URL` /
    /// `ANTHROPIC_BASE_URL` or writes the environment with a run-time name,
    /// a `base_url` read from the environment with no fallback or one that
    /// does not resolve to a string (each with an `ambiguous_client` gap at
    /// each call), a non-literal or non-Azure
    /// `openai.api_type`, or a hosted-platform client this list does not name
    /// (`BedrockOpenAI`, `OpenAI(provider=...)`, `AnthropicAWS`,
    /// `AnthropicFoundry`, `AnthropicGoogleCloud` and their async variants).
    /// The construction's `constructor` field still names the class.
    Unknown,
}

impl Routing {
    /// The snake_case name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Routing::OpenaiDirect => "openai_direct",
            Routing::AzureOpenai => "azure_openai",
            Routing::CustomBaseUrl => "custom_base_url",
            Routing::AnthropicDirect => "anthropic_direct",
            Routing::AnthropicBedrock => "anthropic_bedrock",
            Routing::AnthropicVertex => "anthropic_vertex",
            Routing::GoogleGenai => "google_genai",
            Routing::GoogleGenerativeaiLegacy => "google_generativeai_legacy",
            Routing::Bedrock => "bedrock",
            Routing::Litellm => "litellm",
            Routing::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for Routing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A source range. Lines and columns are 1-based; columns count Unicode
/// scalar values. Bytes are 0-based UTF-8 offsets, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub start_byte: u32,
    pub end_byte: u32,
}

/// What an imported SDK name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    /// A module or package (`import openai`, `from google import genai`).
    Module,
    /// A client constructor (`OpenAI`, `AnthropicBedrock`, `GenerativeModel`).
    ClientClass,
    /// Any other imported name (`ChatCompletion`, `types`, `HUMAN_PROMPT`).
    Symbol,
}

/// A local name bound to a tracked SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkImport {
    pub sdk: Sdk,
    /// `openai`, `anthropic`, `google.genai`, `google.generativeai`, `boto3`
    /// or `litellm`, or the LangChain package a chat model comes from
    /// (`langchain_openai`, `langchain_anthropic`, `langchain_aws`,
    /// `langchain_community`, `langchain`).
    pub package: String,
    /// The module named in the statement (`openai.types.chat`).
    pub module: String,
    /// The imported name for `from m import name`; `None` for `import m`.
    pub name: Option<String>,
    /// The name the file uses (`genai`, `OA`).
    pub local: String,
    pub kind: BindingKind,
    pub file: String,
    pub span: Span,
}

/// How a constructed client is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientScope {
    /// A module-level name (including a `global` assigned in a function).
    Module,
    /// A function-local name.
    Local,
    /// A class-body attribute (`class Bot: client = OpenAI()`).
    Class,
    /// An instance attribute (`self.client = OpenAI()`).
    Attribute,
    /// Not bound to a name: passed, returned or chained directly.
    Inline,
}

/// A client built from a tracked constructor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientConstruction {
    pub sdk: Sdk,
    pub package: String,
    /// Package-qualified constructor (`openai.AzureOpenAI`).
    pub constructor: String,
    /// The constructor as written, arguments elided (`oa.AzureOpenAI`).
    pub chain: String,
    /// The binding it is assigned to (`client`, `self.client`), if any.
    pub binding: Option<String>,
    pub scope: ClientScope,
    pub routing: Routing,
    pub is_async: bool,
    /// Keyword argument names, in source order. Values are never recorded.
    pub kwargs: Vec<String>,
    /// Whether `**kwargs` was passed (routing may then be `unknown`).
    pub splat: bool,
    pub file: String,
    pub span: Span,
    /// Enclosing function or method (`Bot.__init__`), `None` at module level.
    pub symbol: Option<String>,
    /// For `google.genai.Client(vertexai=...)`: how the argument reads.
    /// Only this class is recorded, never the expression. `None` when the
    /// constructor passes no `vertexai=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertexai: Option<VertexAi>,
}

/// How a `google.genai.Client(vertexai=...)` argument reads. The Gemini API
/// and Vertex AI retire models on different schedules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertexAi {
    /// `vertexai=True`: requests go to Vertex AI.
    On,
    /// `vertexai=False` or `vertexai=None`: requests go to the Gemini API.
    Off,
    /// Decided at run time (`vertexai=use_vertex`,
    /// `vertexai=os.getenv(...) == "true"`).
    Unknown,
}

/// What a call's chain is rooted at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Receiver {
    /// The SDK module itself (`openai.ChatCompletion.create`).
    Module,
    /// A constructed or annotated client (`client.chat.completions.create`).
    Client,
    /// A client constructor or other imported name (`genai.GenerativeModel`,
    /// `ChatCompletion.create` after `from openai import ChatCompletion`).
    Symbol,
}

/// Where a captured value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueOrigin {
    /// A string literal at the call.
    Literal,
    /// A module-level name assigned exactly one string literal.
    ModuleConstant,
    /// A class attribute (`MODEL = ...` in the class body, or a single
    /// `self.model = "..."`) assigned exactly one string literal.
    ClassConstant,
    /// A function-local name assigned exactly one string literal.
    LocalConstant,
    /// The default of the enclosing function's parameter: a literal or a
    /// same-file constant (`def ask(model=GPT_MODEL)`). Callers may pass
    /// another value. With no gap only when every call of the function is in
    /// this file and none passes another value; otherwise the call also
    /// carries an `unresolved_model` gap: `parameter_overridden` (listing the
    /// values in-file callers pass), `unseen_callers` (some calls cannot be
    /// seen) or `parameter_default` (a public function another module may
    /// call).
    ParameterDefault,
    /// The literal fallback of an environment or configuration lookup:
    /// `os.getenv("MODEL", "gpt-4o")`, `os.environ.get(...)`,
    /// `config("MODEL", default=...)` (python-decouple, django-environ),
    /// `settings.get("model", "gpt-4o")`, `getattr(cfg, "model", "gpt-4o")`
    /// or `os.getenv("MODEL") or "gpt-4o"`. The deployed value may differ, so
    /// the call also carries an `unresolved_model` gap: confirm before
    /// acting on it.
    EnvDefault,
}

impl ValueOrigin {
    /// The snake_case name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            ValueOrigin::Literal => "literal",
            ValueOrigin::ModuleConstant => "module_constant",
            ValueOrigin::ClassConstant => "class_constant",
            ValueOrigin::LocalConstant => "local_constant",
            ValueOrigin::ParameterDefault => "parameter_default",
            ValueOrigin::EnvDefault => "env_default",
        }
    }
}

/// A value captured for an allowlisted key ([`VALUE_KEYS`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedValue {
    /// `model`, `engine`, `deployment_id` or `model_name` (and, on a Bedrock
    /// call, `modelId` or `model_id`; see [`BEDROCK_VALUE_KEYS`]); for a key inside a
    /// dict literal argument, the path to it (`fallbacks.model` for
    /// `fallbacks=[{"model": ...}]`, `model` when the dict is positional or
    /// expanded with `**`). Every segment is an identifier-shaped key.
    pub key: String,
    pub value: String,
    pub origin: ValueOrigin,
    /// The constant or parameter name when the value is not written inline
    /// (`MODEL`, `self.model`, `MODELS["fast"]`, `model` for a parameter
    /// default).
    pub name: Option<String>,
    /// The value was passed by position rather than by name: the call's
    /// first positional argument (`GenerativeModel("gemini-pro")`, legacy
    /// `genai.embed_content(...)`) or a key of a dict literal passed
    /// positionally. It describes how the call is written, not the SDK
    /// signature. In a language whose SDK methods take one options object
    /// (TypeScript `create({ model })`), that object's properties are by
    /// name: `false`.
    pub positional: bool,
    /// The span of the string literal itself, at its definition.
    pub span: Span,
    /// Which provider's model-ID shape the value matches, if any.
    pub model_family: Option<Sdk>,
}

/// A call whose member chain is rooted at a tracked binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkCall {
    /// The provider: for a raw HTTP request, the one whose API host the
    /// URL names.
    pub sdk: Sdk,
    /// As in [`SdkImport::package`], or the HTTP library (`requests`,
    /// `httpx`) a raw HTTP request is sent with.
    pub package: String,
    /// The callee as written, arguments elided
    /// (`self.client.chat.completions.create`).
    pub chain: String,
    /// The part after the tracked root (`chat.completions.create`), with the
    /// response wrappers `with_raw_response` / `with_streaming_response` and
    /// client copies (`with_options(...)`, `copy(...)`) removed. For a raw
    /// HTTP request (`package` `requests` or `httpx`), the SDK method its
    /// endpoint is (`chat.completions.create` for `/v1/chat/completions`).
    pub member: String,
    /// Whether the call sends a request to the provider: its last member is
    /// a request method (`create`, `stream`, `generate_content`,
    /// `send_message`...). `false` for SDK helpers and types
    /// (`genai.configure`, `GenerativeModel(...)`, `types.Part(...)`,
    /// `ChatCompletion(...)` built from `openai.types`). A name heuristic.
    pub request: bool,
    pub receiver: Receiver,
    pub routing: Routing,
    /// Line of the same-file construction the receiver came from, if known.
    pub client_line: Option<u32>,
    pub file: String,
    pub span: Span,
    pub symbol: Option<String>,
    /// Argument names in source order: Python keyword arguments, or the
    /// properties of the options object in a language whose SDK methods
    /// take one (TypeScript `create({ model, messages })`).
    pub kwargs: Vec<String>,
    /// Number of positional arguments, the options object excluded.
    pub positional: u32,
    /// Whether arguments were spread into the call (`**kwargs`; TypeScript
    /// `...opts` in the options object).
    pub splat: bool,
    /// Values for allowlisted keys that resolved to string literals.
    pub values: Vec<CapturedValue>,
    /// For a call to a function or class defined in another file of the
    /// scan, whose parameter is the model of requests there
    /// (`Sampler(model="o1")` after `from .sampler import Sampler`): that
    /// function and those requests. The call's `sdk`, `package` and
    /// `routing` are the requests' (`routing` is `unknown`, with an
    /// `ambiguous_client` gap, when they differ or one is not confirmed),
    /// `member` is the function's name, `request` is `false` (the requests
    /// are recorded in that file) and `kwargs` are the call's own argument
    /// names. Its values are what the call passes for the parameter, keyed
    /// by the requests' model key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapper: Option<WrapperCall>,
}

/// The function or class a [`SdkCall::wrapper`] call reaches, in another
/// file of the scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapperCall {
    /// The file that defines it, as the scan reports files.
    pub file: String,
    /// Its name there: a module-level function, or a class whose
    /// `__init__` takes the parameter.
    pub function: String,
    /// The requests there whose model the call's argument is.
    pub requests: Vec<WrapperRequest>,
}

/// One request a [`WrapperCall`]'s parameter reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapperRequest {
    /// The parameter of the function that is its model.
    pub parameter: String,
    /// The request as written there (`self.client.chat.completions.create`).
    pub chain: String,
    /// As in [`SdkCall::member`].
    pub member: String,
    pub line: u32,
}

/// A parameter of a module-level function or class that is the model of a
/// request in the same file: what a call from another file passes for it
/// is the model that request sends. The per-file extractor lists one only
/// when it confirms the parameter reaches the request unchanged (see
/// `docs/SDK_USAGES.md`, "Calls through a wrapper in another file"). The
/// repository scan reads them for its second pass; they are not part of
/// the JSON report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelParam {
    /// The module-level name other files call: the function, or the class
    /// whose `__init__` takes the parameter.
    pub function: String,
    pub parameter: String,
    /// Its index among the positional arguments a call passes (`self`
    /// excluded); `None` when it is keyword-only.
    pub position: Option<u32>,
    /// It may be passed by keyword (it is not positional-only).
    pub keyword: bool,
    /// For a class: the instance attribute `__init__` stores it in (`model`
    /// for `self.model = model`), which a write on the instance replaces.
    pub attribute: Option<String>,
    /// The request's model key (`model`, `model_name`, `modelId`...).
    pub key: String,
    pub sdk: Sdk,
    pub package: String,
    pub chain: String,
    pub member: String,
    pub line: u32,
    pub routing: Routing,
    /// The request's routing has no client gap.
    pub route_confirmed: bool,
}

/// The functions and classes of other files that an import names, as the
/// repository scan resolved them, for [`python::extract_with`].
pub trait WrapperLookup {
    /// The file that defines module-level `name` and the model parameters
    /// it lists for it ([`FileUsages::model_params`]), when `module` (as an
    /// import statement in this file writes it: `.sampler`,
    /// `pkg.llm.client`) is another file of the scan.
    fn model_params(&self, module: &str, name: &str) -> Option<(String, Vec<ModelParam>)>;
}

/// Why something was reported as a gap instead of a usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    /// At a request or a model constructor (`GenerativeModel`), an
    /// allowlisted key's value is not a resolvable string literal
    /// (environment, config attribute, function call, f-string, parameter,
    /// `**kwargs`, a name imported from another module...), or a call that
    /// takes a model passes none (`missing_model`: the SDK or server
    /// default applies). A value captured next to such a gap is one the code
    /// names but cannot confirm: an environment fallback, a parameter
    /// default (`parameter_default`, `parameter_overridden`,
    /// `unseen_callers`), an attribute a subclass in another module may
    /// override (`subclass_override`) or a value a dynamic feature of the
    /// file may change (`dynamic_feature`, which names the feature and its
    /// line).
    UnresolvedModel,
    /// A call shaped like an SDK endpoint whose receiver is not a tracked
    /// client: built by an unknown factory, passed in as a parameter, or
    /// imported from another module.
    UnknownClientFactory,
    /// A binding assigned more than one kind of client, so routing depends on
    /// a runtime branch (`multiple_assignments`), or a client known from
    /// only some of the places it can come from: the in-file callers of a
    /// function other modules may also call (`in_file_callers`), a client
    /// whose `base_url` is assigned after it is built (`base_url_assigned`),
    /// a file that sets the SDK's endpoint variable (`endpoint_env`), a
    /// `base_url` read from the environment (`base_url_env`) or not resolved
    /// (`base_url_unresolved`), a Gemini client given an option that may
    /// send it to Vertex AI or another endpoint (`endpoint_option`), a
    /// module-level call that passes `engine` or `deployment_id` in a file
    /// that does not configure the module-level API, or any module-level
    /// call in a file that configures it outside its top-of-file setup block
    /// (`module_config`), or a receiver or configuration a dynamic feature of
    /// the file may change (`dynamic_feature`).
    AmbiguousClient,
    /// `getattr` on a tracked binding, or an SDK method stored or passed as a
    /// value; calls through it are not tracked.
    DynamicDispatch,
    /// SDK usage inside a vendored or virtual-environment directory
    /// (`site-packages`, `venv`, `.venv`, `node_modules`...). Excluded from
    /// usages and counted as skipped.
    VendoredPath,
}

impl GapKind {
    /// The snake_case name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            GapKind::UnresolvedModel => "unresolved_model",
            GapKind::UnknownClientFactory => "unknown_client_factory",
            GapKind::AmbiguousClient => "ambiguous_client",
            GapKind::DynamicDispatch => "dynamic_dispatch",
            GapKind::VendoredPath => "vendored_path",
        }
    }
}

impl std::fmt::Display for GapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Something the extractor saw but could not turn into a confirmed usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageGap {
    pub kind: GapKind,
    /// The provider involved, when known or inferred from the call's shape.
    pub sdk: Option<Sdk>,
    pub file: String,
    /// `None` for a vendored directory.
    pub span: Option<Span>,
    pub symbol: Option<String>,
    /// The callee or expression involved, arguments elided.
    pub chain: Option<String>,
    /// For `unresolved_model`: the key whose value did not resolve.
    pub key: Option<String>,
    /// A stable short code (`env_var`, `f_string`, `parameter`, `getattr`...).
    pub reason: String,
    /// A sentence for people. Never contains a literal value.
    pub detail: String,
    /// Allowlisted values seen at an unconfirmed call
    /// (`unknown_client_factory`), or, for `unresolved_model`, values the key
    /// may take that are known (what in-file callers pass for a parameter,
    /// the entries of a dict looked up with a run-time key, the value each
    /// branch assigns, the items a loop runs over), so it can be triaged.
    /// Never the call's value. Partial unless [`Self::values_complete`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<CapturedValue>,
    /// `values`, together with the value captured for the same key at the
    /// call (a parameter default, an `env_default` fallback), are every
    /// value the key can take as far as this file shows. `false` (omitted
    /// from JSON) whenever the list may be partial: a definition, entry or
    /// caller did not resolve, some callers cannot be seen, another module
    /// may call the function, or a limit cut the list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub values_complete: bool,
}

/// Everything one file says about tracked SDKs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileUsages {
    pub file: String,
    pub language: String,
    pub imports: Vec<SdkImport>,
    pub clients: Vec<ClientConstruction>,
    pub calls: Vec<SdkCall>,
    pub gaps: Vec<UsageGap>,
    /// The parser recovered from syntax errors: calls inside the broken
    /// region may be missing. Set only for files that mention an SDK.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub syntax_errors: bool,
    /// Parameters of this file's module-level functions and classes that
    /// are the model of a request here, for calls from other files (see
    /// [`ModelParam`]). Not serialized.
    #[serde(skip)]
    pub model_params: Vec<ModelParam>,
}

impl FileUsages {
    /// An empty record for `file`.
    pub fn new(file: &str, language: &str) -> Self {
        Self {
            file: file.to_string(),
            language: language.to_string(),
            ..Self::default()
        }
    }

    /// Whether the file reported nothing at all.
    pub fn is_empty(&self) -> bool {
        self.imports.is_empty()
            && self.clients.is_empty()
            && self.calls.is_empty()
            && self.gaps.is_empty()
    }

    /// Keeps only records for `sdk`. Gaps whose provider is unknown are kept.
    pub fn retain_sdk(&mut self, sdk: Sdk) {
        self.imports.retain(|i| i.sdk == sdk);
        self.clients.retain(|c| c.sdk == sdk);
        self.calls.retain(|c| c.sdk == sdk);
        self.gaps.retain(|g| g.sdk.is_none_or(|s| s == sdk));
    }
}

/// Languages that have a usage extractor.
pub const USAGE_LANGUAGES: &[&str] = &["python"];

/// File extensions that have a usage extractor. `.pyi` stubs are excluded:
/// they declare types, they do not make requests. Extensionless scripts are
/// scanned when [`is_python_shebang`] accepts their first line.
pub const USAGE_EXTENSIONS: &[&str] = &["py"];

/// Whether files with this extension have a usage extractor.
pub fn is_usage_extension(extension: &str) -> bool {
    USAGE_EXTENSIONS
        .iter()
        .any(|e| e.eq_ignore_ascii_case(extension))
}

/// Whether the start of an extensionless file is a Python shebang
/// (`#!/usr/bin/env python3`, `#!/usr/bin/python3.11`).
pub fn is_python_shebang(head: &[u8]) -> bool {
    let Some(rest) = head.strip_prefix(b"#!") else {
        return false;
    };
    let line = rest.split(|b| *b == b'\n').next().unwrap_or_default();
    String::from_utf8_lossy(line)
        .split(|c: char| c.is_whitespace() || c == '/')
        .any(|word| word.starts_with("python"))
}

/// Keys whose string value may be captured. Every other key is recorded by
/// name only, except [`BEDROCK_VALUE_KEYS`] on a Bedrock call.
pub const VALUE_KEYS: &[&str] = &["model", "engine", "deployment_id", "model_name"];

/// Keys whose value is also captured on a Bedrock call, and only there:
/// `modelId` on a `boto3` `bedrock-runtime` request, `model_id` on
/// LangChain's `ChatBedrock(...)`. Another SDK's `model_id` argument is not
/// its model, so these are not [`VALUE_KEYS`].
pub const BEDROCK_VALUE_KEYS: &[&str] = &["modelId", "model_id"];

/// Name fragments that mark a credential: `api_key`, `auth_token`,
/// `client_secret`, `PASSWD`, `db_pwd`, `CREDENTIALS`, `private_pem`...
const SECRET_NEEDLES: &[&str] = &[
    "key",
    "token",
    "secret",
    "passw",
    "passphrase",
    "pwd",
    "auth",
    "cred",
    "private",
];

/// Whether `key` may hold a credential. Such a value is never captured,
/// whatever else is configured.
pub fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_NEEDLES.iter().any(|needle| lower.contains(needle))
}

/// Whether a value for `key` may be captured.
pub fn is_value_key(key: &str) -> bool {
    VALUE_KEYS.contains(&key) && !is_secret_key(key)
}

/// Whether a dict key may appear in a [`CapturedValue::key`] path: an ASCII
/// identifier of at most 64 characters that is neither secret-named nor
/// credential-shaped. A dict under any other key is never descended into.
pub fn is_identifier_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    key.len() <= 64
        && (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !is_secret_key(key)
        && !is_credential_value(key)
}

/// Prefixes of API keys and tokens (OpenAI, Anthropic, Stripe, Google,
/// GitHub, Slack, AWS, GitLab, Hugging Face, Replicate, Groq, Perplexity,
/// NVIDIA, Cerebras), PEM blocks and HTTP authorization values.
const CREDENTIAL_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "rk_",
    "pk_live_",
    "pk_test_",
    "AIza",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "xox",
    "xapp-",
    "glpat-",
    "hf_",
    "r8_",
    "gsk_",
    "pplx-",
    "nvapi-",
    "csk-",
    "-----BEGIN",
    "Bearer ",
    "bearer ",
    "Basic ",
];

/// Whether a string looks like a credential rather than a model ID: a known
/// key or token prefix, an AWS key ID, a JWT, a URL with user information
/// (`postgres://admin:pw@db/prod`, `https://<token>@host`), or a long
/// random run of alphanumerics (20 or more mixing upper case, lower case
/// and digits, or 32 or more mixing letters and digits, which covers hex
/// keys). Model IDs are short words joined by `-`, `.`, `:`, `/` or `@`,
/// so they do not match.
pub fn is_credential_value(value: &str) -> bool {
    let value = value.trim();
    if CREDENTIAL_PREFIXES.iter().any(|p| value.starts_with(p)) {
        return true;
    }
    if has_url_userinfo(value) {
        return true;
    }
    // An AWS access key ID.
    if (value.starts_with("AKIA") || value.starts_with("ASIA"))
        && value.len() == 20
        && value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return true;
    }
    // A JWT: base64url segments, the first a JSON header.
    if value.starts_with("eyJ") && value.matches('.').count() >= 2 {
        return true;
    }
    value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|run| {
            let upper = run.chars().any(|c| c.is_ascii_uppercase());
            let lower = run.chars().any(|c| c.is_ascii_lowercase());
            let digit = run.chars().any(|c| c.is_ascii_digit());
            (run.len() >= 20 && upper && lower && digit)
                || (run.len() >= 32 && (upper || lower) && digit)
        })
}

/// Whether `value` holds a URL whose authority carries user information
/// (`scheme://user:password@host`, `scheme://token@host`).
fn has_url_userinfo(value: &str) -> bool {
    value.match_indices("://").any(|(at, sep)| {
        let rest = &value[at + sep.len()..];
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        authority.contains('@')
    })
}

/// Whether a resolved string may be reported as a value: not
/// credential-shaped, no control or bidirectional-override characters (an
/// escape sequence must never reach a terminal) and at most 256 bytes.
pub fn is_capturable_value(value: &str) -> bool {
    value.len() <= 256 && !value.chars().any(is_unsafe_char) && !is_credential_value(value)
}

/// Control characters and bidirectional overrides.
pub fn is_unsafe_char(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Which provider's model-ID naming `value` follows, if any.
///
/// This is a shape check (`gpt-*`, `o3-mini`, `claude-*`, `gemini-*`...),
/// not a list of real models. It is applied only to values that reach an
/// allowlisted key, so a pricing table full of model names reports nothing.
pub fn model_family(value: &str) -> Option<Sdk> {
    let lower = value.trim().to_ascii_lowercase();
    let mut id = lower.as_str();
    // Fine-tuned ids (`ft:gpt-4o-mini:org::id`) and router prefixes.
    for prefix in [
        "ft:",
        "openai/",
        "openai.",
        "models/",
        "publishers/google/models/",
    ] {
        if let Some(rest) = id.strip_prefix(prefix) {
            id = rest;
        }
    }

    // Anthropic: first-party ids, Bedrock ids (with optional region prefix)
    // and router prefixes.
    if id.starts_with("claude-")
        || id.starts_with("anthropic/claude-")
        || id.contains("anthropic.claude-")
    {
        return Some(Sdk::Anthropic);
    }

    // Google before OpenAI: `text-embedding-004` is Google's, and
    // `text-embedding-3-small` is OpenAI's.
    const GOOGLE: &[&str] = &[
        "gemini-",
        "gemma-",
        "text-bison",
        "chat-bison",
        "code-bison",
        "codechat-bison",
        "text-embedding-00",
        "text-multilingual-embedding-",
        "embedding-001",
        "embedding-gecko",
        "imagen-",
        "veo-",
    ];
    if GOOGLE.iter().any(|p| id.starts_with(p)) || id == "gemini" {
        return Some(Sdk::Google);
    }

    const OPENAI: &[&str] = &[
        "gpt-",
        "chatgpt-",
        "text-davinci-",
        "text-curie-",
        "text-babbage-",
        "text-ada-",
        "code-davinci-",
        "code-cushman-",
        "davinci",
        "curie",
        "babbage",
        "text-embedding-",
        "text-moderation-",
        "omni-moderation-",
        "dall-e-",
        "whisper-",
        "tts-",
        "computer-use-preview",
        "codex-mini",
    ];
    if OPENAI.iter().any(|p| id.starts_with(p)) || id == "ada" || id.starts_with("ada-") {
        return Some(Sdk::Openai);
    }
    // Reasoning models: `o1`, `o3-mini`, `o4-mini-2025-04-16`.
    let mut chars = id.chars();
    if chars.next() == Some('o') {
        let rest: String = chars.collect();
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let tail = &rest[digits..];
            if tail.is_empty() || tail.starts_with('-') {
                return Some(Sdk::Openai);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_keys_are_never_secret_shaped() {
        for key in BEDROCK_VALUE_KEYS {
            assert!(!is_secret_key(key), "{key} must not look like a secret");
            assert!(!is_value_key(key), "{key} is read on Bedrock calls only");
        }
        for key in VALUE_KEYS {
            assert!(!is_secret_key(key), "{key} must not look like a secret");
            assert!(is_value_key(key));
        }
        for key in [
            "api_key",
            "API_KEY",
            "auth_token",
            "max_tokens",
            "client_secret",
            "password",
            "authorization",
            "azure_ad_token_provider",
            "PASSWD",
            "db_pwd",
            "credential",
            "CREDENTIALS",
            "gcp_cred",
            "private_pem",
            "passphrase",
        ] {
            assert!(is_secret_key(key), "{key} must be treated as secret");
            assert!(!is_value_key(key));
        }
        assert!(!is_value_key("temperature"));
    }

    #[test]
    fn credential_shaped_values_are_refused() {
        for secret in [
            "sk-proj-AbCdEf0123456789xyz",
            "sk-ant-api03-AbCdEf0123456789",
            "sk-live-SECRET",
            "sk_live_51H8abc",
            "AIzaSyD-abcdefghijklmnop",
            "ghp_0123456789abcdefABCDEF",
            "xoxb-1234-5678-abcdef",
            "AKIAIOSFODNN7EXAMPLE",
            "Bearer abc",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig",
            "Zx8aB2cD4eF6gH8iJ0kL2mN4",
            "0123456789abcdef0123456789abcdef",
            "postgres://admin:hunter2@db/prod",
            "https://user:pw@gateway.example/v1",
            "https://tokenvalue@github.com/org/repo",
        ] {
            assert!(is_credential_value(secret), "{secret} must be refused");
            assert!(!is_capturable_value(secret));
        }
        for model in [
            "gpt-4o-2024-08-06",
            "claude-3-5-sonnet-20241022",
            "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            "ft:gpt-4o-mini-2024-07-18:acme::9xKz3mQp",
            "models/gemini-1.5-flash-001",
            "GPT4o-Prod-Deployment-2024",
            "claude-3-5-sonnet-v2@20241022",
            "projects/123456789012/locations/us-central1/endpoints/1234567890123456789",
            "text-embedding-3-small",
            "o3-mini",
            "https://openrouter.ai/api/v1",
            "anthropic/claude-3-opus@latest",
        ] {
            assert!(!is_credential_value(model), "{model} is a model ID");
            assert!(is_capturable_value(model));
        }
        assert!(!is_capturable_value("gpt-4\u{1b}]8;;x\u{7}"));
        assert!(!is_capturable_value("gpt-\u{202E}4"));
        assert!(!is_capturable_value(&"a".repeat(300)));
    }

    #[test]
    fn identifier_keys_only() {
        for ok in ["model", "fallbacks", "extra_body", "_x1"] {
            assert!(is_identifier_key(ok), "{ok}");
        }
        for bad in [
            "",
            "sk-proj-abc",
            "x-api-key",
            "api_key",
            "1model",
            "has space",
            "sk_live_abc",
            "Zx8aB2cD4eF6gH8iJ0kL2mN4",
        ] {
            assert!(!is_identifier_key(bad), "{bad}");
        }
    }

    #[test]
    fn python_shebangs() {
        for yes in [
            "#!/usr/bin/env python3\nimport openai\n",
            "#!/usr/bin/python\n",
            "#! /usr/local/bin/python3.11 -u\n",
            "#!/usr/bin/env -S python3 -X utf8\n",
        ] {
            assert!(is_python_shebang(yes.as_bytes()), "{yes}");
        }
        for no in [
            "#!/bin/sh\n",
            "import openai\n",
            "",
            "#!/usr/bin/env node\n# python\n",
        ] {
            assert!(!is_python_shebang(no.as_bytes()), "{no}");
        }
    }

    #[test]
    fn model_family_recognises_shapes() {
        for (id, sdk) in [
            ("gpt-3.5-turbo", Sdk::Openai),
            ("gpt-4o-2024-08-06", Sdk::Openai),
            ("o1", Sdk::Openai),
            ("o3-mini", Sdk::Openai),
            ("o4-mini-2025-04-16", Sdk::Openai),
            ("text-davinci-003", Sdk::Openai),
            ("text-embedding-3-small", Sdk::Openai),
            ("ft:gpt-4o-mini:acme::abc", Sdk::Openai),
            ("openai/gpt-4", Sdk::Openai),
            ("openai.gpt-5.4", Sdk::Openai),
            ("claude-3-5-sonnet-20241022", Sdk::Anthropic),
            ("anthropic.claude-3-haiku-20240307-v1:0", Sdk::Anthropic),
            (
                "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
                Sdk::Anthropic,
            ),
            ("global.anthropic.claude-sonnet-5-5", Sdk::Anthropic),
            ("gemini-1.5-flash", Sdk::Google),
            ("models/gemini-pro", Sdk::Google),
            ("text-embedding-004", Sdk::Google),
        ] {
            assert_eq!(model_family(id), Some(sdk), "{id}");
        }
        for not_a_model in ["", "my-deployment", "ocean", "o", "orange-1", "hello"] {
            assert_eq!(model_family(not_a_model), None, "{not_a_model}");
        }
    }

    #[test]
    fn sdk_and_routing_names_round_trip() {
        for sdk in Sdk::ALL {
            assert_eq!(Sdk::parse(sdk.as_str()), Some(sdk));
            let json = serde_json::to_string(&sdk).unwrap();
            assert_eq!(json, format!("\"{}\"", sdk.as_str()));
        }
        assert_eq!(Sdk::parse("Gemini"), Some(Sdk::Google));
        assert_eq!(Sdk::parse("Bedrock"), Some(Sdk::Aws));
        assert_eq!(Sdk::parse("boto3"), Some(Sdk::Aws));
        assert_eq!(Sdk::parse("cohere"), None);
        for routing in [
            Routing::OpenaiDirect,
            Routing::AzureOpenai,
            Routing::CustomBaseUrl,
            Routing::AnthropicDirect,
            Routing::AnthropicBedrock,
            Routing::AnthropicVertex,
            Routing::GoogleGenai,
            Routing::GoogleGenerativeaiLegacy,
            Routing::Bedrock,
            Routing::Litellm,
            Routing::Unknown,
        ] {
            let json = serde_json::to_string(&routing).unwrap();
            assert_eq!(json, format!("\"{}\"", routing.as_str()));
        }
    }
}
