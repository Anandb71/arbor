use super::extract;
use crate::usages::{
    BindingKind, ClientScope, FileUsages, GapKind, Receiver, Routing, Sdk, SdkCall, UsageGap,
    ValueOrigin,
};
use std::collections::HashMap;

fn usages(source: &str) -> FileUsages {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::language())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    extract(&tree, source, "app.py")
}

fn call<'a>(found: &'a FileUsages, chain: &str) -> &'a SdkCall {
    found
        .calls
        .iter()
        .find(|c| c.chain == chain)
        .unwrap_or_else(|| {
            let chains: Vec<&str> = found.calls.iter().map(|c| c.chain.as_str()).collect();
            panic!("no call `{chain}`; calls: {chains:?}")
        })
}

fn value<'a>(call: &'a SdkCall, key: &str) -> Option<&'a str> {
    call.values
        .iter()
        .find(|v| v.key == key)
        .map(|v| v.value.as_str())
}

fn gaps(found: &FileUsages, kind: GapKind) -> Vec<&UsageGap> {
    found.gaps.iter().filter(|g| g.kind == kind).collect()
}

fn routing_of(found: &FileUsages, binding: &str) -> Routing {
    found
        .clients
        .iter()
        .find(|c| c.binding.as_deref() == Some(binding))
        .unwrap_or_else(|| panic!("no client bound to {binding}"))
        .routing
}

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

#[test]
fn records_every_import_form() {
    let found = usages(
        r#"
import openai
import openai as oa
from openai import OpenAI, AsyncOpenAI, AzureOpenAI, AsyncAzureOpenAI as AAO
from anthropic import Anthropic, AsyncAnthropic, AnthropicBedrock, AnthropicVertex, HUMAN_PROMPT
import google.generativeai as genai
from google import genai as gg
from google.genai import types
import json
from .local import helper
"#,
    );
    let local = |name: &str| {
        found
            .imports
            .iter()
            .find(|i| i.local == name)
            .unwrap_or_else(|| panic!("no import bound to {name}"))
    };
    assert_eq!(local("openai").kind, BindingKind::Module);
    assert_eq!(local("oa").module, "openai");
    assert_eq!(local("OpenAI").kind, BindingKind::ClientClass);
    assert_eq!(local("AAO").name.as_deref(), Some("AsyncAzureOpenAI"));
    assert_eq!(local("AnthropicBedrock").sdk, Sdk::Anthropic);
    assert_eq!(local("HUMAN_PROMPT").kind, BindingKind::Symbol);
    assert_eq!(local("genai").package, "google.generativeai");
    assert_eq!(local("gg").package, "google.genai");
    assert_eq!(local("types").package, "google.genai");
    assert!(found.imports.iter().all(|i| i.local != "json"));
    assert!(found.imports.iter().all(|i| i.local != "helper"));
    // 1-based span of the binding.
    assert_eq!(local("openai").span.line, 2);
    assert_eq!(local("openai").span.column, 8);
}

// ---------------------------------------------------------------------------
// Constructions and routing
// ---------------------------------------------------------------------------

#[test]
fn module_level_openai_client_and_call() {
    let found = usages(
        "from openai import OpenAI\n\
         client = OpenAI()\n\
         resp = client.chat.completions.create(model=\"gpt-3.5-turbo\", messages=[])\n",
    );
    let built = &found.clients[0];
    assert_eq!(built.constructor, "openai.OpenAI");
    assert_eq!(built.binding.as_deref(), Some("client"));
    assert_eq!(built.scope, ClientScope::Module);
    assert_eq!(built.routing, Routing::OpenaiDirect);
    assert!(!built.is_async);

    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.sdk, Sdk::Openai);
    assert_eq!(c.member, "chat.completions.create");
    assert_eq!(c.receiver, Receiver::Client);
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert_eq!(c.client_line, Some(2));
    assert_eq!(c.kwargs, vec!["model", "messages"]);
    assert_eq!(c.symbol, None);
    assert_eq!((c.span.line, c.span.column), (3, 8));
    let model = &c.values[0];
    assert_eq!(model.value, "gpt-3.5-turbo");
    assert_eq!(model.origin, ValueOrigin::Literal);
    assert_eq!(model.model_family, Some(Sdk::Openai));
    // The literal's own span, quotes included.
    assert_eq!((model.span.line, model.span.column), (3, 45));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn aliased_module_and_class_imports_resolve() {
    let found = usages(
        r#"
import openai as oa
from openai import OpenAI as OA

first = oa.OpenAI()
second = OA()
first.responses.create(model="gpt-4o", input="hi")
second.completions.create(model="gpt-3.5-turbo-instruct", prompt="x")
"#,
    );
    assert_eq!(found.clients.len(), 2);
    assert_eq!(found.clients[0].chain, "oa.OpenAI");
    assert_eq!(found.clients[0].constructor, "openai.OpenAI");
    assert_eq!(found.clients[1].chain, "OA");
    assert_eq!(
        value(call(&found, "first.responses.create"), "model"),
        Some("gpt-4o")
    );
    assert_eq!(
        value(call(&found, "second.completions.create"), "model"),
        Some("gpt-3.5-turbo-instruct")
    );
}

#[test]
fn async_openai_inside_async_function() {
    let found = usages(
        r#"
from openai import AsyncOpenAI

client = AsyncOpenAI()

async def main():
    resp = await client.chat.completions.create(model="gpt-4", messages=[])
    return resp.choices[0].message.content
"#,
    );
    assert!(found.clients[0].is_async);
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.symbol.as_deref(), Some("main"));
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert_eq!(value(c, "model"), Some("gpt-4"));
    // Methods on the response are not SDK calls.
    assert_eq!(found.calls.len(), 1);
}

#[test]
fn azure_openai_with_azure_endpoint() {
    let found = usages(
        r#"
import os
from openai import AzureOpenAI

client = AzureOpenAI(
    api_key=os.getenv("AZURE_OPENAI_API_KEY"),
    api_version="2024-02-01",
    azure_endpoint=os.getenv("AZURE_OPENAI_ENDPOINT"),
)
client.chat.completions.create(model="my-gpt35-deployment", messages=[])
"#,
    );
    let built = &found.clients[0];
    assert_eq!(built.routing, Routing::AzureOpenai);
    assert_eq!(
        built.kwargs,
        vec!["api_key", "api_version", "azure_endpoint"]
    );
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.routing, Routing::AzureOpenai);
    let model = &c.values[0];
    assert_eq!(model.value, "my-gpt35-deployment");
    // A deployment name is not a model-ID shape.
    assert_eq!(model.model_family, None);
}

#[test]
fn base_url_routes_to_custom_and_splat_is_unknown() {
    let found = usages(
        r#"
import os
from openai import OpenAI
from anthropic import Anthropic

router = OpenAI(base_url="https://openrouter.ai/api/v1", api_key=os.environ["OPENROUTER_API_KEY"])
local = OpenAI(api_base="http://localhost:4000")
proxy = Anthropic(base_url="http://litellm:4000")
configured = OpenAI(**settings)
direct = OpenAI(api_key="x", organization="org")
"#,
    );
    assert_eq!(routing_of(&found, "router"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "local"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "proxy"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "configured"), Routing::Unknown);
    assert_eq!(routing_of(&found, "direct"), Routing::OpenaiDirect);
}

#[test]
fn anthropic_platform_routing() {
    let found = usages(
        r#"
import anthropic
from anthropic import AnthropicBedrock, AnthropicVertex, AsyncAnthropic

direct = anthropic.Anthropic()
bedrock = AnthropicBedrock(aws_region="us-east-1")
vertex = AnthropicVertex(region="us-east5", project_id="p")
aio = AsyncAnthropic()

bedrock.messages.create(model="anthropic.claude-3-haiku-20240307-v1:0", max_tokens=10, messages=[])
vertex.messages.create(model="claude-3-5-sonnet-v2@20241022", max_tokens=10, messages=[])
direct.messages.create(model="claude-3-5-sonnet-20241022", max_tokens=10, messages=[])
"#,
    );
    assert_eq!(routing_of(&found, "direct"), Routing::AnthropicDirect);
    assert_eq!(routing_of(&found, "bedrock"), Routing::AnthropicBedrock);
    assert_eq!(routing_of(&found, "vertex"), Routing::AnthropicVertex);
    assert_eq!(routing_of(&found, "aio"), Routing::AnthropicDirect);
    assert_eq!(
        call(&found, "bedrock.messages.create").routing,
        Routing::AnthropicBedrock
    );
    assert_eq!(
        call(&found, "vertex.messages.create").routing,
        Routing::AnthropicVertex
    );
    let direct = call(&found, "direct.messages.create");
    assert_eq!(direct.routing, Routing::AnthropicDirect);
    assert_eq!(direct.values[0].model_family, Some(Sdk::Anthropic));
    assert_eq!(direct.kwargs, vec!["model", "max_tokens", "messages"]);
}

