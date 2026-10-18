//! Where a client with a `base_url` really sends its requests.
//!
//! [`crate::usages`] marks every OpenAI or Anthropic client given a
//! `base_url` as [`Routing::CustomBaseUrl`], from the keyword's name alone.
//! Many such clients still reach the provider: `base_url` is the provider's
//! own host, `None` (the SDK default), or an environment lookup that falls
//! back to the provider. Hiding their calls as routed would hide real
//! retirements, so this module reads the argument's *value* and classes it
//! ([`BaseUrl`]). [`super::check`] then treats a provider host as a direct
//! call, a value it cannot read as routing unknown (listed, not hidden), and
//! any other host as routed.
//!
//! Only the `base_url` / `api_base` argument of a construction the usage
//! scan already found is read, from source text the caller passes in. The
//! URL itself is never reported: only its class.

use crate::usages::{ClientConstruction, Routing, Sdk};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use tree_sitter::{Node, Parser};

/// How a client's `base_url` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseUrl {
    /// The provider's own API host (`https://api.openai.com/v1`), directly
    /// or through a same-file constant, or `None` (the SDK default).
    ProviderHost,
    /// An environment lookup whose fallback is the provider's host
    /// (`os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1")`): what
    /// the SDK itself does when no `base_url` is given.
    EnvProviderDefault,
    /// An environment lookup with no fallback (`os.environ.get("BASE_URL")`,
    /// `os.environ["BASE_URL"]`): the host depends on the deployment.
    EnvOnly,
    /// Another host (a literal, a constant or an environment fallback):
    /// OpenRouter, LiteLLM, a local server, a proxy.
    OtherHost,
    /// A value Arbor cannot read (a function call, an attribute, an
    /// f-string, a name from another module).
    Unreadable,
}

/// The [`BaseUrl`] of one client construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientBaseUrl {
    pub file: String,
    /// Line of the construction ([`crate::usages::SdkCall::client_line`]).
    pub line: u32,
    pub base_url: BaseUrl,
}

/// Classes the `base_url` of every [`Routing::CustomBaseUrl`] client.
/// `source_of` returns a file's text by the path the scan reported; a file
/// it cannot return keeps its clients routed.
pub fn read_base_urls(
    clients: &[ClientConstruction],
    mut source_of: impl FnMut(&str) -> Option<String>,
) -> Vec<ClientBaseUrl> {
    let mut by_file: BTreeMap<&str, Vec<&ClientConstruction>> = BTreeMap::new();
    for client in clients {
        if client.routing == Routing::CustomBaseUrl
            && matches!(client.sdk, Sdk::Openai | Sdk::Anthropic)
        {
            by_file
                .entry(client.file.as_str())
                .or_default()
                .push(client);
        }
    }
    let mut out = Vec::new();
    for (file, clients) in by_file {
        let Some(source) = source_of(file) else {
            continue;
        };
        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_python::language())
            .is_err()
        {
            continue;
        }
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let reader = Reader::new(&source, tree.root_node());
        for client in clients {
            let base_url = reader
                .construction(
                    client.span.start_byte as usize,
                    client.span.end_byte as usize,
                )
                .map(|call| reader.classify_call(call, client.sdk))
                .unwrap_or(BaseUrl::Unreadable);
            out.push(ClientBaseUrl {
                file: file.to_string(),
                line: client.span.line,
                base_url,
            });
        }
    }
    out
}

/// The provider's own API host.
fn provider_host(sdk: Sdk) -> Option<&'static str> {
    match sdk {
        Sdk::Openai => Some("api.openai.com"),
        Sdk::Anthropic => Some("api.anthropic.com"),
        Sdk::Google => None,
    }
}

/// `https://api.openai.com/v1` → `api.openai.com`; `None` unless the URL is
/// `https` with a host.
fn https_host(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, "443")) => host,
        Some(_) => return None,
        None => authority,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Literal classes before the environment is considered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Value {
    Provider,
    Other,
    /// `None`: the SDK default.
    Default,
    Env(Option<Fallback>),
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fallback {
    Provider,
    Other,
    Unknown,
}

const MAX_DEPTH: u32 = 8;

struct Reader<'s> {
    source: &'s str,
    root: Node<'s>,
    /// Module-level names assigned exactly once, with the assigned value.
    constants: HashMap<&'s str, Option<Node<'s>>>,
}

