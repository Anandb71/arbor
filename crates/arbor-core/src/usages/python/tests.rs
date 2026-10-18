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
    // `ask` is public: other modules may pass another client, so its
    // routing keeps a gap. `_send` is private: every call is in the file.
    let partial: Vec<(Option<&str>, &str)> = gaps(&found, GapKind::AmbiguousClient)
        .iter()
        .map(|g| (g.symbol.as_deref(), g.reason.as_str()))
        .collect();
    assert_eq!(partial, vec![(Some("ask"), "in_file_callers")]);
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

// ---------------------------------------------------------------------------
// Re-review of #276: bounded output, env_default gaps everywhere
// ---------------------------------------------------------------------------

/// `CFG = {"k0": [{"model": "gpt-4"}, ...], ...}` with `keys` x `items`
/// entries, expanded with `**` at `calls` requests.
fn expanded_dict_source(keys: usize, items: usize, calls: usize) -> String {
    let item = vec!["{\"model\": \"gpt-4\"}"; items].join(", ");
    let entries: Vec<String> = (0..keys)
        .map(|i| format!("    \"k{i}\": [{item}]"))
        .collect();
    let mut source = format!(
        "from openai import OpenAI\nclient = OpenAI()\nCFG = {{\n{}\n}}\n",
        entries.join(",\n")
    );
    for _ in 0..calls {
        source.push_str("client.chat.completions.create(**CFG, messages=[])\n");
    }
    source
}

#[test]
fn a_dict_expanded_at_many_calls_stays_bounded() {
    // Each container in an expanded dict had its own 256-entry budget and
    // the walk ran again at every call: this 109 KB file printed 831 MB of
    // JSON and needed 1.9 GB of memory.
    for (keys, items, calls) in [(64, 64, 500), (256, 256, 50), (16, 16, 2_000)] {
        let source = expanded_dict_source(keys, items, calls);
        let found = usages_within(&source, 10);
        let json = serde_json::to_string(&found).unwrap();
        assert!(
            json.len() < 4_000_000 && json.len() < 40 * source.len(),
            "{keys}x{items}x{calls}: {} bytes of JSON for {} bytes of source",
            json.len(),
            source.len()
        );
        assert_eq!(found.calls.len(), calls);
        let listed: usize = found.calls.iter().map(|c| c.values.len()).sum();
        assert!(listed <= 4096, "{listed} nested values listed");
        for c in &found.calls {
            assert!(
                c.values.len() <= 32,
                "{} values at one call",
                c.values.len()
            );
            // Every call whose nested values were cut says so.
            let start = c.span.start_byte;
            assert!(
                found.gaps.iter().any(|g| g.reason == "truncated"
                    && g.span.map(|s| s.start_byte) == Some(start)),
                "no truncated gap at line {}",
                c.span.line
            );
        }
    }

    // A dict small enough to list in full is listed in full, once per call.
    let found = usages(&expanded_dict_source(2, 3, 3));
    for c in &found.calls {
        let keys: Vec<&str> = c.values.iter().map(|v| v.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["k0.model"; 3]
                .into_iter()
                .chain(vec!["k1.model"; 3])
                .collect::<Vec<_>>()
        );
    }
    // Nothing was cut; each call passes no top-level model.
    assert!(
        found.gaps.iter().all(|g| g.reason == "missing_model"),
        "{:?}",
        found.gaps
    );
}

#[test]
fn one_nested_budget_is_shared_by_every_argument_of_a_call() {
    // Each literal argument used to get its own budget, so a call with many
    // container arguments read (and listed) without bound.
    let mut source = String::from("from openai import OpenAI\nclient = OpenAI()\nclient.chat.completions.create(model=\"gpt-4\", messages=[]");
    for i in 0..200 {
        source.push_str(&format!(
            ", a{i}=[{{\"model\": \"gpt-4o\"}}, {{\"model\": \"o3\"}}]"
        ));
    }
    source.push_str(")\n");
    let found = usages(&source);
    let c = &found.calls[0];
    assert_eq!(c.values[0].key, "model");
    assert!(c.values.len() <= 33, "{} values", c.values.len());
    assert_eq!(
        gaps(&found, GapKind::UnresolvedModel)[0].reason,
        "truncated"
    );
}

#[test]
fn env_default_values_always_carry_a_gap() {
    // The re-review's env fixture: nested fallbacks used to be captured as
    // env_default with no gap, which reads as confirmed.
    let found = usages(
        r#"
import os
from openai import OpenAI
from openai.types.chat import ChatCompletion

client = OpenAI()
client.chat.completions.create(model="gpt-4o", messages=[], extra_body={"model": os.getenv("M4", "gpt-3.5-turbo-0125")})
client.chat.completions.create(model="gpt-4o", messages=[], extra_body={"fallbacks": [{"model": os.getenv("M5", "gpt-4-0613")}]})
P7 = {"extra_body": {"model": os.getenv("M7", "o4-mini")}}
client.chat.completions.create(model="gpt-4o", messages=[], **P7)
client.chat.completions.create(model="gpt-4o", messages=[{"role": "user", "content": "x", "model": os.getenv("M12", "gpt-4-0314")}])
P6 = {"model": os.getenv("M6", "o3-mini")}
client.chat.completions.create(**P6, messages=[])
ChatCompletion(model=os.getenv("M17", "gpt-4"))
"#,
    );
    // Every env_default value sits next to a gap for its own key.
    for c in &found.calls {
        for v in c
            .values
            .iter()
            .filter(|v| v.origin == ValueOrigin::EnvDefault)
        {
            assert!(
                found
                    .gaps
                    .iter()
                    .any(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte)
                        && g.key.as_deref() == Some(v.key.as_str())
                        && g.reason == "env_var"),
                "env_default `{}` at line {} has no gap",
                v.key,
                c.span.line
            );
        }
    }
    let env_keys: Vec<(u32, &str)> = found
        .calls
        .iter()
        .flat_map(|c| {
            c.values
                .iter()
                .filter(|v| v.origin == ValueOrigin::EnvDefault)
                .map(move |v| (c.span.line, v.key.as_str()))
        })
        .collect();
    assert_eq!(
        env_keys,
        vec![
            (7, "extra_body.model"),
            (8, "extra_body.fallbacks.model"),
            (10, "extra_body.model"),
            (11, "messages.model"),
            (13, "model"),
        ]
    );
    // On an SDK type no gap can be reported, so the fallback is not
    // captured either.
    let typed = call(&found, "ChatCompletion");
    assert!(typed.values.is_empty(), "{:?}", typed.values);
}

// ---------------------------------------------------------------------------
// Re-review of #276: each use sees the definitions that can reach it
// ---------------------------------------------------------------------------

/// `(line, model value or "-", gap reasons at that line)` for every call.
fn values_and_gaps(found: &FileUsages) -> Vec<(u32, String, Vec<String>)> {
    found
        .calls
        .iter()
        .map(|c| {
            let value = c
                .values
                .iter()
                .find(|v| v.key == "model")
                .map_or("-".to_string(), |v| v.value.clone());
            let reasons = found
                .gaps
                .iter()
                .filter(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte))
                .map(|g| g.reason.clone())
                .collect();
            (c.span.line, value, reasons)
        })
        .collect()
}

#[test]
fn comprehension_targets_have_their_own_scope() {
    // The body of a comprehension comes before its `for` clause, and the
    // target was recorded in the enclosing scope, so these reported the
    // outer `model` with no gap.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()
model = "gpt-4o"
results = [client.chat.completions.create(model=model, messages=[]) for model in ["gpt-3.5-turbo", "o1-mini"]]
gen = (client.chat.completions.create(model=model, messages=[]) for model in ("o3-mini",))
d3 = {model: client.chat.completions.create(model=model, messages=[]) for model in ["gpt-4"]}

def compare(model="gpt-4o"):
    return [client.chat.completions.create(model=model, messages=[]) for model in ["o1-mini", "gpt-3.5-turbo"]]

def compare2():
    model = "gpt-4o"
    return [client.chat.completions.create(model=model, messages=[]) for model in ("o3-mini",)]

kept = [client.chat.completions.create(model=model, messages=[]) for _ in range(2)]
"#,
    );
    let rows = values_and_gaps(&found);
    for (line, value, reasons) in &rows[..5] {
        assert_eq!(value, "-", "line {line} reported the outer `model`");
        assert!(!reasons.is_empty(), "line {line} has no gap");
    }
    // A name the comprehension does not bind is still the enclosing one.
    assert_eq!(rows[5].1, "gpt-4o");
    assert!(rows[5].2.is_empty(), "{:?}", rows[5]);
}