#[test]
fn self_client_set_in_init_is_used_in_another_method() {
    let found = usages(
        r#"
from anthropic import Anthropic

class Bot:
    def __init__(self):
        self.client = Anthropic()

    def ask(self, question):
        return self.client.messages.create(
            model="claude-3-5-sonnet-20241022",
            max_tokens=256,
            messages=[{"role": "user", "content": question}],
        )
"#,
    );
    let built = &found.clients[0];
    assert_eq!(built.binding.as_deref(), Some("self.client"));
    assert_eq!(built.scope, ClientScope::Attribute);
    assert_eq!(built.symbol.as_deref(), Some("Bot.__init__"));

    let c = call(&found, "self.client.messages.create");
    assert_eq!(c.symbol.as_deref(), Some("Bot.ask"));
    assert_eq!(c.routing, Routing::AnthropicDirect);
    assert_eq!(c.client_line, Some(6));
    assert_eq!(value(c, "model"), Some("claude-3-5-sonnet-20241022"));
    assert_eq!((c.span.line, c.span.end_line), (9, 13));
}

#[test]
fn inherited_client_attribute_resolves() {
    let found = usages(
        r#"
from openai import AzureOpenAI

class Base:
    def __init__(self):
        self.llm = AzureOpenAI(azure_endpoint="https://x.openai.azure.com", api_version="v")

class Child(Base):
    def run(self):
        return self.llm.chat.completions.create(model="gpt-4-deployment", messages=[])
"#,
    );
    let c = call(&found, "self.llm.chat.completions.create");
    assert_eq!(c.routing, Routing::AzureOpenai);
    assert_eq!(c.symbol.as_deref(), Some("Child.run"));
}

#[test]
fn class_body_client_and_cls_access() {
    let found = usages(
        r#"
from openai import OpenAI

class Service:
    client = OpenAI()

    @classmethod
    def go(cls):
        return cls.client.embeddings.create(model="text-embedding-3-small", input="x")
"#,
    );
    assert_eq!(found.clients[0].scope, ClientScope::Class);
    let c = call(&found, "cls.client.embeddings.create");
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert_eq!(value(c, "model"), Some("text-embedding-3-small"));
}

#[test]
fn with_statement_and_with_options_keep_the_client() {
    let found = usages(
        r#"
from openai import OpenAI

def job():
    with OpenAI() as client:
        client.with_options(timeout=5).chat.completions.create(model="gpt-4o-mini", messages=[])
"#,
    );
    assert_eq!(found.clients[0].binding.as_deref(), Some("client"));
    assert_eq!(found.clients[0].scope, ClientScope::Local);
    let c = call(&found, "client.with_options(...).chat.completions.create");
    assert_eq!(c.routing, Routing::OpenaiDirect);
    // `with_options` returns the same client, so the member is the endpoint.
    assert_eq!(c.member, "chat.completions.create");
    assert_eq!(value(c, "model"), Some("gpt-4o-mini"));
}

#[test]
fn annotated_parameters_type_the_client() {
    let found = usages(
        r#"
from typing import Optional
from openai import OpenAI, AzureOpenAI

def azure(client: AzureOpenAI):
    return client.chat.completions.create(model="dep", messages=[])

def direct(client: Optional[OpenAI] = None):
    return client.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let calls: Vec<&SdkCall> = found
        .calls
        .iter()
        .filter(|c| c.chain == "client.chat.completions.create")
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].routing, Routing::AzureOpenai);
    // An annotation alone cannot rule out a base_url.
    assert_eq!(calls[1].routing, Routing::Unknown);
    assert_eq!(calls[1].client_line, None);
    assert!(gaps(&found, GapKind::AmbiguousClient).is_empty());
}

#[test]
fn global_assignment_in_function_is_a_module_client() {
    let found = usages(
        r#"
from openai import OpenAI

client = None

def init():
    global client
    client = OpenAI()

def ask():
    return client.chat.completions.create(model="gpt-3.5-turbo", messages=[])
"#,
    );
    assert_eq!(found.clients[0].scope, ClientScope::Module);
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert!(gaps(&found, GapKind::AmbiguousClient).is_empty());
}