impl<'s> Reader<'s> {
    fn new(source: &'s str, root: Node<'s>) -> Self {
        let mut constants: HashMap<&'s str, Option<Node<'s>>> = HashMap::new();
        let mut cursor = root.walk();
        for statement in root.named_children(&mut cursor) {
            if statement.kind() != "expression_statement" {
                continue;
            }
            let Some(assignment) = statement.named_child(0) else {
                continue;
            };
            if assignment.kind() != "assignment" {
                continue;
            }
            let (Some(left), right) = (
                assignment.child_by_field_name("left"),
                assignment.child_by_field_name("right"),
            ) else {
                continue;
            };
            if left.kind() != "identifier" {
                continue;
            }
            let name = &source[left.byte_range()];
            // A name assigned twice could hold either value.
            constants
                .entry(name)
                .and_modify(|value| *value = None)
                .or_insert(right);
        }
        Self {
            source,
            root,
            constants,
        }
    }

    fn text(&self, node: Node<'_>) -> &'s str {
        &self.source[node.byte_range()]
    }

    /// The call node spanning exactly `start..end`.
    fn construction(&self, start: usize, end: usize) -> Option<Node<'s>> {
        let mut node = self.root.descendant_for_byte_range(start, end)?;
        loop {
            if node.kind() == "call" && node.start_byte() == start && node.end_byte() == end {
                return Some(node);
            }
            if node.start_byte() < start || node.end_byte() > end {
                return None;
            }
            node = node.parent()?;
        }
    }

