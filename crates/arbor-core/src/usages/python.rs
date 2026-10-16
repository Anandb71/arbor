//! Python SDK usage extraction.
//!
//! Two passes over the tree-sitter tree:
//!
//! 1. **Collect.** Scopes (module, class, function), imports, every
//!    assignment target (names, `self.x`, class attributes, `with ... as`,
//!    walrus, `global`), parameters, every call and every attribute chain
//!    that is used as a value.
//! 2. **Analyze.** Each call's callee is evaluated against those bindings.
//!    A chain rooted at a tracked SDK binding becomes a construction or a
//!    call; an SDK-shaped chain on an unknown receiver, `getattr` on a
//!    tracked binding and an unresolved model value become typed gaps.
//!
//! Names resolve the way Python scopes them, approximately: function locals,
//! parameters and imports shadow enclosing functions, class bodies are
//! skipped from inside methods, `global` re-homes an assignment to the
//! module, and a name assigned more than once combines every value it is
//! given. Assignment order and control flow are not modelled.

use super::{
    is_secret_key, is_value_key, model_family, BindingKind, CapturedValue, ClientConstruction,
    ClientScope, FileUsages, GapKind, Receiver, Routing, Sdk, SdkCall, SdkImport, Span, UsageGap,
    ValueOrigin,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Tree};

/// A file mentioning none of these cannot report anything, so it is not
/// walked at all.
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
    "generate_content",
    "generate_content_async",
    "generate_content_stream",
    "embed_content",
    "embed_content_async",
    "batch_embed_contents",
    "count_tokens",
    "count_tokens_async",
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
    "messages.create",
    "messages.stream",
    "messages.count_tokens",
    "models.generate_content",
    "models.generate_content_stream",
    "models.embed_content",
    "chats.create",
    "ChatCompletion.create",
    "ChatCompletion.acreate",
    "Completion.create",
    "Completion.acreate",
    "Embedding.create",
    "GenerativeModel",
];

/// Maximum evaluation depth; deeper chains are treated as unresolved.
const MAX_DEPTH: u32 = 64;

/// Extracts SDK usages from a parsed Python file.
pub fn extract(tree: &Tree, source: &str, file_path: &str) -> FileUsages {
    if !PREFILTER.iter().any(|needle| source.contains(needle)) {
        return FileUsages::new(file_path, "python");
    }
    let mut cx = Ctx::new(source, file_path);
    cx.collect(tree.root_node());
    cx.analyze();
    cx.finish()
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
        let custom = kwargs.iter().any(|k| k == "base_url" || k == "api_base");
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
    /// Assigned more than one kind of client, or a client and something else.
    ambiguous: bool,
    /// Known only from a type annotation, not from a construction.
    typed: bool,
}

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
struct Unresolved {
    reason: &'static str,
    shape: String,
    via: Option<String>,
}

impl Unresolved {
    fn new(reason: &'static str, shape: String) -> Self {
        Self {
            reason,
            shape,
            via: None,
        }
    }

    fn via(mut self, name: &str) -> Self {
        if self.via.is_none() {
            self.via = Some(name.to_string());
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
            "parameter" => "a parameter with no literal default",
            "reassigned" => "assigned more than once",
            "imported_name" => "imported from another module",
            "unbound" => "not defined in this file",
            "splat" => "possibly passed through **kwargs",
            _ => "not a string literal",
        }
    }
}

/// A captured value before it is attached to a key.
struct Resolved {
    value: String,
    origin: ValueOrigin,
    name: Option<String>,
    span: Span,
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

struct Scope {
    kind: ScopeKind,
    parent: Option<usize>,
    /// Nearest enclosing class (through functions), for `self`.
    class: Option<usize>,
    /// The symbol name the graph uses for this scope (`Bot.ask`).
    symbol: Option<String>,
    /// For a class: its own name.
    name: String,
    globals: HashSet<String>,
    /// For a class: base class names as written.
    bases: Vec<String>,
}

#[derive(Clone, Copy)]
struct Assign<'t> {
    /// The scope the value expression lives in.
    scope: usize,
    value: Option<Node<'t>>,
    annotation: Option<Node<'t>>,
    /// Bound by a loop, an unpacking or an augmented assignment.
    unknowable: bool,
}

#[derive(Clone, Copy)]
struct Param<'t> {
    annotation: Option<Node<'t>>,
    default: Option<Node<'t>>,
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

type Key = (usize, String);