#[test]
fn import_fallback_to_none_still_tracks_the_class() {
    let found = usages(
        r#"
try:
    from openai import OpenAI
except ImportError:
    OpenAI = None

client = OpenAI()
client.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    assert_eq!(found.clients.len(), 1);
    assert_eq!(found.calls.len(), 1);
}

#[test]
fn local_name_shadows_a_module_client() {
    let found = usages(
        r#"
import boto3
from anthropic import Anthropic

client = Anthropic()

def bedrock_direct():
    client = boto3.client("bedrock-runtime")
    return client.converse(modelId="anthropic.claude-3-haiku")
"#,
    );
    assert!(found.calls.is_empty(), "{:?}", found.calls);
}

// ---------------------------------------------------------------------------
// Model values
// ---------------------------------------------------------------------------

#[test]
fn module_class_local_constants_and_parameter_defaults_resolve() {
    let found = usages(
        r#"
from openai import OpenAI

MODEL = "gpt-3.5-turbo"
DEFAULT = "gpt-4"
ALIAS = DEFAULT
client = OpenAI()

class Chat:
    MODEL = "gpt-4-turbo"

    def __init__(self):
        self.model = "o3-mini"

    def a(self):
        return client.chat.completions.create(model=MODEL, messages=[])

    def b(self):
        return client.chat.completions.create(model=self.MODEL, messages=[])

    def c(self):
        return client.chat.completions.create(model=self.model, messages=[])

    def d(self):
        return client.chat.completions.create(model=ALIAS, messages=[])

def local():
    name = "gpt-4o"
    return client.chat.completions.create(model=name, messages=[])

def param(model="o1-mini"):
    return client.chat.completions.create(model=model, messages=[])
"#,
    );
    let by_symbol = |symbol: &str| {
        found
            .calls
            .iter()
            .find(|c| c.symbol.as_deref() == Some(symbol))
            .unwrap_or_else(|| panic!("no call in {symbol}"))
            .values[0]
            .clone()
    };
    let a = by_symbol("Chat.a");
    assert_eq!(
        (a.value.as_str(), a.origin, a.name.as_deref()),
        ("gpt-3.5-turbo", ValueOrigin::ModuleConstant, Some("MODEL"))
    );
    // The span points at the literal where the constant is defined.
    assert_eq!(a.span.line, 4);
    let b = by_symbol("Chat.b");
    assert_eq!(
        (b.value.as_str(), b.origin),
        ("gpt-4-turbo", ValueOrigin::ClassConstant)
    );
    let c = by_symbol("Chat.c");
    assert_eq!(
        (c.value.as_str(), c.origin, c.name.as_deref()),
        ("o3-mini", ValueOrigin::ClassConstant, Some("self.model"))
    );
    let d = by_symbol("Chat.d");
    assert_eq!(
        (d.value.as_str(), d.origin, d.span.line),
        ("gpt-4", ValueOrigin::ModuleConstant, 5)
    );
    let local = by_symbol("local");
    assert_eq!(
        (local.value.as_str(), local.origin),
        ("gpt-4o", ValueOrigin::LocalConstant)
    );
    let param = by_symbol("param");
    assert_eq!(
        (param.value.as_str(), param.origin, param.name.as_deref()),
        ("o1-mini", ValueOrigin::ParameterDefault, Some("model"))
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn unresolvable_models_are_typed_gaps() {
    let found = usages(
        r#"
import os
from openai import OpenAI
from config import IMPORTED_MODEL
from settings import settings

ENV_MODEL = os.getenv("OPENAI_MODEL", "gpt-3.5-turbo")
TWICE = "gpt-4"
if os.getenv("FAST"):
    TWICE = "gpt-4o"
client = OpenAI()

def run(version, model, params):
    client.chat.completions.create(model=f"gpt-{version}", messages=[])
    client.chat.completions.create(model=os.getenv("MODEL"), messages=[])
    client.chat.completions.create(model=os.environ["MODEL"], messages=[])
    client.chat.completions.create(model=ENV_MODEL, messages=[])
    client.chat.completions.create(model=settings.model, messages=[])
    client.chat.completions.create(model=pick_model(), messages=[])
    client.chat.completions.create(model=model, messages=[])
    client.chat.completions.create(model=IMPORTED_MODEL, messages=[])
    client.chat.completions.create(model=TWICE, messages=[])
    client.chat.completions.create(model="a" if version else "b", messages=[])
    client.chat.completions.create(**params)
"#,
    );
    let reasons: Vec<&str> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        vec![
            "f_string",
            "env_var",
            "env_var",
            "env_var",
            "attribute",
            "function_call",
            "parameter",
            "imported_name",
            "reassigned",
            "conditional",
            "splat",
        ]
    );
    // Every call is still a usage; only the value is missing, except the
    // environment lookup whose literal fallback is captured as env_default.
    assert_eq!(found.calls.len(), 11);
    let with_values: Vec<&SdkCall> = found
        .calls
        .iter()
        .filter(|c| !c.values.is_empty())
        .collect();
    assert_eq!(with_values.len(), 1);
    let fallback = &with_values[0].values[0];
    assert_eq!(
        (
            fallback.value.as_str(),
            fallback.origin,
            fallback.name.as_deref()
        ),
        ("gpt-3.5-turbo", ValueOrigin::EnvDefault, Some("ENV_MODEL"))
    );
    let via = gaps(&found, GapKind::UnresolvedModel)[3];
    assert!(via.detail.contains("ENV_MODEL"), "{}", via.detail);
    assert!(via.detail.contains("env_default"), "{}", via.detail);
    assert_eq!(via.key.as_deref(), Some("model"));
    // No literal reaches a gap detail.
    for gap in &found.gaps {
        assert!(!gap.detail.contains("gpt-3.5-turbo"), "{}", gap.detail);
        assert!(!gap.detail.contains("OPENAI_MODEL"), "{}", gap.detail);
    }
}

#[test]
fn pricing_tables_and_unrelated_strings_are_not_reported() {
    let found = usages(
        r#"
import openai

PRICES = {"gpt-3.5-turbo": 0.002, "claude-3-5-sonnet-20241022": 3.0}
SUPPORTED = ["gpt-4", "gpt-4o"]
print("gpt-3.5-turbo is retired")

def cost(model):
    return PRICES[model]
"#,
    );
    assert!(found.calls.is_empty());
    assert!(found.gaps.is_empty());
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("gpt-3.5-turbo"));
    assert!(!json.contains("claude-3-5-sonnet"));
}

#[test]
fn secrets_are_never_captured() {
    let found = usages(
        r#"
import openai
from openai import OpenAI

openai.api_key = "sk-live-SECRET-ONE"
client = OpenAI(api_key="sk-live-SECRET-TWO", organization="org-SECRET-THREE")
client.chat.completions.create(
    model="gpt-4",
    api_key="sk-live-SECRET-FOUR",
    extra_headers={"Authorization": "Bearer SECRET-FIVE"},
    messages=[{"role": "user", "content": "password: SECRET-SIX"}],
)
OpenAI(api_key="sk-live-SECRET-SEVEN").chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    for secret in ["SECRET", "sk-live", "Bearer", "org-"] {
        assert!(!json.contains(secret), "`{secret}` leaked: {json}");
    }
    assert_eq!(found.clients[0].kwargs, vec!["api_key", "organization"]);
    let c = call(&found, "client.chat.completions.create");
    assert!(c.kwargs.contains(&"api_key".to_string()));
    assert_eq!(c.values.len(), 1);
    assert_eq!(c.values[0].key, "model");
    // An inline construction renders without its arguments.
    assert!(found
        .calls
        .iter()
        .any(|c| c.chain == "OpenAI(...).chat.completions.create"));
}

// ---------------------------------------------------------------------------
// v0 SDK and module configuration
// ---------------------------------------------------------------------------

#[test]
fn openai_v0_module_calls() {
    let found = usages(
        r#"
import openai
from openai import ChatCompletion

openai.ChatCompletion.create(model="gpt-3.5-turbo", messages=[])
openai.Completion.create(engine="text-davinci-003", prompt="x")
ChatCompletion.create(model="gpt-4", messages=[])
"#,
    );
    let v0 = call(&found, "openai.ChatCompletion.create");
    assert_eq!(v0.receiver, Receiver::Module);
    assert_eq!(v0.member, "ChatCompletion.create");
    assert_eq!(v0.routing, Routing::OpenaiDirect);
    assert_eq!(value(v0, "model"), Some("gpt-3.5-turbo"));
    assert_eq!(
        value(call(&found, "openai.Completion.create"), "engine"),
        Some("text-davinci-003")
    );
    let symbol = call(&found, "ChatCompletion.create");
    assert_eq!(symbol.receiver, Receiver::Symbol);
    assert_eq!(symbol.member, "ChatCompletion.create");
}

#[test]
fn openai_v0_module_configuration_sets_routing() {
    let azure = usages(
        r#"
import openai
openai.api_type = "azure"
openai.api_base = "https://example.openai.azure.com/"
openai.ChatCompletion.create(engine="gpt35", deployment_id="gpt35", messages=[])
"#,
    );
    let c = call(&azure, "openai.ChatCompletion.create");
    assert_eq!(c.routing, Routing::AzureOpenai);
    assert_eq!(value(c, "engine"), Some("gpt35"));
    assert_eq!(value(c, "deployment_id"), Some("gpt35"));

    let custom = usages(
        "import openai\nopenai.api_base = \"http://localhost:8000/v1\"\nopenai.ChatCompletion.create(model=\"gpt-4\")\n",
    );
    assert_eq!(
        call(&custom, "openai.ChatCompletion.create").routing,
        Routing::CustomBaseUrl
    );

    let unknown = usages(
        "import os, openai\nopenai.api_type = os.environ[\"OPENAI_API_TYPE\"]\nopenai.chat.completions.create(model=\"gpt-4\")\n",
    );
    assert_eq!(
        call(&unknown, "openai.chat.completions.create").routing,
        Routing::Unknown
    );
}

// ---------------------------------------------------------------------------
// Google
// ---------------------------------------------------------------------------

#[test]
fn legacy_gemini_sdk() {
    let found = usages(
        r#"
import os
import google.generativeai as genai

genai.configure(api_key=os.environ["GOOGLE_API_KEY"])
model = genai.GenerativeModel("gemini-pro")
response = model.generate_content("Write a haiku")
chat = model.start_chat()
chat.send_message("hello")
"#,
    );
    let configure = call(&found, "genai.configure");
    assert_eq!(configure.kwargs, vec!["api_key"]);
    assert!(configure.values.is_empty());
    assert_eq!(configure.routing, Routing::GoogleGenerativeaiLegacy);

    let ctor = call(&found, "genai.GenerativeModel");
    let model = &ctor.values[0];
    assert_eq!((model.key.as_str(), model.positional), ("model_name", true));
    assert_eq!(model.value, "gemini-pro");
    assert_eq!(model.model_family, Some(Sdk::Google));
    assert_eq!(found.clients[0].routing, Routing::GoogleGenerativeaiLegacy);

    let generate = call(&found, "model.generate_content");
    assert_eq!(generate.routing, Routing::GoogleGenerativeaiLegacy);
    assert_eq!(generate.client_line, Some(6));
    assert_eq!(
        call(&found, "chat.send_message").routing,
        Routing::GoogleGenerativeaiLegacy
    );
}

#[test]
fn legacy_gemini_keyword_and_imported_class() {
    let found = usages(
        r#"
from google.generativeai import GenerativeModel

MODEL = "gemini-1.5-flash"
model = GenerativeModel(model_name=MODEL)
"#,
    );
    let ctor = call(&found, "GenerativeModel");
    assert_eq!(ctor.receiver, Receiver::Symbol);
    assert_eq!(ctor.values[0].value, "gemini-1.5-flash");
    assert_eq!(ctor.values[0].origin, ValueOrigin::ModuleConstant);
    assert!(!ctor.values[0].positional);
}

#[test]
fn google_genai_client() {
    let found = usages(
        r#"
from google import genai
from google.genai import types
import google.genai

client = genai.Client()
other = google.genai.Client(vertexai=True, project="p", location="us-central1")
client.models.generate_content(
    model="gemini-2.0-flash",
    contents="hi",
    config=types.GenerateContentConfig(temperature=0),
)
chat = client.chats.create(model="gemini-2.0-flash")
chat.send_message("hello")
other.aio.models.generate_content(model="gemini-1.5-pro", contents="x")
"#,
    );
    assert_eq!(found.clients.len(), 2);
    assert!(found
        .clients
        .iter()
        .all(|c| c.routing == Routing::GoogleGenai));
    assert_eq!(found.clients[1].chain, "google.genai.Client");
    let generate = call(&found, "client.models.generate_content");
    assert_eq!(value(generate, "model"), Some("gemini-2.0-flash"));
    assert_eq!(
        call(&found, "types.GenerateContentConfig").receiver,
        Receiver::Symbol
    );
    assert_eq!(
        value(call(&found, "client.chats.create"), "model"),
        Some("gemini-2.0-flash")
    );
    assert_eq!(
        call(&found, "chat.send_message").routing,
        Routing::GoogleGenai
    );
    assert_eq!(
        value(call(&found, "other.aio.models.generate_content"), "model"),
        Some("gemini-1.5-pro")
    );
}

// ---------------------------------------------------------------------------
// Gaps
// ---------------------------------------------------------------------------

#[test]
fn unknown_client_sources_are_gaps_with_triage_values() {
    let found = usages(
        r#"
import instructor
from openai import OpenAI
from .llm import shared

client = make_client()
patched = instructor.from_openai(OpenAI())

def ask(llm):
    llm.chat.completions.create(model="gpt-3.5-turbo", messages=[])

client.chat.completions.create(model="gpt-3.5-turbo", messages=[])
patched.chat.completions.create(model="gpt-4", response_model=Model, messages=[])
shared.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let unknown = gaps(&found, GapKind::UnknownClientFactory);
    let reasons: Vec<&str> = unknown.iter().map(|g| g.reason.as_str()).collect();
    assert_eq!(
        reasons,
        vec!["parameter", "factory_call", "factory_call", "imported_name"]
    );
    assert!(unknown.iter().all(|g| g.sdk == Some(Sdk::Openai)));
    assert_eq!(unknown[0].values[0].value, "gpt-3.5-turbo");
    assert_eq!(unknown[0].symbol.as_deref(), Some("ask"));
    assert!(unknown[2].detail.contains("instructor.from_openai"));
    // The wrapped construction is still recorded, as an inline client.
    assert_eq!(found.clients.len(), 1);
    assert_eq!(found.clients[0].scope, ClientScope::Inline);
    assert!(found.calls.is_empty());
}

#[test]
fn messages_create_counts_only_where_anthropic_is_imported() {
    let twilio = usages(
        r#"
from twilio.rest import Client
client = Client(sid, token)
client.messages.create(body="hi", from_="+1", to="+2")
"#,
    );
    assert!(twilio.gaps.is_empty(), "{:?}", twilio.gaps);

    let anthropic = usages(
        r#"
import anthropic
client = get_client()
client.messages.create(model="claude-3-opus-20240229", max_tokens=5, messages=[])
"#,
    );
    let gap = &gaps(&anthropic, GapKind::UnknownClientFactory)[0];
    assert_eq!(gap.sdk, Some(Sdk::Anthropic));
}

#[test]
fn getattr_and_method_references_are_dynamic_dispatch() {
    let found = usages(
        r#"
import functools
from openai import OpenAI

client = OpenAI()
create = client.chat.completions.create
bound = functools.partial(client.chat.completions.create, model="gpt-4")
getattr(client.chat.completions, "create")(model="gpt-4", messages=[])
method = getattr(client, name)
"#,
    );
    let dynamic = gaps(&found, GapKind::DynamicDispatch);
    let reasons: Vec<&str> = dynamic.iter().map(|g| g.reason.as_str()).collect();
    assert_eq!(
        reasons,
        vec!["method_reference", "method_reference", "getattr", "getattr"]
    );
    assert_eq!(
        dynamic[2].chain.as_deref(),
        Some("getattr(client.chat.completions, ...)")
    );
    assert!(found.calls.is_empty(), "{:?}", found.calls);
}

#[test]
fn ambiguous_client_is_unknown_routing_with_a_gap() {
    let found = usages(
        r#"
import os
from openai import OpenAI, AzureOpenAI

client = AzureOpenAI() if os.getenv("USE_AZURE") else OpenAI()
client.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.routing, Routing::Unknown);
    // Both constructions are on line 5.
    assert_eq!(c.client_line, Some(5));
    assert_eq!(gaps(&found, GapKind::AmbiguousClient).len(), 1);
    assert_eq!(found.clients.len(), 2);

    let split = usages(
        r#"
from openai import OpenAI, AzureOpenAI

if azure:
    client = AzureOpenAI()
else:
    client = OpenAI()
client.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let c = call(&split, "client.chat.completions.create");
    assert_eq!((c.routing, c.client_line), (Routing::Unknown, None));
    assert_eq!(gaps(&split, GapKind::AmbiguousClient).len(), 1);
}

#[test]
fn injected_or_default_client_is_ambiguous() {
    let found = usages(
        r#"
from openai import OpenAI

class Agent:
    def __init__(self, client=None):
        self.client = client or OpenAI()

    def run(self):
        return self.client.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let c = call(&found, "self.client.chat.completions.create");
    assert_eq!(c.routing, Routing::Unknown);
    assert_eq!(gaps(&found, GapKind::AmbiguousClient).len(), 1);
    assert_eq!(found.clients[0].binding.as_deref(), Some("self.client"));
}

// ---------------------------------------------------------------------------
// Positions, symbols and misc
// ---------------------------------------------------------------------------

#[test]
fn columns_count_characters_not_bytes() {
    let found = usages(
        "from openai import OpenAI\nclient = OpenAI()\nnote = \"héllo\"; client.models.list()\n",
    );
    let c = call(&found, "client.models.list");
    // `note = "héllo"; ` is 16 characters (17 bytes).
    assert_eq!(c.span.column, 17);
    assert_eq!(c.span.start_byte as usize, found_byte(&found, 2) + 17);
}

fn found_byte(_found: &FileUsages, line_index: usize) -> usize {
    let source =
        "from openai import OpenAI\nclient = OpenAI()\nnote = \"héllo\"; client.models.list()\n";
    source
        .split_inclusive('\n')
        .take(line_index)
        .map(str::len)
        .sum()
}

#[test]
fn nested_functions_use_the_graph_symbol_names() {
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()

class Worker:
    def run(self):
        def inner():
            return client.chat.completions.create(model="gpt-4", messages=[])
        return inner()

def outer():
    def helper():
        return client.chat.completions.create(model="gpt-4", messages=[])
    return [lambda: client.chat.completions.create(model="gpt-4", messages=[])]
"#,
    );
    let symbols: Vec<Option<&str>> = found.calls.iter().map(|c| c.symbol.as_deref()).collect();
    assert_eq!(
        symbols,
        vec![Some("Worker.inner"), Some("helper"), Some("outer")]
    );
}

#[test]
fn wildcard_import_binds_client_classes() {
    let found = usages("from anthropic import *\nc = Anthropic()\nc.messages.create(model=\"claude-2.1\", max_tokens=1, messages=[])\n");
    assert_eq!(found.clients.len(), 1);
    assert_eq!(found.calls.len(), 1);
}

#[test]
fn annotated_module_assignment() {
    let found = usages(
        "from openai import OpenAI\nclient: OpenAI = OpenAI(base_url=URL)\nclient.chat.completions.create(model=\"gpt-4\", messages=[])\n",
    );
    assert_eq!(
        call(&found, "client.chat.completions.create").routing,
        Routing::CustomBaseUrl
    );
}

#[test]
fn files_without_sdk_mentions_are_skipped() {
    let found = usages("import json\n\ndef f():\n    return json.dumps({})\n");
    assert!(found.is_empty());
    assert_eq!(found.language, "python");
    assert_eq!(found.file, "app.py");
}

#[test]
fn retain_sdk_filters_every_list() {
    let mut found = usages(
        r#"
from openai import OpenAI
from anthropic import Anthropic
a = OpenAI()
b = Anthropic()
a.chat.completions.create(model="gpt-4", messages=[])
b.messages.create(model="claude-2.1", max_tokens=1, messages=[])
"#,
    );
    found.retain_sdk(Sdk::Anthropic);
    assert!(found.imports.iter().all(|i| i.sdk == Sdk::Anthropic));
    assert_eq!(found.clients.len(), 1);
    assert_eq!(found.calls.len(), 1);
    assert_eq!(found.calls[0].sdk, Sdk::Anthropic);
}

#[test]
fn hosted_platform_clients_are_tracked_with_unknown_routing() {
    let found = usages(
        r#"
from openai import OpenAI, BedrockOpenAI
from openai.providers import bedrock
from anthropic import AnthropicFoundry, AnthropicGoogleCloud, AsyncAnthropicAWS

adapter = OpenAI(provider=bedrock(region="us-west-2"))
openai_on_bedrock = BedrockOpenAI()
foundry = AnthropicFoundry(resource="r", api_key="not-a-real-key")
gcp = AnthropicGoogleCloud(project="p")
aws = AsyncAnthropicAWS()

adapter.responses.create(model="openai.gpt-5.4", input="hi")
foundry.messages.create(model="claude-sonnet-5-5", max_tokens=1, messages=[])
gcp.messages.create(model="claude-haiku-4-5", max_tokens=1, messages=[])
"#,
    );
    for binding in ["adapter", "openai_on_bedrock", "foundry", "gcp", "aws"] {
        assert_eq!(routing_of(&found, binding), Routing::Unknown, "{binding}");
    }
    let constructors: Vec<&str> = found
        .clients
        .iter()
        .map(|c| c.constructor.as_str())
        .collect();
    assert!(constructors.contains(&"anthropic.AnthropicFoundry"));
    let adapter = call(&found, "adapter.responses.create");
    assert_eq!(adapter.routing, Routing::Unknown);
    assert_eq!(adapter.values[0].model_family, Some(Sdk::Openai));
    assert_eq!(
        call(&found, "foundry.messages.create").routing,
        Routing::Unknown
    );
    assert!(!serde_json::to_string(&found)
        .unwrap()
        .contains("not-a-real-key"));
}

#[test]
fn sdk_shaped_call_on_an_unknown_sdk_result_is_a_gap() {
    let found = usages(
        r#"
import anthropic

client = anthropic.AnthropicNextPlatform(region="x")
client.messages.create(model="claude-sonnet-5-5", max_tokens=1, messages=[])
"#,
    );
    // The unknown constructor is still a call rooted at the SDK.
    assert_eq!(
        call(&found, "anthropic.AnthropicNextPlatform").member,
        "AnthropicNextPlatform"
    );
    let gap = &gaps(&found, GapKind::UnknownClientFactory)[0];
    assert_eq!(gap.reason, "sdk_call_result");
    assert_eq!(gap.sdk, Some(Sdk::Anthropic));
    assert_eq!(gap.values[0].value, "claude-sonnet-5-5");
}

#[test]
fn module_api_type_other_than_openai_or_azure_is_unknown() {
    let bedrock = usages(
        "import openai\nopenai.api_type = \"amazon-bedrock\"\nopenai.chat.completions.create(model=\"openai.gpt-5.4\")\n",
    );
    assert_eq!(
        call(&bedrock, "openai.chat.completions.create").routing,
        Routing::Unknown
    );
    let explicit = usages(
        "import openai\nopenai.api_type = \"openai\"\nopenai.chat.completions.create(model=\"gpt-4\")\n",
    );
    assert_eq!(
        call(&explicit, "openai.chat.completions.create").routing,
        Routing::OpenaiDirect
    );
}

#[test]
fn allowlisted_keys_inside_dict_arguments_are_captured() {
    let found = usages(
        r#"
from anthropic import Anthropic, BetaRefusalFallbackMiddleware

FALLBACK = "claude-opus-4-8"
client = Anthropic(middleware=[BetaRefusalFallbackMiddleware([{"model": "claude-opus-4-1"}])])
client.beta.messages.create(
    model="claude-fable-5",
    fallbacks=[{"model": FALLBACK}, {"model": pick()}],
    extra_headers={"x-api-key": "not-a-real-key", "auth": {"model": "nope-secret"}},
    auth_options=[{"model": "nope-secret-two"}],
    metadata={"user_id": "u-123"},
    messages=[{"role": "user", "content": "model: gpt-4"}],
    max_tokens=10,
)
"#,
    );
    let c = call(&found, "client.beta.messages.create");
    let keys: Vec<(&str, &str)> = c
        .values
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("model", "claude-fable-5"),
            ("fallbacks.model", "claude-opus-4-8"),
        ]
    );
    assert_eq!(c.values[1].origin, ValueOrigin::ModuleConstant);
    // A nested value that does not resolve is skipped, not a gap.
    assert!(gaps(&found, GapKind::UnresolvedModel).is_empty());

    let middleware = call(&found, "BetaRefusalFallbackMiddleware");
    assert_eq!(middleware.values[0].key, "model");
    assert!(middleware.values[0].positional);
    assert_eq!(middleware.values[0].value, "claude-opus-4-1");

    let json = serde_json::to_string(&found).unwrap();
    for leaked in ["not-a-real-key", "nope-secret", "u-123", "gpt-4\""] {
        assert!(!json.contains(leaked), "{leaked} leaked");
    }
}

#[test]
fn imports_inside_functions_bind_in_that_function() {
    // Two classes import different Google SDKs under the same local name.
    let found = usages(
        r#"
class NewEmbedder:
    def __init__(self):
        import google.genai as genai
        self.client = genai.Client()

    def embed(self, text):
        return self.client.models.embed_content(model="gemini-embedding-001", contents=text)


class LegacyEmbedder:
    def __init__(self, key):
        import google.generativeai as genai
        genai.configure(api_key=key)
        self._genai = genai

    def embed(self, text):
        return self._genai.embed_content(model="models/text-embedding-004", content=text)
"#,
    );
    assert_eq!(
        call(&found, "self.client.models.embed_content").routing,
        Routing::GoogleGenai
    );
    assert_eq!(
        call(&found, "genai.configure").routing,
        Routing::GoogleGenerativeaiLegacy
    );
    let legacy = call(&found, "self._genai.embed_content");
    assert_eq!(legacy.routing, Routing::GoogleGenerativeaiLegacy);
    assert_eq!(legacy.receiver, Receiver::Module);
    assert_eq!(value(legacy, "model"), Some("models/text-embedding-004"));

    // A function-local import is not visible from another function.
    let scoped = usages(
        r#"
def make():
    from openai import OpenAI
    return OpenAI()

def use():
    return OpenAI().chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    assert_eq!(scoped.clients.len(), 1);
    assert_eq!(scoped.clients[0].symbol.as_deref(), Some("make"));
    assert!(scoped.calls.is_empty());
}

#[test]
fn request_flag_separates_api_requests_from_sdk_helpers_and_types() {
    let found = usages(
        r#"
import google.generativeai as genai
from openai import OpenAI
from openai.types.chat import ChatCompletion

genai.configure(api_key=KEY)
model = genai.GenerativeModel("gemini-pro")
model.generate_content("hi")
client = OpenAI()
client.chat.completions.create(model="gpt-4", messages=[])
client.files.list()
fake = ChatCompletion(id="x", choices=[], created=0, model="gpt-4", object="chat.completion")
"#,
    );
    let requests: Vec<(&str, bool)> = found
        .calls
        .iter()
        .map(|c| (c.chain.as_str(), c.request))
        .collect();
    assert_eq!(
        requests,
        vec![
            ("genai.configure", false),
            ("genai.GenerativeModel", false),
            ("model.generate_content", true),
            ("client.chat.completions.create", true),
            ("client.files.list", true),
            ("ChatCompletion", false),
        ]
    );
}

#[test]
fn unresolved_models_on_sdk_types_are_not_gaps() {
    let found = usages(
        r#"
from openai import OpenAI
from openai.types.chat import ChatCompletion

client = OpenAI()

def respond(model, choices):
    client.chat.completions.create(model=model, messages=[])
    return ChatCompletion(id="x", choices=choices, created=0, model=model, object="chat.completion")
"#,
    );
    let unresolved = gaps(&found, GapKind::UnresolvedModel);
    assert_eq!(unresolved.len(), 1);
    assert_eq!(
        unresolved[0].chain.as_deref(),
        Some("client.chat.completions.create")
    );
    let typed = call(&found, "ChatCompletion");
    assert!(!typed.request);
    assert!(typed.kwargs.contains(&"model".to_string()));
    assert!(typed.values.is_empty());
}

#[test]
fn legacy_gemini_embedding_with_kwargs_is_a_splat_gap() {
    let found = usages(
        r#"
import google.generativeai as genai

def embed(**kwargs):
    return genai.embed_content(**kwargs)
"#,
    );
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.reason, "splat");
    assert_eq!(gap.sdk, Some(Sdk::Google));
    assert!(call(&found, "genai.embed_content").request);
}

// ---------------------------------------------------------------------------
// Review of #276: credentials
// ---------------------------------------------------------------------------

/// The five cases from the review, each of which printed a secret.
const REVIEW_SECRETS: &str = r#"
from openai import OpenAI

client = OpenAI()
OPENAI_API_KEY = "sk-proj-AAAAREVIEWSECRETONEAAAA"

client.chat.completions.create(model=OPENAI_API_KEY, messages=[])
client.chat.completions.create(model="sk-ant-api03-BBBBREVIEWSECRETTWOBBBB", messages=[])

def f(token="sk-CCCCREVIEWSECRETTHREECCCC"):
    return client.chat.completions.create(model=token, messages=[])

class Bot:
    def __init__(self):
        self.secret = "sk-DDDDREVIEWSECRETFOURDDDD"

    def go(self):
        return client.chat.completions.create(model=self.secret, messages=[])

client.chat.completions.create(model="gpt-4", messages=[], extra_body={"sk-proj-EEEEREVIEWSECRETFIVEEEEE": {"model": "gpt-4"}})
"#;

#[test]
fn review_credential_cases_never_reach_the_output() {
    let found = usages(REVIEW_SECRETS);
    let json = serde_json::to_string(&found).unwrap();
    for leaked in [
        "sk-",
        "REVIEWSECRET",
        "AAAA",
        "BBBB",
        "CCCC",
        "DDDD",
        "EEEE",
    ] {
        assert!(!json.contains(leaked), "`{leaked}` leaked: {json}");
    }
    // The four refused models are typed gaps that name the binding only.
    let refused: Vec<&UsageGap> = gaps(&found, GapKind::UnresolvedModel)
        .into_iter()
        .filter(|g| g.reason == "credential")
        .collect();
    assert_eq!(refused.len(), 4, "{:?}", found.gaps);
    assert!(refused[0].detail.contains("OPENAI_API_KEY"));
    // The dict under a credential-shaped key is never descended into.
    let last = found.calls.last().unwrap();
    let keys: Vec<&str> = last.values.iter().map(|v| v.key.as_str()).collect();
    assert_eq!(keys, vec!["model"]);
}

#[test]
fn credential_shaped_values_are_refused_through_any_name() {
    let found = usages(
        r#"
import os
from openai import OpenAI

client = OpenAI()
MODEL = "sk-proj-FFFFSHAPEDSECRETFFFF"
PARAMS = {"model": "AIzaSyGGGGSHAPEDSECRETGGGG"}
CONFIG = {"api_key": "gpt-4"}
SECRET_MODEL = "gpt-4"

client.chat.completions.create(model=MODEL, messages=[])
client.chat.completions.create(**PARAMS, messages=[])
client.chat.completions.create(model=CONFIG["api_key"], messages=[])
client.chat.completions.create(model=SECRET_MODEL, messages=[])
client.chat.completions.create(model=os.getenv("OPENAI_API_KEY", "gpt-4"), messages=[])
client.chat.completions.create(model=os.getenv("MODEL", "sk-HHHHSHAPEDSECRETHHHH"), messages=[])
client.chat.completions.create(model="0123456789abcdef0123456789abcdef", messages=[])
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    for leaked in ["SHAPEDSECRET", "sk-", "AIza", "0123456789abcdef"] {
        assert!(!json.contains(leaked), "`{leaked}` leaked: {json}");
    }
    assert!(found.calls.iter().all(|c| c.values.is_empty()), "{json}");
    let reasons: Vec<&str> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(reasons, vec!["credential"; 7], "{json}");
}

#[test]
fn control_characters_are_never_captured() {
    let found = usages(
        "from openai import OpenAI\nclient = OpenAI()\nclient.chat.completions.create(model=\"gpt-4\u{1b}]8;;http://x\u{7}\", messages=[])\n",
    );
    assert!(found.calls[0].values.is_empty());
    assert_eq!(
        gaps(&found, GapKind::UnresolvedModel)[0].reason,
        "non_literal"
    );
    assert!(!serde_json::to_string(&found).unwrap().contains("8;;"));
}

// ---------------------------------------------------------------------------
// Review of #276: bounded work
// ---------------------------------------------------------------------------

#[test]
fn cyclic_multi_base_classes_resolve_in_bounded_time() {
    // Every class lists every class (itself included) as a base. With no
    // visited set, attribute lookup was exponential: 7 classes took 13 s
    // and 10 never finished.
    let names: Vec<String> = (0..12).map(|i| format!("C{i}")).collect();
    let bases = names.join(", ");
    let mut source = String::from("from openai import OpenAI\nclient = OpenAI()\n");
    for name in &names {
        source.push_str(&format!(
            "class {name}({bases}):\n    def go(self):\n        return self.client.chat.completions.create(model=self.model, messages=[])\n"
        ));
    }
    let started = std::time::Instant::now();
    let found = usages(&source);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "took {elapsed:?}"
    );
    assert_eq!(gaps(&found, GapKind::UnknownClientFactory).len(), 12);
}

#[test]
fn a_twenty_thousand_link_chain_is_linear() {
    // `Node::parent()` descends from the root, so asking it for every node
    // of a deep chain was quadratic: 20,000 links took 50 s.
    let mut source = String::from("from openai import OpenAI\nclient = OpenAI()\nx = client");
    for _ in 0..20_000 {
        source.push_str(".m()");
    }
    source.push_str("\nclient.chat.completions.create(model=\"gpt-4\", messages=[])\n");
    let started = std::time::Instant::now();
    let found = usages(&source);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "took {elapsed:?}"
    );
    assert_eq!(
        value(call(&found, "client.chat.completions.create"), "model"),
        Some("gpt-4")
    );
}

/// Extracts `source` and fails if that takes longer than `limit` seconds.
fn usages_within(source: &str, limit: u64) -> FileUsages {
    let started = std::time::Instant::now();
    let found = usages(source);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(limit),
        "took {elapsed:?}"
    );
    found
}