    fn classify_call(&self, call: Node<'s>, sdk: Sdk) -> BaseUrl {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return BaseUrl::Unreadable;
        };
        let mut value = None;
        let mut cursor = arguments.walk();
        for argument in arguments.named_children(&mut cursor) {
            if argument.kind() != "keyword_argument" {
                continue;
            }
            let name = argument
                .child_by_field_name("name")
                .map(|n| self.text(n))
                .unwrap_or_default();
            if name == "base_url" || name == "api_base" {
                value = argument.child_by_field_name("value");
            }
        }
        let Some(value) = value else {
            // Given through `**kwargs`: not readable here.
            return BaseUrl::Unreadable;
        };
        match self.value(value, sdk, 0) {
            Value::Provider | Value::Default => BaseUrl::ProviderHost,
            Value::Other => BaseUrl::OtherHost,
            Value::Env(None) => BaseUrl::EnvOnly,
            Value::Env(Some(Fallback::Provider)) => BaseUrl::EnvProviderDefault,
            Value::Env(Some(Fallback::Other)) => BaseUrl::OtherHost,
            Value::Env(Some(Fallback::Unknown)) | Value::Unknown => BaseUrl::Unreadable,
        }
    }

    fn value(&self, node: Node<'s>, sdk: Sdk, depth: u32) -> Value {
        if depth > MAX_DEPTH {
            return Value::Unknown;
        }
        match node.kind() {
            "none" => Value::Default,
            "string" | "concatenated_string" => match self.literal(node) {
                Some(url) => match (https_host(&url), provider_host(sdk)) {
                    (Some(host), Some(provider)) if host == provider => Value::Provider,
                    _ => Value::Other,
                },
                None => Value::Unknown,
            },
            "parenthesized_expression" => node
                .named_child(0)
                .map_or(Value::Unknown, |inner| self.value(inner, sdk, depth + 1)),
            "identifier" => match self.constants.get(self.text(node)) {
                Some(Some(assigned)) => self.value(*assigned, sdk, depth + 1),
                _ => Value::Unknown,
            },
            "subscript" => {
                // `os.environ["BASE_URL"]`: no fallback.
                let value = node.child_by_field_name("value").map(|v| self.text(v));
                if matches!(value, Some("os.environ" | "environ")) {
                    Value::Env(None)
                } else {
                    Value::Unknown
                }
            }
            "call" => self.env_lookup(node, sdk, depth),
            "boolean_operator" => {
                // `os.getenv("BASE_URL") or "https://..."`.
                let operator = node
                    .child_by_field_name("operator")
                    .map(|o| self.text(o))
                    .unwrap_or_default();
                let (Some(left), Some(right)) = (
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                ) else {
                    return Value::Unknown;
                };
                match (operator, self.value(left, sdk, depth + 1)) {
                    ("or", Value::Env(None)) => {
                        Value::Env(Some(fallback(self.value(right, sdk, depth + 1))))
                    }
                    _ => Value::Unknown,
                }
            }
            _ => Value::Unknown,
        }
    }

    /// `os.getenv(key[, default])`, `os.environ.get(key[, default])` and
    /// their `from os import ...` forms.
    fn env_lookup(&self, call: Node<'s>, sdk: Sdk, depth: u32) -> Value {
        let function = call
            .child_by_field_name("function")
            .map(|f| self.text(f))
            .unwrap_or_default();
        if !matches!(
            function,
            "os.getenv" | "getenv" | "os.environ.get" | "environ.get"
        ) {
            return Value::Unknown;
        }
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return Value::Unknown;
        };
        let mut positional = Vec::new();
        let mut default = None;
        let mut cursor = arguments.walk();
        for argument in arguments.named_children(&mut cursor) {
            match argument.kind() {
                "keyword_argument" => {
                    let name = argument
                        .child_by_field_name("name")
                        .map(|n| self.text(n))
                        .unwrap_or_default();
                    if name == "default" {
                        default = argument.child_by_field_name("value");
                    } else {
                        return Value::Unknown;
                    }
                }
                "comment" => {}
                "list_splat" | "dictionary_splat" => return Value::Unknown,
                _ => positional.push(argument),
            }
        }
        if positional.len() > 2 || positional.is_empty() {
            return Value::Unknown;
        }
        let default = default.or(positional.get(1).copied());
        match default {
            None => Value::Env(None),
            Some(node) => match self.value(node, sdk, depth + 1) {
                Value::Default => Value::Env(None),
                other => Value::Env(Some(fallback(other))),
            },
        }
    }

    /// A plain string literal's value: no f-string, bytes or escapes.
    fn literal(&self, node: Node<'s>) -> Option<String> {
        match node.kind() {
            "string" => {
                let mut value = String::new();
                let mut cursor = node.walk();
                for part in node.children(&mut cursor) {
                    match part.kind() {
                        "string_start" => {
                            let prefix = self.text(part).to_ascii_lowercase();
                            if prefix.contains('b') || prefix.contains('f') {
                                return None;
                            }
                        }
                        "string_content" => value.push_str(self.text(part)),
                        "interpolation" | "escape_sequence" => return None,
                        _ => {}
                    }
                }
                (!value.contains('\\')).then_some(value)
            }
            "concatenated_string" => {
                let mut value = String::new();
                let mut cursor = node.walk();
                for part in node.named_children(&mut cursor) {
                    value.push_str(&self.literal(part)?);
                }
                Some(value)
            }
            _ => None,
        }
    }
}