#[test]
fn nested_functions_see_every_later_definition() {
    // A use in a function was resolved at the end of the enclosing scope, so
    // the last assignment won and the earlier request was missed.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()

MODEL = "o1-mini"
def ask(q):
    return client.chat.completions.create(model=MODEL, messages=[q])
print(ask("hi"))
MODEL = "gpt-4o"

def ask5():
    return CL5.chat.completions.create(model="gpt-4o-mini", messages=[])
CL5 = OpenAI()
ask5()
CL5 = OpenAI(base_url="https://openrouter.ai/api/v1")
ask5()

M = "gpt-4"
gen = (client.chat.completions.create(model=M, messages=[]) for _ in range(2))
M = "o3-mini"

STABLE = "gpt-4.1"
def ok():
    return client.chat.completions.create(model=STABLE, messages=[])
"#,
    );
    let rows = values_and_gaps(&found);
    assert_eq!(rows[0].1, "-");
    assert_eq!(rows[0].2, vec!["reassigned"]);
    let ask5 = call(&found, "CL5.chat.completions.create");
    assert_eq!(ask5.routing, Routing::Unknown);
    assert_eq!(gaps(&found, GapKind::AmbiguousClient).len(), 1);
    // A generator runs when consumed, so it is deferred too.
    assert_eq!(rows[2].1, "-");
    assert_eq!(rows[2].2, vec!["reassigned"]);
    assert_eq!(rows[3].1, "gpt-4.1");
    assert!(rows[3].2.is_empty());
}

#[test]
fn class_bodies_resolve_where_they_run() {
    // `c = CL7` runs when the class is created, before `CL7` is rebound.
    let found = usages(
        r#"
from openai import OpenAI
CL7 = OpenAI()
class Bot7:
    c = CL7
    def go(self):
        return self.c.chat.completions.create(model="gpt-4", messages=[])
CL7 = OpenAI(base_url="https://gw")
"#,
    );
    let go = call(&found, "self.c.chat.completions.create");
    assert_eq!(go.routing, Routing::OpenaiDirect);
    assert_eq!(go.client_line, Some(3));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn a_rebinding_in_another_branch_of_a_loop_does_not_reach() {
    // Track C h042: the definition in each branch dominates its call, so the
    // other branch's client cannot reach it on the next iteration.
    let found = usages(
        r#"import anthropic
from openai import OpenAI

def run(providers):
    for provider in providers:
        if provider == "openai":
            client = OpenAI()
            client.chat.completions.create(model="gpt-4o", messages=[])
        else:
            client = anthropic.Anthropic()
            client.messages.create(model="claude-3-opus-20240229", max_tokens=1, messages=[])
    return client
"#,
    );
    let routes: Vec<(u32, Routing)> = found
        .calls
        .iter()
        .map(|c| (c.span.line, c.routing))
        .collect();
    assert_eq!(
        routes,
        vec![(8, Routing::OpenaiDirect), (11, Routing::AnthropicDirect)]
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);

    // A later rebinding in an inner loop that does not re-run the
    // dominating definition still reaches the call.
    let found = usages(
        r#"from openai import OpenAI, AzureOpenAI

def run(batches):
    for batch in batches:
        client = OpenAI()
        for item in batch:
            client.chat.completions.create(model="gpt-4o", messages=[])
            client = AzureOpenAI()
"#,
    );
    assert_eq!(found.calls[0].routing, Routing::Unknown);
    assert_eq!(gaps(&found, GapKind::AmbiguousClient).len(), 1);
}

// ---------------------------------------------------------------------------
// Re-review of #276: a dict that may change is never read
// ---------------------------------------------------------------------------

#[test]
fn dicts_changed_through_chains_aliases_helpers_or_attributes_are_not_read() {
    // Each of these used to report the literal's `gpt-4o` with no gap while
    // the request sent another model.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()
CFG = {"openai": {"model": "gpt-4o"}}
CFG["openai"]["model"] = "o1-mini"
client.chat.completions.create(**CFG["openai"], messages=[])
CFG2 = {"openai": {"model": "gpt-4o"}}
CFG2["openai"].update(model="o3-mini")
client.chat.completions.create(model=CFG2["openai"]["model"], messages=[])
P3 = {"model": "gpt-4o"}
alias = P3
alias["model"] = "gpt-3.5-turbo"
client.chat.completions.create(**P3, messages=[])
P4 = {"model": "gpt-4o"}
def tweak(d):
    d["model"] = "o4-mini"
tweak(P4)
client.chat.completions.create(**P4, messages=[])
P5 = {"model": "gpt-4o"}
class H: pass
H.d = P5
H.d["model"] = "gpt-4-0613"
client.chat.completions.create(model=P5["model"], messages=[])
P6 = {"extra_body": {"model": "gpt-4o"}}
for key, part in P6.items():
    part["model"] = "o1-mini"
client.chat.completions.create(model="gpt-4", messages=[], **P6)
P7 = {"openai": {"model": "gpt-4o"}}
sub = P7["openai"]
sub["model"] = "o1-mini"
client.chat.completions.create(**P7["openai"], messages=[])
P8 = {"fallbacks": [{"model": "gpt-4o"}]}
P8["fallbacks"].append({"model": "o1-mini"})
client.chat.completions.create(model="gpt-4", messages=[], **P8)
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    assert!(
        !json.contains("\"gpt-4o\""),
        "a stale value was reported: {json}"
    );
    for c in &found.calls {
        if c.values.iter().any(|v| v.key == "model") {
            continue;
        }
        let start = c.span.start_byte;
        assert!(
            found
                .gaps
                .iter()
                .any(|g| g.span.map(|s| s.start_byte) == Some(start)),
            "silent call at line {}",
            c.span.line
        );
    }
}

#[test]
fn dicts_that_are_only_read_still_resolve() {
    let found = usages(
        r#"
import json
from openai import OpenAI
client = OpenAI()
CONFIG = {"model": "gpt-4o", "openai": {"model": "o3"}, "temperature": 0}
MODEL = CONFIG["model"]
OPENAI = CONFIG["openai"]
print(json.dumps(CONFIG), len(CONFIG), "model" in CONFIG)
for key in CONFIG:
    print(key, CONFIG.get(key))
for key, value in {"a": "b"}.items():
    pass
client.chat.completions.create(model=MODEL, messages=[])
client.chat.completions.create(**OPENAI, messages=[])
client.chat.completions.create(model=CONFIG.get("model"), messages=[])
"#,
    );
    let models: Vec<&str> = found
        .calls
        .iter()
        .map(|c| value(c, "model").unwrap_or("-"))
        .collect();
    assert_eq!(models, vec!["gpt-4o", "o3", "gpt-4o"]);
}

#[test]
fn same_named_nested_functions_resolve_their_callers_in_linear_time() {
    // Each nested `f` scanned every call of `f` in the file: 16,500 of them
    // in a 2 MB file took 62 s.
    let mut source = String::from("from openai import OpenAI\nclient = OpenAI()\n");
    for i in 0..6_000 {
        source.push_str(&format!(
            "def o{i}():\n    def f(c):\n        return c.chat.completions.create(model=\"gpt-4\", messages=[])\n    return f(client)\n"
        ));
    }
    // Calls of `f` that bind elsewhere (the module) do not count.
    for _ in 0..20_000 {
        source.push_str("f()\n");
    }
    // Same-named methods called on `self` in many classes.
    for i in 0..3_000 {
        source.push_str(&format!(
            "class K{i}:\n    def _send(self, c):\n        return c.chat.completions.create(model=\"gpt-4\", messages=[])\n    def run(self):\n        return self._send(client)\n"
        ));
    }
    let found = usages_within(&source, 10);
    // Nested functions resolve from their only caller.
    let resolved = found
        .calls
        .iter()
        .filter(|c| c.chain == "c.chat.completions.create" && c.routing == Routing::OpenaiDirect)
        .count();
    assert_eq!(resolved, 6_000);
    // Methods of the same name in other classes may be inherited, so those
    // stay gaps.
    assert_eq!(gaps(&found, GapKind::UnknownClientFactory).len(), 3_000);
}

// ---------------------------------------------------------------------------
// Second re-review of #276
// ---------------------------------------------------------------------------

#[test]
fn a_dict_get_default_resolves_where_the_call_runs() {
    // The default of `DEFAULTS.get("model", M)` was resolved in the dict's
    // scope, so the module's `M` was reported for a request that sends the
    // function's own `M`.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()
DEFAULTS = {"temperature": 0}
M = "gpt-3.5-turbo-instruct"
def ask4():
    M = "gpt-4o"
    return client.chat.completions.create(model=DEFAULTS.get("model", M), messages=[])
client.chat.completions.create(model=DEFAULTS.get("model", M), messages=[])
"#,
    );
    let models: Vec<(&str, ValueOrigin)> = found
        .calls
        .iter()
        .map(|c| (c.values[0].value.as_str(), c.values[0].origin))
        .collect();
    assert_eq!(
        models,
        vec![
            ("gpt-4o", ValueOrigin::LocalConstant),
            ("gpt-3.5-turbo-instruct", ValueOrigin::ModuleConstant)
        ]
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn a_refused_lookup_key_is_never_echoed() {
    // `D.get(<credential-shaped key>, ...)` put the key itself in the gap's
    // detail.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()
D3 = {"a": "b"}
client.chat.completions.create(model=D3.get("sk-proj-tokenFAKEFAKE0000fakekeyXYZ", "gpt-4"), messages=[])
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("FAKEFAKE"), "{json}");
    let refused = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(refused.reason, "credential");
    assert!(refused.detail.contains("D3.get(...)"), "{}", refused.detail);
}

#[test]
fn wider_secret_names_and_urls_with_credentials_are_refused() {
    // Each of these printed its value: the needles missed `passwd`, `pwd`,
    // `cred` and `private`, a class name in `ApiKeys.openai` was not
    // checked, and a URL with a password was not credential-shaped.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()
class ApiKeys:
    openai = "hunter2-FLOWN"
client.chat.completions.create(model=ApiKeys.openai, messages=[])
PASSWD = "hunter2-FLOWO"
client.chat.completions.create(model=PASSWD, messages=[])
CREDENTIALS = "hunter2-FLOWP"
client.chat.completions.create(model=CREDENTIALS, messages=[])
DATABASE_URL = "postgres://admin:hunter2FLOWQ@db/prod"
client.chat.completions.create(model=DATABASE_URL, messages=[])
db_pwd = "hunter2-FLOWR"
client.chat.completions.create(model=db_pwd, messages=[])
private_model = "hunter2-FLOWS"
client.chat.completions.create(model=private_model, messages=[])
GATEWAY = "https://hunter2FLOWT@gw.example/v1"
client.chat.completions.create(model=GATEWAY, messages=[])
P19 = {"extra_body": {"credentials": {"model": "hunter2-D19"}}}
client.chat.completions.create(model="gpt-4", messages=[], **P19)
client.chat.completions.create(model="gpt-4", messages=[], extra_body={"cred": {"model": "hunter2-D20"}})
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("hunter2"), "{json}");
    let refused = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .filter(|g| g.reason == "credential")
        .count();
    assert_eq!(refused, 7, "{json}");
    // The safe values next to them are still captured.
    let last = found.calls.last().unwrap();
    let keys: Vec<&str> = last.values.iter().map(|v| v.key.as_str()).collect();
    assert_eq!(keys, vec!["model"]);
}