#[test]
fn branching_chains_past_the_name_depth_stay_linear() {
    // Each link uses the previous one twice. A value cut off at the name
    // depth used to be recomputed on every use, so 70 links never
    // finished; it is now memoized with the budget it had.
    let head = "from openai import OpenAI\nclient = OpenAI()\n";
    let shapes = [
        (
            "x{i} = x{p} or x{p}\n",
            "x0 = client\n",
            "x{n}.chat.completions.create(model=\"gpt-4\", messages=[])\n",
        ),
        (
            "x{i} = x{p} if flag else x{p}\n",
            "x0 = client\n",
            "x{n}.chat.completions.create(model=\"gpt-4\", messages=[])\n",
        ),
        (
            "class C{i}:\n    a = C{p}.a or C{p}.a\n",
            "class C0:\n    a = client\n",
            "C{n}.a.chat.completions.create(model=\"gpt-4\", messages=[])\n",
        ),
    ];
    for (link, first, last) in shapes {
        for (n, resolves) in [(40, true), (300, false)] {
            let mut source = format!("{head}{first}");
            for i in 1..=n {
                let p = i - 1;
                source.push_str(
                    &link
                        .replace("{i}", &i.to_string())
                        .replace("{p}", &p.to_string()),
                );
            }
            source.push_str(&last.replace("{n}", &n.to_string()));
            let found = usages_within(&source, 5);
            if resolves {
                assert_eq!(found.calls.len(), 1, "{link} x {n}");
                assert_eq!(found.calls[0].routing, Routing::OpenaiDirect);
            } else {
                // Past the depth the receiver is a typed gap, never a silent
                // drop.
                assert!(found.calls.is_empty(), "{link} x {n}");
                let gap = &gaps(&found, GapKind::UnknownClientFactory)[0];
                assert_eq!(gap.reason, "unresolved_receiver");
            }
        }
    }

    // `self.a{i}` assigned from `self.a{i-1}` twice, and a client parameter
    // whose only callers pass it on twice, deepest function first.
    let mut methods = String::from(
        "from openai import OpenAI\nclass B:\n    def __init__(self):\n        self.a0 = OpenAI()\n",
    );
    let mut params = String::from(
        "from openai import OpenAI\ndef g0(c):\n    return c.chat.completions.create(model=\"gpt-4\", messages=[])\n",
    );
    for i in 1..300 {
        let p = i - 1;
        methods.push_str(&format!(
            "    def m{i}(self):\n        self.a{i} = self.a{p} or self.a{p}\n"
        ));
        params.push_str(&format!("def g{i}(c):\n    g{p}(c)\n    g{p}(c)\n"));
    }
    methods.push_str(
        "    def go(self):\n        return self.a299.chat.completions.create(model=\"gpt-4\", messages=[])\n",
    );
    params.push_str("g299(OpenAI())\n");
    for source in [methods, params] {
        let found = usages_within(&source, 5);
        assert_eq!(gaps(&found, GapKind::UnknownClientFactory).len(), 1);
    }
}