struct Ctx<'t> {
    src: &'t str,
    file: String,
    scopes: Vec<Scope>,
    /// Tracked imports, keyed by the scope they bind in: a function for an
    /// import inside its body, the module for everything else.
    imports: HashMap<Key, Vec<Val>>,
    /// Other imports (for gap details), keyed the same way.
    other_imports: HashMap<Key, String>,
    sdk_imported: HashSet<Sdk>,
    assigns: HashMap<Key, Vec<Assign<'t>>>,
    attr_assigns: HashMap<Key, Vec<Assign<'t>>>,
    params: HashMap<Key, Param<'t>>,
    classes_by_name: HashMap<String, Vec<usize>>,
    config_assigns: Vec<ConfigAssign<'t>>,
    calls: Vec<(Node<'t>, usize)>,
    refs: Vec<(Node<'t>, usize)>,
    call_scope: HashMap<usize, usize>,
    constructions: HashMap<usize, usize>,
    name_memo: HashMap<Key, Val>,
    attr_memo: HashMap<Key, Val>,
    visiting: HashSet<(bool, usize, String)>,
    openai_routing: Routing,
    out: FileUsages,
}

impl<'t> Ctx<'t> {
    fn new(src: &'t str, file: &str) -> Self {
        Self {
            src,
            file: file.to_string(),
            scopes: vec![Scope {
                kind: ScopeKind::Module,
                parent: None,
                class: None,
                symbol: None,
                name: String::new(),
                globals: HashSet::new(),
                bases: Vec::new(),
            }],
            imports: HashMap::new(),
            other_imports: HashMap::new(),
            sdk_imported: HashSet::new(),
            assigns: HashMap::new(),
            attr_assigns: HashMap::new(),
            params: HashMap::new(),
            classes_by_name: HashMap::new(),
            config_assigns: Vec::new(),
            calls: Vec::new(),
            refs: Vec::new(),
            call_scope: HashMap::new(),
            constructions: HashMap::new(),
            name_memo: HashMap::new(),
            attr_memo: HashMap::new(),
            visiting: HashSet::new(),
            openai_routing: Routing::OpenaiDirect,
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
    /// tree-sitter column (in bytes) is `byte_column`.
    fn char_column(&self, byte: usize, byte_column: usize) -> u32 {
        let line_start = byte.saturating_sub(byte_column);
        self.src
            .get(line_start..byte)
            .map(|prefix| prefix.chars().count())
            .unwrap_or(byte_column) as u32
            + 1
    }

    fn symbol(&self, scope: usize) -> Option<String> {
        self.scopes[scope].symbol.clone()
    }

    // ------------------------------------------------------------------
    // Pass 1: collect
    // ------------------------------------------------------------------

    fn collect(&mut self, root: Node<'t>) {
        let mut stack = vec![(root, 0usize)];
        while let Some((node, scope)) = stack.pop() {
            let mut inner = scope;
            match node.kind() {
                "function_definition" => inner = self.enter_function(node, scope),
                "class_definition" => inner = self.enter_class(node, scope),
                "import_statement" => self.import_statement(node, scope),
                "import_from_statement" => self.import_from(node, scope),
                "assignment" => {
                    if let Some(left) = node.child_by_field_name("left") {
                        let value = node.child_by_field_name("right");
                        let annotation = node.child_by_field_name("type");
                        self.record_target(left, value, annotation, scope, false);
                    }
                }
                "augmented_assignment" => {
                    if let Some(left) = node.child_by_field_name("left") {
                        self.record_target(left, None, None, scope, true);
                    }
                }
                "with_item" => self.record_with_item(node, scope),
                "named_expression" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        let value = node.child_by_field_name("value");
                        self.record_target(name, value, None, scope, false);
                    }
                }
                "for_statement" | "for_in_clause" => {
                    if let Some(left) = node.child_by_field_name("left") {
                        self.record_target(left, None, None, scope, true);
                    }
                }
                "global_statement" => {
                    if self.scopes[scope].kind == ScopeKind::Function {
                        for i in 0..node.named_child_count() {
                            if let Some(name) = node.named_child(i) {
                                let name = self.text(name).to_string();
                                self.scopes[scope].globals.insert(name);
                            }
                        }
                    }
                }
                "call" => {
                    self.calls.push((node, scope));
                    self.call_scope.insert(node.id(), scope);
                }
                "attribute" if is_value_reference(node) => self.refs.push((node, scope)),
                _ => {}
            }
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i) {
                    stack.push((child, inner));
                }
            }
        }
    }

    fn enter_function(&mut self, node: Node<'t>, parent: usize) -> usize {
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
        let id = self.scopes.len();
        self.scopes.push(Scope {
            kind: ScopeKind::Function,
            parent: Some(parent),
            class,
            symbol: Some(symbol),
            name,
            globals: HashSet::new(),
            bases: Vec::new(),
        });
        if let Some(params) = node.child_by_field_name("parameters") {
            self.record_params(params, id);
        }
        id
    }

    fn record_params(&mut self, params: Node<'t>, scope: usize) {
        for i in 0..params.named_child_count() {
            let Some(param) = params.named_child(i) else {
                continue;
            };
            let (name, annotation, default) = match param.kind() {
                "identifier" => (Some(param), None, None),
                "typed_parameter" => {
                    let name = (0..param.named_child_count())
                        .filter_map(|j| param.named_child(j))
                        .find(|c| c.kind() == "identifier");
                    (name, param.child_by_field_name("type"), None)
                }
                "default_parameter" => (
                    param.child_by_field_name("name"),
                    None,
                    param.child_by_field_name("value"),
                ),
                "typed_default_parameter" => (
                    param.child_by_field_name("name"),
                    param.child_by_field_name("type"),
                    param.child_by_field_name("value"),
                ),
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    (param.named_child(0), None, None)
                }
                _ => (None, None, None),
            };
            if let Some(name) = name.filter(|n| n.kind() == "identifier") {
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
    }

    fn enter_class(&mut self, node: Node<'t>, parent: usize) -> usize {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        let mut bases = Vec::new();
        if let Some(list) = node.child_by_field_name("superclasses") {
            for i in 0..list.named_child_count() {
                if let Some(base) = list.named_child(i) {
                    if matches!(base.kind(), "identifier" | "attribute") {
                        bases.push(self.text(base).to_string());
                    }
                }
            }
        }
        let id = self.scopes.len();
        self.scopes.push(Scope {
            kind: ScopeKind::Class,
            parent: Some(parent),
            class: Some(id),
            symbol: Some(name.clone()),
            name: name.clone(),
            globals: HashSet::new(),
            bases,
        });
        self.classes_by_name.entry(name).or_default().push(id);
        id
    }

    fn record_with_item(&mut self, node: Node<'t>, scope: usize) {
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
            self.record_target(target, expr, None, scope, false);
        }
    }

    fn record_target(
        &mut self,
        target: Node<'t>,
        value: Option<Node<'t>>,
        annotation: Option<Node<'t>>,
        scope: usize,
        unknowable: bool,
    ) {
        let assign = Assign {
            scope,
            value,
            annotation,
            unknowable,
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
                self.config_assigns.push(ConfigAssign {
                    object,
                    attr,
                    value,
                    scope,
                });
            }
            "pattern_list"
            | "tuple_pattern"
            | "list_pattern"
            | "tuple"
            | "list"
            | "list_splat_pattern"
            | "parenthesized_expression" => {
                for i in 0..target.named_child_count() {
                    if let Some(child) = target.named_child(i) {
                        self.record_target(child, None, None, scope, true);
                    }
                }
            }
            _ => {}
        }
    }

    /// The scope an import or assignment of `name` in `scope` binds in:
    /// the function itself (unless the name is declared `global`), else the
    /// module. Class-body imports are treated as module-level.
    fn import_home(&self, scope: usize, name: &str) -> usize {
        let current = &self.scopes[scope];
        if current.kind == ScopeKind::Function && !current.globals.contains(name) {
            scope
        } else {
            0
        }
    }

    fn other_import(&mut self, scope: usize, local: String, module: String) {
        let home = self.import_home(scope, &local);
        self.other_imports.insert((home, local), module);
    }

    fn import_statement(&mut self, node: Node<'t>, scope: usize) {
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
                self.other_import(scope, local, module);
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
            self.add_import(binding, val, scope, item);
        }
    }

    fn import_from(&mut self, node: Node<'t>, scope: usize) {
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        let module = self.text(module_node).to_string();
        let relative = module_node.kind() == "relative_import";

        let mut cursor = node.walk();
        let names: Vec<Node<'t>> = node.children_by_field_name("name", &mut cursor).collect();
        let wildcard = (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
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
                    self.imports
                        .entry(key)
                        .or_default()
                        .push(Val::Class(*class));
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
                        self.other_import(scope, local, module.clone());
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
                self.add_import(binding, val, scope, item);
                continue;
            }

            let Some((pkg, rest)) = target.clone() else {
                self.other_import(scope, local, module.clone());
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
            self.add_import(binding, val, scope, item);
        }
    }

    /// Binds a tracked import in the scope it appears in and records it.
    fn add_import(&mut self, binding: Binding<'_>, val: Val, scope: usize, node: Node<'t>) {
        let key = (
            self.import_home(scope, binding.local),
            binding.local.to_string(),
        );
        self.imports.entry(key).or_default().push(val);
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
                self.resolve_name(&name, scope, depth + 1)
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
                    return self.resolve_attr(class, &attr, depth + 1);
                }
                let base = self.eval(object, scope, depth + 1);
                self.member(base, &attr)
            }
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return Val::untracked(Origin::Other);
                };
                let callee = self.eval(function, scope, depth + 1);
                self.apply_call(callee, node, scope, depth + 1)
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
                let count = node.named_child_count();
                let mut vals = Vec::new();
                for i in [0, count.saturating_sub(1)] {
                    if let Some(side) = node.named_child(i) {
                        vals.push(self.eval(side, scope, depth + 1));
                    }
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

    /// Whether scope `s` itself binds `name` (assignment, parameter or import).
    fn binds(&self, s: usize, name: &str) -> bool {
        let key = (s, name.to_string());
        self.assigns.contains_key(&key)
            || self.params.contains_key(&key)
            || self.imports.contains_key(&key)
            || self.other_imports.contains_key(&key)
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

    fn resolve_name(&mut self, name: &str, scope: usize, depth: u32) -> Val {
        let home = self.binding_scope(name, scope);
        let key = (home, name.to_string());
        if let Some(val) = self.name_memo.get(&key) {
            return val.clone();
        }
        let guard = (false, home, name.to_string());
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }

        let mut vals = Vec::new();
        if let Some(imports) = self.imports.get(&key) {
            vals.extend(imports.iter().cloned());
        }
        if let Some(param) = self.params.get(&key).copied() {
            vals.push(self.param_value(name, param, home, depth));
        }
        let assigns = self.assigns.get(&key).cloned().unwrap_or_default();
        for assign in assigns {
            vals.push(self.assign_value(assign, depth));
        }
        let val = if vals.is_empty() {
            match self.other_imports.get(&key) {
                Some(module) => Val::untracked(Origin::Imported {
                    module: module.clone(),
                }),
                None => Val::untracked(Origin::Unbound),
            }
        } else {
            combine(vals)
        };

        self.visiting.remove(&guard);
        self.name_memo.insert(key, val.clone());
        val
    }

    fn param_value(&mut self, name: &str, param: Param<'t>, scope: usize, depth: u32) -> Val {
        if let Some(annotation) = param.annotation {
            if let Some(val) = self.typed(annotation, scope, depth) {
                return val;
            }
        }
        if let Some(default) = param.default {
            let parent = self.scopes[scope].parent.unwrap_or(0);
            let val = self.eval(default, parent, depth + 1);
            if matches!(val, Val::Client { .. }) {
                return val;
            }
        }
        Val::untracked(Origin::Parameter {
            name: name.to_string(),
        })
    }

    fn assign_value(&mut self, assign: Assign<'t>, depth: u32) -> Val {
        if assign.unknowable {
            return Val::untracked(Origin::Other);
        }
        let value = assign.value.map(|v| self.eval(v, assign.scope, depth + 1));
        if let Some(val @ Val::Client { .. }) = &value {
            return val.clone();
        }
        if let Some(annotation) = assign.annotation {
            if let Some(val) = self.typed(annotation, assign.scope, depth) {
                return val;
            }
        }
        value.unwrap_or(Val::untracked(Origin::Placeholder))
    }

    /// A client typed by an annotation (`client: OpenAI`,
    /// `Optional[AzureOpenAI]`).
    fn typed(&mut self, annotation: Node<'t>, scope: usize, depth: u32) -> Option<Val> {
        let mut stack = vec![annotation];
        while let Some(node) = stack.pop() {
            let top_chain = matches!(node.kind(), "identifier" | "attribute")
                && node.parent().is_none_or(|p| p.kind() != "attribute");
            if top_chain {
                if let Val::Class(class) = self.eval(node, scope, depth + 1) {
                    return Some(Val::Client {
                        client: Client {
                            pkg: class.pkg,
                            routing: class.typed_routing(),
                            line: None,
                            ambiguous: false,
                            typed: true,
                        },
                        path: Vec::new(),
                    });
                }
                continue;
            }
            for i in (0..node.named_child_count()).rev() {
                if let Some(child) = node.named_child(i) {
                    stack.push(child);
                }
            }
        }
        None
    }

    /// All assignments to attribute `attr` of `class`: class-body names and
    /// `self.attr = ...`, falling back to same-file base classes.
    fn attr_assignments(&self, class: usize, attr: &str, depth: u32) -> Vec<Assign<'t>> {
        let key = (class, attr.to_string());
        let mut found: Vec<Assign<'t>> = self.assigns.get(&key).cloned().unwrap_or_default();
        found.extend(self.attr_assigns.get(&key).cloned().unwrap_or_default());
        if found.is_empty() && depth < 8 {
            for base in self.scopes[class].bases.clone() {
                if let Some([base_class]) = self.classes_by_name.get(&base).map(Vec::as_slice) {
                    let inherited = self.attr_assignments(*base_class, attr, depth + 1);
                    if !inherited.is_empty() {
                        return inherited;
                    }
                }
            }
        }
        found
    }

    fn resolve_attr(&mut self, class: usize, attr: &str, depth: u32) -> Val {
        let key = (class, attr.to_string());
        if let Some(val) = self.attr_memo.get(&key) {
            return val.clone();
        }
        let guard = (true, class, attr.to_string());
        if !self.visiting.insert(guard.clone()) {
            return Val::untracked(Origin::Cycle);
        }
        let assigns = self.attr_assignments(class, attr, 0);
        let val = if assigns.is_empty() {
            Val::untracked(Origin::UnassignedAttribute {
                name: attr.to_string(),
            })
        } else {
            let vals = assigns
                .into_iter()
                .map(|a| self.assign_value(a, depth))
                .collect();
            combine(vals)
        };
        self.visiting.remove(&guard);
        self.attr_memo.insert(key, val.clone());
        val
    }

    fn member(&mut self, base: Val, attr: &str) -> Val {
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
                    // Copies of the same client.
                    "with_options" | "copy" => Val::Client {
                        client,
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
                    .map(|f| self.render(f))
                    .unwrap_or_else(|| "<expr>".to_string());
                let wraps = self.wrapped_pkg(call, scope, depth);
                Val::untracked(Origin::Factory { callee, wraps })
            }
        }
    }

    /// The SDK of a tracked client or constructor passed to a call
    /// (`instructor.from_openai(OpenAI())`).
    fn wrapped_pkg(&mut self, call: Node<'t>, scope: usize, depth: u32) -> Option<Pkg> {
        let args = call.child_by_field_name("arguments")?;
        for i in 0..args.named_child_count() {
            let Some(arg) = args.named_child(i) else {
                continue;
            };
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
        if let Some(&index) = self.constructions.get(&call.id()) {
            let built = &self.out.clients[index];
            return Client {
                pkg: class.pkg,
                routing: built.routing,
                line: Some(built.span.line),
                ambiguous: false,
                typed: false,
            };
        }
        let scope = self.call_scope.get(&call.id()).copied().unwrap_or(0);
        let args = self.arguments(call);
        let routing = class.routing(&args.kwargs, args.splat);
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
            ambiguous: false,
            typed: false,
        }
    }

    /// The name a constructed client is assigned to, and how it is held.
    fn binding_target(&self, call: Node<'t>, scope: usize) -> (Option<String>, ClientScope) {
        let mut child = call;
        let mut parent = call.parent();
        while let Some(p) = parent {
            if matches!(
                p.kind(),
                "parenthesized_expression"
                    | "boolean_operator"
                    | "conditional_expression"
                    | "await"
            ) {
                child = p;
                parent = p.parent();
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
                (Some(name), held)
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
        for (call, scope) in self.calls.clone() {
            self.analyze_call(call, scope);
        }
        for (reference, scope) in self.refs.clone() {
            self.analyze_reference(reference, scope);
        }
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
                };
                self.record_call(call, function, scope, target);
            }
            Val::Client { client, path } if !path.is_empty() => {
                let target = CallTarget {
                    pkg: client.pkg,
                    member: path.join("."),
                    receiver: Receiver::Client,
                    routing: client.routing,
                    client_line: client.line,
                    ambiguous: client.ambiguous,
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

        // An unresolved model matters where it reaches a request or a model
        // object; on an SDK type (`ChatCompletion(model=model)`) the call
        // still lists `model` in its kwargs, without a gap.
        let gap_worthy = is_request(&target.member) || is_model_bearing(&target.member);
        let mut values = Vec::new();
        let mut keyed = false;
        for (key, expr) in &args.value_args {
            keyed = true;
            match self.resolve_value(*expr, scope, 0) {
                Ok(resolved) => values.push(captured(key, resolved, false)),
                Err(unresolved) if gap_worthy => {
                    self.unresolved_gap(sdk, call, &chain, key, &unresolved, scope)
                }
                Err(_) => {}
            }
        }

        // `GenerativeModel("gemini-pro")`: the model name is positional.
        if target.member == "GenerativeModel" && !keyed {
            if let Some(first) = args.first_positional {
                keyed = true;
                match self.resolve_value(first, scope, 0) {
                    Ok(resolved) => values.push(captured("model_name", resolved, true)),
                    Err(unresolved) => {
                        self.unresolved_gap(sdk, call, &chain, "model_name", &unresolved, scope)
                    }
                }
            }
        }

        for (keyword, expr) in &args.literal_args {
            let prefix = keyword
                .as_ref()
                .map(|k| format!("{k}."))
                .unwrap_or_default();
            self.nested_values(*expr, &prefix, keyword.is_none(), scope, 0, &mut values);
        }

        if !keyed && args.splat && is_model_bearing(&target.member) {
            let key = if target.member == "GenerativeModel" {
                "model_name"
            } else {
                "model"
            };
            let unresolved = Unresolved::new("splat", "**kwargs".to_string());
            self.unresolved_gap(sdk, call, &chain, key, &unresolved, scope);
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
        }

        self.out.calls.push(SdkCall {
            sdk,
            package: target.pkg.package().to_string(),
            chain,
            request: is_request(&target.member),
            member: target.member,
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

    fn unresolved_gap(
        &mut self,
        sdk: Sdk,
        call: Node<'t>,
        chain: &str,
        key: &str,
        unresolved: &Unresolved,
        scope: usize,
    ) {
        let phrase = unresolved.phrase();
        let shape = &unresolved.shape;
        let detail = match unresolved.via.as_deref() {
            _ if unresolved.reason == "splat" => {
                format!("`{key}` for `{chain}` is not passed by name and may come from `**kwargs`")
            }
            Some(via) if !shape.is_empty() && via != shape => {
                format!("`{key}` for `{chain}` comes from `{via}`, which is {phrase} (`{shape}`)")
            }
            Some(via) => format!("`{key}` for `{chain}` comes from `{via}`, which is {phrase}"),
            None => format!("`{key}` for `{chain}` is `{shape}`, which is {phrase}"),
        };
        self.out.gaps.push(UsageGap {
            kind: GapKind::UnresolvedModel,
            sdk: Some(sdk),
            file: self.file.clone(),
            span: Some(self.span(call)),
            symbol: self.symbol(scope),
            chain: Some(chain.to_string()),
            key: Some(key.to_string()),
            reason: unresolved.reason.to_string(),
            detail,
            values: Vec::new(),
        });
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
                format!("`{root}` comes from parameter `{name}`, which has no SDK type annotation"),
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
        for i in 0..list.named_child_count() {
            let Some(arg) = list.named_child(i) else {
                continue;
            };
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
                "dictionary_splat" => args.splat = true,
                "list_splat" | "comment" => {}
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

    /// Allowlisted keys inside dict literals passed to a call
    /// (`fallbacks=[{"model": "claude-opus-4-8"}]`). Only keys
    /// [`is_value_key`] accepts are read; an unresolvable nested value is
    /// skipped, not reported, because the argument may not configure the
    /// request at all.
    fn nested_values(
        &mut self,
        node: Node<'t>,
        prefix: &str,
        positional: bool,
        scope: usize,
        depth: u32,
        out: &mut Vec<CapturedValue>,
    ) {
        if depth > 4 {
            return;
        }
        match node.kind() {
            "dictionary" => {
                for i in 0..node.named_child_count() {
                    let Some(pair) = node.named_child(i).filter(|p| p.kind() == "pair") else {
                        continue;
                    };
                    let (Some(key), Some(value)) = (
                        pair.child_by_field_name("key"),
                        pair.child_by_field_name("value"),
                    ) else {
                        continue;
                    };
                    let Ok(key) = self.literal(key) else {
                        continue;
                    };
                    // Nothing under a credential-shaped key is read, at any
                    // depth (`extra_headers={"auth": {...}}`).
                    if is_secret_key(&key) {
                        continue;
                    }
                    let path = format!("{prefix}{key}");
                    if is_value_key(&key) {
                        if let Ok(resolved) = self.resolve_value(value, scope, 0) {
                            out.push(captured(&path, resolved, positional));
                        }
                    } else if is_container(value) {
                        let nested = format!("{path}.");
                        self.nested_values(value, &nested, positional, scope, depth + 1, out);
                    }
                }
            }
            "list" | "tuple" | "set" | "parenthesized_expression" => {
                for i in 0..node.named_child_count() {
                    if let Some(item) = node.named_child(i) {
                        self.nested_values(item, prefix, positional, scope, depth + 1, out);
                    }
                }
            }
            _ => {}
        }
    }

    /// A plain string literal's value. `Err` carries the reason it is not.
    fn literal(&self, node: Node<'t>) -> Result<String, &'static str> {
        match node.kind() {
            "string" => {
                let mut value = String::new();
                for i in 0..node.child_count() {
                    let Some(part) = node.child(i) else {
                        continue;
                    };
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
                for i in 0..node.named_child_count() {
                    if let Some(part) = node.named_child(i) {
                        value.push_str(&self.literal(part)?);
                    }
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
        match expr.kind() {
            "string" | "concatenated_string" => match self.literal(expr) {
                Ok(value) => Ok(Resolved {
                    value,
                    origin: ValueOrigin::Literal,
                    name: None,
                    span: self.span(expr),
                }),
                Err(reason) => Err(Unresolved::new(reason, self.render(expr))),
            },
            "parenthesized_expression" => match expr.named_child(0) {
                Some(inner) => self.resolve_value(inner, scope, depth + 1),
                None => Err(Unresolved::new("non_literal", self.render(expr))),
            },
            "identifier" => {
                let name = self.text(expr).to_string();
                self.resolve_constant(&name, scope, depth)
            }
            "attribute" => {
                let rendered = self.render(expr);
                if let (Some(object), Some(attr)) = (
                    expr.child_by_field_name("object"),
                    expr.child_by_field_name("attribute"),
                ) {
                    if let Some(class) = self.class_receiver(object, scope) {
                        let attr = self.text(attr).to_string();
                        let assigns = self.attr_assignments(class, &attr, 0);
                        return self
                            .single_constant(&assigns, ValueOrigin::ClassConstant, depth)
                            .map(|mut r| {
                                if r.origin != ValueOrigin::ParameterDefault {
                                    r.name = Some(rendered.clone());
                                }
                                r
                            })
                            .map_err(|u| u.via(&rendered));
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
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "function_call"
                };
                Err(Unresolved::new(reason, self.render(expr)))
            }
            "subscript" => {
                let reason = if self.contains_env(expr) {
                    "env_var"
                } else {
                    "subscript"
                };
                Err(Unresolved::new(reason, self.render(expr)))
            }
            "conditional_expression" | "boolean_operator" => {
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

    fn resolve_constant(
        &mut self,
        name: &str,
        scope: usize,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        let home = self.binding_scope(name, scope);
        let key = (home, name.to_string());
        let assigns = self.assigns.get(&key).cloned().unwrap_or_default();
        let result = if let Some(param) = self.params.get(&key).copied() {
            if !assigns.is_empty() {
                Err(Unresolved::new("reassigned", name.to_string()))
            } else {
                match param.default.map(|d| (d, self.literal(d))) {
                    Some((default, Ok(value))) => Ok(Resolved {
                        value,
                        origin: ValueOrigin::ParameterDefault,
                        name: Some(name.to_string()),
                        span: self.span(default),
                    }),
                    _ => Err(Unresolved::new("parameter", name.to_string())),
                }
            }
        } else if !assigns.is_empty() {
            let origin = match self.scopes[home].kind {
                ScopeKind::Module => ValueOrigin::ModuleConstant,
                ScopeKind::Class => ValueOrigin::ClassConstant,
                ScopeKind::Function => ValueOrigin::LocalConstant,
            };
            self.single_constant(&assigns, origin, depth)
                .map(|mut r| {
                    if r.origin != ValueOrigin::ParameterDefault {
                        r.name = Some(name.to_string());
                    }
                    r
                })
                .map_err(|u| u.via(name))
        } else if self.imports.contains_key(&key) || self.other_imports.contains_key(&key) {
            Err(Unresolved::new("imported_name", name.to_string()))
        } else {
            Err(Unresolved::new("unbound", name.to_string()))
        };
        result
    }

    /// A name or attribute assigned exactly once, to something that itself
    /// resolves to a string literal.
    fn single_constant(
        &mut self,
        assigns: &[Assign<'t>],
        origin: ValueOrigin,
        depth: u32,
    ) -> Result<Resolved, Unresolved> {
        match assigns {
            [] => Err(Unresolved::new("unbound", String::new())),
            [assign] => {
                let Some(value) = assign.value.filter(|_| !assign.unknowable) else {
                    return Err(Unresolved::new("non_literal", String::new()));
                };
                let mut resolved = self.resolve_value(value, assign.scope, depth + 1)?;
                if resolved.origin == ValueOrigin::Literal {
                    resolved.origin = origin;
                }
                Ok(resolved)
            }
            many => Err(Unresolved::new(
                "reassigned",
                format!("{} assignments", many.len()),
            )),
        }
    }

    /// Whether `node` reads the process environment
    /// (`os.getenv(...)`, `os.environ[...]`, `os.environ.get(...)`).
    fn contains_env(&self, node: Node<'t>) -> bool {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let rendered = match current.kind() {
                "call" => current
                    .child_by_field_name("function")
                    .map(|f| self.render(f)),
                "subscript" => current.child_by_field_name("value").map(|v| self.render(v)),
                "attribute" => Some(self.render(current)),
                _ => None,
            };
            if let Some(rendered) = rendered {
                if matches!(
                    rendered.as_str(),
                    "os.getenv"
                        | "getenv"
                        | "os.environ"
                        | "environ"
                        | "os.environ.get"
                        | "environ.get"
                ) {
                    return true;
                }
            }
            for i in 0..current.named_child_count() {
                if let Some(child) = current.named_child(i) {
                    stack.push(child);
                }
            }
        }
        false
    }

    /// An expression's shape with every argument and literal elided, so a
    /// rendering can never carry a value.
    fn render(&self, node: Node<'t>) -> String {
        self.render_depth(node, 0)
    }

    fn render_depth(&self, node: Node<'t>, depth: u32) -> String {
        if depth > MAX_DEPTH {
            return "<expr>".to_string();
        }
        stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || match node.kind() {
            "identifier" => self.text(node).to_string(),
            "attribute" => {
                let object = node
                    .child_by_field_name("object")
                    .map(|o| self.render_depth(o, depth + 1))
                    .unwrap_or_default();
                let attr = node
                    .child_by_field_name("attribute")
                    .map(|a| self.text(a))
                    .unwrap_or_default();
                format!("{object}.{attr}")
            }
            "call" => {
                let function = node
                    .child_by_field_name("function")
                    .map(|f| self.render_depth(f, depth + 1))
                    .unwrap_or_default();
                if self.has_args(node) {
                    format!("{function}(...)")
                } else {
                    format!("{function}()")
                }
            }
            "subscript" => {
                let value = node
                    .child_by_field_name("value")
                    .map(|v| self.render_depth(v, depth + 1))
                    .unwrap_or_default();
                format!("{value}[...]")
            }
            "parenthesized_expression" | "await" => node
                .named_child(0)
                .map(|inner| self.render_depth(inner, depth + 1))
                .unwrap_or_else(|| "<expr>".to_string()),
            "string" => {
                let interpolated = (0..node.child_count())
                    .filter_map(|i| node.child(i))
                    .any(|c| c.kind() == "interpolation");
                if interpolated {
                    "f\"...\"".to_string()
                } else {
                    "\"...\"".to_string()
                }
            }
            "concatenated_string" => "\"...\"".to_string(),
            "conditional_expression" => "<conditional>".to_string(),
            "boolean_operator" => "<boolean>".to_string(),
            _ => "<expr>".to_string(),
        })
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

/// A dict, list, tuple or set literal.
fn is_container(node: Node<'_>) -> bool {
    matches!(node.kind(), "dictionary" | "list" | "tuple" | "set")
}

fn is_model_bearing(member: &str) -> bool {
    MODEL_BEARING
        .iter()
        .any(|m| member == *m || member.ends_with(&format!(".{m}")))
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
/// not the object of a longer chain, not an assignment target.
fn is_value_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let is_field = |field: &str| {
        parent
            .child_by_field_name(field)
            .is_some_and(|child| child.id() == node.id())
    };
    match parent.kind() {
        "attribute" => !is_field("object"),
        "call" => !is_field("function"),
        "assignment" | "augmented_assignment" => !is_field("left"),
        "as_pattern_target" | "delete_statement" => false,
        _ => true,
    }
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
                ambiguous,
                typed: false,
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