/// `(line, routing, gap kinds and reasons at that line)` for every call.
fn routes_and_gaps(found: &FileUsages) -> Vec<(u32, Routing, Vec<String>)> {
    found
        .calls
        .iter()
        .map(|c| {
            let reasons = found
                .gaps
                .iter()
                .filter(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte))
                .map(|g| format!("{}/{}", g.kind, g.reason))
                .collect();
            (c.span.line, c.routing, reasons)
        })
        .collect()
}

#[test]
fn a_client_default_is_combined_with_its_callers() {
    // A client-valued default was returned before the callers were read, so
    // `ask5("x", c=gw)` was reported as `openai_direct` with no gap.
    let found = usages(
        r#"from openai import OpenAI
client = OpenAI()
gw = OpenAI(base_url="https://openrouter.ai/api/v1")
def ask5(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
ask5("x", c=gw)
def ask14(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
ask14("x", gw)
f12 = lambda q, c=client: c.chat.completions.create(model="gpt-4", messages=[q])
f12("x", c=gw)
def _only_gw(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
_only_gw("x", c=gw)
def _only_default(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
_only_default("x")
def _unknown(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
_unknown("x", c=make())
def _splat(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
_splat("x", **opts)
def uncalled(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
"#,
    );
    let ambiguous = || vec!["ambiguous_client/multiple_assignments".to_string()];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            // Public: the callers' client and the default are both possible.
            (5, Routing::Unknown, ambiguous()),
            (8, Routing::Unknown, ambiguous()),
            (10, Routing::Unknown, ambiguous()),
            // Private: every call is in the file.
            (13, Routing::CustomBaseUrl, vec![]),
            (16, Routing::OpenaiDirect, vec![]),
            // Public with no caller here: other modules may pass another.
            (
                25,
                Routing::OpenaiDirect,
                vec!["ambiguous_client/parameter_default".to_string()]
            ),
        ]
    );
    // A caller passing an unknown client, or `**kwargs`, leaves the
    // receiver unresolved.
    let unknown: Vec<(u32, &str)> = gaps(&found, GapKind::UnknownClientFactory)
        .iter()
        .map(|g| (g.span.unwrap().line, g.reason.as_str()))
        .collect();
    assert_eq!(
        unknown,
        vec![(19, "factory_call"), (22, "unresolved_receiver")]
    );
}

/// `(line, model value or "-", [(gap reason, listed values)])` for a call.
type ValueRow = (u32, String, Vec<(String, Vec<String>)>);

#[test]
fn a_parameter_default_that_callers_override_keeps_a_gap() {
    // A literal parameter default was reported with no gap even when the
    // in-file callers pass other models.
    let found = usages(
        r#"from openai import OpenAI
client = OpenAI()
GPT_MODEL = "gpt-3.5-turbo"
def ask(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
ask("hi", model="gpt-3.5-turbo")
ask("hi", "o1-mini")
ask("hi", "gpt-4o")
def ask2(q, model: str = GPT_MODEL):
    return client.chat.completions.create(model=model, messages=[q])
ask2("hi")
ask2("hi", model=GPT_MODEL)
f13 = lambda q, model="gpt-4o": client.chat.completions.create(model=model, messages=[q])
f13("x", model="gpt-3.5-turbo")
def ask4(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
ask4("x", model=pick())
def ask5(q, model):
    return client.chat.completions.create(model=model, messages=[q])
ask5("x", "o3-mini")
ask5("y", model="o3-mini")
ask5("z", "gpt-4.1")
"#,
    );
    let rows: Vec<ValueRow> = found
        .calls
        .iter()
        .map(|c| {
            let value = value(c, "model").unwrap_or("-").to_string();
            let gaps = found
                .gaps
                .iter()
                .filter(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte))
                .map(|g| {
                    let listed = g.values.iter().map(|v| v.value.clone()).collect();
                    (g.reason.clone(), listed)
                })
                .collect();
            (c.span.line, value, gaps)
        })
        .collect();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        rows,
        vec![
            (
                5,
                "gpt-4o".to_string(),
                vec![(
                    "parameter_overridden".to_string(),
                    s(&["gpt-3.5-turbo", "o1-mini"])
                )]
            ),
            // Callers pass the default or nothing: no gap.
            (10, "gpt-3.5-turbo".to_string(), vec![]),
            (
                13,
                "gpt-4o".to_string(),
                vec![("parameter_overridden".to_string(), s(&["gpt-3.5-turbo"]))]
            ),
            // A caller passes a value that does not resolve.
            (
                16,
                "gpt-4o".to_string(),
                vec![("parameter_overridden".to_string(), vec![])]
            ),
            // No default: the values the callers pass are listed.
            (
                19,
                "-".to_string(),
                vec![("parameter".to_string(), s(&["o3-mini", "gpt-4.1"]))]
            ),
        ]
    );
    let overridden = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert!(
        overridden
            .detail
            .contains("its default is captured with origin `parameter_default`"),
        "{}",
        overridden.detail
    );
    // Each listed value points at the caller's literal.
    assert_eq!(overridden.values[1].span.line, 7);
}

#[test]
fn a_base_url_assigned_after_construction_keeps_a_gap() {
    // `client.base_url = ...` after `OpenAI()` was ignored: the calls were
    // reported as `openai_direct` with no gap.
    let found = usages(
        r#"from openai import OpenAI
from anthropic import Anthropic
CL8 = OpenAI()
CL8.base_url = "https://openrouter.ai/api/v1"
CL8.chat.completions.create(model="gpt-4", messages=[])
alias = CL8
alias.chat.completions.create(model="gpt-4", messages=[])
other = OpenAI()
other.chat.completions.create(model="gpt-4", messages=[])
class Bot:
    def __init__(self):
        self.client = Anthropic()
        self.client.base_url = "https://gw.example"
    def go(self):
        return self.client.messages.create(model="claude-3-opus-20240229", max_tokens=1, messages=[])
"#,
    );
    let gap = || vec!["ambiguous_client/base_url_assigned".to_string()];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (5, Routing::Unknown, gap()),
            (7, Routing::Unknown, gap()),
            (9, Routing::OpenaiDirect, vec![]),
            (15, Routing::Unknown, gap()),
        ]
    );
}

#[test]
fn an_endpoint_set_in_the_environment_keeps_a_gap() {
    // `os.environ["OPENAI_BASE_URL"] = ...` before `OpenAI()` was ignored.
    let found = usages(
        r#"import os
import openai
from openai import OpenAI
os.environ["OPENAI_BASE_URL"] = "http://localhost:11434/v1"
CL9 = OpenAI()
CL9.chat.completions.create(model="gpt-4", messages=[])
own = OpenAI(base_url="https://gw.example")
own.chat.completions.create(model="gpt-4", messages=[])
openai.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let gap = || vec!["ambiguous_client/endpoint_env".to_string()];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (6, Routing::Unknown, gap()),
            (8, Routing::CustomBaseUrl, vec![]),
            (9, Routing::Unknown, gap()),
        ]
    );
    assert_eq!(routing_of(&found, "CL9"), Routing::Unknown);

    let found = usages(
        r#"import os
from anthropic import Anthropic
os.environ.setdefault("ANTHROPIC_BASE_URL", "https://gw.example")
a = Anthropic()
a.messages.create(model="claude-3-opus-20240229", max_tokens=1, messages=[])
"#,
    );
    assert_eq!(routes_and_gaps(&found), vec![(5, Routing::Unknown, gap())]);
    for write in [
        "os.environ.update({\"ANTHROPIC_BASE_URL\": \"https://gw\"})",
        "os.environ.update(ANTHROPIC_BASE_URL=\"https://gw\")",
        "os.putenv(\"ANTHROPIC_BASE_URL\", \"https://gw\")",
    ] {
        let found = usages(&format!(
            "import os
from anthropic import Anthropic
{write}
Anthropic().messages.create(model=\"claude-3-opus-20240229\", max_tokens=1, messages=[])
"
        ));
        assert_eq!(routes_and_gaps(&found), vec![(4, Routing::Unknown, gap())]);
    }
    // Another variable, or none, changes nothing.
    let found = usages(
        "import os
from anthropic import Anthropic
os.environ[\"ANTHROPIC_LOG\"] = \"debug\"
Anthropic().messages.create(model=\"claude-3-opus-20240229\", max_tokens=1, messages=[])
",
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![(4, Routing::AnthropicDirect, vec![])]
    );
}

#[test]
fn a_request_with_no_model_is_a_gap() {
    // Track C: a request that passes no model at all (the SDK or server
    // default applies) was silent.
    let found = usages(
        r#"import google.generativeai as legacy
from openai import OpenAI
from anthropic import Anthropic
client = OpenAI()
ac = Anthropic()
client.chat.completions.create(messages=[])
client.images.generate(prompt="a cat")
ac.messages.create(max_tokens=1, messages=[])
legacy.GenerativeModel()
P = {"temperature": 0}
client.chat.completions.create(**P, messages=[])
client.chat.completions.create(*args)
client.chat.completions.create(**kwargs)
client.chat.completions.create(model="gpt-4o", messages=[])
client.files.list()
"#,
    );
    let rows: Vec<(u32, Option<&str>, &str)> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| (g.span.unwrap().line, g.key.as_deref(), g.reason.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (6, Some("model"), "missing_model"),
            (7, Some("model"), "missing_model"),
            (8, Some("model"), "missing_model"),
            (9, Some("model_name"), "missing_model"),
            (11, Some("model"), "missing_model"),
            (13, Some("model"), "splat"),
        ]
    );
}

#[test]
fn lookups_with_a_run_time_key_list_the_values_they_may_take() {
    // Track C: `MODELS[os.getenv("TIER")]` was a bare gap. The dict's
    // values (never one under a secret-named key) are now listed with it,
    // and so are the values of a name assigned in two branches, through a
    // parameter default too.
    let found = usages(
        r#"import os
from openai import OpenAI
client = OpenAI()
MODELS = {"fast": "gpt-4o-mini", "smart": "o1-mini", "legacy": "gpt-3.5-turbo", "api_key": "hunter2-E3", "dup": "o1-mini"}
client.chat.completions.create(model=MODELS[os.getenv("TIER")], messages=[])
tier = input()
client.chat.completions.create(model=MODELS[tier], messages=[])
client.chat.completions.create(model=MODELS.get(tier, "gpt-4"), messages=[])
client.chat.completions.create(model=MODELS.get(tier), messages=[])
if os.getenv("FAST"):
    M = "gpt-3.5-turbo-0125"
else:
    M = "gpt-4-0613"
def ask(q, model=M):
    return client.chat.completions.create(model=model, messages=[q])
client.chat.completions.create(model=M, messages=[])
class Bot:
    def a(self):
        self.model = "o1-preview"
    def b(self):
        self.model = "gpt-4-32k"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("hunter2"), "{json}");
    let listed: Vec<(u32, &str, Vec<&str>)> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| {
            let values = g.values.iter().map(|v| v.value.as_str()).collect();
            (g.span.unwrap().line, g.reason.as_str(), values)
        })
        .collect();
    let dict = vec!["gpt-4o-mini", "o1-mini", "gpt-3.5-turbo"];
    let branches = vec!["gpt-3.5-turbo-0125", "gpt-4-0613"];
    assert_eq!(
        listed,
        vec![
            (5, "env_var", dict.clone()),
            (7, "subscript", dict.clone()),
            (8, "function_call", dict.clone()),
            (9, "function_call", dict),
            (15, "parameter", branches.clone()),
            (16, "reassigned", branches),
            (23, "reassigned", vec!["o1-preview", "gpt-4-32k"]),
        ]
    );
    // Listed values are never the call's own value.
    let values: Vec<usize> = found.calls.iter().map(|c| c.values.len()).collect();
    assert_eq!(values, vec![0, 0, 1, 0, 0, 0, 0]);
    // Each listed value points at its literal.
    let fast = &gaps(&found, GapKind::UnresolvedModel)[0].values[0];
    assert_eq!(
        (fast.span.line, fast.name.as_deref()),
        (4, Some("MODELS[\"fast\"]"))
    );
}

#[test]
fn listed_values_stay_bounded_per_gap_and_per_file() {
    // A 2,000-entry dict looked up with a run-time key at 2,000 calls: at
    // most 32 values per gap and 4,096 per file, read once per dict, and
    // never marked complete.
    let entries: Vec<String> = (0..2_000)
        .map(|i| format!("\"k{i}\": \"gpt-4-{i}\""))
        .collect();
    let mut source = format!(
        "from openai import OpenAI\nclient = OpenAI()\nD = {{{}}}\n",
        entries.join(", ")
    );
    for _ in 0..2_000 {
        source.push_str("client.chat.completions.create(model=D[k], messages=[])\n");
    }
    let found = usages_within(&source, 10);
    let per_gap: Vec<usize> = found.gaps.iter().map(|g| g.values.len()).collect();
    assert!(per_gap.iter().all(|n| *n <= 32));
    assert_eq!(per_gap.iter().sum::<usize>(), 4_096);
    assert!(found.gaps.iter().all(|g| !g.values_complete));
    let json = serde_json::to_string(&found).unwrap();
    assert!(json.len() < 8_000_000, "{} bytes", json.len());
}

#[test]
fn a_long_name_reported_at_every_use_cannot_multiply_the_report() {
    // A name written once and repeated in every record (a function's
    // symbol, a constant behind a short alias, a client bound to a long
    // attribute chain) made a 1.5 MB file print 0.4 to 1 GB of JSON.
    let long = "x".repeat(10_000);
    let calls = 2_000;
    let shapes = [
        format!(
            "def f{long}():\n{}",
            "    client.chat.completions.create()\n".repeat(calls)
        ),
        format!(
            "{long} = os.getenv('M')\nM = {long}\n{}",
            "client.chat.completions.create(model=M)\n".repeat(calls)
        ),
        format!("c = client.{long}.{long}\n{}", "c.create()\n".repeat(calls)),
        format!(
            "from {long} import x\n{}",
            "x.chat.completions.create(model=\"gpt-4\")\n".repeat(calls)
        ),
    ];
    for body in shapes {
        let source = format!("import os\nfrom openai import OpenAI\nclient = OpenAI()\n{body}");
        let found = usages_within(&source, 10);
        let json = serde_json::to_string(&found).unwrap();
        // A call and its gap fit in 2 KB however long the name: before,
        // each repeated the whole 10 KB (or 20 KB) name.
        assert!(
            json.len() < calls * 2_000,
            "{} bytes of JSON for {calls} calls",
            json.len()
        );
        assert!(!json.contains(&"x".repeat(MAX_TEXT_FOR_TESTS + 1)));
    }
}

/// The longest run of one name a report may hold (see `MAX_TEXT`).
const MAX_TEXT_FOR_TESTS: usize = 200;

#[test]
fn a_file_reports_at_most_twenty_thousand_records() {
    let mut source = String::from("from openai import OpenAI\n");
    for _ in 0..25_000 {
        source.push_str("x.chat.completions.create(model=\"gpt-4\")\n");
    }
    let found = usages_within(&source, 10);
    // The import counts too: 19,999 gaps, then the truncated one.
    assert_eq!(found.imports.len() + found.gaps.len(), 20_001);
    let last = found.gaps.last().unwrap();
    assert_eq!(
        (last.reason.as_str(), last.span.unwrap().line),
        ("truncated", 20_001)
    );
}

#[test]
fn a_constructor_imported_two_ways_builds_an_ambiguous_client() {
    // `Maker` is `AzureOpenAI` or `OpenAI` depending on the import: the
    // first one won, so the client was reported as `azure_openai`.
    let found = usages(
        r#"try:
    from openai import AzureOpenAI as Maker
except ImportError:
    from openai import OpenAI as Maker
CL16 = Maker()
CL16.chat.completions.create(model="gpt-4", messages=[])
try:
    from openai import AsyncOpenAI as Same
except ImportError:
    from openai import OpenAI as Same
CL17 = Same()
CL17.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (
                6,
                Routing::Unknown,
                vec!["ambiguous_client/multiple_assignments".to_string()]
            ),
            (12, Routing::OpenaiDirect, vec![]),
        ]
    );
}

// ---------------------------------------------------------------------------
// Third review of #276
// ---------------------------------------------------------------------------

#[test]
fn imports_stay_bounded_however_many_names_a_statement_binds() {
    // Every imported name copied the whole module path into its record, its
    // binding and (for another module) its gap detail, and imports were not
    // counted against the record cap: a 34 KB file printed 43.5 MB of JSON
    // and peaked at 1.4 GB, a 2 MB one crashed after 100 s.
    let names = vec!["a"; 10_000].join(", ");
    let deep = ".x".repeat(2_000);
    let call = "OpenAI().chat.completions.create(model=\"gpt-4\", messages=[])\n";
    for source in [
        format!("from openai{deep} import ({names})\nfrom openai import OpenAI\n{call}"),
        format!(
            "import openai\nfrom mylib{deep} import ({names})\nfrom openai import OpenAI\n{call}"
        ),
        format!("from openai import OpenAI\nimport openai{deep} as oa\n{call}"),
    ] {
        let found = usages_within(&source, 10);
        let json = serde_json::to_string(&found).unwrap();
        // About 400 bytes per imported name (the module clipped to 200).
        assert!(
            json.len() < 5_000_000,
            "{} bytes of JSON for {} bytes of source",
            json.len(),
            source.len()
        );
        assert!(found
            .imports
            .iter()
            .all(|i| i.module.len() <= MAX_TEXT_FOR_TESTS));
        // The call after the imports is still analyzed.
        assert_eq!(found.calls.len(), 1);
        assert_eq!(value(&found.calls[0], "model"), Some("gpt-4"));
    }

    // More imported names than a file may bind: the imports stop at the cap
    // and no call is analyzed, since a name left unbound could change what
    // any of them resolves to.
    let mut source = String::from("from openai import OpenAI\nclient = OpenAI()\n");
    for _ in 0..3_000 {
        source.push_str("from openai.types import (a, b, c, d, e, f, g, h)\n");
    }
    source.push_str("from openai import AzureOpenAI as OpenAI\n");
    source.push_str("client.chat.completions.create(model=\"gpt-4\", messages=[])\n");
    let found = usages_within(&source, 10);
    assert_eq!(found.imports.len(), 20_000);
    assert!(found.calls.is_empty() && found.clients.is_empty());
    let gap = found.gaps.last().unwrap();
    assert_eq!(
        (gap.reason.as_str(), gap.span.unwrap().line),
        ("truncated", 2_502)
    );

    // Wildcard imports bind every client class: they count as well.
    let source = "from openai import *\n".repeat(5_000);
    let found = usages_within(&source, 10);
    assert!(found.imports.len() < 5_000, "{}", found.imports.len());
    assert_eq!(found.gaps.len(), 1);
}

#[test]
fn a_nonlocal_rebinding_reaches_the_enclosing_function() {
    // `nonlocal` was ignored, so the nested assignment bound a name of its
    // own and the enclosing call reported `gpt-4o` and an `openai_direct`
    // client with no gap.
    let found = usages(
        r#"from openai import OpenAI, AzureOpenAI
client = OpenAI()
def run_a():
    c = OpenAI()
    def switch():
        nonlocal c
        c = AzureOpenAI()
    switch()
    return c.chat.completions.create(model="gpt-4o", messages=[])
def run_b():
    model = "gpt-4o"
    def pick():
        nonlocal model
        model = "o1-mini"
    pick()
    return client.chat.completions.create(model=model, messages=[])
def run_c():
    model = "gpt-4o"
    def middle():
        nonlocal model
        def inner():
            nonlocal model
            model = "o3-mini"
        inner()
    middle()
    return client.chat.completions.create(model=model, messages=[])
def run_d():
    model = "gpt-4o"
    def reader():
        return client.chat.completions.create(model=model, messages=[])
    return reader()
"#,
    );
    let rows: Vec<(u32, Routing, String, Vec<String>)> = found
        .calls
        .iter()
        .map(|c| {
            let reasons = found
                .gaps
                .iter()
                .filter(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte))
                .map(|g| g.reason.clone())
                .collect();
            let model = value(c, "model").unwrap_or("-").to_string();
            (c.span.line, c.routing, model, reasons)
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                9,
                Routing::Unknown,
                "gpt-4o".to_string(),
                vec!["multiple_assignments".to_string()]
            ),
            (
                16,
                Routing::OpenaiDirect,
                "-".to_string(),
                vec!["reassigned".to_string()]
            ),
            (
                26,
                Routing::OpenaiDirect,
                "-".to_string(),
                vec!["reassigned".to_string()]
            ),
            // No `nonlocal`: the nested function only reads.
            (30, Routing::OpenaiDirect, "gpt-4o".to_string(), vec![]),
        ]
    );
    let listed: Vec<&str> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .find(|g| g.span.unwrap().line == 16)
        .unwrap()
        .values
        .iter()
        .map(|v| v.value.as_str())
        .collect();
    assert_eq!(listed, vec!["gpt-4o", "o1-mini"]);
}