#[test]
fn large_definitions_used_many_times_stay_linear() {
    let mut source = String::from("import os\nfrom openai import OpenAI\nclient = OpenAI()\n");
    // A constant whose value is a call with 20,000 arguments, read at
    // 2,000 sites: it is not a lookup, so its arguments are never scanned.
    source.push_str("MODEL = pick(");
    for i in 0..20_000 {
        source.push_str(&format!("a{i}, "));
    }
    source.push_str(")\n");
    // A name assigned 2,000 times in branches, read at 2,000 sites.
    for i in 0..2_000 {
        source.push_str(&format!("if c{i}:\n    TIER = \"gpt-4\"\n"));
    }
    for _ in 0..2_000 {
        source.push_str("client.chat.completions.create(model=MODEL, messages=[])\n");
        source.push_str("client.chat.completions.create(model=TIER, messages=[])\n");
    }
    // Requests nested 1,500 deep as each other's model argument: each asks
    // whether its argument reads the environment.
    let mut nested = String::from("os.getenv(\"M\")");
    for _ in 0..1_500 {
        nested = format!("client.chat.completions.create(model={nested}, messages=[])");
    }
    source.push_str(&nested);
    source.push('\n');

    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 4_000 + 1_500);
    let reasons: HashMap<&str, usize> =
        gaps(&found, GapKind::UnresolvedModel)
            .iter()
            .fold(HashMap::new(), |mut counts, g| {
                *counts.entry(g.reason.as_str()).or_default() += 1;
                counts
            });
    assert_eq!(reasons.get("function_call"), Some(&2_000));
    assert_eq!(reasons.get("reassigned"), Some(&2_000));
    // Every nested request reads the environment somewhere below it.
    assert_eq!(reasons.get("env_var"), Some(&1_500));
}