fn fallback(value: Value) -> Fallback {
    match value {
        Value::Provider => Fallback::Provider,
        Value::Other => Fallback::Other,
        _ => Fallback::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usages::{ClientScope, Span};

    /// Clients for every `OpenAI(` / `Anthropic(` construction in `source`.
    fn clients(source: &str) -> Vec<ClientConstruction> {
        let mut out = Vec::new();
        for (line_index, line) in source.lines().enumerate() {
            for (name, sdk) in [("OpenAI(", Sdk::Openai), ("Anthropic(", Sdk::Anthropic)] {
                let Some(column) = line.find(name) else {
                    continue;
                };
                let line_start: usize = source.lines().take(line_index).map(|l| l.len() + 1).sum();
                let start = line_start + column;
                let end = line_start + line.rfind(')').unwrap() + 1;
                out.push(ClientConstruction {
                    sdk,
                    package: sdk.as_str().into(),
                    constructor: name.trim_end_matches('(').into(),
                    chain: name.trim_end_matches('(').into(),
                    binding: None,
                    scope: ClientScope::Module,
                    routing: Routing::CustomBaseUrl,
                    is_async: false,
                    kwargs: vec!["base_url".into()],
                    splat: false,
                    file: "app.py".into(),
                    span: Span {
                        line: line_index as u32 + 1,
                        column: column as u32 + 1,
                        end_line: line_index as u32 + 1,
                        end_column: 1,
                        start_byte: start as u32,
                        end_byte: end as u32,
                    },
                    symbol: None,
                });
            }
        }
        out
    }

    fn classes(source: &str) -> Vec<BaseUrl> {
        read_base_urls(&clients(source), |_| Some(source.to_string()))
            .into_iter()
            .map(|c| c.base_url)
            .collect()
    }

    #[test]
    fn provider_hosts_and_the_sdk_default_are_direct() {
        let source = r#"import os
URL = "https://api.openai.com/v1"
a = OpenAI(base_url="https://api.openai.com/v1")
b = OpenAI(base_url="https://API.openai.com:443/v1/")
c = OpenAI(base_url=None)
d = OpenAI(base_url=URL)
e = Anthropic(base_url="https://api.anthropic.com")
f = OpenAI(base_url=os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1"))
g = OpenAI(base_url=os.environ.get("OPENAI_BASE_URL", default="https://api.openai.com/v1"))
h = OpenAI(base_url=os.getenv("OPENAI_BASE_URL") or "https://api.openai.com/v1")
"#;
        assert_eq!(
            classes(source),
            [
                BaseUrl::ProviderHost,
                BaseUrl::ProviderHost,
                BaseUrl::ProviderHost,
                BaseUrl::ProviderHost,
                BaseUrl::ProviderHost,
                BaseUrl::EnvProviderDefault,
                BaseUrl::EnvProviderDefault,
                BaseUrl::EnvProviderDefault,
            ]
        );
    }

    #[test]
    fn other_hosts_are_routed_and_unknowns_are_not_guessed() {
        let source = r#"import os
from os import getenv
PROXY = "http://localhost:4000"
TWICE = "https://api.openai.com/v1"
TWICE = "https://openrouter.ai/api/v1"
a = OpenAI(base_url="https://openrouter.ai/api/v1")
b = OpenAI(base_url="http://api.openai.com/v1")
c = OpenAI(base_url=PROXY)
d = OpenAI(base_url=os.getenv("OPENAI_BASE_URL", "http://localhost:4000"))
e = Anthropic(base_url="https://api.openai.com/v1")
f = OpenAI(base_url=os.environ.get("OPENAI_BASE_URL"))
g = OpenAI(base_url=os.environ["LLM_PROXY"])
h = OpenAI(base_url=getenv("X", None))
i = OpenAI(base_url=settings.base_url)
j = OpenAI(base_url=f"https://{host}/v1")
k = OpenAI(base_url=TWICE)
l = OpenAI(base_url="https://user:pw@api.openai.com/v1")
"#;
        assert_eq!(
            classes(source),
            [
                BaseUrl::OtherHost,
                BaseUrl::OtherHost,
                BaseUrl::OtherHost,
                BaseUrl::OtherHost,
                BaseUrl::OtherHost,
                BaseUrl::EnvOnly,
                BaseUrl::EnvOnly,
                BaseUrl::EnvOnly,
                BaseUrl::Unreadable,
                BaseUrl::Unreadable,
                BaseUrl::Unreadable,
                BaseUrl::OtherHost,
            ]
        );
    }

    #[test]
    fn unreadable_files_keep_their_routing() {
        let source = "a = OpenAI(base_url=\"https://api.openai.com/v1\")\n";
        assert!(read_base_urls(&clients(source), |_| None).is_empty());
    }
}