#[test]
fn a_function_used_as_a_value_keeps_a_gap_on_its_defaults() {
    // Only direct calls were read, so a default overridden through an
    // executor, `functools.partial`, `map`, a callback or a task queue
    // reported `gpt-4o` (or the default's routing) with no gap.
    let found = usages(
        r#"import functools
from concurrent.futures import ThreadPoolExecutor
from openai import OpenAI
client = OpenAI()
gw = OpenAI(base_url="https://gw.example/v1")
def ask5(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
with ThreadPoolExecutor() as pool:
    pool.submit(ask5, "hi", "o1-mini")
def ask6(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
fast6 = functools.partial(ask6, model="o1-mini")
def ask7(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
list(map(ask7, ["a", "b"], ["o1-mini", "o3-mini"]))
def _ask8(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
callbacks = {"fast": lambda q: _ask8(q, model="o1-mini"), "slow": _ask8}
def task(f):
    return f
@task
def ask9(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
ask9.delay("hi", model="o1-mini")
def ask10(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
via_gw = functools.partial(ask10, c=gw)
def _ask11(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])
_ask11("x")
def ask12(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
ask12("x")
"#,
    );
    let rows: Vec<(u32, String, Vec<String>)> = values_and_gaps(&found);
    let unseen = || vec!["unseen_callers".to_string()];
    assert_eq!(
        rows,
        vec![
            (7, "gpt-4o".to_string(), unseen()),
            (11, "gpt-4o".to_string(), unseen()),
            (14, "gpt-4o".to_string(), unseen()),
            (17, "gpt-4o".to_string(), unseen()),
            (23, "gpt-4o".to_string(), unseen()),
            (26, "gpt-4".to_string(), unseen()),
            // Private and called only directly: the default holds.
            (29, "gpt-4".to_string(), vec![]),
            // Public, called here only without the argument.
            (32, "gpt-4o".to_string(), vec![]),
        ]
    );
    // The values the visible calls pass are listed with the gap.
    let listed: Vec<&str> = found
        .gaps
        .iter()
        .find(|g| g.span.unwrap().line == 17)
        .unwrap()
        .values
        .iter()
        .map(|v| v.value.as_str())
        .collect();
    assert_eq!(listed, vec!["o1-mini"]);
    // The client default keeps its routing, with the gap beside it.
    assert_eq!(
        routes_and_gaps(&found)[5],
        (
            26,
            Routing::OpenaiDirect,
            vec!["ambiguous_client/unseen_callers".to_string()]
        )
    );
}

#[test]
fn a_method_default_is_weighed_against_calls_on_any_object() {
    // Calls on an instance (`bot.ask(...)`) were not read, so the method's
    // default was reported with no gap.
    let found = usages(
        r#"from openai import OpenAI
client = OpenAI()
class Bot:
    def ask(self, q, model="gpt-4o"):
        return client.chat.completions.create(model=model, messages=[q])
    def plain(self, q, model="gpt-4o"):
        return client.chat.completions.create(model=model, messages=[q])
bot = Bot()
bot.ask("hi", model="o1-mini")
bot.plain("hi")
"#,
    );
    assert_eq!(
        values_and_gaps(&found),
        vec![
            (
                5,
                "gpt-4o".to_string(),
                vec!["parameter_overridden".to_string()]
            ),
            (7, "gpt-4o".to_string(), vec![]),
        ]
    );
}

#[test]
fn init_defaults_are_weighed_against_constructor_calls() {
    // Building a class was not counted as calling its `__init__`, so a
    // default stored on `self` reported `gpt-4o` and the default client
    // `openai_direct` while the constructor call passed another.
    let found = usages(
        r#"from openai import OpenAI
client = OpenAI()
class Agent:
    def __init__(self, model="gpt-4o"):
        self.model = model
    def run(self):
        return client.chat.completions.create(model=self.model, messages=[])
Agent(model="o1-mini").run()
class Svc2:
    def __init__(self, c=client):
        self.c = c
    def go(self):
        return self.c.chat.completions.create(model="gpt-4", messages=[])
Svc2(OpenAI(base_url="https://gw.example/v1")).go()
class Quiet:
    def __init__(self, model="gpt-4o"):
        self.model = model
    def run(self):
        return client.chat.completions.create(model=self.model, messages=[])
q: Quiet = Quiet()
"#,
    );
    assert_eq!(
        values_and_gaps(&found),
        vec![
            (
                7,
                "gpt-4o".to_string(),
                vec!["parameter_overridden".to_string()]
            ),
            (
                13,
                "gpt-4".to_string(),
                vec!["multiple_assignments".to_string()]
            ),
            // Built here without the argument (an annotation is not a use).
            (19, "gpt-4o".to_string(), vec![]),
        ]
    );
    assert_eq!(found.calls[1].routing, Routing::Unknown);
}

/// `(line, model value or "-", gap reasons)` for each call of `source`.
fn rows_of(source: &str) -> Vec<(u32, String, Vec<String>)> {
    values_and_gaps(&usages(source))
}

#[test]
fn self_attributes_see_subclasses_and_outside_writes() {
    // `self.MODEL` read only the class and its bases, so a subclass that
    // overrides it, `bot.model = ...` and `setattr(bot, "model", ...)`
    // were invisible and the base's `gpt-4o` was reported with no gap.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    let reassigned = || vec!["reassigned".to_string()];
    for (body, line) in [
        (
            r#"class Base:
    MODEL = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.MODEL, messages=[])
class Fast(Base):
    MODEL = "o1-mini"
Fast().go()
"#,
            6,
        ),
        (
            r#"class Bot:
    model = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
bot = Bot()
bot.model = "o1-mini"
bot.go()
"#,
            6,
        ),
        (
            r#"class Bot2:
    model = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
b2 = Bot2()
setattr(b2, "model", "o1-mini")
b2.go()
"#,
            6,
        ),
        (
            r#"class Lazy:
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
class Real(Lazy):
    model = "gpt-4o"
"#,
            5,
        ),
    ] {
        let found = usages(&format!("{header}{body}"));
        let rows = values_and_gaps(&found);
        assert_eq!(rows, vec![(line, "-".to_string(), reassigned())], "{body}");
    }
    // An attribute named at run time may be any of them.
    for body in [
        r#"class Cfg:
    model = "gpt-4o"
    def __init__(self, **kw):
        for k, v in kw.items():
            setattr(self, k, v)
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
"#,
        r#"class Cfg:
    model = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
c = Cfg()
c.__dict__.update(opts)
"#,
        r#"class Cfg:
    model = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
c = Cfg()
vars(c)[key] = value
"#,
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "-", "{body}");
        assert_eq!(rows[0].2, reassigned(), "{body}");
    }
    // A class read by name is not an instance: outside writes do not reach.
    let rows = rows_of(&format!(
        "{header}class K:\n    MODEL = \"gpt-4o\"\nk = K()\nk.MODEL = \"o1-mini\"\nclient.chat.completions.create(model=K.MODEL, messages=[])\n"
    ));
    assert_eq!(rows, vec![(7, "gpt-4o".to_string(), vec![])]);
}

#[test]
fn self_clients_see_subclasses_and_setattr() {
    // A subclass that swaps the client, and `setattr(client, "base_url",
    // ...)`, left the client `openai_direct` with no gap.
    let found = usages(
        r#"from openai import OpenAI
class Svc:
    client = OpenAI()
    def go(self):
        return self.client.chat.completions.create(model="gpt-4", messages=[])
class GwSvc(Svc):
    client = OpenAI(base_url="https://gw.example/v1")
GwSvc().go()
c8 = OpenAI()
setattr(c8, "base_url", "https://gw.example/v1")
c8.chat.completions.create(model="gpt-4", messages=[])
c9 = OpenAI()
setattr(c9, attr_name, value)
c9.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (
                5,
                Routing::Unknown,
                vec!["ambiguous_client/multiple_assignments".to_string()]
            ),
            (
                11,
                Routing::Unknown,
                vec!["ambiguous_client/base_url_assigned".to_string()]
            ),
            (
                14,
                Routing::Unknown,
                vec!["ambiguous_client/base_url_assigned".to_string()]
            ),
        ]
    );
}

#[test]
fn a_constructor_chosen_against_another_value_builds_an_ambiguous_client() {
    // "A tracked import beats a fallback" picked the SDK branch, so these
    // reported `azure_openai` or `openai_direct` with no gap.
    let found = usages(
        r#"import os, functools
from openai import OpenAI, AzureOpenAI
def build():
    return OpenAI(base_url="https://gw.example/v1")
make = AzureOpenAI if os.getenv("AZ") else build
c1 = make()
c1.chat.completions.create(model="gpt-4", messages=[])
make2 = OpenAI if os.getenv("D") else make_client
c2 = make2()
c2.chat.completions.create(model="gpt-4", messages=[])
make3 = AzureOpenAI if os.getenv("AZ") else functools.partial(OpenAI, base_url="https://gw.example/v1")
c3 = make3()
c3.chat.completions.create(model="gpt-4", messages=[])
try:
    from openai import AzureOpenAI as Maker
except ImportError:
    from mylib import FakeClient as Maker
c4 = Maker()
c4.chat.completions.create(model="gpt-4", messages=[])
try:
    from openai import OpenAI as Plain
except ImportError:
    Plain = None
c5 = Plain()
c5.chat.completions.create(model="gpt-4", messages=[])
"#,
    );
    let ambiguous = || vec!["ambiguous_client/multiple_assignments".to_string()];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (7, Routing::Unknown, ambiguous()),
            (10, Routing::Unknown, ambiguous()),
            (13, Routing::Unknown, ambiguous()),
            (19, Routing::Unknown, ambiguous()),
            // `None` is a placeholder: the import is the only constructor.
            (25, Routing::OpenaiDirect, vec![]),
        ]
    );
}

#[test]
fn dicts_aliased_by_a_chain_a_class_attribute_or_the_namespace_are_not_read() {
    // `x = y = CFG2` aliased only `y`, a mutation through `K.cfg[...]` or
    // `self.cfg[...]` marked only `K` / `self`, and `globals()["CFG7"]`
    // marked `globals`, so each call read the literal's `gpt-4o`.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "CFG = {\"model\": \"gpt-4o\"}\nclass K:\n    cfg = CFG\nK.cfg[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**CFG, messages=[])\n",
        "CFG2 = {\"model\": \"gpt-4o\"}\nx = y = CFG2\nx[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**CFG2, messages=[])\n",
        "CFG2 = {\"model\": \"gpt-4o\"}\na = b = c = CFG2\na[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**CFG2, messages=[])\n",
        "CFG3 = {\"model\": \"gpt-4o\"}\nclass K3:\n    cfg = CFG3\n    def tune(self):\n        self.cfg[\"model\"] = \"o3-mini\"\nclient.chat.completions.create(**CFG3, messages=[])\n",
        "CFG4 = {\"model\": \"gpt-4o\"}\nclass K4:\n    cfg = CFG4\nx = K4.cfg\nx[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**CFG4, messages=[])\n",
        "CFG5 = {\"model\": \"gpt-4o\"}\nclass K5:\n    cfg = CFG5\n    def tune(self):\n        self.cfg.update(model=\"o1-mini\")\nclient.chat.completions.create(**CFG5, messages=[])\n",
        "CFG7 = {\"model\": \"gpt-4o\"}\nglobals()[\"CFG7\"][\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**CFG7, messages=[])\n",
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "-", "{body}");
        assert_eq!(rows[0].2, vec!["splat".to_string()], "{body}");
    }
    // Read-only uses of the same shapes still resolve.
    for body in [
        "CFG = {\"model\": \"gpt-4o\"}\nclass K:\n    cfg = CFG\nprint(K.cfg[\"model\"])\nclient.chat.completions.create(**CFG, messages=[])\n",
        "CFG2 = {\"model\": \"gpt-4o\"}\nx = y = CFG2\nprint(x[\"model\"])\nclient.chat.completions.create(**CFG2, messages=[])\n",
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows[0].1, "gpt-4o", "{body}");
    }
}