// ---------------------------------------------------------------------------
// Review of #276: gaps instead of silent drops
// ---------------------------------------------------------------------------

#[test]
fn every_model_bearing_call_reports_a_value_or_a_gap() {
    let found = usages(
        r#"
import os
from openai import OpenAI
import google.generativeai as genai

client = OpenAI()

def run(kw, f):
    client.images.generate(model=os.getenv("IMAGE_MODEL"), prompt="x")
    client.images.edit(model=os.getenv("IMAGE_MODEL"), image=f, prompt="x")
    client.beta.realtime.connect(model=os.getenv("REALTIME_MODEL"))
    client.chat.completions.with_raw_response.create(**kw)
    client.chat.completions.with_streaming_response.create(model=os.getenv("M"), messages=[])
    client.audio.transcriptions.create(**kw)
    client.audio.speech.create(**kw)
    client.vector_stores.files.poll(model=os.getenv("M"))
    genai.embed_content("models/embedding-001", "text")
    genai.embed_content(model=os.getenv("EMBED"), content="text")
"#,
    );
    let calls_without_values: Vec<&SdkCall> =
        found.calls.iter().filter(|c| c.values.is_empty()).collect();
    for c in &calls_without_values {
        let start = c.span.start_byte;
        assert!(
            found
                .gaps
                .iter()
                .any(|g| g.span.map(|s| s.start_byte) == Some(start)),
            "silent drop at `{}`",
            c.chain
        );
    }
    assert_eq!(calls_without_values.len(), 9);
    let raw = call(&found, "client.chat.completions.with_raw_response.create");
    assert_eq!(raw.member, "chat.completions.create");
    assert!(raw.request);
    let reasons: Vec<&str> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        vec![
            "env_var", "env_var", "env_var", "splat", "env_var", "splat", "splat", "env_var",
            "env_var"
        ]
    );
    let legacy = found
        .calls
        .iter()
        .find(|c| c.chain == "genai.embed_content" && !c.values.is_empty())
        .unwrap();
    let model = &legacy.values[0];
    assert_eq!(
        (model.key.as_str(), model.value.as_str(), model.positional),
        ("model", "models/embedding-001", true)
    );
    assert_eq!(model.model_family, Some(Sdk::Google));
}

// ---------------------------------------------------------------------------
// Review of #276: routing
// ---------------------------------------------------------------------------

#[test]
fn with_options_and_copy_reroute_the_client() {
    let found = usages(
        r#"
from openai import OpenAI
from anthropic import Anthropic

client = OpenAI()
claude = Anthropic()
LOCAL = {"base_url": "http://localhost:8000"}

def run(opts):
    client.with_options(base_url="https://openrouter.ai/api/v1").chat.completions.create(model="gpt-4", messages=[])
    client.copy(base_url="http://localhost:8000").chat.completions.create(model="gpt-4", messages=[])
    client.with_options(**opts).chat.completions.create(model="gpt-4", messages=[])
    client.with_options(timeout=5).chat.completions.create(model="gpt-4", messages=[])
    client.with_options(**LOCAL).chat.completions.create(model="gpt-4", messages=[])
    claude.with_options(base_url="http://litellm:4000").messages.create(model="claude-2.1", max_tokens=1, messages=[])
"#,
    );
    let routes: Vec<(String, Routing)> = found
        .calls
        .iter()
        .filter(|c| c.request)
        .map(|c| (c.member.clone(), c.routing))
        .collect();
    let chat = "chat.completions.create".to_string();
    assert_eq!(
        routes,
        vec![
            (chat.clone(), Routing::CustomBaseUrl),
            (chat.clone(), Routing::CustomBaseUrl),
            (chat.clone(), Routing::Unknown),
            (chat.clone(), Routing::OpenaiDirect),
            (chat, Routing::CustomBaseUrl),
            ("messages.create".to_string(), Routing::CustomBaseUrl),
        ]
    );
}