#[test]
fn match_captures_except_targets_star_imports_and_globals_rebind() {
    // `case str() as model5`, `except ... as model11`, `from mod import *`
    // and `globals()["M"] = ...` bound nothing, so each call reported the
    // earlier `gpt-4o` with no gap.
    let header = "import os\nfrom openai import OpenAI\nclient = OpenAI()\n";
    for (body, reason) in [
        (
            "model5 = \"gpt-4o\"\nmatch os.getenv(\"M\"):\n    case str() as model5:\n        pass\nclient.chat.completions.create(model=model5, messages=[])\n",
            "reassigned",
        ),
        (
            "m = \"gpt-4o\"\nmatch os.getenv(\"M\"):\n    case m:\n        pass\nclient.chat.completions.create(model=m, messages=[])\n",
            "reassigned",
        ),
        (
            "m = \"gpt-4o\"\nmatch cfg:\n    case {\"model\": m, **rest}:\n        pass\nclient.chat.completions.create(model=m, messages=[])\n",
            "reassigned",
        ),
        (
            "m = \"gpt-4o\"\nmatch cfg:\n    case Cfg(model=m) | Other(m):\n        pass\nclient.chat.completions.create(model=m, messages=[])\n",
            "reassigned",
        ),
        (
            "m = \"gpt-4o\"\nmatch cfg:\n    case [first, *m]:\n        pass\nclient.chat.completions.create(model=m, messages=[])\n",
            "reassigned",
        ),
        (
            "model11 = \"gpt-4o\"\ntry:\n    pass\nexcept Exception as model11:\n    pass\nclient.chat.completions.create(model=model11, messages=[])\n",
            "reassigned",
        ),
        (
            "MODEL4 = \"gpt-4o\"\nfrom local_overrides import *\nclient.chat.completions.create(model=MODEL4, messages=[])\n",
            "reassigned",
        ),
        (
            "from .settings import *\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            "imported_name",
        ),
        (
            "M = \"gpt-4o\"\ndef tune():\n    globals()[\"M\"] = \"o1-mini\"\nclient.chat.completions.create(model=M, messages=[])\n",
            "reassigned",
        ),
        (
            "M = \"gpt-4o\"\nglobals().update(overrides)\nclient.chat.completions.create(model=M, messages=[])\n",
            "reassigned",
        ),
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "-", "{body}");
        // A wildcard import may rebind `client` too.
        assert!(
            rows[0].2.contains(&reason.to_string()),
            "{body}: {:?}",
            rows[0].2
        );
    }
    // What a pattern does not bind stays resolved: a value pattern, a class
    // name, a keyword name, `_`; and a definition after a wildcard import
    // dominates it.
    for source in [
        format!("{header}m = \"gpt-4o\"\nmatch cfg:\n    case Color.m:\n        pass\n    case m():\n        pass\n    case P(m=1):\n        pass\n    case _:\n        pass\nclient.chat.completions.create(model=m, messages=[])\n"),
        "from local_overrides import *\nfrom openai import OpenAI\nclient = OpenAI()\nm = \"gpt-4o\"\nclient.chat.completions.create(model=m, messages=[])\n".to_string(),
    ] {
        let rows = rows_of(&source);
        assert_eq!(rows[0].1, "gpt-4o", "{source}");
        assert!(rows[0].2.is_empty(), "{source}: {:?}", rows[0].2);
    }
    // At module level, `globals()["M"] = v` is `M = v`.
    let rows = rows_of(&format!(
        "{header}M = \"gpt-4o\"\nglobals()[\"M\"] = \"o1-mini\"\nclient.chat.completions.create(model=M, messages=[])\n"
    ));
    assert_eq!(rows, vec![(6, "o1-mini".to_string(), vec![])]);
    // A client, or a constructor, the wildcard import may rebind is
    // ambiguous; one imported after it still resolves.
    let found = usages(
        "from openai import OpenAI\nclient = OpenAI()\nfrom helpers import *\nclient.chat.completions.create(model=\"gpt-4\", messages=[])\ndef ask():\n    return client.chat.completions.create(model=\"gpt-4\", messages=[])\nlater = OpenAI()\nlater.chat.completions.create(model=\"gpt-4\", messages=[])\nfrom openai import OpenAI as Direct\nDirect().chat.completions.create(model=\"gpt-4\", messages=[])\n",
    );
    let routes: Vec<(u32, Routing)> = found
        .calls
        .iter()
        .map(|c| (c.span.line, c.routing))
        .collect();
    assert_eq!(
        routes,
        vec![
            (4, Routing::Unknown),
            (6, Routing::Unknown),
            (8, Routing::Unknown),
            (10, Routing::OpenaiDirect)
        ]
    );
}