#[test]
fn a_module_name_rebound_to_a_client_routes_as_that_client() {
    let found = usages(
        r#"
import openai

openai = openai.OpenAI(base_url="http://localhost:8000/v1")
openai.chat.completions.create(model="gpt-4", messages=[])

def later():
    return openai.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    assert_eq!(found.clients[0].routing, Routing::CustomBaseUrl);
    assert_eq!(found.calls.len(), 2);
    for c in &found.calls {
        assert_eq!(c.receiver, Receiver::Client, "{}", c.chain);
        assert_eq!(c.routing, Routing::CustomBaseUrl, "{}", c.chain);
        assert_eq!(c.client_line, Some(4));
    }
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn lambda_parameters_are_bindings() {
    let found = usages(
        r#"
import openai
from openai import OpenAI

client = OpenAI()
ask = lambda client, q: client.chat.completions.create(model="gpt-4", messages=[q])
legacy = lambda openai: openai.ChatCompletion.create(model="gpt-4", messages=[])
direct = lambda q: client.chat.completions.create(model="gpt-4", messages=[q])
"#,
    );
    // Only the lambda that closes over the module client is a usage.
    let chains: Vec<&str> = found.calls.iter().map(|c| c.chain.as_str()).collect();
    assert_eq!(chains, vec!["client.chat.completions.create"]);
    assert_eq!(found.calls[0].span.line, 8);
    let reasons: Vec<&str> = gaps(&found, GapKind::UnknownClientFactory)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(reasons, vec!["parameter", "parameter"]);
}

#[test]
fn each_call_site_sees_its_reaching_assignment() {
    let found = usages(
        r#"
from openai import OpenAI, AzureOpenAI

client = OpenAI()
client.chat.completions.create(model="gpt-4", messages=[])
client = AzureOpenAI()
client.chat.completions.create(model="dep", messages=[])

def later():
    return client.chat.completions.create(model="dep", messages=[])

def looped(items):
    c = OpenAI()
    for item in items:
        c.chat.completions.create(model="gpt-4", messages=[])
        c = AzureOpenAI()
    return c
"#,
    );
    let routes: Vec<(u32, Routing)> = found
        .calls
        .iter()
        .map(|c| (c.span.line, c.routing))
        .collect();
    assert_eq!(
        routes,
        vec![
            (5, Routing::OpenaiDirect),
            (7, Routing::AzureOpenai),
            (10, Routing::AzureOpenai),
            // A later assignment in the loop reaches the call on the next
            // iteration.
            (15, Routing::Unknown),
        ]
    );
    let ambiguous = gaps(&found, GapKind::AmbiguousClient);
    assert_eq!(ambiguous.len(), 1);
    assert_eq!(ambiguous[0].span.unwrap().line, 15);
}

// ---------------------------------------------------------------------------
// Review of #276 and the Track C misses: values
// ---------------------------------------------------------------------------

#[test]
fn a_constant_parameter_default_resolves() {
    let found = usages(
        r#"
from openai import OpenAI

GPT_MODEL = "gpt-3.5-turbo"
client = OpenAI()

def ask(q, model: str = GPT_MODEL):
    return client.chat.completions.create(model=model, messages=[q])
"#,
    );
    let model = &call(&found, "client.chat.completions.create").values[0];
    assert_eq!(
        (
            model.value.as_str(),
            model.origin,
            model.name.as_deref(),
            model.span.line
        ),
        (
            "gpt-3.5-turbo",
            ValueOrigin::ParameterDefault,
            Some("model"),
            4
        )
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn environment_and_config_fallbacks_are_env_defaults_with_a_gap() {
    let found = usages(
        r#"
import os
from decouple import config
from openai import OpenAI
from settings import settings

client = OpenAI()
MODEL = os.getenv("OPENAI_MODEL", "gpt-4o-mini")

client.chat.completions.create(model=MODEL, messages=[])
client.chat.completions.create(model=os.environ.get("CHAT_MODEL", "gpt-4o"), messages=[])
client.chat.completions.create(model=config("CHAT_MODEL", default="gpt-4.1"), messages=[])
client.chat.completions.create(model=settings.get("model", "gpt-4-turbo"), messages=[])
client.chat.completions.create(model=os.getenv("CHAT_MODEL") or "gpt-3.5-turbo", messages=[])
client.chat.completions.create(model=getattr(settings, "MODEL", "o3-mini"), messages=[])
"#,
    );
    let values: Vec<(&str, ValueOrigin)> = found
        .calls
        .iter()
        .map(|c| (c.values[0].value.as_str(), c.values[0].origin))
        .collect();
    assert_eq!(
        values,
        vec![
            ("gpt-4o-mini", ValueOrigin::EnvDefault),
            ("gpt-4o", ValueOrigin::EnvDefault),
            ("gpt-4.1", ValueOrigin::EnvDefault),
            ("gpt-4-turbo", ValueOrigin::EnvDefault),
            ("gpt-3.5-turbo", ValueOrigin::EnvDefault),
            ("o3-mini", ValueOrigin::EnvDefault),
        ]
    );
    // Each call keeps its gap: the deployed value may differ.
    let reasons: Vec<&str> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        vec![
            "env_var",
            "env_var",
            "env_var",
            "function_call",
            "env_var",
            "attribute"
        ]
    );
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("OPENAI_MODEL"));
    assert!(!json.contains("CHAT_MODEL"));
    assert!(json.contains("\"env_default\""));
}

#[test]
fn same_file_dict_literals_expand() {
    let found = usages(
        r#"
from openai import OpenAI

client = OpenAI()
PARAMS = {"model": "gpt-4o", "temperature": 0}
MODELS = {"fast": "gpt-4o-mini", "smart": "o3"}
CONFIG = {"openai": {"model": "gpt-4.1"}}
MUTATED = {"model": "gpt-4"}
MUTATED["model"] = pick()

def run(tier):
    client.chat.completions.create(**PARAMS, messages=[])
    client.chat.completions.create(model=MODELS["fast"], messages=[])
    client.chat.completions.create(model=CONFIG["openai"]["model"], messages=[])
    client.chat.completions.create(**dict(model="gpt-4-turbo"), messages=[])
    client.chat.completions.create(model=MODELS.get("smart", "gpt-4"), messages=[])
    client.chat.completions.create(**MUTATED, messages=[])
    client.chat.completions.create(model=MODELS[tier], messages=[])
"#,
    );
    let values: Vec<(u32, &str, ValueOrigin, Option<&str>)> = found
        .calls
        .iter()
        .flat_map(|c| {
            c.values
                .iter()
                .map(move |v| (c.span.line, v.value.as_str(), v.origin, v.name.as_deref()))
        })
        .collect();
    assert_eq!(
        values,
        vec![
            (
                12,
                "gpt-4o",
                ValueOrigin::ModuleConstant,
                Some("PARAMS[\"model\"]")
            ),
            (
                13,
                "gpt-4o-mini",
                ValueOrigin::ModuleConstant,
                Some("MODELS[\"fast\"]")
            ),
            (
                14,
                "gpt-4.1",
                ValueOrigin::ModuleConstant,
                Some("CONFIG[\"openai\"][\"model\"]")
            ),
            (15, "gpt-4-turbo", ValueOrigin::Literal, None),
            (
                16,
                "o3",
                ValueOrigin::ModuleConstant,
                Some("MODELS[\"smart\"]")
            ),
        ]
    );
    let reasons: Vec<(u32, &str)> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| (g.span.unwrap().line, g.reason.as_str()))
        .collect();
    assert_eq!(reasons, vec![(17, "splat"), (18, "subscript")]);
}

#[test]
fn a_client_passed_by_every_in_file_caller_resolves() {
    let found = usages(
        r#"
from openai import OpenAI, AzureOpenAI

client = OpenAI()

def ask(llm, q):
    return llm.chat.completions.create(model="gpt-4", messages=[q])

ask(client, "a")
ask(llm=client, q="b")

class Bot:
    def __init__(self):
        self.client = AzureOpenAI()

    def run(self):
        return self._send(self.client, "x")

    def _send(self, c, q):
        return c.chat.completions.create(model="dep", messages=[q])

def mixed(llm):
    return llm.chat.completions.create(model="gpt-4", messages=[])

mixed(OpenAI())
mixed(AzureOpenAI())

def escaped(llm):
    return llm.chat.completions.create(model="gpt-4", messages=[])

callbacks = [escaped]
escaped(client)
"#,
    );
    let ask = call(&found, "llm.chat.completions.create");
    assert_eq!(
        (ask.routing, ask.client_line),
        (Routing::OpenaiDirect, Some(4))
    );
    assert_eq!(ask.symbol.as_deref(), Some("ask"));
    let send = call(&found, "c.chat.completions.create");
    assert_eq!(send.routing, Routing::AzureOpenai);
    // Callers that disagree, or a function that escapes, stay gaps.
    let unknown: Vec<(Option<&str>, &str)> = gaps(&found, GapKind::UnknownClientFactory)
        .iter()
        .map(|g| (g.symbol.as_deref(), g.reason.as_str()))
        .collect();
    assert_eq!(
        unknown,
        vec![(Some("mixed"), "parameter"), (Some("escaped"), "parameter")]
    );
}

#[test]
fn syntax_errors_are_flagged() {
    let found = usages(
        "from openai import OpenAI\nclient = OpenAI()\ndef broken(:\n    pass\nclient.chat.completions.create(model=\"gpt-4\", messages=[])\n",
    );
    assert!(found.syntax_errors);
    let clean = usages("from openai import OpenAI\nclient = OpenAI()\n");
    assert!(!clean.syntax_errors);
    assert!(!serde_json::to_string(&clean)
        .unwrap()
        .contains("syntax_errors"));
}