/// `(line, reason, listed values, complete)` for every `unresolved_model` gap.
fn listed_with_completeness(found: &FileUsages) -> Vec<(u32, String, Vec<String>, bool)> {
    gaps(found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| {
            (
                g.span.unwrap().line,
                g.reason.clone(),
                g.values.iter().map(|v| v.value.clone()).collect(),
                g.values_complete,
            )
        })
        .collect()
}

#[test]
fn listed_values_say_whether_they_are_complete() {
    // A gap's `values` were silently partial: a loop over a literal list,
    // a dict key that is not an identifier, the 17th branch, a tuple
    // unpacking and `+=` were dropped, so a consumer reading the list as
    // complete missed `o1-mini` or `gpt-4-0613`.
    let mut branches = String::from("k = os.getenv(\"K\")\nif k == \"0\": M3 = \"gpt-4o-0\"\n");
    for i in 1..17 {
        branches.push_str(&format!("elif k == \"{i}\": M3 = \"gpt-4o-{i}\"\n"));
    }
    branches.push_str("else: M3 = \"o1-mini\"\n");
    let source = format!(
        r#"import os
from openai import OpenAI
client = OpenAI()
model = "gpt-4o"
for model in ["gpt-4-0613"]:
    client.chat.completions.create(model=model, messages=[])
TIERS = {{"fast-tier": "o1-mini", "default": "gpt-4o"}}
client.chat.completions.create(model=TIERS[os.getenv("TIER", "default")], messages=[])
M7 = "gpt-4o"
if os.getenv("Y"):
    M7, temp = "o1-mini", 0
client.chat.completions.create(model=M7, messages=[])
M8 = "gpt-4"
if os.getenv("Z"):
    M8 += "-0613"
client.chat.completions.create(model=M8, messages=[])
if os.getenv("X"):
    M4 = "gpt-4o"
else:
    M4 = os.environ["LEGACY_MODEL"]
client.chat.completions.create(model=M4, messages=[])
if os.getenv("F"):
    M9 = "gpt-4o"
else:
    M9 = "o1-mini"
client.chat.completions.create(model=M9, messages=[])
client.chat.completions.create(model="gpt-4o" if os.getenv("F") else "o3-mini", messages=[])
def _private(q, model):
    return client.chat.completions.create(model=model, messages=[q])
_private("a", "gpt-4o")
_private("b", "o1-mini")
def public(q, model):
    return client.chat.completions.create(model=model, messages=[q])
public("a", "gpt-4o")
def _mixed(q, model):
    return client.chat.completions.create(model=model, messages=[q])
_mixed("a", "gpt-4o")
_mixed("b", load_model())
def _over(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])
_over("a", model="o1-mini")
for m in ("gpt-4o", "o1-mini"):
    client.chat.completions.create(model=m, messages=[])
{branches}client.chat.completions.create(model=M3, messages=[])
"#
    );
    let found = usages(&source);
    let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut eighteen = strings(&["gpt-4o-0"]);
    eighteen.extend((1..17).map(|i| format!("gpt-4o-{i}")));
    eighteen.push("o1-mini".to_string());
    assert_eq!(
        listed_with_completeness(&found),
        vec![
            // The loop item is listed; the outer value too (a superset).
            (
                6,
                "reassigned".into(),
                strings(&["gpt-4o", "gpt-4-0613"]),
                true
            ),
            // A key that is not an identifier still lists its value.
            (8, "env_var".into(), strings(&["o1-mini", "gpt-4o"]), true),
            // Tuple unpacking and `+=` are not read: partial.
            (12, "reassigned".into(), strings(&["gpt-4o"]), false),
            (16, "reassigned".into(), strings(&["gpt-4"]), false),
            // One branch reads the environment: partial.
            (21, "reassigned".into(), strings(&["gpt-4o"]), false),
            (
                26,
                "reassigned".into(),
                strings(&["gpt-4o", "o1-mini"]),
                true
            ),
            // The condition reads the environment; both sides are listed.
            (27, "env_var".into(), strings(&["gpt-4o", "o3-mini"]), true),
            // Every caller of a private function is here.
            (
                29,
                "parameter".into(),
                strings(&["gpt-4o", "o1-mini"]),
                true
            ),
            // Other modules may call a public one.
            (33, "parameter".into(), strings(&["gpt-4o"]), false),
            (36, "parameter".into(), strings(&["gpt-4o"]), false),
            (
                40,
                "parameter_overridden".into(),
                strings(&["o1-mini"]),
                true
            ),
            (
                43,
                "non_literal".into(),
                strings(&["gpt-4o", "o1-mini"]),
                true
            ),
            (63, "reassigned".into(), eighteen, true),
        ]
    );
    // Past 32 values the list is cut and says so.
    let mut many = String::from("import os\nfrom openai import OpenAI\nclient = OpenAI()\nk = os.getenv(\"K\")\nif k == \"0\": M = \"m-0\"\n");
    for i in 1..40 {
        many.push_str(&format!("elif k == \"{i}\": M = \"m-{i}\"\n"));
    }
    many.push_str("client.chat.completions.create(model=M, messages=[])\n");
    let found = usages(&many);
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.values.len(), 32);
    assert!(!gap.values_complete);
    assert!(gap.detail.contains("not necessarily all"), "{}", gap.detail);
    let json = serde_json::to_string(gap).unwrap();
    assert!(!json.contains("values_complete"), "{json}");
}

#[test]
fn a_repeated_keyword_is_read_once() {
    // tree-sitter accepts `f(model=A, model=A, ...)` (Python does not):
    // each copy was resolved again, which made one 4 MiB call take 5.8 s.
    let mut source = String::from(
        "import os\nfrom openai import OpenAI\nclient = OpenAI()\nA = os.getenv(\"MODEL\", \"gpt-4o\")\nclient.chat.completions.create(",
    );
    source.push_str(&"model=A, ".repeat(30_000));
    source.push_str("messages=[])\n");
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 1);
    assert_eq!(found.calls[0].values.len(), 1);
    assert_eq!(found.gaps.len(), 1);
}
