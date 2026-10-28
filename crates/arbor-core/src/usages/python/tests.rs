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
    // The local Bedrock client, not the module's Anthropic one.
    assert_eq!(found.calls.len(), 1, "{:?}", found.calls);
    let converse = call(&found, "client.converse");
    assert_eq!(converse.sdk, Sdk::Aws);
    assert_eq!(converse.routing, Routing::Bedrock);
    assert_eq!(converse.client_line, Some(8));
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
    // Policy round (P1): other modules may call a public function with
    // another model, so its default keeps a `parameter_default` gap; and
    // (policy round 2, lead decision 8) they may subclass the public class
    // `Chat` and set `model`, so its reads through `self` keep a
    // `subclass_override` gap. `Chat.a` and `Chat.d` read module constants.
    let reasons: Vec<(Option<&str>, &str)> = found
        .gaps
        .iter()
        .map(|g| (g.symbol.as_deref(), g.reason.as_str()))
        .collect();
    assert_eq!(
        reasons,
        vec![
            (Some("Chat.b"), "subclass_override"),
            (Some("Chat.c"), "subclass_override"),
            (Some("param"), "parameter_default")
        ]
    );
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
    assert_eq!(found.clients[0].routing, Routing::GoogleGenai);
    // `vertexai=True` sends it to Vertex AI: `unknown` with an
    // `endpoint_option` gap (policy round 2, should-fix 5; it was
    // `google_genai` with no gap).
    assert_eq!(found.clients[1].routing, Routing::Unknown);
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
    let vertex = call(&found, "other.aio.models.generate_content");
    assert_eq!(value(vertex, "model"), Some("gemini-1.5-pro"));
    assert_eq!(vertex.routing, Routing::Unknown);
    assert!(found.gaps.iter().any(|g| g.span.map(|s| s.start_byte)
        == Some(vertex.span.start_byte)
        && g.reason == "endpoint_option"));
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
        "from openai import OpenAI\nURL = \"https://gw.example/v1\"\nclient: OpenAI = OpenAI(base_url=URL)\nclient.chat.completions.create(model=\"gpt-4\", messages=[])\n",
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

def _ask(q, model: str = GPT_MODEL):
    return client.chat.completions.create(model=model, messages=[q])

def ask(q, model: str = GPT_MODEL):
    return client.chat.completions.create(model=model, messages=[q])
"#,
    );
    for c in &found.calls {
        let model = &c.values[0];
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
    }
    // Policy round (P1): the private function's default is the supported
    // grammar; the public one's keeps a `parameter_default` gap, since
    // other modules may pass another model.
    let reasons: Vec<(Option<&str>, &str)> = found
        .gaps
        .iter()
        .map(|g| (g.symbol.as_deref(), g.reason.as_str()))
        .collect();
    assert_eq!(reasons, vec![(Some("ask"), "parameter_default")]);
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
print(json.dumps(CONFIG), len(CONFIG), "model" in CONFIG)
for key in CONFIG:
    print(key, CONFIG.get(key))
for key, value in {"a": "b"}.items():
    pass
client.chat.completions.create(model=MODEL, messages=[])
client.chat.completions.create(**CONFIG["openai"], messages=[])
client.chat.completions.create(model=CONFIG.get("model"), messages=[])
"#,
    );
    let models: Vec<&str> = found
        .calls
        .iter()
        .map(|c| value(c, "model").unwrap_or("-"))
        .collect();
    assert_eq!(models, vec!["gpt-4o", "o3", "gpt-4o"]);
    // Policy round (P2): a nested dict handed to another name may change
    // through it, mutated or not, so the dict is no longer read.
    let found = usages(
        r#"
from openai import OpenAI
client = OpenAI()
CONFIG = {"model": "gpt-4o", "openai": {"model": "o3"}}
OPENAI = CONFIG["openai"]
client.chat.completions.create(**CONFIG, messages=[])
client.chat.completions.create(**OPENAI, messages=[])
"#,
    );
    assert_eq!(
        values_and_gaps(&found),
        vec![
            (6, "-".to_string(), vec!["splat".to_string()]),
            (7, "-".to_string(), vec!["splat".to_string()]),
        ]
    );
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

/// `(line, routing, gap kinds and reasons at that line)` for one call.
type RouteRow = (u32, Routing, Vec<String>);

/// A [`RouteRow`] for every call.
fn routes_and_gaps(found: &FileUsages) -> Vec<RouteRow> {
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
            // Callers pass the default or nothing, but the function is
            // public: other modules may pass another model (policy round,
            // P1).
            (
                10,
                "gpt-3.5-turbo".to_string(),
                vec![("parameter_default".to_string(), vec![])]
            ),
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
            // Public, called here only without the argument: other modules
            // may pass another model (policy round, P1).
            (
                32,
                "gpt-4o".to_string(),
                vec!["parameter_default".to_string()]
            ),
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
    def _quiet(self, q, model="gpt-4o"):
        return client.chat.completions.create(model=model, messages=[q])
bot = Bot()
bot.ask("hi", model="o1-mini")
bot.plain("hi")
bot._quiet("hi")
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
            // Public: other modules may pass another model (policy round,
            // P1); the private method's calls are all here.
            (
                7,
                "gpt-4o".to_string(),
                vec!["parameter_default".to_string()]
            ),
            (9, "gpt-4o".to_string(), vec![]),
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
class _Hidden:
    def __init__(self, model="gpt-4o"):
        self.model = model
    def run(self):
        return client.chat.completions.create(model=self.model, messages=[])
_Hidden()
"#,
    );
    assert_eq!(
        values_and_gaps(&found),
        vec![
            // A public class's `self.model` also keeps a `subclass_override`
            // gap (policy round 2, lead decision 8).
            (
                7,
                "gpt-4o".to_string(),
                vec![
                    "parameter_overridden".to_string(),
                    "subclass_override".to_string()
                ]
            ),
            (
                13,
                "gpt-4".to_string(),
                vec!["multiple_assignments".to_string()]
            ),
            // Built here without the argument (an annotation is not a use),
            // but other modules may build a public class with another model
            // (policy round, P1); a private class's constructor calls are
            // all here.
            (
                19,
                "gpt-4o".to_string(),
                vec![
                    "parameter_default".to_string(),
                    "subclass_override".to_string()
                ]
            ),
            (25, "gpt-4o".to_string(), vec![]),
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
    // A direct subclass that overrides it is one more assignment.
    let rows = rows_of(&format!(
        "{header}{}",
        r#"class Base:
    MODEL = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.MODEL, messages=[])
class Fast(Base):
    MODEL = "o1-mini"
Fast().go()
"#
    ));
    assert_eq!(rows, vec![(6, "-".to_string(), reassigned())]);
    // Policy round (P2): a write of the attribute on any other object, and
    // an attribute only subclasses assign, are dynamic features. The
    // value the class gives is kept with a `dynamic_feature` gap; the
    // writes are no longer copied into every class's lookup.
    let dynamic = || vec!["dynamic_feature".to_string()];
    for (body, line, value) in [
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
            "gpt-4o",
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
            "gpt-4o",
        ),
        (
            r#"class Lazy:
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
class Real(Lazy):
    model = "gpt-4o"
"#,
            5,
            "gpt-4o",
        ),
    ] {
        let found = usages(&format!("{header}{body}"));
        let rows = values_and_gaps(&found);
        assert_eq!(rows, vec![(line, value.to_string(), dynamic())], "{body}");
    }
    // An attribute named at run time may be any of them: `setattr(self,
    // ...)` in the class, and `__dict__` or `vars()` anywhere in the file,
    // which may rebind anything (so the route keeps a gap too).
    for (body, reasons) in [
        (
            r#"class Cfg:
    model = "gpt-4o"
    def __init__(self, **kw):
        for k, v in kw.items():
            setattr(self, k, v)
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
"#,
            dynamic(),
        ),
        (
            r#"class Cfg:
    model = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
c = Cfg()
c.__dict__.update(opts)
"#,
            vec!["dynamic_feature".to_string(), "dynamic_feature".to_string()],
        ),
        (
            r#"class Cfg:
    model = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
c = Cfg()
vars(c)[key] = value
"#,
            vec!["dynamic_feature".to_string(), "dynamic_feature".to_string()],
        ),
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "gpt-4o", "{body}");
        assert_eq!(rows[0].2, reasons, "{body}");
    }
    // Policy round (P2): `k` is not known to be an instance, so a write of
    // `MODEL` on it may be a write of the class's: a read by class name
    // keeps a gap too.
    let rows = rows_of(&format!(
        "{header}class K:\n    MODEL = \"gpt-4o\"\nk = K()\nk.MODEL = \"o1-mini\"\nclient.chat.completions.create(model=K.MODEL, messages=[])\n"
    ));
    assert_eq!(rows, vec![(7, "gpt-4o".to_string(), dynamic())]);
    // With no other write, a read by class name is the supported grammar.
    let rows = rows_of(&format!(
        "{header}class K:\n    MODEL = \"gpt-4o\"\nclient.chat.completions.create(model=K.MODEL, messages=[])\n"
    ));
    assert_eq!(rows, vec![(5, "gpt-4o".to_string(), vec![])]);
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
        // Policy round (P2): read only, but aliased: the supported grammar
        // reads a dict only through its one name.
        "CFG = {\"model\": \"gpt-4o\"}\nclass K:\n    cfg = CFG\nprint(K.cfg[\"model\"])\nclient.chat.completions.create(**CFG, messages=[])\n",
        "CFG2 = {\"model\": \"gpt-4o\"}\nx = y = CFG2\nprint(x[\"model\"])\nclient.chat.completions.create(**CFG2, messages=[])\n",
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "-", "{body}");
        assert_eq!(rows[0].2, vec!["splat".to_string()], "{body}");
    }
    // `globals()` may rebind anything in the file (policy round, P2): the
    // route keeps a `dynamic_feature` gap too.
    let rows = rows_of(&format!(
        "{header}CFG7 = {{\"model\": \"gpt-4o\"}}\nglobals()[\"CFG7\"][\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**CFG7, messages=[])\n"
    ));
    assert_eq!(
        rows,
        vec![(
            5,
            "-".to_string(),
            vec!["splat".to_string(), "dynamic_feature".to_string()]
        )]
    );
    // Read without an alias, the dict resolves.
    let rows = rows_of(&format!(
        "{header}CFG = {{\"model\": \"gpt-4o\"}}\nprint(CFG[\"model\"], len(CFG))\nclient.chat.completions.create(**CFG, messages=[])\n"
    ));
    assert_eq!(rows, vec![(5, "gpt-4o".to_string(), vec![])]);
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
    // At module level, `globals()["M"] = v` is `M = v`; `globals()` may
    // rebind anything else too (policy round, P2), so the value and the
    // route keep a `dynamic_feature` gap.
    let rows = rows_of(&format!(
        "{header}M = \"gpt-4o\"\nglobals()[\"M\"] = \"o1-mini\"\nclient.chat.completions.create(model=M, messages=[])\n"
    ));
    let dynamic = || "dynamic_feature".to_string();
    assert_eq!(
        rows,
        vec![(6, "o1-mini".to_string(), vec![dynamic(), dynamic()])]
    );
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

#[test]
fn callers_past_the_read_limit_are_unseen() {
    // A private function's client default with more calls than are read:
    // the one passing the gateway could be past the limit, so the default
    // must keep a gap.
    let mut source = String::from(
        "from openai import OpenAI\nclient = OpenAI()\ngw = OpenAI(base_url=\"https://gw.example/v1\")\ndef _ask(q, c=client):\n    return c.chat.completions.create(model=\"gpt-4\", messages=[q])\n",
    );
    source.push_str(&"_ask(\"x\")\n".repeat(300));
    source.push_str("_ask(\"y\", c=gw)\n");
    let found = usages_within(&source, 10);
    assert_eq!(
        routes_and_gaps(&found),
        vec![(
            5,
            Routing::OpenaiDirect,
            vec!["ambiguous_client/unseen_callers".to_string()]
        )]
    );
}

#[test]
fn a_parameter_default_never_completes_a_list() {
    // Another module may pass the parameter, so a candidate that is a
    // parameter default leaves the list partial.
    let found = usages(
        r#"from openai import OpenAI
client = OpenAI()
class Bot:
    def __init__(self, model="gpt-4o"):
        self.model = model
    def fast(self):
        self.model = "o1-mini"
    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])
"#,
    );
    assert_eq!(
        listed_with_completeness(&found),
        vec![(
            9,
            "reassigned".to_string(),
            vec!["gpt-4o".to_string(), "o1-mini".to_string()],
            false
        )]
    );
}

#[test]
fn writes_and_lookups_through_dunders_and_the_module_object_are_seen() {
    // `object.__setattr__(self, ...)`, `super().__setattr__(...)` and
    // `sys.modules[__name__].M = ...` wrote what a call reads without
    // being counted; `getattr(bot, "ask")(...)` called a method with an
    // argument no caller showed.
    let header = "import sys\nfrom openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "class A:\n    model = \"gpt-4o\"\n    def fast(self):\n        object.__setattr__(self, \"model\", \"o1-mini\")\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
        "class A:\n    model = \"gpt-4o\"\n    def fast(self):\n        super().__setattr__(\"model\", \"o1-mini\")\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "-", "{body}");
        assert_eq!(rows[0].2, vec!["reassigned".to_string()], "{body}");
    }
    // Policy round (P2): a write on another object is a dynamic feature
    // (the class's value is kept with the gap), and `sys.modules` may
    // rebind anything in the file, so the route keeps a gap too.
    let dynamic = || "dynamic_feature".to_string();
    for (body, value, reasons) in [
        (
            "class A:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\na = A()\na.__setattr__(\"model\", \"o1-mini\")\n",
            "gpt-4o",
            vec![dynamic()],
        ),
        (
            "M = \"gpt-4o\"\ndef tune():\n    sys.modules[__name__].M = \"o1-mini\"\nclient.chat.completions.create(model=M, messages=[])\n",
            "-",
            vec!["reassigned".to_string(), dynamic()],
        ),
        (
            "M = \"gpt-4o\"\ndef tune():\n    setattr(sys.modules[__name__], \"M\", \"o1-mini\")\nclient.chat.completions.create(model=M, messages=[])\n",
            "-",
            vec!["reassigned".to_string(), dynamic()],
        ),
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows, vec![(rows[0].0, value.to_string(), reasons)], "{body}");
    }
    for (body, reasons) in [
        (
            "class Bot:\n    def ask(self, q, model=\"gpt-4o\"):\n        return client.chat.completions.create(model=model, messages=[q])\nbot = Bot()\ngetattr(bot, \"ask\")(\"hi\", model=\"o1-mini\")\n",
            vec!["unseen_callers".to_string()],
        ),
        (
            "class Bot:\n    def ask(self, q, model=\"gpt-4o\"):\n        return client.chat.completions.create(model=model, messages=[q])\nbot = Bot()\ngetattr(bot, action)(\"hi\", model=\"o1-mini\")\n",
            vec!["unseen_callers".to_string()],
        ),
        // `globals()` anywhere: the default and the route keep a
        // `dynamic_feature` gap too.
        (
            "def ask(q, model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[q])\nglobals()[\"ask\"](\"hi\", model=\"o1-mini\")\n",
            vec!["unseen_callers".to_string(), dynamic(), dynamic()],
        ),
        (
            "def ask(q, model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[q])\nglobals()[name](\"hi\", model=\"o1-mini\")\n",
            vec!["unseen_callers".to_string(), dynamic(), dynamic()],
        ),
    ] {
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "gpt-4o", "{body}");
        assert_eq!(rows[0].2, reasons, "{body}");
    }
}

#[test]
fn annotated_fields_of_data_classes_are_constructor_defaults() {
    // `model: str = "gpt-4o"` in a dataclass, a pydantic model or a
    // NamedTuple was read through `self` as a class constant, though
    // `Agent(model="o1-mini")` sets the instance's field.
    let header = "from dataclasses import dataclass\nfrom typing import NamedTuple\nfrom pydantic import BaseModel\nfrom openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "@dataclass\nclass Agent:\n    model: str = \"gpt-4o\"\n    def run(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nAgent(model=\"o1-mini\").run()\n",
        "class Agent(BaseModel):\n    model: str = \"gpt-4o\"\n    def run(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
        "class Agent(NamedTuple):\n    model: str = \"gpt-4o\"\n    def run(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
    ] {
        // Policy round (P2): a class decorator, or a base from outside the
        // file, is a dynamic feature; the field's default is kept with
        // the gap.
        let rows = rows_of(&format!("{header}{body}"));
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "gpt-4o", "{body}");
        assert_eq!(rows[0].2, vec!["dynamic_feature".to_string()], "{body}");
    }
    // A plain class's annotated attribute is a class attribute: confirmed in
    // a private class; a public one keeps a `subclass_override` gap (policy
    // round 2, lead decision 8).
    let rows = rows_of(&format!(
        "{header}class _Plain:\n    model: str = \"gpt-4o\"\n    def run(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
    ));
    assert_eq!(rows, vec![(9, "gpt-4o".to_string(), vec![])]);
    let rows = rows_of(&format!(
        "{header}class Plain:\n    model: str = \"gpt-4o\"\n    def run(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
    ));
    assert_eq!(
        rows,
        vec![(
            9,
            "gpt-4o".to_string(),
            vec!["subclass_override".to_string()]
        )]
    );
}

// ---------------------------------------------------------------------------
// Policy round (re-review of d6132ac): a supported grammar, and dynamic
// features as gaps
// ---------------------------------------------------------------------------

/// Every call of `source` with its model value (or `-`) and gap reasons;
/// fails when a call reports a value with no gap at all, unless its line
/// is in `confirmed`.
fn rows_gapped(source: &str, confirmed: &[u32]) -> Vec<(u32, String, Vec<String>)> {
    let found = usages(source);
    let rows = values_and_gaps(&found);
    for (line, value, reasons) in &rows {
        if !confirmed.contains(line) {
            assert!(
                !reasons.is_empty(),
                "line {line} reports `{value}` with no gap:\n{source}"
            );
        }
    }
    rows
}

#[test]
fn self_attribute_reads_stay_linear_however_many_classes_and_outside_writes() {
    // Finding 1: every outside write of `.model` was copied into each
    // class's lookup, so N classes x M writes took 150 bytes a pair (1.28
    // MB: 11 GB). Now any outside write of the name is one dynamic
    // feature, read in constant time per class.
    let mut source = String::from("from openai import OpenAI\nclient = OpenAI()\n");
    for i in 0..2_000 {
        source.push_str(&format!(
            "class C{i}:\n    model = 'gpt-4o'\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
        ));
    }
    for i in 0..4_000 {
        source.push_str(&format!("o{}.model = 'o1-mini-{i}'\n", i % 7));
    }
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 2_000);
    // The class's value is kept, with the gap naming the first write.
    assert!(found.gaps.iter().all(|g| g.reason == "dynamic_feature"));
    assert_eq!(found.gaps.len(), 2_000);
    assert!(found.gaps[0]
        .detail
        .contains("a write of the attribute on another object at line 8003"));
}

#[test]
fn candidate_lists_stay_linear_however_many_definitions_and_uses() {
    // Finding 2: a use whose every definition reaches it cloned all of
    // them, and loop items were re-read for each use and each reaching
    // loop: a 73 KB file took 30 s, a 1.48 MB one never finished.
    let head = "import os\nfrom openai import OpenAI\nclient = OpenAI()\n";
    let names: Vec<String> = (0..5_000).map(|i| format!("\"gpt-{i}\"")).collect();
    let mut long_list = format!("{head}L = [{}]\n", names.join(", "));
    for _ in 0..200 {
        long_list
            .push_str("for m in L:\n    client.chat.completions.create(model=m, messages=[])\n");
    }
    let mut branches = String::from(head);
    for i in 0..20_000 {
        branches.push_str(&format!("if os.getenv(\"C{i}\"):\n    M = \"gpt-{i}\"\n"));
    }
    for _ in 0..3_000 {
        branches.push_str("client.chat.completions.create(model=M, messages=[])\n");
    }
    let mut cases = format!("{head}M = 'gpt-4o'\nmatch os.getenv('X'):\n");
    for i in 0..8_000 {
        cases.push_str(&format!(
            "    case 'k{i}' as M:\n        client.chat.completions.create(model=M, messages=[])\n"
        ));
    }
    let mut excepts = format!("{head}M = 'gpt-4o'\n");
    for i in 0..7_000 {
        excepts.push_str(&format!(
            "try:\n    pass\nexcept E{i} as M:\n    pass\nclient.chat.completions.create(model=M, messages=[])\n"
        ));
    }
    for (source, calls) in [
        (long_list, 200),
        (branches, 3_000),
        (cases, 8_000),
        (excepts, 7_000),
    ] {
        let found = usages_within(&source, 10);
        assert_eq!(found.calls.len(), calls);
        // Each use is a gap; a list cut at the read limit is not complete.
        assert!(found.calls.iter().all(|c| c.values.is_empty()));
        assert!(gaps(&found, GapKind::UnresolvedModel)
            .iter()
            .all(|g| !g.values_complete));
    }
}

#[test]
fn class_attributes_read_by_name_see_every_dynamic_write() {
    // Finding 3: `Settings.MODEL` read only the class body, so a
    // `setattr` loader, a helper given the class, a decorator, an alias, a
    // loop over classes or a write in a function defined above the class
    // left `gpt-4o` with no gap.
    let header = "import os\nfrom openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "class Settings:\n    MODEL = \"gpt-4o\"\ndef load(overrides):\n    for k, v in overrides.items():\n        setattr(Settings, k, v)\nload({\"MODEL\": \"o1-mini\"})\nclient.chat.completions.create(model=Settings.MODEL, messages=[])\n",
        "class Settings:\n    MODEL = \"gpt-4o\"\nfor key in (\"MODEL\",):\n    if key in os.environ:\n        setattr(Settings, key, os.environ[key])\nclient.chat.completions.create(model=Settings.MODEL, messages=[])\n",
        "class Settings:\n    MODEL = \"gpt-4o\"\ndef configure(cfg):\n    cfg.MODEL = \"o1-mini\"\nconfigure(Settings)\nclient.chat.completions.create(model=Settings.MODEL, messages=[])\n",
        "def use_mini(cls):\n    cls.MODEL = \"gpt-4o-mini\"\n    return cls\n@use_mini\nclass Settings:\n    MODEL = \"gpt-4o\"\nclient.chat.completions.create(model=Settings.MODEL, messages=[])\n",
        "class Bot4:\n    model = \"gpt-4o\"\nAlias4 = Bot4\nAlias4.model = \"o1-mini\"\nclient.chat.completions.create(model=Bot4.model, messages=[])\n",
        "class Bot5:\n    model = \"gpt-4o\"\nfor k5 in (Bot5,):\n    k5.model = \"o1-mini\"\nclient.chat.completions.create(model=Bot5.model, messages=[])\n",
        "def tweak1():\n    Bot1.model = \"o1-mini\"\nclass Bot1:\n    model = \"gpt-4o\"\ntweak1()\nclient.chat.completions.create(model=Bot1.model, messages=[])\n",
        "class Bot9:\n    model = \"gpt-4o\"\ntype.__setattr__(Bot9, \"model\", \"o1-mini\")\nclient.chat.completions.create(model=Bot9.model, messages=[])\n",
        "class Bot:\n    model = \"gpt-4o\"\nBot = make(Bot)\nclient.chat.completions.create(model=Bot.model, messages=[])\n",
    ] {
        rows_gapped(&format!("{header}{body}"), &[]);
    }
    // The same for a client attribute: the route keeps a gap.
    let found = usages(
        r#"from openai import OpenAI
GW = "https://openrouter.ai/api/v1"
class Svc1:
    client = OpenAI()
def configure1(k):
    k.client = OpenAI(base_url=GW)
configure1(Svc1)
Svc1.client.chat.completions.create(model="gpt-4o", messages=[])
def gateway(cls):
    cls.client = OpenAI(base_url=GW)
    return cls
@gateway
class Svc2:
    client = OpenAI()
Svc2.client.chat.completions.create(model="gpt-4o", messages=[])
def reroute4():
    Svc4.client = OpenAI(base_url=GW)
class Svc4:
    client = OpenAI()
reroute4()
Svc4.client.chat.completions.create(model="gpt-4o", messages=[])
"#,
    );
    let routes = routes_and_gaps(&found);
    assert_eq!(routes.len(), 3);
    for (line, _, reasons) in routes {
        assert!(!reasons.is_empty(), "line {line}");
    }
    // Inside the grammar a class constant read by name resolves.
    let rows = rows_gapped(
        &format!("{header}class K:\n    MODEL = \"gpt-4o\"\nclient.chat.completions.create(model=K.MODEL, messages=[])\n"),
        &[6],
    );
    assert_eq!(rows, vec![(6, "gpt-4o".to_string(), vec![])]);
}

#[test]
fn multiple_or_deep_inheritance_is_a_dynamic_feature() {
    // Finding 4: `class C(A, B)` was read depth first, so `Root.model`
    // (`gpt-4o`) was reported where Python's MRO reads `B.model`
    // (`o1-mini`); and finding 8: past 64 subclasses, or 64 levels, the
    // walk stopped silently.
    let header = "from openai import OpenAI\nclient = OpenAI()\nGW = \"https://gw.example/v1\"\n";
    let diamond = "class Root:\n    model = \"gpt-4o\"\n    client = OpenAI()\nclass A(Root):\n    pass\nclass B(Root):\n    model = \"o1-mini\"\n    client = OpenAI(base_url=GW)\n";
    for tail in [
        "class C(A, B):\n    def ask(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nC().ask()\n",
        "class C(A, B):\n    pass\nclient.chat.completions.create(model=C.model, messages=[])\n",
    ] {
        let rows = rows_gapped(&format!("{header}{diamond}{tail}"), &[]);
        assert!(rows.iter().all(|r| r.1 != "gpt-4o"), "{rows:?}");
    }
    let found = usages(&format!(
        "{header}{diamond}class C(A, B):\n    pass\nC.client.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
    ));
    let receiver = &gaps(&found, GapKind::UnknownClientFactory)[0];
    assert_eq!(receiver.reason, "dynamic_feature");
    assert!(receiver.detail.contains("multiple inheritance"));

    // 70 empty subclasses and a 71st that overrides; a 70-deep chain.
    let mut wide = format!(
        "{header}class Base:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
    );
    for i in 0..70 {
        wide.push_str(&format!("class S{i}(Base):\n    pass\n"));
    }
    wide.push_str("class Last(Base):\n    model = \"o1-mini\"\nLast().go()\n");
    let mut deep = format!(
        "{header}class B0:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
    );
    for i in 1..70 {
        deep.push_str(&format!("class B{i}(B{}):\n    pass\n", i - 1));
    }
    deep.push_str("class B70(B69):\n    model = \"o1-mini\"\nB70().go()\n");
    for source in [wide, deep] {
        let rows = rows_gapped(&source, &[]);
        assert_eq!(rows.len(), 1);
    }
    // One level of single inheritance is the supported grammar. The classes
    // are private: a public one's `self.model` keeps a `subclass_override`
    // gap, since other modules may subclass it (policy round 2, lead
    // decision 8).
    let rows = rows_gapped(
        &format!("{header}class _Base:\n    model = \"gpt-4o\"\nclass _Child(_Base):\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"),
        &[8],
    );
    assert_eq!(rows, vec![(8, "gpt-4o".to_string(), vec![])]);
    let rows = rows_gapped(
        &format!("{header}class Base:\n    model = \"gpt-4o\"\nclass Child(Base):\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"),
        &[8],
    );
    assert_eq!(
        rows,
        vec![(
            8,
            "gpt-4o".to_string(),
            vec!["subclass_override".to_string()]
        )]
    );
}

#[test]
fn fields_inherited_through_a_same_file_base_are_dynamic() {
    // Finding 5: the data-class check stopped at a same-file base, so a
    // shared pydantic base (`AppModel(BaseModel)`) or a decorated base
    // left the field default `gpt-4o` with no gap.
    let header = "from dataclasses import dataclass\nfrom pydantic import BaseModel\nfrom openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "class AppModel(BaseModel):\n    pass\nclass AgentConfig(AppModel):\n    model: str = \"gpt-4o\"\n    def ask(self, q):\n        return client.chat.completions.create(model=self.model, messages=[q])\nAgentConfig(model=\"o1-mini\").ask(\"hi\")\n",
        "@dataclass\nclass Base:\n    model: str = \"gpt-4o\"\nclass Agent(Base):\n    def ask(self, q):\n        return client.chat.completions.create(model=self.model, messages=[q])\nAgent(model=\"o1-mini\").ask(\"hi\")\n",
        "class Meta(type):\n    pass\nclass Base(metaclass=Meta):\n    pass\nclass Agent(Base):\n    model: str = \"gpt-4o\"\n    def ask(self, q):\n        return client.chat.completions.create(model=self.model, messages=[q])\n",
        "import enum\nclass M14(str, enum.Enum):\n    FAST = \"gpt-4o-mini\"\nclient.chat.completions.create(model=M14.FAST, messages=[])\n",
    ] {
        rows_gapped(&format!("{header}{body}"), &[]);
    }
    let found = usages(&format!(
        "{header}class AppModel(BaseModel):\n    pass\nclass Svc(AppModel):\n    client: OpenAI = OpenAI()\n    def ask(self, q):\n        return self.client.chat.completions.create(model=\"gpt-4o\", messages=[q])\nSvc(client=OpenAI(base_url=\"https://gw.example/v1\")).ask(\"x\")\n"
    ));
    assert_eq!(
        routes_and_gaps(&found),
        vec![(
            10,
            Routing::OpenaiDirect,
            vec!["ambiguous_client/dynamic_feature".to_string()]
        )]
    );
}

#[test]
fn dicts_and_lists_aliased_or_mutated_through_any_name_are_not_read() {
    // Finding 6: `params = DEFAULTS; params |= {...}`, a nested list
    // extended with `+=` through a name, an alias assigned in one branch
    // and a chain of more than 8 aliases left the literal's value with no
    // gap. Now any alias of a whole container, and any name given a
    // nested container, makes the dict a gap.
    let header = "import os\nfrom openai import OpenAI\nclient = OpenAI()\n";
    let mut chain = String::from("D = {\"model\": \"gpt-4o\"}\na0 = D\n");
    for i in 1..70 {
        chain.push_str(&format!("a{i} = a{}\n", i - 1));
    }
    chain.push_str(
        "a69[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**D, messages=[])\n",
    );
    for body in [
        "DEFAULTS = {\"model\": \"gpt-4o\", \"temperature\": 0}\nparams = DEFAULTS\nparams |= {\"model\": \"o1-mini\"}\nclient.chat.completions.create(**DEFAULTS, messages=[])\n".to_string(),
        "P = {\"model\": \"gpt-4o\", \"extra_body\": {\"fallbacks\": [{\"model\": \"gpt-4o-mini\"}]}}\nfb = P[\"extra_body\"][\"fallbacks\"]\nfb += [{\"model\": \"o1-mini\"}]\nclient.chat.completions.create(**P, messages=[])\n".to_string(),
        "D = {\"model\": \"gpt-4o\"}\nif os.getenv(\"X\"):\n    cfg = D\nelse:\n    cfg = {\"model\": \"gpt-4o-mini\"}\nparams = cfg\nparams[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**D, messages=[])\n".to_string(),
        "D = {\"model\": \"gpt-4o\"}\nQ = D if os.getenv(\"X\") else {}\nQ[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**D, messages=[])\n".to_string(),
        "D = {\"model\": \"gpt-4o\"}\nfor d in (D,):\n    d[\"model\"] = \"o1-mini\"\nclient.chat.completions.create(**D, messages=[])\n".to_string(),
        chain,
    ] {
        let rows = rows_gapped(&format!("{header}{body}"), &[]);
        assert!(rows.iter().all(|r| r.1 == "-"), "{body}: {rows:?}");
    }
    // A copy is not an alias.
    let rows = rows_gapped(
        &format!("{header}P18 = {{\"model\": \"gpt-4o\"}}\nns = dict(**P18)\nclient.chat.completions.create(**P18, messages=[])\n"),
        &[6],
    );
    assert_eq!(rows, vec![(6, "gpt-4o".to_string(), vec![])]);
}

#[test]
fn instances_built_without_their_class_name_are_unseen_constructor_calls() {
    // Finding 7: `type(self)(model=...)`, `self.__class__(**...)` and
    // `cls(...)` were not counted as calls of `__init__`, so its default
    // was reported with no gap.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    for clone in [
        "return type(self)(model=\"o1-mini\")",
        "return self.__class__(**{\"model\": self.model, **overrides})",
        "klass = self.__class__\n        return klass(model=\"o1-mini\")",
    ] {
        let source = format!(
            "{header}class _Agent:\n    def __init__(self, model=\"gpt-4o\"):\n        self.model = model\n    def other(self, **overrides):\n        {clone}\n    def run(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n_Agent().other().run()\n"
        );
        let rows = rows_gapped(&source, &[]);
        assert_eq!(rows.len(), 1, "{clone}");
        assert_eq!(rows[0].1, "gpt-4o", "{clone}");
        assert_eq!(rows[0].2, vec!["unseen_callers".to_string()], "{clone}");
    }
    let found = usages(&format!(
        "{header}class _Svc:\n    def __init__(self, client=OpenAI()):\n        self.client = client\n    def clone_with(self, client):\n        return type(self)(client=client)\n    def ask(self, q):\n        return self.client.chat.completions.create(model=\"gpt-4o\", messages=[q])\n_Svc().clone_with(OpenAI(base_url=\"https://gw.example/v1\")).ask(\"x\")\n"
    ));
    assert_eq!(
        routes_and_gaps(&found),
        vec![(
            9,
            Routing::OpenaiDirect,
            vec!["ambiguous_client/unseen_callers".to_string()]
        )]
    );
}

#[test]
fn environment_writes_with_run_time_names_may_set_the_endpoint() {
    // Finding 9: a loop over `os.environ[k] = v`, a name held in a
    // constant, `os.environ |= {...}`, an alias of `os.environ` and
    // `__setitem__` left `OpenAI()` `openai_direct` with no gap.
    let header = "import os\nfrom openai import OpenAI\nGW = \"https://openrouter.ai/api/v1\"\n";
    for body in [
        "for k, v in {\"OPENAI_BASE_URL\": GW}.items():\n    os.environ[k] = v\n",
        "KEY = \"OPENAI_BASE_URL\"\nos.environ[KEY] = GW\n",
        "os.environ |= {\"OPENAI_BASE_URL\": GW}\n",
        "env = os.environ\nenv[\"OPENAI_BASE_URL\"] = GW\n",
        "os.environ.__setitem__(\"OPENAI_BASE_URL\", GW)\n",
        "os.environ.update(**overrides)\n",
        "os.putenv(name, GW)\n",
        "from os import environ\nenviron[key] = GW\n",
    ] {
        let found = usages(&format!(
            "{header}{body}client = OpenAI()\nclient.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        ));
        let routes = routes_and_gaps(&found);
        assert_eq!(routes.len(), 1, "{body}");
        assert_eq!(routes[0].1, Routing::Unknown, "{body}");
        assert_eq!(routes[0].2.len(), 1, "{body}");
    }
    // Reads, copies and a WSGI-style `environ` are not writes.
    let found = usages(&format!(
        "{header}x = os.environ.get(\"X\")\ny = dict(os.environ)\nz = {{**os.environ, \"A\": \"b\"}}\ndef app(environ, start):\n    environ[\"wsgi.x\"] = 1\nclient = OpenAI()\nclient.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
    ));
    assert_eq!(
        routes_and_gaps(&found),
        vec![(10, Routing::OpenaiDirect, vec![])]
    );
}

#[test]
fn the_most_dynamic_features_downgrade_the_whole_file() {
    // Finding 10 and nit 13: the module namespace written through
    // `me = sys.modules[__name__]`, `g = globals()`, `setup(globals())`,
    // `setattr(this, ...)`, `exec`, `obj.__dict__`, `import __main__` and
    // the module importing itself left the old constant with no gap. Any
    // of them now makes every value not written at its call, and every
    // route, a `dynamic_feature` gap naming the feature and its line.
    let header =
        "import sys, importlib\nfrom openai import OpenAI\nclient = OpenAI()\nM = \"gpt-4o\"\n";
    for (body, feature) in [
        (
            "me = sys.modules[__name__]\nme.M = \"o1-mini\"\n",
            "`sys.modules` at line 5",
        ),
        (
            "g = globals()\ng[\"M\"] = \"o1-mini\"\n",
            "`globals()` at line 5",
        ),
        (
            "def setup(ns):\n    ns[\"M\"] = \"o1-mini\"\nsetup(globals())\n",
            "`globals()` at line 7",
        ),
        (
            "this = sys.modules[__name__]\nsetattr(this, \"M\", \"o1-mini\")\n",
            "`sys.modules` at line 5",
        ),
        ("exec(\"M = 'o1-mini'\")\n", "`exec` at line 5"),
        ("exec(open(\"override.py\").read())\n", "`exec` at line 5"),
        (
            "d = obj.__dict__\nd[\"model\"] = \"o1-mini\"\n",
            "`__dict__` at line 5",
        ),
        (
            "import __main__; __main__.M = \"o1-mini\"\n",
            "`import __main__` at line 5",
        ),
        (
            "importlib.import_module(__name__).M = \"o1-mini\"\n",
            "the module importing itself at line 5",
        ),
        (
            "vars(sys.modules[__name__])[\"M\"] = \"o1-mini\"\n",
            "at line 5",
        ),
    ] {
        let source = format!(
            "{header}{body}client.chat.completions.create(model=M, messages=[])\nclient.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        );
        let found = usages(&source);
        let rows = rows_gapped(&source, &[]);
        assert_eq!(rows.len(), 2, "{body}");
        // A literal at the call is never downgraded; the route is.
        assert_eq!(rows[1].1, "gpt-4o", "{body}");
        assert_eq!(rows[1].2, vec!["dynamic_feature".to_string()], "{body}");
        let route = gaps(&found, GapKind::AmbiguousClient);
        assert!(
            route[0].detail.contains(feature),
            "{body}: {}",
            route[0].detail
        );
    }
    // The module's routing too: `sys.modules["openai"]` may be configured.
    let found = usages(
        "import openai\nimport sys\nmod = sys.modules[\"openai\"]\nmod.base_url = \"https://gw.example/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
    );
    assert_eq!(routes_and_gaps(&found)[0].2.len(), 1);
    // `if __name__ == "__main__":` is not a dynamic feature.
    let found = usages(
        "from openai import OpenAI\nclient = OpenAI()\nM = \"gpt-4o\"\nif __name__ == \"__main__\":\n    client.chat.completions.create(model=M, messages=[])\n",
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn values_complete_only_for_one_scope_with_no_alias_or_mutation() {
    // Finding 11: a list mutated through an alias, and a class-body
    // comprehension's first iterable, were listed as complete.
    let header = "import os\nfrom openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "L = [\"gpt-4o\"]\nB = L\nB.append(\"o1-mini\")\nfor m in L:\n    client.chat.completions.create(model=m, messages=[])\n",
        "L = [\"gpt-4o\"]\nB = L\nB[0] = \"o1-mini\"\nfor m in L:\n    client.chat.completions.create(model=m, messages=[])\n",
        "L = [\"gpt-4o\"]\nB = L\nB += [\"o1-mini\"]\nfor m in L:\n    client.chat.completions.create(model=m, messages=[])\n",
        "L = [\"gpt-4o\"]\nB = L\nB.extend([\"o1-mini\"])\nr = [client.chat.completions.create(model=m, messages=[]) for m in L]\n",
        "L = [\"gpt-4o\"]\nL.append(\"o1-mini\")\ndef run():\n    for m in L:\n        client.chat.completions.create(model=m, messages=[])\n",
        "M4 = [\"gpt-4o\"]\nclass Bot:\n    M4 = [\"o1-mini\"]\n    r = [client.chat.completions.create(model=m, messages=[]) for m in M4]\n",
        // A definition from another scope (`global`) is not in the list's
        // scope.
        "if os.getenv(\"X\"):\n    M = \"gpt-4o\"\nelse:\n    M = \"o1-mini\"\ndef tune():\n    global M\n    M = \"o3-mini\"\nclient.chat.completions.create(model=M, messages=[])\n",
        // `exec` may add any value.
        "if os.getenv(\"X\"):\n    M = \"gpt-4o\"\nelse:\n    M = \"o1-mini\"\nexec(code)\nclient.chat.completions.create(model=M, messages=[])\n",
    ] {
        let found = usages(&format!("{header}{body}"));
        assert_eq!(found.calls.len(), 1, "{body}");
        assert!(found.calls[0].values.is_empty(), "{body}");
        for gap in gaps(&found, GapKind::UnresolvedModel) {
            assert!(!gap.values_complete, "{body}: {gap:?}");
        }
    }
    // Inside the grammar: literal branches in one scope, a literal list.
    let found = usages(&format!(
        "{header}if os.getenv(\"X\"):\n    M = \"gpt-4o\"\nelse:\n    M = \"o1-mini\"\nclient.chat.completions.create(model=M, messages=[])\nfor m in [\"gpt-4o\", \"o1-mini\"]:\n    client.chat.completions.create(model=m, messages=[])\n"
    ));
    let listed = listed_with_completeness(&found);
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|row| row.3), "{listed:?}");
}

#[test]
fn a_base_url_routes_by_its_host() {
    // P6: the provider's own host is direct; an environment lookup with
    // no fallback, or a value that does not resolve, is `unknown` with a
    // gap; the provider's host as a fallback is direct with a gap; another
    // host is a custom route. The URL itself is never reported.
    let found = usages(
        r#"import os
import openai
from openai import OpenAI
from anthropic import Anthropic
a = OpenAI(base_url="https://api.openai.com/v1")
a.chat.completions.create(model="gpt-4o", messages=[])
b = OpenAI(base_url=os.environ["OPENAI_BASE_URL"])
b.chat.completions.create(model="gpt-4o", messages=[])
c = OpenAI(base_url=os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1"))
c.chat.completions.create(model="gpt-4o", messages=[])
d = OpenAI(base_url=os.getenv("GATEWAY", "https://gw.example/v1"))
d.chat.completions.create(model="gpt-4o", messages=[])
e = OpenAI(base_url=None)
e.chat.completions.create(model="gpt-4o", messages=[])
f = OpenAI(base_url=settings.base_url)
f.chat.completions.create(model="gpt-4o", messages=[])
g = Anthropic(base_url="https://API.anthropic.com:443/")
g.messages.create(model="claude-3-haiku-20240307", max_tokens=1, messages=[])
h = OpenAI(base_url="https://user:pw@gw.example/v1")
h.chat.completions.create(model="gpt-4o", messages=[])
i = OpenAI().with_options(base_url=os.getenv("BASE"))
i.chat.completions.create(model="gpt-4o", messages=[])
"#,
    );
    let gap = |reason: &str| vec![format!("ambiguous_client/{reason}")];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (6, Routing::OpenaiDirect, vec![]),
            (8, Routing::Unknown, gap("base_url_env")),
            (10, Routing::OpenaiDirect, gap("base_url_env")),
            // Another host as an environment fallback: the deployment may
            // point it anywhere (policy round 2, lead decision 6; it was
            // `custom_base_url` with no gap).
            (12, Routing::CustomBaseUrl, gap("base_url_env")),
            (14, Routing::OpenaiDirect, vec![]),
            (16, Routing::Unknown, gap("base_url_unresolved")),
            (18, Routing::AnthropicDirect, vec![]),
            (20, Routing::CustomBaseUrl, vec![]),
            // `with_options` itself is a call on the fresh direct client.
            (21, Routing::OpenaiDirect, vec![]),
            (22, Routing::Unknown, gap("base_url_env")),
        ]
    );
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("pw@") && !json.contains("gw.example"));
    // Module configuration: the provider's own host stays direct; one set
    // inside a function may not have run.
    let found = usages(
        "import openai\nopenai.base_url = \"https://api.openai.com/v1/\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![(3, Routing::OpenaiDirect, vec![])]
    );
    let found = usages(
        "import openai\nGW = \"https://gw.example/v1\"\ndef setup():\n    openai.base_url = GW\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![(
            5,
            Routing::CustomBaseUrl,
            vec!["ambiguous_client/module_config".to_string()]
        )]
    );
}

#[test]
fn attribute_hooks_and_shadowing_methods_are_dynamic() {
    // P2: a class that defines `__getattr__`/`__setattr__`, a method or
    // property named like the attribute, and `delattr` with a run-time
    // name may hand out anything.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "class Bot:\n    model = \"gpt-4o\"\n    def __setattr__(self, k, v):\n        object.__setattr__(self, k, v)\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
        "class Bot:\n    model = \"gpt-4o\"\n    @property\n    def model(self):\n        return \"o1-mini\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
        "class Bot:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nb = Bot()\ndelattr(b, name)\n",
    ] {
        rows_gapped(&format!("{header}{body}"), &[]);
    }
}

#[test]
fn deleting_a_name_or_attribute_is_a_write() {
    // `del Child.model` makes `Child.model` read the base's value, and
    // `del M` unbinds the constant: each is an assignment of no value, so
    // a later read keeps a gap instead of the deleted value.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    for body in [
        "class Base:\n    model = \"gpt-4o\"\nclass Child(Base):\n    model = \"o1-mini\"\ndel Child.model\nclient.chat.completions.create(model=Child.model, messages=[])\n",
        "class Bot:\n    model = \"gpt-4o\"\n    def reset(self):\n        del self.model\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
        "M = \"gpt-4o\"\ndel M\nclient.chat.completions.create(model=M, messages=[])\n",
        "M = \"gpt-4o\"\nN = \"o1-mini\"\ndel N, M\nclient.chat.completions.create(model=M, messages=[])\n",
    ] {
        let rows = rows_gapped(&format!("{header}{body}"), &[]);
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "-", "{body}");
    }
    // Deleting a dict entry still only makes the dict unreadable.
    let rows = rows_of(&format!(
        "{header}D = {{\"model\": \"gpt-4o\", \"x\": 1}}\ndel D[\"x\"]\nclient.chat.completions.create(**D, messages=[])\n"
    ));
    assert_eq!(rows, vec![(5, "-".to_string(), vec!["splat".to_string()])]);
}

#[test]
fn locals_in_a_function_is_not_a_dynamic_feature() {
    // A function's `locals()` (or `vars()`) cannot rebind its names, so it
    // leaves the file's values alone; at module level it is the module's
    // namespace, and `vars(obj)` is `obj.__dict__`: both downgrade the
    // file.
    let header = "from openai import OpenAI\nclient = OpenAI()\nM = \"gpt-4o\"\n";
    let rows = rows_of(&format!(
        "{header}def show():\n    print(\"{{x}}\".format(**locals()), vars())\nclient.chat.completions.create(model=M, messages=[])\n"
    ));
    assert_eq!(rows, vec![(6, "gpt-4o".to_string(), vec![])]);
    for (body, feature) in [
        ("print(locals())\n", "`locals()` at line 4"),
        ("print(vars(args))\n", "`vars()` at line 4"),
        (
            "def f():\n    print([locals() for _ in range(1)])\n",
            "`locals()` at line 5",
        ),
    ] {
        let source =
            format!("{header}{body}client.chat.completions.create(model=M, messages=[])\n");
        let found = usages(&source);
        let rows = rows_gapped(&source, &[]);
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].1, "gpt-4o", "{body}");
        let route = gaps(&found, GapKind::AmbiguousClient);
        assert!(
            route[0].detail.contains(feature),
            "{body}: {}",
            route[0].detail
        );
    }
}

// ---------------------------------------------------------------------------
// Policy round 2 (re-review of faa403f)
// ---------------------------------------------------------------------------

/// `(line, values, gap reasons)` of one key at one call.
type KeyedRow = (u32, Vec<String>, Vec<String>);

/// Every value or gap reason of `key` per call.
fn keyed_rows(source: &str, key: &str) -> Vec<KeyedRow> {
    let found = usages(source);
    found
        .calls
        .iter()
        .map(|c| {
            let values = c
                .values
                .iter()
                .filter(|v| v.key == key)
                .map(|v| v.value.clone())
                .collect();
            let reasons = found
                .gaps
                .iter()
                .filter(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte))
                .filter(|g| g.key.as_deref().is_none_or(|k| k == key))
                .map(|g| g.reason.clone())
                .collect();
            (c.span.line, values, reasons)
        })
        .collect()
}

#[test]
fn an_entry_a_later_expansion_or_computed_key_may_replace_is_a_gap() {
    // Blocker 1: `dict_lookup` honoured `**other` and computed keys after
    // an entry, but `**PARAMS` at a call, `dict_nested` and the nested
    // reader did not, so the defaults-then-overrides idiom reported the
    // default with no gap. Each entry is now a `splat` gap unless nothing
    // unreadable follows it, and a nested dict keeps its last entry.
    let header = "from openai import OpenAI\nfrom anthropic import Anthropic\nclient = OpenAI()\nclaude = Anthropic()\n";
    let splat = |line: u32| (line, vec![], vec!["splat".to_string()]);
    let rows = keyed_rows(
        &format!(
            "{header}\
def ask(messages, **overrides):
    params = {{\"model\": \"gpt-4o\", \"temperature\": 0, **overrides}}
    return client.chat.completions.create(messages=messages, **params)
def _ask(messages, **overrides):
    return client.chat.completions.create(messages=messages, **{{\"model\": \"gpt-4o\", **overrides}})
def ask_claude(prompt, **overrides):
    request = {{\"model\": \"claude-3-5-sonnet-latest\", \"max_tokens\": 1024, **overrides}}
    return claude.messages.create(messages=[], **request)
def ask_dict(messages, **kwargs):
    params = dict(model=\"gpt-4o\", temperature=0, **kwargs)
    return client.chat.completions.create(messages=messages, **params)
client.chat.completions.create(**{{\"model\": \"gpt-4o\", KEY: \"o1-mini\"}}, messages=[])
client.chat.completions.create(**{{\"model\": \"gpt-4o\", **{{\"model\": \"o1-mini\"}}}}, messages=[])
PARAMS = {{\"model\": \"gpt-4o\", **{{\"model\": \"o1-mini\"}}}}
client.chat.completions.create(messages=[], **PARAMS)
client.chat.completions.create(**{{**BASE, \"model\": \"gpt-4o\"}}, messages=[])
client.chat.completions.create(**{{\"model\": \"gpt-4o\", \"model\": \"o1-mini\"}}, messages=[])
client.chat.completions.create(**dict({{\"model\": \"x\"}}, model=\"gpt-4o\"), messages=[])
"
        ),
        "model",
    );
    assert_eq!(
        rows,
        vec![
            splat(7),
            splat(9),
            splat(12),
            splat(15),
            splat(16),
            splat(17),
            splat(19),
            // Nothing unreadable after the entry: still confirmed.
            (20, vec!["gpt-4o".to_string()], vec![]),
            (21, vec!["o1-mini".to_string()], vec![]),
            (22, vec!["gpt-4o".to_string()], vec![]),
        ]
    );
    // Nested values: the last entry of a key, and a gap for one a later
    // expansion may replace (inside the dict, or around it).
    let path = "extra_body.fallbacks.model";
    let rows = keyed_rows(
        &format!(
            "{header}\
client.chat.completions.create(model=\"gpt-4o\", messages=[], extra_body={{\"fallbacks\": [{{\"model\": \"a\", **X}}]}})
client.chat.completions.create(model=\"gpt-4o\", messages=[], extra_body={{\"fallbacks\": [{{\"model\": \"a\"}}], **EB}})
client.chat.completions.create(model=\"gpt-4o\", messages=[], extra_body={{\"fallbacks\": [{{\"model\": \"a\"}}], \"fallbacks\": []}})
client.chat.completions.create(model=\"gpt-4o\", messages=[], extra_body={{\"fallbacks\": [{{\"model\": \"a\"}}, {{\"model\": \"b\", \"model\": \"c\"}}]}})
EB2 = {{\"fallbacks\": [{{\"model\": \"a\"}}], **EB}}
client.chat.completions.create(model=\"gpt-4o\", messages=[], **{{\"extra_body\": EB2}})
OPTS = {{\"extra_body\": {{\"fallbacks\": [{{\"model\": \"a\"}}]}}, **MORE}}
client.chat.completions.create(model=\"gpt-4o\", messages=[], **OPTS)
"
        ),
        path,
    );
    let overridden = |line: u32| (line, vec![], vec!["splat".to_string()]);
    assert_eq!(
        rows,
        vec![
            overridden(5),
            overridden(6),
            (7, vec![], vec![]),
            (8, vec!["a".to_string(), "c".to_string()], vec![]),
            // A name in a nested position is not read (no value, no gap).
            (10, vec![], vec![]),
            overridden(12),
        ]
    );
    // A client built from an expansion: its `base_url` may be in it.
    let found = usages(
        r#"import os
from openai import OpenAI
GW = {"base_url": os.environ["GW_URL"]}
c2 = OpenAI(**{"base_url": "https://api.openai.com/v1", **GW})
c2.chat.completions.create(model="gpt-4o", messages=[])
CFG = {"base_url": "https://api.openai.com/v1", **GW}
c5 = OpenAI(**CFG)
c5.chat.completions.create(model="gpt-4o", messages=[])
c6 = OpenAI().with_options(**{"base_url": "https://api.openai.com/v1", **GW})
c6.chat.completions.create(model="gpt-4o", messages=[])
def ask(opts):
    c7 = OpenAI().with_options(**opts)
    return c7.chat.completions.create(model="gpt-4o", messages=[])
"#,
    );
    let gap = |reason: &str| vec![format!("ambiguous_client/{reason}")];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (5, Routing::Unknown, gap("base_url_unresolved")),
            (8, Routing::Unknown, gap("base_url_unresolved")),
            (9, Routing::OpenaiDirect, vec![]),
            (10, Routing::Unknown, gap("base_url_unresolved")),
            (12, Routing::OpenaiDirect, vec![]),
            // `with_options(**opts)` routed `unknown` with no gap.
            (13, Routing::Unknown, gap("base_url_unresolved")),
        ]
    );
}

#[test]
fn module_level_configuration_holds_only_in_the_top_of_file_setup_block() {
    // Blocker 2: the module-level API's routing was the OR of every
    // configuration in the file, so `if USE_AZURE: openai.api_type =
    // "azure"` reported `azure_openai` with no gap. A routing setting now
    // counts with no gap only as a plain top-level statement in the
    // top-of-file setup block, assigned once, before any call that runs at
    // import (engine round 1, lead decision D2); anything else is a
    // `module_config` gap that keeps the suggested routing as a candidate.
    let dynamic = || vec!["ambiguous_client/module_config".to_string()];
    let cases: Vec<(&str, Vec<RouteRow>)> = vec![
        (
            // The reviewer's v0 Azure switch.
            "import os\nimport openai\nif os.getenv(\"USE_AZURE\") == \"1\":\n    openai.api_type = \"azure\"\n    openai.api_base = \"https://myco.openai.azure.com/\"\ndef ask(p):\n    return openai.ChatCompletion.create(model=\"gpt-4\", messages=[])\n",
            vec![(7, Routing::AzureOpenai, dynamic())],
        ),
        (
            "import os\nimport openai\nif os.getenv(\"LLM_PROXY\"):\n    openai.base_url = \"https://litellm.internal/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(5, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            "import openai\ntry:\n    import proxy_settings\n    openai.base_url = proxy_settings.URL\nexcept ImportError:\n    pass\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(7, Routing::Unknown, dynamic())],
        ),
        (
            // Configured after a call that runs at import.
            "import openai\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\nopenai.base_url = \"https://proxy.internal/v1\"\n",
            vec![(2, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            "import openai\nclass Cfg:\n    openai.base_url = \"https://proxy.internal/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(4, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            "import openai\nfor _ in range(0):\n    openai.base_url = \"https://proxy.internal/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(4, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            // Assigned twice: Python sends it direct.
            "import openai\nopenai.base_url = \"https://proxy.internal/v1\"\nopenai.base_url = \"https://api.openai.com/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(4, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            "import os\nimport openai\nopenai.base_url = \"https://proxy.internal/v1\"\nif os.getenv(\"DIRECT\"):\n    openai.base_url = \"https://api.openai.com/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(6, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            "import openai\nif __name__ == \"__main__\":\n    openai.base_url = \"https://proxy.internal/v1\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(4, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            "import openai\nok = True\nok and setattr(openai, \"base_url\", \"https://proxy.internal/v1\")\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(4, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            // `setattr` is a call, not an assignment: it ends the block.
            "import openai\nsetattr(openai, \"base_url\", \"https://proxy.internal/v1\")\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(3, Routing::CustomBaseUrl, dynamic())],
        ),
        (
            // After a `def`, this file's own code may have run first.
            "import openai\ndef ask():\n    return openai.chat.completions.create(model=\"gpt-4o\", messages=[])\nopenai.base_url = \"https://proxy.internal/v1\"\n",
            vec![(3, Routing::CustomBaseUrl, dynamic())],
        ),
        // Inside the grammar: still confirmed.
        (
            "import openai\nopenai.api_key = \"x\"\nopenai.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
            vec![(3, Routing::OpenaiDirect, vec![])],
        ),
        (
            "import openai\nopenai.api_type = \"azure\"\nopenai.api_base = \"https://myco.openai.azure.com/\"\nopenai.ChatCompletion.create(engine=\"gpt-35-turbo\", messages=[])\n",
            vec![(4, Routing::AzureOpenai, vec![])],
        ),
        (
            // A function defined after the setup block runs after it.
            "import openai\nopenai.base_url = \"https://proxy.internal/v1\"\ndef ask():\n    return openai.chat.completions.create(model=\"gpt-4o\", messages=[])\nask()\n",
            vec![(4, Routing::CustomBaseUrl, vec![])],
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(routes_and_gaps(&usages(source)), expected, "{source}");
    }
}

#[test]
fn an_azure_argument_on_an_unconfigured_module_api_is_a_gap() {
    // Lead decision 7: the SDK's module state is process-wide and v0 code
    // configures it in a settings module, so `engine=` / `deployment_id=`
    // (what Azure deployments take) on a module-level call in a file that
    // configures nothing was `openai_direct` with no gap. It is now
    // `unknown` with a `module_config` gap; a call without them stays
    // direct, and a file that configures Azure stays confirmed (above).
    let found = usages(
        r#"import openai

def ask(prompt):
    return openai.ChatCompletion.create(engine="gpt-35-turbo", messages=[])

PARAMS = {"deployment_id": "gpt-4", "messages": []}
openai.ChatCompletion.create(**PARAMS)
openai.chat.completions.create(model="gpt-4o", messages=[])
"#,
    );
    let module_config = || vec!["ambiguous_client/module_config".to_string()];
    assert_eq!(
        routes_and_gaps(&found),
        vec![
            (4, Routing::Unknown, module_config()),
            (7, Routing::Unknown, module_config()),
            (8, Routing::OpenaiDirect, vec![]),
        ]
    );
}

#[test]
fn every_way_a_name_or_attribute_is_rebound_counts() {
    // Should-fix 3: tree-sitter reads `type(self).model = v` as a type
    // alias statement, which was never handled, so the class attribute kept
    // its literal with no gap. It is now a write of `model` on an object
    // this file cannot identify (a dynamic feature for every `model`
    // attribute read). The nits that rebind a name the same quiet way are
    // now definitions too: `type M = ...`, a `def` or `class` of an
    // assigned name, a class-body import, `global`/`nonlocal` in a class
    // body, `except* E as M`, and a walrus in a default argument (which
    // binds in the scope the `def` or `lambda` runs in).
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    let gap = |line: u32, reason: &str| (line, vec![], vec![reason.to_string()]);
    let gapped = |line: u32, value: &str, reason: &str| {
        (line, vec![value.to_string()], vec![reason.to_string()])
    };
    let cases: Vec<(&str, Vec<KeyedRow>)> = vec![
        (
            "class B1:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n    def switch(self):\n        type(self).model = \"o1-mini\"\n",
            vec![gapped(6, "gpt-4o", "dynamic_feature")],
        ),
        (
            "MODEL = \"gpt-4o\"\ntype MODEL = int\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            vec![gap(5, "non_literal")],
        ),
        (
            "class B48:\n    model = \"gpt-4o\"\n    from cfg import model\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
            vec![gap(7, "reassigned")],
        ),
        (
            "MODEL = \"gpt-4o\"\ndef MODEL():\n    pass\ndef go():\n    return client.chat.completions.create(model=MODEL, messages=[])\n",
            vec![gap(7, "reassigned")],
        ),
        (
            "MODEL = \"gpt-4o\"\nclass MODEL:\n    pass\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            vec![gap(6, "reassigned")],
        ),
        (
            "MODEL = \"gpt-4o\"\ndef f():\n    global MODEL\n    def MODEL(): pass\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            vec![gap(7, "reassigned")],
        ),
        (
            "MODEL = \"gpt-4o\"\nclass K:\n    global MODEL\n    MODEL = \"o1-mini\"\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            vec![gap(7, "reassigned")],
        ),
        (
            "def outer():\n    M = \"gpt-4o\"\n    class K:\n        nonlocal M\n        M = \"o1-mini\"\n    return client.chat.completions.create(model=M, messages=[])\n",
            vec![gap(8, "reassigned")],
        ),
        (
            "MODEL = \"gpt-4o\"\ndef f():\n    global MODEL\n    try:\n        pass\n    except* Exception as MODEL:\n        pass\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            vec![gap(10, "reassigned")],
        ),
        (
            "def f():\n    M = \"gpt-4o\"\n    def g(a=(M := \"o1-mini\")):\n        pass\n    return client.chat.completions.create(model=M, messages=[])\n",
            vec![gap(7, "reassigned")],
        ),
        (
            "def f():\n    M = \"gpt-4o\"\n    g = lambda a=(M := \"o1-mini\"): a\n    return client.chat.completions.create(model=M, messages=[])\n",
            vec![gap(6, "reassigned")],
        ),
        // Inside the grammar: a def, class or import that does not rebind
        // an assigned name changes nothing.
        (
            "MODEL = \"gpt-4o\"\ndef ask():\n    pass\nclass Bot:\n    pass\nclient.chat.completions.create(model=MODEL, messages=[])\n",
            vec![(8, vec!["gpt-4o".to_string()], vec![])],
        ),
        (
            "def f():\n    M = \"gpt-4o\"\n    def g(a=(N := \"o1-mini\")):\n        pass\n    return client.chat.completions.create(model=M, messages=[])\n",
            vec![(7, vec!["gpt-4o".to_string()], vec![])],
        ),
    ];
    for (body, expected) in cases {
        let source = format!("{header}{body}");
        assert_eq!(keyed_rows(&source, "model"), expected, "{source}");
    }
    // `type(B8()).client = OpenAI(base_url=...)` re-routes what
    // `self.client` reads.
    let found = usages(
        r#"from openai import OpenAI
class B8:
    client = OpenAI()
    def go(self):
        return self.client.chat.completions.create(model="gpt-4o", messages=[])
type(B8()).client = OpenAI(base_url="https://proxy.internal/v1")
"#,
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![(
            5,
            Routing::OpenaiDirect,
            vec!["ambiguous_client/dynamic_feature".to_string()]
        )]
    );
}

#[test]
fn environment_writes_through_an_os_alias_are_seen() {
    // Should-fix 4: the environment scan matched the spelled names `os` and
    // `environ`, so a write through `import os as _os`, `import_module`,
    // `getattr(os, "environ")` or `from os import environ as E` left a
    // direct client `openai_direct` with no gap. A write is now matched by
    // the attribute name `environ` on any object, and the other shapes alias
    // the process environment (a `dynamic_feature`).
    let endpoint = || vec!["ambiguous_client/endpoint_env".to_string()];
    let dynamic = || vec!["ambiguous_client/dynamic_feature".to_string()];
    let cases: Vec<(&str, Vec<RouteRow>)> = vec![
        (
            "import os as _os\n_os.environ[\"OPENAI_BASE_URL\"] = \"https://proxy.internal/v1\"\n",
            vec![(5, Routing::Unknown, endpoint())],
        ),
        (
            "import os as _os\n_os.environ.setdefault(\"OPENAI_BASE_URL\", \"https://proxy.internal/v1\")\n",
            vec![(5, Routing::Unknown, endpoint())],
        ),
        (
            "import os as o\no.environ.update({\"OPENAI_BASE_URL\": \"https://proxy.internal/v1\"})\n",
            vec![(5, Routing::Unknown, endpoint())],
        ),
        (
            "import os as o\no.putenv(\"OPENAI_BASE_URL\", \"https://proxy.internal/v1\")\n",
            vec![(5, Routing::Unknown, endpoint())],
        ),
        (
            "import importlib\nimportlib.import_module(\"os\").environ[\"OPENAI_BASE_URL\"] = \"https://proxy.internal/v1\"\n",
            vec![(5, Routing::Unknown, endpoint())],
        ),
        (
            "import os\ngetattr(os, \"environ\")[\"OPENAI_BASE_URL\"] = \"https://proxy.internal/v1\"\n",
            vec![(5, Routing::Unknown, dynamic())],
        ),
        (
            "from os import environ as E\nE[\"OPENAI_BASE_URL\"] = \"https://proxy.internal/v1\"\n",
            vec![(5, Routing::Unknown, dynamic())],
        ),
        (
            "from os import putenv as P\nP(\"OPENAI_BASE_URL\", \"https://proxy.internal/v1\")\n",
            vec![(5, Routing::Unknown, dynamic())],
        ),
        // Inside the grammar: reading through an alias changes nothing.
        (
            "import os as o\nTIMEOUT = o.getenv(\"TIMEOUT\")\n",
            vec![(5, Routing::OpenaiDirect, vec![])],
        ),
    ];
    for (setup, expected) in cases {
        let source = format!(
            "from openai import OpenAI\n{setup}c = OpenAI()\nc.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        );
        assert_eq!(routes_and_gaps(&usages(&source)), expected, "{source}");
    }
}

#[test]
fn namespace_builtins_reached_under_another_name_downgrade_the_whole_file() {
    // Nit: `exec`, `globals`, `sys.modules` and the module importing itself
    // were matched by spelling, so `run = exec`, `g = globals`,
    // `builtins.exec`, `getattr(builtins, "exec")`, `from sys import
    // modules` and `import app as me` (in `app.py`) left the constant with
    // no gap. Each is now a whole-file dynamic feature. A parameter or a
    // keyword that happens to be named `eval` is not the builtin.
    let header = "import builtins\nfrom openai import OpenAI\nclient = OpenAI()\nM = \"gpt-4o\"\n";
    for body in [
        "run = exec\nrun(\"M = 'o1-mini'\")\n",
        "g = globals\ng()[\"M\"] = \"o1-mini\"\n",
        "builtins.globals()[\"M\"] = \"o1-mini\"\n",
        "builtins.exec(\"M = 'o1-mini'\")\n",
        "getattr(builtins, \"exec\")(\"M = 'o1-mini'\")\n",
        "__builtins__.exec(\"M = 'o1-mini'\")\n",
        "from sys import modules\nsetattr(modules[__name__], \"M\", \"o1-mini\")\n",
        "from builtins import exec as run\nrun(\"M = 'o1-mini'\")\n",
        "import app as me\nme.M = \"o1-mini\"\n",
        "from app import M as _m\n",
    ] {
        let source =
            format!("{header}{body}client.chat.completions.create(model=M, messages=[])\n");
        let rows = keyed_rows(&source, "model");
        // The constant may still be listed, next to its gap.
        assert_eq!(rows.len(), 1, "{body}");
        assert!(
            rows[0].2.iter().any(|r| r == "dynamic_feature"),
            "{body}: {rows:?}"
        );
        let found = usages(&source);
        assert!(
            gaps(&found, GapKind::AmbiguousClient)
                .iter()
                .any(|g| g.reason == "dynamic_feature"),
            "{body}"
        );
    }
    // Not the builtin: a parameter, a keyword and an attribute named `eval`.
    let source = format!(
        "{header}def train(eval=False):\n    if eval:\n        return model.eval()\n    return run(eval=eval)\nclient.chat.completions.create(model=M, messages=[])\n"
    );
    assert_eq!(
        keyed_rows(&source, "model"),
        vec![(9, vec!["gpt-4o".to_string()], vec![])]
    );
}

#[test]
fn gemini_clients_built_for_vertex_or_another_endpoint_are_gaps() {
    // Should-fix 5: `google.genai.Client` was `google_genai` with no gap
    // whatever it was given, although `vertexai=True`, an `http_options`
    // `base_url` or `GOOGLE_GENAI_USE_VERTEXAI` send it to Vertex AI or
    // another endpoint (which retire models on another schedule). Each now
    // routes `unknown` with an `endpoint_option` (or `endpoint_env`) gap;
    // the legacy SDK's `configure(client_options=...)` does the same for
    // every legacy call. Options that name no endpoint stay confirmed.
    let option = || vec!["ambiguous_client/endpoint_option".to_string()];
    let cases: Vec<(&str, Vec<RouteRow>)> = vec![
        (
            "c = genai.Client(vertexai=True, project=\"p\", location=\"us-central1\")\n",
            vec![(5, Routing::Unknown, option())],
        ),
        (
            "c = genai.Client(http_options=types.HttpOptions(base_url=\"https://proxy.internal\"))\n",
            // `types.HttpOptions(...)` is an SDK type call of its own.
            vec![
                (4, Routing::GoogleGenai, vec![]),
                (5, Routing::Unknown, option()),
            ],
        ),
        (
            "c = genai.Client(http_options={\"base_url\": \"https://proxy.internal\"})\n",
            vec![(5, Routing::Unknown, option())],
        ),
        (
            "OPTS = {\"api_version\": \"v1\"}\nc = genai.Client(http_options=OPTS)\n",
            vec![(6, Routing::Unknown, option())],
        ),
        (
            "def make(**opts):\n    return genai.Client(**opts)\nc = make()\nc2 = genai.Client(**{})\n",
            vec![],
        ),
        (
            "os.environ[\"GOOGLE_GENAI_USE_VERTEXAI\"] = \"true\"\nc = genai.Client()\n",
            vec![(
                6,
                Routing::Unknown,
                vec!["ambiguous_client/endpoint_env".to_string()],
            )],
        ),
        (
            "os.environ[NAME] = \"true\"\nc = genai.Client()\n",
            vec![(
                6,
                Routing::Unknown,
                vec!["ambiguous_client/dynamic_feature".to_string()],
            )],
        ),
        // Inside the grammar.
        (
            "c = genai.Client(api_key=os.environ[\"GEMINI_API_KEY\"])\n",
            vec![(5, Routing::GoogleGenai, vec![])],
        ),
        (
            "c = genai.Client(vertexai=False, http_options=types.HttpOptions(api_version=\"v1alpha\"))\n",
            vec![
                (4, Routing::GoogleGenai, vec![]),
                (5, Routing::GoogleGenai, vec![]),
            ],
        ),
    ];
    for (setup, expected) in cases {
        let source = format!(
            "import os\nfrom google import genai\nfrom google.genai import types\n{setup}c.models.generate_content(model=\"gemini-2.0-flash\", contents=\"x\")\n"
        );
        let found = usages(&source);
        // `make()` is a factory this file does not resolve: no route at all.
        if expected.is_empty() {
            assert!(
                found
                    .calls
                    .iter()
                    .all(|c| c.routing != Routing::GoogleGenai),
                "{source}"
            );
            continue;
        }
        assert_eq!(routes_and_gaps(&found), expected, "{source}");
    }
    // The legacy SDK.
    let found = usages(
        r#"import google.generativeai as genai
genai.configure(api_key=KEY, client_options={"api_endpoint": "proxy.internal"})
model = genai.GenerativeModel("gemini-1.5-flash")
model.generate_content("x")
"#,
    );
    let rows = routes_and_gaps(&found);
    assert!(
        rows.iter()
            .filter(|r| r.0 >= 3)
            .all(|r| r.1 == Routing::Unknown && r.2 == option()),
        "{rows:?}"
    );
    assert_eq!(rows.iter().filter(|r| r.0 >= 3).count(), 2, "{rows:?}");
    let found = usages(
        r#"import google.generativeai as genai
genai.configure(api_key=KEY)
model = genai.GenerativeModel("gemini-1.5-flash")
model.generate_content("x")
"#,
    );
    assert!(routes_and_gaps(&found)
        .iter()
        .filter(|r| r.0 >= 3)
        .all(|r| r.1 == Routing::GoogleGenerativeaiLegacy && r.2.is_empty()));
}

#[test]
fn an_environment_base_url_with_another_host_as_fallback_is_a_gap() {
    // Lead decision 6: `base_url=os.getenv("LLM_URL", "https://proxy...")`
    // was `custom_base_url` with no gap, but the deployment may point it at
    // the provider or another gateway. Only a literal or absent `base_url`
    // is confirmed (P1), so it keeps `custom_base_url` with a
    // `base_url_env` gap, like the provider-host fallback keeps
    // `openai_direct` with one.
    let env = || vec!["ambiguous_client/base_url_env".to_string()];
    let cases: Vec<(&str, Vec<RouteRow>)> = vec![
        (
            "c = OpenAI(base_url=os.getenv(\"LLM_URL\", \"https://proxy.internal/v1\"))\n",
            vec![(5, Routing::CustomBaseUrl, env())],
        ),
        (
            "c = OpenAI(base_url=os.getenv(\"LLM_URL\") or \"https://proxy.internal/v1\")\n",
            vec![(5, Routing::CustomBaseUrl, env())],
        ),
        (
            "c = OpenAI(base_url=os.environ.get(\"LLM_URL\", \"https://proxy.internal/v1\"))\n",
            vec![(5, Routing::CustomBaseUrl, env())],
        ),
        (
            "BASE = os.environ.get(\"OPENAI_BASE_URL\", \"https://proxy.internal/v1\")\nc = OpenAI(base_url=BASE)\n",
            vec![(6, Routing::CustomBaseUrl, env())],
        ),
        (
            "c = OpenAI().with_options(base_url=os.getenv(\"LLM_URL\", \"https://proxy.internal/v1\"))\n",
            vec![
                (4, Routing::OpenaiDirect, vec![]),
                (5, Routing::CustomBaseUrl, env()),
            ],
        ),
        (
            "c = openai\nopenai.base_url = os.getenv(\"LLM_URL\", \"https://proxy.internal/v1\")\n",
            vec![(6, Routing::CustomBaseUrl, env())],
        ),
        // Unchanged: the provider's host as fallback, and no fallback.
        (
            "c = OpenAI(base_url=os.getenv(\"LLM_URL\", \"https://api.openai.com/v1\"))\n",
            vec![(5, Routing::OpenaiDirect, env())],
        ),
        (
            "c = OpenAI(base_url=os.getenv(\"LLM_URL\"))\n",
            vec![(5, Routing::Unknown, env())],
        ),
        (
            "c = OpenAI(base_url=\"https://proxy.internal/v1\")\n",
            vec![(5, Routing::CustomBaseUrl, vec![])],
        ),
    ];
    for (setup, expected) in cases {
        let source = format!(
            "import os\nimport openai\nfrom openai import OpenAI\n{setup}c.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        );
        assert_eq!(routes_and_gaps(&usages(&source)), expected, "{source}");
    }
}

#[test]
fn self_is_the_instance_only_as_a_methods_first_parameter() {
    // Nits: a `self` that is not the instance still read the class: a
    // `staticmethod`'s parameter named `self`, a nested `def inner(self)`,
    // and a method another class borrows (`B2.go = B.go`, `go = B.go` in a
    // class body). `self`/`cls` now hold the class only as the first
    // parameter of a method directly in its class body (and in closures
    // that do not rebind it), and not in a method read as a value through
    // its class; a write through such a method's `self` is an outside
    // write. The classes are private so that only this rule applies.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    let cases: Vec<(&str, u32)> = vec![
        (
            "class _B3:\n    model = \"gpt-4o\"\n    @staticmethod\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
            7,
        ),
        (
            "class _B4:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nclass _B4b:\n    model = \"o1-mini\"\n_B4b.go = _B4.go\n_B4b().go()\n",
            6,
        ),
        (
            "class _B4:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nclass _B4b:\n    model = \"o1-mini\"\n    go = _B4.go\n_B4b().go()\n",
            6,
        ),
        (
            "class _B5:\n    model = \"gpt-4o\"\n    def go(self):\n        def inner(self):\n            return client.chat.completions.create(model=self.model, messages=[])\n        return inner(object())\n",
            7,
        ),
        (
            // The borrowed method writes the borrower's instances.
            "class _B6:\n    def setm(self):\n        self.model = \"o1-mini\"\nclass _B7:\n    model = \"gpt-4o\"\n    setm = _B6.setm\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n",
            10,
        ),
    ];
    for (body, line) in cases {
        let source = format!("{header}{body}");
        let rows = keyed_rows(&source, "model");
        let row = rows.iter().find(|r| r.0 == line).expect("the call");
        assert!(!row.2.is_empty(), "{source}: {rows:?}");
        assert!(
            row.1.is_empty() || row.2.iter().any(|r| r == "dynamic_feature"),
            "{source}: {rows:?}"
        );
    }
    // A `staticmethod`'s `self.client` is not the class's client.
    let found = usages(
        r#"from openai import OpenAI
class _B9:
    client = OpenAI()
    @staticmethod
    def go(self):
        return self.client.chat.completions.create(model="gpt-4o", messages=[])
"#,
    );
    assert!(found.calls.iter().all(|c| c.span.line != 6));
    assert!(gaps(&found, GapKind::UnknownClientFactory)
        .iter()
        .any(|g| g.span.is_some_and(|s| s.line == 6)));
    // Inside the grammar: a closure in a method, and a classmethod's `cls`.
    let source = format!(
        "{header}class _B8:\n    model = \"gpt-4o\"\n    def go(self):\n        ask = lambda: client.chat.completions.create(model=self.model, messages=[])\n        return ask()\n    @classmethod\n    def make(cls):\n        return client.chat.completions.create(model=cls.model, messages=[])\n"
    );
    assert_eq!(
        keyed_rows(&source, "model"),
        vec![
            (6, vec!["gpt-4o".to_string()], vec![]),
            (10, vec!["gpt-4o".to_string()], vec![]),
        ]
    );
}

#[test]
fn a_public_classs_attribute_read_through_self_keeps_a_gap() {
    // Lead decision 8: other modules may subclass a public class and set
    // the attribute a method reads through `self`, the way they may pass
    // another model to a public function (rule 4). Such a value keeps a
    // `subclass_override` gap. Read by class name, in a private class, or in
    // a class defined in a function it is confirmed; routes through
    // `self.client` are kept.
    let found = usages(
        r#"from openai import OpenAI
client = OpenAI()
class Bot:
    MODEL = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.MODEL, messages=[])
    def by_name(self):
        return client.chat.completions.create(model=Bot.MODEL, messages=[])
class _Hidden:
    MODEL = "gpt-4o"
    def go(self):
        return client.chat.completions.create(model=self.MODEL, messages=[])
def factory():
    class Local:
        MODEL = "gpt-4o"
        def go(self):
            return client.chat.completions.create(model=self.MODEL, messages=[])
    return Local().go()
class Svc:
    def __init__(self):
        self.client = OpenAI()
    def go(self):
        return self.client.chat.completions.create(model="gpt-4o", messages=[])
"#,
    );
    let gpt = || vec!["gpt-4o".to_string()];
    assert_eq!(
        values_and_gap_rows(&found),
        vec![
            (6, gpt(), vec!["subclass_override".to_string()]),
            (8, gpt(), vec![]),
            (12, gpt(), vec![]),
            (17, gpt(), vec![]),
            (23, gpt(), vec![]),
        ]
    );
    assert_eq!(found.calls[4].routing, Routing::OpenaiDirect);
}

/// `(line, model values, gap reasons)` per call.
fn values_and_gap_rows(found: &FileUsages) -> Vec<KeyedRow> {
    found
        .calls
        .iter()
        .map(|c| {
            let values = c
                .values
                .iter()
                .filter(|v| v.key == "model")
                .map(|v| v.value.clone())
                .collect();
            let reasons = found
                .gaps
                .iter()
                .filter(|g| g.span.map(|s| s.start_byte) == Some(c.span.start_byte))
                .map(|g| g.reason.clone())
                .collect();
            (c.span.line, values, reasons)
        })
        .collect()
}

#[test]
fn exported_and_name_mangled_private_functions_have_unseen_callers() {
    // Nits: a `_private` function listed in `__all__` is exported, and
    // `_A()._A__go(...)` calls `A.__go` from outside under its mangled name;
    // neither call was counted, so the default had no gap.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    for (body, line) in [
        (
            "__all__ = [\"_ask\"]\ndef _ask(prompt, model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[])\n_ask(\"x\")\n",
            5,
        ),
        (
            "__all__ = []\n__all__ += [\"_ask\"]\ndef _ask(prompt, model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[])\n",
            6,
        ),
        (
            "__all__ = []\n__all__.extend([\"_ask\"])\ndef _ask(prompt, model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[])\n",
            6,
        ),
        (
            "class _A:\n    def __go(self, model=\"gpt-4o\"):\n        return client.chat.completions.create(model=model, messages=[])\n    def run(self):\n        return self.__go()\n_A()._A__go(model=\"o1-mini\")\n",
            5,
        ),
    ] {
        let source = format!("{header}{body}");
        let rows = keyed_rows(&source, "model");
        assert_eq!(
            rows,
            vec![(
                line,
                vec!["gpt-4o".to_string()],
                vec!["unseen_callers".to_string()]
            )],
            "{source}"
        );
    }
    // Inside the grammar: a private function not exported.
    let source = format!(
        "{header}__all__ = [\"ask\"]\ndef _ask(prompt, model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[])\n_ask(\"x\")\n"
    );
    assert_eq!(
        keyed_rows(&source, "model"),
        vec![(5, vec!["gpt-4o".to_string()], vec![])]
    );
}

#[test]
fn an_sdk_member_replaced_in_the_file_gaps_its_routes() {
    // Nit: `client.chat.completions.create = patched`, `openai.OpenAI =
    // Sub` and `client.chat = ...` replace what a call runs, but its route
    // had no gap. Each now makes every route of that SDK a
    // `dynamic_feature` gap (a literal model at the call stays, as P2
    // says). Settings such as `openai.api_key` change nothing.
    let dynamic = || vec!["ambiguous_client/dynamic_feature".to_string()];
    for setup in [
        "client.chat.completions.create = _patched\n",
        "setattr(client.chat.completions, \"create\", _patched)\n",
        "client.chat = Other()\n",
        "openai.OpenAI = Other\n",
        "openai.chat.completions.create = _patched\n",
    ] {
        let source = format!(
            "import openai\nfrom openai import OpenAI\nclient = OpenAI()\n{setup}client.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        );
        let found = usages(&source);
        let call = found.calls.last().unwrap();
        assert_eq!(value(call, "model"), Some("gpt-4o"), "{setup}");
        assert_eq!(
            routes_and_gaps(&found).last().unwrap().2,
            dynamic(),
            "{setup}"
        );
    }
    let found = usages(
        "import openai\nfrom openai import OpenAI\nclient = OpenAI()\nopenai.api_key = \"x\"\nclient.max_retries = 3\nclient.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
    );
    assert_eq!(
        routes_and_gaps(&found),
        vec![(6, Routing::OpenaiDirect, vec![])]
    );
}

#[test]
fn a_private_class_with_a_public_subclass_keeps_a_subclass_override_gap() {
    // Engine round 1: `self.model` in a private class was confirmed even
    // when a public same-file subclass inherits the method, although
    // another module may subclass that public class and set `model`, as
    // it may for a public class (lead decision 8). Reads by class name
    // stay confirmed, and so does a private class whose subclasses are
    // private.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    let subclass_override = || vec!["subclass_override".to_string()];
    for (body, reasons) in [
        (
            "class _Base:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nclass Pub(_Base):\n    pass\n",
            subclass_override(),
        ),
        (
            "class _Base:\n    model = \"gpt-4o\"\n    @classmethod\n    def go(cls):\n        return client.chat.completions.create(model=cls.model, messages=[])\nclass Pub(_Base):\n    pass\n",
            subclass_override(),
        ),
        (
            "class _Base:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=_Base.model, messages=[])\nclass Pub(_Base):\n    pass\n",
            vec![],
        ),
        (
            "class _Base:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\nclass _Other(_Base):\n    pass\n",
            vec![],
        ),
    ] {
        let source = format!("{header}{body}");
        let rows = rows_of(&source);
        assert_eq!(rows.len(), 1, "{source}");
        assert_eq!(rows[0].1, "gpt-4o", "{source}");
        assert_eq!(rows[0].2, reasons, "{source}");
    }
}

// ---------------------------------------------------------------------------
// Engine round-1 re-review of #276 (head a3a1256)
// ---------------------------------------------------------------------------

#[test]
fn engine_keys_of_a_large_expanded_dict_are_found_in_constant_time() {
    // Blocker 1: in a file that does not configure the module-level API,
    // each `openai.Completion.create(**P)` walked every key of `P` looking
    // for `engine`: 10k calls x 400k keys took 26 s in a release build. It
    // is now two lookups per call.
    for (last, gapped) in [("'tail': 0", false), ("'engine': 'gpt-4o'", true)] {
        let mut source = String::from("import openai\nP = {");
        for i in 0..100_000 {
            source.push_str(&format!("'k{i}': 0, "));
        }
        source.push_str(last);
        source.push_str("}\n");
        for f in 0..100 {
            source.push_str(&format!("def f{f}():\n"));
            for _ in 0..100 {
                source.push_str("    openai.Completion.create(**P)\n");
            }
        }
        let found = usages_within(&source, 10);
        assert_eq!(found.calls.len(), 10_000);
        let module_config = found
            .gaps
            .iter()
            .filter(|g| g.reason == "module_config")
            .count();
        assert_eq!(module_config, if gapped { 10_000 } else { 0 }, "{last}");
    }
}

#[test]
fn scopes_nested_past_the_cap_are_one_truncated_gap_in_linear_time() {
    // Blocker 2: each call rooted in a name walked every enclosing scope
    // to find the name's binding. Comprehensions, generators and lambdas
    // nest without indentation, so 45k levels fit in 1 MB and 5k levels
    // around 50k calls took 19 s in a release build. Scopes nested past 64
    // are now not walked: a `truncated` gap, and a dynamic feature of the
    // whole file, since what they bind or write is not seen.
    let head =
        "from openai import OpenAI\nclient = OpenAI()\nM = \"gpt-4o\"\nR = [1]\ndef g(): pass\n";
    let tail = "client.chat.completions.create(model=M, messages=[])\nclient.chat.completions.create(model=\"o1-mini\", messages=[])\n";
    let shapes: [(&str, &str, usize, &str, usize); 5] = [
        ("[", " for a in R]", 45_000, "g(),", 100_000),
        ("(", " for a in R)", 20_000, "g(),", 50_000),
        ("(lambda: ", ")", 20_000, "g.h(),", 50_000),
        ("(lambda a: ", ")()", 20_000, "a(),", 50_000),
        ("[", " for a in R]", 20_000, "(y := g()),", 50_000),
    ];
    for (open, close, levels, call, calls) in shapes {
        let source = format!(
            "{head}X = {}({}){}\n{tail}",
            open.repeat(levels),
            call.repeat(calls),
            close.repeat(levels)
        );
        assert!(source.len() <= 1 << 20, "{open} x {levels}");
        let found = usages_within(&source, 10);
        let truncated = gaps(&found, GapKind::UnresolvedModel)
            .into_iter()
            .filter(|g| g.reason == "truncated")
            .count();
        assert_eq!(truncated, 1, "{open} x {levels}");
        // Every route, and every value read through a name, keeps a gap; a
        // literal at the call stays confirmed.
        let dynamic = |n: usize| vec!["dynamic_feature".to_string(); n];
        assert_eq!(
            keyed_rows(&source, "model"),
            vec![
                (7, vec!["gpt-4o".to_string()], dynamic(2)),
                (8, vec!["o1-mini".to_string()], dynamic(1)),
            ],
            "{open} x {levels}"
        );
    }
    // Within the cap a deep nest is analyzed as before.
    let mut source = String::from(head);
    for i in 0..60 {
        source.push_str(&format!("{}def f{i}():\n", "    ".repeat(i)));
    }
    source.push_str(&format!(
        "{}return client.chat.completions.create(model=M, messages=[])\n",
        "    ".repeat(60)
    ));
    assert_eq!(
        keyed_rows(&source, "model"),
        vec![(66, vec!["gpt-4o".to_string()], vec![])]
    );
}

#[test]
fn module_configuration_counts_only_in_the_top_of_file_setup_block() {
    // Blocker 3 (lead decision D2): with `openai.base_url = ...` at the
    // bottom of the file, a decorator, a default argument, a generator, a
    // bound lambda, a class body, an alias, `map`, a `key=` lambda, an
    // immediately called lambda or `__init_subclass__` ran `ask()` at
    // import first, and the call was confirmed. Rather than following
    // import-time execution, a routing setting now counts only in the
    // top-of-file setup block: before the first `def`, `class`, decorator,
    // bare expression or compound statement. Anything else gives every
    // module-level call a `module_config` gap.
    let ask = "def ask(p=None):\n    return openai.chat.completions.create(model=\"gpt-4o\", messages=[])\n";
    let config = "openai.base_url = \"https://proxy.internal/v1\"\n";
    for runs in [
        "def warm(f):\n    f(\"w\")\n    return f\n@warm\ndef other(p):\n    return p\n",
        "def register(cls):\n    ask(\"w\")\n    return cls\n@register\nclass Bot:\n    pass\n",
        "def handler(x=ask(\"w\")):\n    return x\n",
        "P = [\"a\"]\nwarm = list(ask(p) for p in P)\n",
        "class Warm:\n    def go(p):\n        return ask(p)\n    go(\"w\")\n",
        "run = ask\nrun()\n",
        "list(map(ask, [\"a\"]))\n",
        "sorted([\"a\"], key=lambda p: ask(p))\n",
        "(lambda: ask())()\n",
        "class Base:\n    def __init_subclass__(cls, **kw):\n        ask()\nclass Sub(Base):\n    pass\n",
        "class H(ask(\"w\").__class__):\n    pass\n",
        "if __name__ == \"__main__\":\n    ask()\n",
        "from dotenv import load_dotenv\nload_dotenv()\n",
        "",
    ] {
        let source = format!("import openai\n{ask}{runs}{config}");
        let rows = routes_and_gaps(&usages(&source));
        assert_eq!(
            rows,
            vec![(
                3,
                Routing::CustomBaseUrl,
                vec!["ambiguous_client/module_config".to_string()]
            )],
            "{source}"
        );
    }
    // A lambda bound before the configuration ends the block too.
    let source = format!(
        "import openai\nask = lambda p: openai.chat.completions.create(model=\"gpt-4o\", messages=[])\nask(\"w\")\n{config}"
    );
    assert_eq!(
        routes_and_gaps(&usages(&source)),
        vec![(
            2,
            Routing::CustomBaseUrl,
            vec!["ambiguous_client/module_config".to_string()]
        )]
    );
    // `+=`, an alias chosen at run time, and a run-time attribute name.
    for (setup, line) in [
        ("openai.api_base = \"https://proxy.internal\"\nopenai.api_base += \"/v1\"\n", 5),
        ("import json, os\no = openai if os.getenv(\"X\") else json\no.base_url = \"https://proxy.internal/v1\"\n", 6),
        ("import os\nsetattr(openai, os.getenv(\"ATTR\"), \"https://proxy.internal/v1\")\n", 5),
    ] {
        let source = format!(
            "import openai\n{setup}def ask():\n    return openai.chat.completions.create(model=\"gpt-4o\", messages=[])\n"
        );
        let rows = routes_and_gaps(&usages(&source));
        assert_eq!(rows.len(), 1, "{source}");
        assert_eq!(rows[0].0, line, "{source}");
        assert_eq!(
            rows[0].2,
            vec!["ambiguous_client/module_config".to_string()],
            "{source}"
        );
    }
    // Inside the grammar: a setup block of imports, a docstring and
    // assignments (calls of other modules' code included), then code. A
    // call in the block itself counts only after the configuration.
    let source = r#""""Configured at the top."""
from __future__ import annotations

import os

import openai

TIMEOUT = 30
openai.api_key = os.getenv("OPENAI_API_KEY")
early = openai.chat.completions.create(model="gpt-4o", messages=[])
openai.base_url = "https://proxy.internal/v1"
openai.default_headers = {"x-team": "ml"}
stream = openai.chat.completions.create(model="gpt-4o", messages=[])


def ask():
    return openai.chat.completions.create(model="gpt-4o", messages=[])


ask()
"#;
    assert_eq!(
        routes_and_gaps(&usages(source)),
        vec![
            (
                10,
                Routing::CustomBaseUrl,
                vec!["ambiguous_client/module_config".to_string()]
            ),
            (13, Routing::CustomBaseUrl, vec![]),
            (17, Routing::CustomBaseUrl, vec![]),
        ]
    );
}

#[test]
fn a_method_that_rebinds_self_or_cls_reads_nothing_through_it() {
    // Blocker 4 (lead decision D3): `self = other`, a tuple, `for`,
    // `with`, walrus, `except` or `match` target, a closure's `nonlocal
    // self`, or `cls = _Other` in a classmethod left reads through the name
    // as the class's attribute. A method that binds its first parameter's
    // name in any other way now reads nothing through it: every read there
    // is a `dynamic_feature` gap, and a write through it is a write on
    // another object.
    let header = "from openai import OpenAI\nclient = OpenAI()\nclass _Err(Exception):\n    model = \"o1-mini\"\nclass _Other:\n    model = \"o1-mini\"\n";
    let read = "client.chat.completions.create(model=self.model, messages=[])";
    for body in [
        format!("    def go(self, other):\n        self = other\n        return {read}\n"),
        format!("    def go(self, other):\n        self, n = other, 1\n        return {read}\n"),
        format!("    def go(self, others):\n        for self in others:\n            {read}\n"),
        format!("    def go(self, cm):\n        with cm as self:\n            {read}\n"),
        format!("    def go(self, other):\n        if (self := other):\n            {read}\n"),
        format!("    def go(self):\n        try:\n            pass\n        except _Err as self:\n            {read}\n"),
        format!("    def go(self, x):\n        match x:\n            case _Other() as self:\n                {read}\n"),
        format!("    def go(self, other):\n        def inner():\n            nonlocal self\n            self = other\n        inner()\n        return {read}\n"),
        format!("    def go(self, other):\n        def inner():\n            self = other\n            return {read}\n        return inner()\n"),
        format!("    def go(self, other):\n        self += other\n        return {read}\n"),
        format!("    def go(self):\n        from cfg import other as self\n        return {read}\n"),
        format!("    def go(self, other):\n        del self\n        self = other\n        return {read}\n"),
        format!("    def go(self, others):\n        return [{read} for self in others]\n"),
        format!("    def go(self):\n        def self():\n            pass\n        return {read}\n"),
        format!("    def go(self):\n        class self:\n            model = \"o1-mini\"\n        return {read}\n"),
        format!("    @classmethod\n    def go(cls):\n        cls = _Other\n        return {}\n", read.replace("self.", "cls.")),
    ] {
        let source = format!("{header}class _Bot:\n    model = \"gpt-4o\"\n{body}");
        let rows = keyed_rows(&source, "model");
        assert_eq!(rows.len(), 1, "{source}");
        assert!(rows[0].1.is_empty(), "{source}");
        assert_eq!(rows[0].2, vec!["dynamic_feature".to_string()], "{source}");
    }
    // The route through a rebound `self` is a gap too.
    let source = "from openai import OpenAI\nclass _Proxy:\n    def __init__(self):\n        self.client = OpenAI(base_url=\"https://proxy.internal/v1\")\nclass _Svc:\n    def __init__(self):\n        self.client = OpenAI()\n    def go(self, other):\n        self = other\n        return self.client.chat.completions.create(model=\"gpt-4o\", messages=[])\n    def ok(self):\n        return self.client.chat.completions.create(model=\"gpt-4o\", messages=[])\n";
    let found = usages(source);
    assert_eq!(
        routes_and_gaps(&found),
        vec![(12, Routing::OpenaiDirect, vec![])]
    );
    let receiver = gaps(&found, GapKind::UnknownClientFactory);
    assert_eq!(receiver.len(), 1);
    assert_eq!(receiver[0].reason, "dynamic_feature");
    assert_eq!(receiver[0].span.map(|s| s.line), Some(10));
    // A write through a rebound `self` may set another class's attribute.
    let source = format!(
        "{header}class _Bot:\n    def poison(self, other):\n        self = other\n        self.model = \"gpt-4o\"\nclass _Third:\n    model = \"o1\"\n    def go(self):\n        return {read}\n"
    );
    let rows = keyed_rows(&source, "model");
    assert_eq!(rows[0].2, vec!["dynamic_feature".to_string()], "{source}");
    // A lambda passed in a class body is not a method; one assigned to a
    // name is.
    let source = format!(
        "{header}class _Bot:\n    model = \"gpt-4o\"\n    items = []\n    calls = list(map(lambda self: {read}, items))\n    go = lambda self: {read}\n"
    );
    let rows = keyed_rows(&source, "model");
    assert_eq!(rows.len(), 2, "{source}");
    assert!(rows[0].1.is_empty() && !rows[0].2.is_empty(), "{source}");
    assert_eq!(
        rows[1],
        (11, vec!["gpt-4o".to_string()], vec![]),
        "{source}"
    );
    // Inside the grammar: other methods of the class, and the method
    // itself when it only reads.
    let source = format!(
        "{header}class _Bot:\n    model = \"gpt-4o\"\n    def bad(self, other):\n        self = other\n    def go(self):\n        return {read}\n    @classmethod\n    def go_cls(cls):\n        return {}\n",
        read.replace("self.", "cls.")
    );
    let rows = keyed_rows(&source, "model");
    assert_eq!(
        rows,
        vec![
            (12, vec!["gpt-4o".to_string()], vec![]),
            (15, vec!["gpt-4o".to_string()], vec![]),
        ],
        "{source}"
    );
}

#[test]
fn every_class_body_binding_is_a_write_of_the_attribute() {
    // Blocker 5 (lead decision D4): a class-body `def`, `class` or import
    // of a name counted only when the same class also assigned it, so
    // `class _Fast(_Base): from cfg import model` left `self.model` in
    // `_Base` as `gpt-4o`. Every binding form in a class body is now a
    // write of that attribute for the class, read from subclasses and bases
    // alike: only a class's single literal assignment is confirmed.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    let base = "class _Base:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n";
    for binding in [
        "from cfg import model",
        "import model",
        "class model:\n        pass",
        "def model(self):\n        return \"o1-mini\"",
        "async def model(self):\n        return \"o1-mini\"",
        "for model in [\"o1-mini\"]:\n        pass",
        "with open(\"m\") as model:\n        pass",
        "try:\n        pass\n    except Exception as model:\n        pass",
        "(model := \"o1-mini\")",
        "match \"o1-mini\":\n        case str() as model:\n            pass",
        "model: str = \"o1-mini\"",
        "model = \"o1-mini\"\n    del model",
        "model += \"-mini\"",
    ] {
        let source = format!("{header}{base}class _Fast(_Base):\n    {binding}\n");
        let rows = keyed_rows(&source, "model");
        assert_eq!(rows.len(), 1, "{source}");
        assert!(rows[0].1.is_empty(), "{source}");
        assert_eq!(rows[0].2.len(), 1, "{source}");
    }
    // The reading class binds the name while its base assigns it.
    let source = format!(
        "{header}class _Base:\n    model = \"gpt-4o\"\nclass _Fast(_Base):\n    from cfg import model\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n    def by_name(self):\n        return client.chat.completions.create(model=_Fast.model, messages=[])\n"
    );
    let rows = keyed_rows(&source, "model");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.1.is_empty() && r.2.len() == 1),
        "{rows:?}"
    );
    // A client a subclass imports into its body.
    let source = "from openai import OpenAI\nclass _Svc:\n    client = OpenAI()\n    def go(self):\n        return self.client.chat.completions.create(model=\"gpt-4o\", messages=[])\nclass _Proxy(_Svc):\n    from clients import client\n";
    let rows = routes_and_gaps(&usages(source));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, Routing::Unknown);
    assert!(!rows[0].2.is_empty());
    // Inside the grammar: a base read by its own name, and a class whose
    // body binds the name once.
    let source = format!(
        "{header}class _Base:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=_Base.model, messages=[])\nclass _Fast(_Base):\n    from cfg import model\nclass _Fine:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
    );
    assert_eq!(
        keyed_rows(&source, "model"),
        vec![
            (6, vec!["gpt-4o".to_string()], vec![]),
            (12, vec!["gpt-4o".to_string()], vec![]),
        ]
    );
}

#[test]
fn environb_module_aliases_and_configure_under_any_name_are_seen() {
    // Should-fix 6: `os.environb`, which shares its data with `os.environ`
    // on POSIX, and `setattr(os, "environ", {...})` left a direct client
    // with no gap.
    let client =
        "client = OpenAI()\nclient.chat.completions.create(model=\"gpt-4o\", messages=[])\n";
    for setup in [
        "import os\nos.environb[b\"OPENAI_BASE_URL\"] = b\"https://proxy.internal/v1\"\n",
        "import os\nos.environb.update({b\"OPENAI_BASE_URL\": b\"https://proxy.internal/v1\"})\n",
        "import os\nsetattr(os, \"environ\", {\"OPENAI_BASE_URL\": \"https://proxy.internal/v1\"})\n",
        "from os import environb\nenvironb[b\"OPENAI_BASE_URL\"] = b\"https://proxy.internal/v1\"\n",
        "import os\nE = os.environb\nE[b\"OPENAI_BASE_URL\"] = b\"https://proxy.internal/v1\"\n",
    ] {
        let source = format!("from openai import OpenAI\n{setup}{client}");
        let rows = routes_and_gaps(&usages(&source));
        assert_eq!(rows.len(), 1, "{source}");
        assert_eq!(rows[0].1, Routing::Unknown, "{source}");
        assert!(!rows[0].2.is_empty(), "{source}");
    }
    // Should-fix 7: `builtins` and `sys` under another name, and the
    // exotic nits: `__import__("builtins")`, `import_module("builtins")`
    // and a frame's `f_globals`.
    let read = "client.chat.completions.create(model=MODEL, messages=[])\n";
    for setup in [
        "import builtins as B\nB.exec(\"MODEL = 'o1-mini'\")\n",
        "import sys as S\nsetattr(S.modules[__name__], \"MODEL\", \"o1-mini\")\n",
        "__import__(\"builtins\").exec(\"MODEL = 'o1-mini'\")\n",
        "import importlib\nimportlib.import_module(\"builtins\").exec(\"MODEL = 'o1-mini'\")\n",
        "import inspect\ninspect.currentframe().f_globals[\"MODEL\"] = \"o1-mini\"\n",
        "import sys\ndef poke():\n    sys._getframe(1).f_globals[\"MODEL\"] = \"o1-mini\"\npoke()\n",
        "import sys\ns = sys\nsetattr(s.modules[__name__], \"MODEL\", \"o1-mini\")\n",
    ] {
        let source = format!("from openai import OpenAI\nMODEL = \"gpt-4o\"\n{setup}{client}{read}");
        let rows = keyed_rows(&source, "model");
        assert!(
            rows[1].2.contains(&"dynamic_feature".to_string()),
            "{source}"
        );
    }
    // `sys` read through its attributes stays inside the grammar.
    let source = format!(
        "import sys\nfrom openai import OpenAI\nMODEL = \"gpt-4o\"\nprint(\"go\", file=sys.stderr)\n{client}{read}"
    );
    assert_eq!(
        keyed_rows(&source, "model")[1],
        (7, vec!["gpt-4o".to_string()], vec![])
    );
    // Should-fix 8: the legacy `configure` under another name.
    let call =
        "model = genai.GenerativeModel(\"gemini-1.5-pro\")\nmodel.generate_content(\"hi\")\n";
    for setup in [
        "from google.generativeai import configure as setup\nsetup(client_options={\"api_endpoint\": \"proxy.internal\"})\n",
        "cfg = genai.configure\ncfg(client_options={\"api_endpoint\": \"proxy.internal\"})\n",
        "OPTS = {\"client_options\": {\"api_endpoint\": \"proxy.internal\"}}\nsetup = genai.configure\nsetup(**OPTS)\n",
    ] {
        let source = format!("import google.generativeai as genai\n{setup}{call}");
        let found = usages(&source);
        let last = found.calls.last().unwrap();
        assert_eq!(last.routing, Routing::Unknown, "{source}");
        assert!(
            found.gaps.iter().any(|g| g.reason == "endpoint_option"
                && g.span.map(|s| s.start_byte) == Some(last.span.start_byte)),
            "{source}"
        );
    }
    let source = format!(
        "import os\nimport google.generativeai as genai\ngenai.configure(api_key=os.environ[\"GEMINI_API_KEY\"])\n{call}"
    );
    let found = usages(&source);
    assert_eq!(
        found.calls.last().unwrap().routing,
        Routing::GoogleGenerativeaiLegacy
    );
    assert!(found.gaps.is_empty());
}

#[test]
fn exports_built_from_names_mangled_getattr_and_exported_private_classes_count() {
    // Nits: `__all__ = _EXPORTS` (or `+=`/`extend` with a name) may export
    // any module-level name; `getattr(_A(), "_A__go")` calls `A.__go` from
    // outside under its mangled name; a `_Private` class listed in
    // `__all__` may be subclassed by other modules.
    let header = "from openai import OpenAI\nclient = OpenAI()\n";
    let private = "def _ask(model=\"gpt-4o\"):\n    return client.chat.completions.create(model=model, messages=[])\n";
    for exports in [
        "_EXPORTS = [\"_ask\"]\n__all__ = _EXPORTS\n",
        "__all__ = []\n__all__ += PUBLIC\n",
        "__all__ = []\n__all__.extend(names)\n",
    ] {
        let source = format!("{header}{exports}{private}");
        let rows = keyed_rows(&source, "model");
        assert_eq!(
            rows.last().unwrap().2,
            vec!["unseen_callers".to_string()],
            "{source}"
        );
    }
    let source = format!(
        "{header}class _A:\n    def __go(self, model=\"gpt-4o\"):\n        return client.chat.completions.create(model=model, messages=[])\n    def run(self):\n        return self.__go()\ngetattr(_A(), \"_A__go\")(\"o1-mini\")\n"
    );
    assert_eq!(
        keyed_rows(&source, "model")[0].2,
        vec!["unseen_callers".to_string()]
    );
    let source = format!(
        "{header}__all__ = [\"_Bot\"]\nclass _Bot:\n    model = \"gpt-4o\"\n    def go(self):\n        return client.chat.completions.create(model=self.model, messages=[])\n"
    );
    assert_eq!(
        keyed_rows(&source, "model")[0].2,
        vec!["subclass_override".to_string()]
    );
    // Inside the grammar: a literal `__all__` that lists other names.
    let source = format!("{header}__all__ = [\"ask\"]\n{private}def ask():\n    return _ask()\n");
    assert_eq!(
        keyed_rows(&source, "model"),
        vec![(5, vec!["gpt-4o".to_string()], vec![])]
    );
}

// ---------------------------------------------------------------------------
// LangChain chat models
// ---------------------------------------------------------------------------

/// The call recorded on `line`.
fn call_on(found: &FileUsages, line: u32) -> &SdkCall {
    found
        .calls
        .iter()
        .find(|c| c.span.line == line)
        .unwrap_or_else(|| {
            let lines: Vec<(u32, &str)> = found
                .calls
                .iter()
                .map(|c| (c.span.line, c.chain.as_str()))
                .collect();
            panic!("no call on line {line}; calls: {lines:?}")
        })
}

/// `kind/reason` of every gap on `line`.
fn gaps_on(found: &FileUsages, line: u32) -> Vec<String> {
    found
        .gaps
        .iter()
        .filter(|g| g.span.is_some_and(|s| s.line == line))
        .map(|g| format!("{}/{}", g.kind.as_str(), g.reason))
        .collect()
}

#[test]
fn langchain_chat_models_bind_their_model_and_route_like_their_client() {
    let found = usages(
        r#"
from langchain_openai import ChatOpenAI, AzureChatOpenAI as Azure
from langchain_anthropic import ChatAnthropic

MODEL = "gpt-4o-mini"
llm = ChatOpenAI(model="gpt-4o", temperature=0)
small = ChatOpenAI(model_name=MODEL, max_tokens=256)
claude = ChatAnthropic(model_name="claude-3-5-sonnet-20241022")
azure = Azure(azure_deployment="prod-4o", model="gpt-4o", api_version="2024-06-01")
"#,
    );
    let import = |name: &str| found.imports.iter().find(|i| i.local == name).unwrap();
    assert_eq!(import("ChatOpenAI").package, "langchain_openai");
    assert_eq!(import("ChatOpenAI").sdk, Sdk::Openai);
    assert_eq!(import("ChatOpenAI").kind, BindingKind::ClientClass);
    assert_eq!(import("Azure").name.as_deref(), Some("AzureChatOpenAI"));
    assert_eq!(import("ChatAnthropic").package, "langchain_anthropic");
    assert_eq!(import("ChatAnthropic").sdk, Sdk::Anthropic);

    assert_eq!(found.clients[0].constructor, "langchain_openai.ChatOpenAI");
    assert_eq!(found.clients[0].package, "langchain_openai");
    assert_eq!(routing_of(&found, "llm"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "small"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "claude"), Routing::AnthropicDirect);
    assert_eq!(routing_of(&found, "azure"), Routing::AzureOpenai);

    // The constructor binds the model its requests use.
    let llm = call_on(&found, 6);
    assert_eq!(llm.chain, "ChatOpenAI");
    assert_eq!(llm.member, "ChatOpenAI");
    assert_eq!(llm.package, "langchain_openai");
    assert_eq!(llm.sdk, Sdk::Openai);
    assert!(!llm.request);
    assert_eq!(llm.receiver, Receiver::Symbol);
    assert_eq!(llm.routing, Routing::OpenaiDirect);
    assert_eq!(llm.client_line, Some(6));
    assert_eq!(llm.kwargs, vec!["model", "temperature"]);
    assert_eq!(value(llm, "model"), Some("gpt-4o"));
    assert_eq!(llm.values[0].origin, ValueOrigin::Literal);

    let small = call_on(&found, 7);
    assert_eq!(value(small, "model_name"), Some("gpt-4o-mini"));
    assert_eq!(small.values[0].origin, ValueOrigin::ModuleConstant);
    assert_eq!(small.values[0].name.as_deref(), Some("MODEL"));

    let claude = call_on(&found, 8);
    assert_eq!(claude.sdk, Sdk::Anthropic);
    assert_eq!(claude.routing, Routing::AnthropicDirect);
    assert_eq!(
        value(claude, "model_name"),
        Some("claude-3-5-sonnet-20241022")
    );

    let azure = call_on(&found, 9);
    assert_eq!(azure.chain, "Azure");
    assert_eq!(azure.member, "AzureChatOpenAI");
    assert_eq!(azure.routing, Routing::AzureOpenai);
    assert_eq!(value(azure, "model"), Some("gpt-4o"));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn langchain_endpoints_route_by_host() {
    let found = usages(
        r#"
import os
from langchain_openai import ChatOpenAI
from langchain_anthropic import ChatAnthropic

router = ChatOpenAI(model="gpt-4o", base_url="https://openrouter.ai/api/v1")
legacy_name = ChatOpenAI(model="gpt-4o", openai_api_base="http://localhost:4000/v1")
own_host = ChatOpenAI(model="gpt-4o", openai_api_base="https://api.openai.com/v1")
none = ChatOpenAI(model="gpt-4o", base_url=None)
proxy = ChatAnthropic(model="claude-3-5-haiku-20241022", anthropic_api_url="http://litellm:4000")
env = ChatOpenAI(model="gpt-4o", base_url=os.environ["GATEWAY_URL"])

def build(options):
    return ChatOpenAI(**options)
"#,
    );
    assert_eq!(routing_of(&found, "router"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "legacy_name"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "own_host"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "none"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "proxy"), Routing::CustomBaseUrl);
    assert_eq!(call_on(&found, 6).routing, Routing::CustomBaseUrl);
    assert_eq!(call_on(&found, 10).routing, Routing::CustomBaseUrl);
    for line in [6, 7, 8, 9, 10] {
        assert!(gaps_on(&found, line).is_empty(), "line {line}");
    }
    // An environment lookup with no fallback: not known.
    assert_eq!(call_on(&found, 11).routing, Routing::Unknown);
    assert_eq!(
        gaps_on(&found, 11),
        vec!["ambiguous_client/base_url_env".to_string()]
    );
    // `**options` may hold the model and the endpoint.
    let built = call_on(&found, 14);
    assert_eq!(built.routing, Routing::Unknown);
    let mut gaps = gaps_on(&found, 14);
    gaps.sort();
    assert_eq!(
        gaps,
        vec![
            "ambiguous_client/base_url_unresolved".to_string(),
            "unresolved_model/splat".to_string(),
        ]
    );
}

#[test]
fn langchain_endpoint_variables_leave_the_route_unconfirmed() {
    for (var, openai, anthropic) in [
        ("OPENAI_API_BASE", true, false),
        ("OPENAI_BASE_URL", true, false),
        ("ANTHROPIC_API_URL", false, true),
        ("ANTHROPIC_BASE_URL", false, true),
        ("LANGSMITH_GATEWAY", true, true),
    ] {
        let source = format!(
            "import os\nfrom langchain_openai import ChatOpenAI\nfrom langchain_anthropic import ChatAnthropic\nos.environ[\"{var}\"] = \"http://gateway:4000\"\nllm = ChatOpenAI(model=\"gpt-4o\")\nclaude = ChatAnthropic(model=\"claude-3-5-haiku-20241022\")\nrouted = ChatOpenAI(model=\"gpt-4o\", base_url=\"https://api.openai.com/v1\")\n"
        );
        let found = usages(&source);
        let endpoint_env = vec!["ambiguous_client/endpoint_env".to_string()];
        let (routing, gaps) = if openai {
            (Routing::Unknown, endpoint_env.clone())
        } else {
            (Routing::OpenaiDirect, Vec::new())
        };
        assert_eq!(call_on(&found, 5).routing, routing, "{var}");
        assert_eq!(gaps_on(&found, 5), gaps, "{var}");
        let (routing, gaps) = if anthropic {
            (Routing::Unknown, endpoint_env.clone())
        } else {
            (Routing::AnthropicDirect, Vec::new())
        };
        assert_eq!(call_on(&found, 6).routing, routing, "{var}");
        assert_eq!(gaps_on(&found, 6), gaps, "{var}");
        // An endpoint given to the constructor wins over the environment.
        assert_eq!(call_on(&found, 7).routing, Routing::OpenaiDirect, "{var}");
        assert!(gaps_on(&found, 7).is_empty(), "{var}");
    }
    // `OPENAI_API_BASE` is LangChain's: the SDK's own client ignores it.
    let found = usages(
        "import os\nfrom openai import OpenAI\nos.environ[\"OPENAI_API_BASE\"] = \"http://gateway:4000\"\nclient = OpenAI()\nclient.chat.completions.create(model=\"gpt-4o\", messages=[])\n",
    );
    assert_eq!(call_on(&found, 5).routing, Routing::OpenaiDirect);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn langchain_models_that_are_missing_or_given_twice_are_gaps() {
    let found = usages(
        r#"
from langchain_openai import ChatOpenAI, AzureChatOpenAI
from langchain_anthropic import ChatAnthropic

default = ChatOpenAI(temperature=0)
both = ChatOpenAI(model="gpt-4o", model_name="gpt-4o-mini")
claude = ChatAnthropic(model=pick_model())
azure = AzureChatOpenAI(azure_deployment="prod-4o")
"#,
    );
    // `ChatOpenAI()` uses LangChain's default model.
    let missing = gaps(&found, GapKind::UnresolvedModel)
        .into_iter()
        .find(|g| g.span.unwrap().line == 5)
        .unwrap();
    assert_eq!(missing.reason, "missing_model");
    assert_eq!(missing.key.as_deref(), Some("model"));
    // Given both names, either value may be the one LangChain keeps: both
    // are listed, and a gap with no key leaves both unconfirmed.
    let both = call_on(&found, 6);
    assert_eq!(value(both, "model"), Some("gpt-4o"));
    assert_eq!(value(both, "model_name"), Some("gpt-4o-mini"));
    let twice = found
        .gaps
        .iter()
        .find(|g| g.span.unwrap().line == 6)
        .unwrap();
    assert_eq!(twice.kind, GapKind::UnresolvedModel);
    assert_eq!(twice.reason, "reassigned");
    assert_eq!(twice.key, None);
    assert!(twice.chain.is_some());
    assert_eq!(gaps_on(&found, 6).len(), 1);
    assert_eq!(
        gaps_on(&found, 7),
        vec!["unresolved_model/function_call".to_string()]
    );
    // An Azure chat model serves its deployment: no model is not a gap.
    assert!(gaps_on(&found, 8).is_empty(), "{:?}", gaps_on(&found, 8));
    assert_eq!(call_on(&found, 8).routing, Routing::AzureOpenai);

    // The other name may come through `**`: from a same-file dict, or from
    // an expansion the file cannot read. A second `model=` there would be a
    // `TypeError`, but `model_name` is not.
    let found = usages(
        r#"
from langchain_openai import ChatOpenAI

DEFAULTS = {"model_name": "gpt-4o-mini", "temperature": 0}
OPTIONS = {"temperature": 0}

def build(**kwargs):
    return ChatOpenAI(model="gpt-4o", **kwargs)

both = ChatOpenAI(model="gpt-4o", **DEFAULTS)
plain = ChatOpenAI(model="gpt-4o", **OPTIONS)
"#,
    );
    let unresolved = |line: u32| -> Vec<(String, Option<String>)> {
        found
            .gaps
            .iter()
            .filter(|g| g.span.unwrap().line == line && g.kind == GapKind::UnresolvedModel)
            .map(|g| (g.reason.clone(), g.key.clone()))
            .collect()
    };
    assert_eq!(value(call_on(&found, 8), "model"), Some("gpt-4o"));
    assert_eq!(
        unresolved(8),
        vec![("splat".to_string(), Some("model".to_string()))]
    );
    assert_eq!(
        value(call_on(&found, 10), "model_name"),
        Some("gpt-4o-mini")
    );
    assert_eq!(unresolved(10), vec![("reassigned".to_string(), None)]);
    assert!(unresolved(11).is_empty(), "{:?}", found.gaps);
}

#[test]
fn deprecated_langchain_chat_models_are_tracked_by_class() {
    let found = usages(
        r#"
from langchain.chat_models import ChatOpenAI, ChatAnthropic
from langchain_community.chat_models.openai import ChatOpenAI as CommunityOpenAI
from langchain_community.chat_models import AzureChatOpenAI, ChatOllama
import langchain_community.chat_models as chat_models

old = ChatOpenAI(model_name="gpt-3.5-turbo", temperature=0)
claude = ChatAnthropic(model="claude-2.1")
community = CommunityOpenAI(model="gpt-4")
azure = AzureChatOpenAI(deployment_name="gpt35", openai_api_version="2023-05-15")
local = ChatOllama(model="llama3")
followed_not = chat_models.ChatOpenAI(model="gpt-4-0613")
"#,
    );
    let import = |name: &str| found.imports.iter().find(|i| i.local == name);
    assert_eq!(import("ChatOpenAI").unwrap().package, "langchain");
    assert_eq!(
        import("ChatOpenAI").unwrap().module,
        "langchain.chat_models"
    );
    assert_eq!(import("ChatAnthropic").unwrap().sdk, Sdk::Anthropic);
    assert_eq!(
        import("CommunityOpenAI").unwrap().package,
        "langchain_community"
    );
    assert_eq!(
        import("CommunityOpenAI").unwrap().name.as_deref(),
        Some("ChatOpenAI")
    );
    assert!(import("ChatOllama").is_none());
    assert!(import("chat_models").is_none());

    let old = call_on(&found, 7);
    assert_eq!(old.package, "langchain");
    assert_eq!(old.routing, Routing::OpenaiDirect);
    assert_eq!(value(old, "model_name"), Some("gpt-3.5-turbo"));
    assert_eq!(found.clients[0].constructor, "langchain.ChatOpenAI");
    let claude = call_on(&found, 8);
    assert_eq!(claude.sdk, Sdk::Anthropic);
    assert_eq!(claude.routing, Routing::AnthropicDirect);
    assert_eq!(value(claude, "model"), Some("claude-2.1"));
    assert_eq!(call_on(&found, 9).package, "langchain_community");
    assert_eq!(value(call_on(&found, 9), "model"), Some("gpt-4"));
    assert_eq!(call_on(&found, 10).routing, Routing::AzureOpenai);
    // Other providers' chat models, and the re-exporting modules
    // themselves, are not followed.
    assert_eq!(found.calls.len(), 4, "{:?}", found.calls);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);

    // A file that names no SDK in lower case is still read; one that uses
    // only LangChain's other packages is not.
    let source = "from langchain.chat_models import ChatOpenAI\nllm = ChatOpenAI(model_name=\"gpt-3.5-turbo\")\n";
    assert!(super::may_use_sdk(source));
    assert!(!super::may_use_sdk(
        "from langchain_core.prompts import ChatPromptTemplate\nfrom langchain_ollama import ChatOllama\n"
    ));
    let found = usages(source);
    assert_eq!(
        value(call_on(&found, 2), "model_name"),
        Some("gpt-3.5-turbo")
    );
}

#[test]
fn deprecated_langchain_openai_follows_the_module_configuration() {
    let found = usages(
        r#"
import openai
from langchain.chat_models import ChatOpenAI
from langchain_community.chat_models import ChatOpenAI as CommunityOpenAI
from langchain_openai import ChatOpenAI as PartnerOpenAI

openai.api_type = "azure"
openai.api_base = "https://example.openai.azure.com/"

old = ChatOpenAI(model_name="gpt-35-turbo")
community = CommunityOpenAI(model_name="gpt-35-turbo")
given = ChatOpenAI(model_name="gpt-35-turbo", openai_api_base="http://localhost:4000")
partner = PartnerOpenAI(model="gpt-4o")
"#,
    );
    // On the pre-1.0 SDK their requests go through the module-level API.
    for line in [10, 11] {
        assert_eq!(
            call_on(&found, line).routing,
            Routing::Unknown,
            "line {line}"
        );
        assert_eq!(
            gaps_on(&found, line),
            vec!["ambiguous_client/langchain_module_config".to_string()],
            "line {line}"
        );
    }
    assert_eq!(call_on(&found, 12).routing, Routing::CustomBaseUrl);
    assert!(gaps_on(&found, 12).is_empty());
    // `langchain_openai` needs openai>=1, whose module-level settings do
    // not reach a client built with its own options.
    assert_eq!(call_on(&found, 13).routing, Routing::OpenaiDirect);
    assert!(gaps_on(&found, 13).is_empty());

    // With no routing setting they are direct.
    let found = usages(
        "import openai\nfrom langchain.chat_models import ChatOpenAI\nopenai.api_key = \"placeholder\"\nllm = ChatOpenAI(model_name=\"gpt-4\")\n",
    );
    assert_eq!(call_on(&found, 4).routing, Routing::OpenaiDirect);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn only_langchain_chat_models_and_their_objects_are_followed() {
    let found = usages(
        r#"
import langchain_openai
from langchain_openai import OpenAIEmbeddings, ChatOpenAI
from langchain_anthropic import convert_to_anthropic_tool

module_form = langchain_openai.ChatOpenAI(model="gpt-4o")
embeddings = OpenAIEmbeddings(model="text-embedding-ada-002")
other = langchain_openai.OpenAIEmbeddings(model="text-embedding-3-small")
tool = convert_to_anthropic_tool(search)
checked = ChatOpenAI.model_validate({"model": "gpt-4"})

llm = ChatOpenAI(model="gpt-4o-mini", base_url="https://openrouter.ai/api/v1")
llm.invoke("hi")
structured = llm.with_structured_output(Answer)
structured.invoke("hi")
gateway.messages.create(model="claude-3-haiku-20240307", messages=[])
"#,
    );
    let module_form = call_on(&found, 6);
    assert_eq!(module_form.chain, "langchain_openai.ChatOpenAI");
    assert_eq!(module_form.receiver, Receiver::Module);
    assert_eq!(value(module_form, "model"), Some("gpt-4o"));
    let import = |name: &str| found.imports.iter().find(|i| i.local == name);
    assert_eq!(
        import("langchain_openai").unwrap().kind,
        BindingKind::Module
    );
    assert!(import("OpenAIEmbeddings").is_none());
    assert!(import("convert_to_anthropic_tool").is_none());
    for line in [7, 8, 9, 10, 15, 16] {
        assert!(
            found.calls.iter().all(|c| c.span.line != line),
            "line {line}: {:?}",
            found.calls
        );
    }
    // A call on the chat model keeps its routing.
    let invoke = call(&found, "llm.invoke");
    assert_eq!(invoke.receiver, Receiver::Client);
    assert_eq!(invoke.routing, Routing::CustomBaseUrl);
    assert_eq!(invoke.client_line, Some(12));
    assert_eq!(invoke.package, "langchain_openai");
    assert!(!invoke.request);
    assert!(invoke.values.is_empty());
    // Importing LangChain's Anthropic package does not make an unknown
    // receiver's `messages.create` the Anthropic SDK's.
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn langchain_chat_models_rerouted_or_replaced_after_they_are_built() {
    let found = usages(
        r#"
from langchain_openai import ChatOpenAI
from langchain_anthropic import ChatAnthropic

llm = ChatOpenAI(model="gpt-4o")
llm.openai_api_base = "http://localhost:4000"
llm.invoke("hi")

claude = ChatAnthropic(model="claude-3-5-haiku-20241022")
claude._client = Recorder()
claude.invoke("hi")
"#,
    );
    assert_eq!(call(&found, "llm.invoke").routing, Routing::Unknown);
    assert_eq!(
        gaps_on(&found, 7),
        vec!["ambiguous_client/base_url_assigned".to_string()]
    );
    assert_eq!(
        gaps_on(&found, 11),
        vec!["ambiguous_client/dynamic_feature".to_string()]
    );
    // The constructors route the same way; neither write touches the model.
    assert_eq!(
        gaps_on(&found, 5),
        vec!["ambiguous_client/base_url_assigned".to_string()]
    );
    assert_eq!(
        gaps_on(&found, 9),
        vec!["ambiguous_client/dynamic_feature".to_string()]
    );

    // A copy may change any field: it is not followed.
    let found = usages(
        r#"
from langchain_openai import ChatOpenAI

llm = ChatOpenAI(model="gpt-4o")
local = llm.copy(update={"openai_api_base": "http://localhost:4000"})
local.invoke("hi")
"#,
    );
    assert!(found.calls.iter().all(|c| c.chain != "local.invoke"));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);

    // Its requests read the model field when they are sent: a write of it,
    // or of an attribute named at run time, after it is built.
    for write in [
        "llm.model_name = \"gpt-5\"",
        "settings.model = choose()",
        "setattr(llm, field, value)",
    ] {
        let source = format!(
            "from langchain_openai import ChatOpenAI\nllm = ChatOpenAI(model=\"gpt-4o\")\n{write}\nllm.invoke(\"hi\")\n"
        );
        let found = usages(&source);
        let built = call_on(&found, 2);
        assert_eq!(value(built, "model"), Some("gpt-4o"), "{write}");
        let rewritten = found
            .gaps
            .iter()
            .find(|g| g.span.unwrap().line == 2 && g.kind == GapKind::UnresolvedModel)
            .unwrap_or_else(|| panic!("{write}: {:?}", found.gaps));
        assert_eq!(rewritten.reason, "dynamic_feature", "{write}");
        assert_eq!(rewritten.key.as_deref(), Some("model"), "{write}");
        assert!(rewritten.detail.contains("line 3"), "{}", rewritten.detail);
    }
    // `configurable_fields` on the model, or `configurable_alternatives`,
    // choose it at run time; another field made configurable does not.
    for (made, gapped) in [
        (
            "configurable_fields(model_name=ConfigurableField(id=\"model\"))",
            true,
        ),
        (
            "configurable_alternatives(ConfigurableField(id=\"llm\"), default_key=\"openai\")",
            true,
        ),
        (
            "configurable_fields(temperature=ConfigurableField(id=\"t\"))",
            false,
        ),
    ] {
        let source = format!(
            "from langchain_openai import ChatOpenAI\nllm = ChatOpenAI(model=\"gpt-4o\")\nchooser = llm.{made}\n"
        );
        let found = usages(&source);
        let model_gaps: Vec<&UsageGap> = found
            .gaps
            .iter()
            .filter(|g| g.span.unwrap().line == 2 && g.kind == GapKind::UnresolvedModel)
            .collect();
        assert_eq!(
            model_gaps.len(),
            usize::from(gapped),
            "{made}: {model_gaps:?}"
        );
        if gapped {
            assert_eq!(model_gaps[0].reason, "dynamic_feature");
            assert!(
                model_gaps[0].detail.contains("line 3"),
                "{}",
                model_gaps[0].detail
            );
        }
    }

    // An SDK client given to the constructor decides where requests go.
    let found = usages(
        r#"
from openai import OpenAI
from langchain_openai import ChatOpenAI

gateway = OpenAI(base_url="http://localhost:4000/v1")
llm = ChatOpenAI(model="gpt-4o", client=gateway.chat.completions, root_client=gateway)
"#,
    );
    let built = call_on(&found, 6);
    assert_eq!(built.routing, Routing::Unknown);
    assert_eq!(value(built, "model"), Some("gpt-4o"));
    assert_eq!(
        gaps_on(&found, 6),
        vec!["ambiguous_client/given_client".to_string()]
    );

    // LangChain's run options never reach the request: a model key there
    // is not a model value.
    let found = usages(
        r#"
from langchain_anthropic import ChatAnthropic

llm = ChatAnthropic(model="claude-sonnet-5-5", metadata={"model": "claude-3-opus-20240229"}, tags=["x"])
llm.invoke("hi", config={"configurable": {"model": "gpt-3.5-turbo"}})
llm.invoke("hi", {"metadata": {"model": "gpt-4"}})
tuned = llm.with_config(metadata={"model_name": "gpt-4"}, configurable={"model": "gpt-4"})
"#,
    );
    let keys: Vec<(u32, &str, &str)> = found
        .calls
        .iter()
        .flat_map(|c| {
            c.values
                .iter()
                .map(move |v| (c.span.line, v.key.as_str(), v.value.as_str()))
        })
        .collect();
    assert_eq!(keys, vec![(4, "model", "claude-sonnet-5-5")]);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);

    // A write on `self` of a same-file class is that class's attribute.
    let found = usages(
        "from langchain_openai import ChatOpenAI\nclass Bot:\n    def __init__(self):\n        self.model = \"gpt-4o\"\n        self.llm = ChatOpenAI(model=\"gpt-4o\")\n",
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

// ---------------------------------------------------------------------------
// Amazon Bedrock (boto3, LangChain) and LiteLLM
// ---------------------------------------------------------------------------

const MODEL_ID: &str = "us.anthropic.claude-sonnet-4-5-20250929-v1:0";

#[test]
fn bedrock_runtime_clients_route_bedrock_for_any_model() {
    let found = usages(
        r#"
import boto3
from boto3 import client as make_client
from boto3.session import Session

MODEL = "us.anthropic.claude-sonnet-4-5-20250929-v1:0"
runtime = boto3.client("bedrock-runtime", region_name="us-east-1")
other = make_client(service_name="bedrock-runtime")
pooled = boto3.Session(profile_name="dev").client("bedrock-runtime")
s3 = boto3.client("s3")
control = boto3.client("bedrock")

runtime.converse(modelId=MODEL, messages=[], inferenceConfig={"maxTokens": 512})
runtime.invoke_model(modelId="amazon.nova-pro-v1:0", body="{}")
other.converse_stream(modelId="meta.llama3-70b-instruct-v1:0", messages=[])
pooled.invoke_model_with_response_stream(modelId="anthropic.claude-3-haiku-20240307-v1:0", body="{}")
Session().client("bedrock-runtime").converse(modelId="mistral.mistral-large-2402-v1:0", messages=[])
s3.put_object(Bucket="b", Key="k", Body=b"")
control.list_foundation_models()
boto3.resource("dynamodb")
"#,
    );
    let import = |name: &str| found.imports.iter().find(|i| i.local == name).unwrap();
    assert_eq!(import("boto3").sdk, Sdk::Aws);
    assert_eq!(import("boto3").package, "boto3");
    assert_eq!(import("boto3").kind, BindingKind::Module);
    assert_eq!(import("make_client").kind, BindingKind::ClientClass);
    assert_eq!(import("Session").kind, BindingKind::Symbol);

    // Only the `bedrock-runtime` clients are followed.
    assert_eq!(found.clients.len(), 4, "{:?}", found.clients);
    assert!(found
        .clients
        .iter()
        .all(|c| c.constructor == "boto3.client" && c.sdk == Sdk::Aws));
    assert_eq!(routing_of(&found, "runtime"), Routing::Bedrock);
    assert_eq!(routing_of(&found, "other"), Routing::Bedrock);
    assert_eq!(routing_of(&found, "pooled"), Routing::Bedrock);

    let converse = call(&found, "runtime.converse");
    assert_eq!(converse.sdk, Sdk::Aws);
    assert_eq!(converse.package, "boto3");
    assert_eq!(converse.member, "converse");
    assert!(converse.request);
    assert_eq!(converse.receiver, Receiver::Client);
    assert_eq!(converse.routing, Routing::Bedrock);
    assert_eq!(converse.client_line, Some(7));
    assert_eq!(value(converse, "modelId"), Some(MODEL_ID));
    assert_eq!(converse.values[0].origin, ValueOrigin::ModuleConstant);
    assert_eq!(converse.values[0].model_family, Some(Sdk::Anthropic));
    // The other request fields name no model.
    assert_eq!(converse.values.len(), 1);

    for (chain, model) in [
        ("runtime.invoke_model", "amazon.nova-pro-v1:0"),
        ("other.converse_stream", "meta.llama3-70b-instruct-v1:0"),
        (
            "pooled.invoke_model_with_response_stream",
            "anthropic.claude-3-haiku-20240307-v1:0",
        ),
        (
            "Session().client(...).converse",
            "mistral.mistral-large-2402-v1:0",
        ),
    ] {
        let request = call(&found, chain);
        assert!(request.request, "{chain}");
        assert_eq!(request.routing, Routing::Bedrock, "{chain}");
        assert_eq!(value(request, "modelId"), Some(model), "{chain}");
    }
    // Other services and boto3's other functions are not followed.
    assert_eq!(found.calls.len(), 5, "{:?}", found.calls);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn bedrock_service_names_resolve_under_the_value_rules() {
    let found = usages(
        r#"
import os
import boto3

SERVICE = "bedrock-runtime"
named = boto3.client(SERVICE)
chosen = boto3.client(os.getenv("SERVICE", "bedrock-runtime"))

def ask(brt, model_id):
    return brt.converse(modelId=model_id, messages=[])

named.converse(modelId="amazon.titan-text-express-v1", messages=[])
chosen.converse(modelId="amazon.titan-text-lite-v1", messages=[])
"#,
    );
    assert_eq!(routing_of(&found, "named"), Routing::Bedrock);
    assert_eq!(call_on(&found, 12).routing, Routing::Bedrock);
    // A service name read from the environment is not followed, and a
    // parameter is not a known client: Bedrock-shaped calls on either
    // are gaps.
    assert_eq!(found.clients.len(), 1);
    let unknown = gaps(&found, GapKind::UnknownClientFactory);
    let reasons: Vec<(u32, &str)> = unknown
        .iter()
        .map(|g| (g.span.unwrap().line, g.reason.as_str()))
        .collect();
    assert_eq!(reasons, vec![(10, "parameter"), (13, "sdk_call_result")]);
    assert!(unknown.iter().all(|g| g.sdk == Some(Sdk::Aws)));
    assert_eq!(unknown[1].values[0].key, "modelId");
    assert_eq!(unknown[1].values[0].value, "amazon.titan-text-lite-v1");

    // Without a boto3 import, `converse` is not taken for Bedrock's.
    let found = usages(
        "from chatkit import bot\nimport openai\n\ndef run(agent):\n    agent.converse(modelId=\"x\")\n",
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn bedrock_model_values_and_gaps() {
    let found = usages(
        r#"
import os
import boto3

client = boto3.client("bedrock-runtime")
PARAMS = {"modelId": "amazon.titan-text-express-v1", "body": "{}"}
client.invoke_model(**PARAMS)
client.converse(messages=[])
client.converse(**load_params())
client.converse(modelId=os.getenv("MODEL_ID", "anthropic.claude-3-haiku-20240307-v1:0"), messages=[])
client.converse(modelId="amazon.nova-lite-v1:0", model="gpt-4o", requestMetadata={"model": "gpt-4o"}, additionalModelRequestFields={"model": "x"})
"#,
    );
    let expanded = call_on(&found, 7);
    assert_eq!(
        value(expanded, "modelId"),
        Some("amazon.titan-text-express-v1")
    );
    assert_eq!(expanded.values[0].origin, ValueOrigin::ModuleConstant);
    assert_eq!(
        expanded.values[0].name.as_deref(),
        Some("PARAMS[\"modelId\"]")
    );
    assert!(gaps_on(&found, 7).is_empty());
    // No `modelId`: botocore rejects the request, no default applies.
    assert!(call_on(&found, 8).values.is_empty());
    assert!(gaps_on(&found, 8).is_empty());
    // An expansion the file cannot read may hold the model.
    let splat = gaps(&found, GapKind::UnresolvedModel)
        .into_iter()
        .find(|g| g.span.unwrap().line == 9)
        .unwrap();
    assert_eq!(splat.reason, "splat");
    assert_eq!(splat.key.as_deref(), Some("modelId"));
    let env = call_on(&found, 10);
    assert_eq!(
        value(env, "modelId"),
        Some("anthropic.claude-3-haiku-20240307-v1:0")
    );
    assert_eq!(env.values[0].origin, ValueOrigin::EnvDefault);
    assert_eq!(
        gaps_on(&found, 10),
        vec!["unresolved_model/env_var".to_string()]
    );
    // Only `modelId` is a Bedrock request's model: botocore rejects
    // `model=`, and the other fields do not name the model the request
    // goes to.
    let only = call_on(&found, 11);
    let keys: Vec<(&str, &str)> = only
        .values
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_str()))
        .collect();
    assert_eq!(keys, vec![("modelId", "amazon.nova-lite-v1:0")]);
}

#[test]
fn bedrock_endpoints_route_by_host() {
    let found = usages(
        r#"
import os
import boto3

aws = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime.us-west-2.amazonaws.com")
vpce = boto3.client("bedrock-runtime", endpoint_url="https://vpce-0a1b2c3d.bedrock-runtime.us-east-1.vpce.amazonaws.com")
fips = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime-fips.us-gov-west-1.amazonaws.com")
none = boto3.client("bedrock-runtime", endpoint_url=None)
proxy = boto3.client("bedrock-runtime", endpoint_url="http://localhost:4000/bedrock")
lookalike = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime.amazonaws.com.example.net")
s3_host = boto3.client("bedrock-runtime", endpoint_url="https://s3.us-east-1.amazonaws.com")
env = boto3.client("bedrock-runtime", endpoint_url=os.environ["BEDROCK_URL"])
env.converse(modelId="amazon.nova-pro-v1:0", messages=[])

def build(options):
    built = boto3.client("bedrock-runtime", **options)
    return built.converse(modelId="amazon.nova-pro-v1:0", messages=[])
"#,
    );
    for name in ["aws", "vpce", "fips", "none"] {
        assert_eq!(routing_of(&found, name), Routing::Bedrock, "{name}");
    }
    for name in ["proxy", "lookalike", "s3_host"] {
        assert_eq!(routing_of(&found, name), Routing::CustomBaseUrl, "{name}");
    }
    assert_eq!(routing_of(&found, "env"), Routing::Unknown);
    assert_eq!(call_on(&found, 13).routing, Routing::Unknown);
    assert_eq!(
        gaps_on(&found, 13),
        vec!["ambiguous_client/base_url_env".to_string()]
    );
    let env_gap = &gaps(&found, GapKind::AmbiguousClient)[0];
    assert!(
        env_gap.detail.contains("`endpoint_url`"),
        "{}",
        env_gap.detail
    );
    // `**options` may hold an `endpoint_url`.
    assert_eq!(call_on(&found, 17).routing, Routing::Unknown);
    assert_eq!(
        gaps_on(&found, 17),
        vec!["ambiguous_client/base_url_unresolved".to_string()]
    );

    // `endpoint_url` is boto3's sixth positional argument: by position or
    // through `*args` it is not read. A bucket named `bedrock-runtime` on
    // S3, or a host with no region, is not Bedrock's.
    let found = usages(
        r#"
import boto3

positional = boto3.client("bedrock-runtime", "us-east-1", None, True, None, "https://proxy.example.com")
positional.converse(modelId="amazon.nova-pro-v1:0", messages=[])
spread = boto3.client("bedrock-runtime", *REGION_AND_ENDPOINT)
spread.converse(modelId="amazon.nova-pro-v1:0", messages=[])
region = boto3.client("bedrock-runtime", "us-east-1")
bucket = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime.s3.amazonaws.com")
no_region = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime.amazonaws.com")
dual = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime.us-east-1.api.aws")
china = boto3.client("bedrock-runtime", endpoint_url="https://bedrock-runtime.cn-north-1.amazonaws.com.cn")
"#,
    );
    for line in [5, 7] {
        assert_eq!(call_on(&found, line).routing, Routing::Unknown, "{line}");
        assert_eq!(
            gaps_on(&found, line),
            vec!["ambiguous_client/base_url_unresolved".to_string()],
            "{line}"
        );
    }
    assert_eq!(routing_of(&found, "region"), Routing::Bedrock);
    assert_eq!(routing_of(&found, "bucket"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "no_region"), Routing::CustomBaseUrl);
    assert_eq!(routing_of(&found, "dual"), Routing::Bedrock);
    assert_eq!(routing_of(&found, "china"), Routing::Bedrock);
}

#[test]
fn bedrock_endpoint_variables_leave_the_route_unconfirmed() {
    for (var, boto, langchain) in [
        ("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", true, true),
        ("AWS_ENDPOINT_URL", true, true),
        ("LANGSMITH_GATEWAY", false, true),
        ("OPENAI_BASE_URL", false, false),
    ] {
        let source = format!(
            "import os\nimport boto3\nfrom langchain_aws import ChatBedrockConverse\nos.environ[\"{var}\"] = \"http://gateway:4000\"\nclient = boto3.client(\"bedrock-runtime\")\nclient.converse(modelId=\"amazon.nova-pro-v1:0\", messages=[])\nllm = ChatBedrockConverse(model=\"amazon.nova-pro-v1:0\")\nfixed = boto3.client(\"bedrock-runtime\", endpoint_url=\"https://bedrock-runtime.eu-west-1.amazonaws.com\")\nfixed.converse(modelId=\"amazon.nova-pro-v1:0\", messages=[])\n"
        );
        let found = usages(&source);
        let endpoint_env = vec!["ambiguous_client/endpoint_env".to_string()];
        for (line, affected) in [(6, boto), (7, langchain)] {
            let (routing, gaps) = if affected {
                (Routing::Unknown, endpoint_env.clone())
            } else {
                (Routing::Bedrock, Vec::new())
            };
            assert_eq!(call_on(&found, line).routing, routing, "{var} {line}");
            assert_eq!(gaps_on(&found, line), gaps, "{var} {line}");
        }
        // An endpoint given to the constructor wins over the environment.
        assert_eq!(call_on(&found, 9).routing, Routing::Bedrock, "{var}");
        assert!(gaps_on(&found, 9).is_empty(), "{var}");
    }
}

#[test]
fn langchain_bedrock_models_bind_their_model() {
    let found = usages(
        r#"
import boto3
from langchain_aws import ChatBedrock, ChatBedrockConverse
from langchain_aws.chat_models.bedrock import ChatBedrock as Legacy
from langchain_aws import BedrockEmbeddings

llm = ChatBedrock(model_id="anthropic.claude-3-5-sonnet-20240620-v1:0", region_name="us-east-1", provider="anthropic")
conv = ChatBedrockConverse(model="us.amazon.nova-pro-v1:0", temperature=0)
legacy = Legacy(model_id="meta.llama3-1-70b-instruct-v1:0")
both = ChatBedrockConverse(model="amazon.nova-lite-v1:0", model_id="amazon.nova-pro-v1:0")
opaque = ChatBedrock(model_id="amazon.nova-pro-v1:0", **settings)
proxy = ChatBedrockConverse(model="amazon.nova-pro-v1:0", base_url="http://localhost:4000")
given = ChatBedrock(model_id="amazon.nova-pro-v1:0", client=boto3.client("bedrock-runtime", endpoint_url=URL))
missing = ChatBedrock(region_name="us-east-1")
embed = BedrockEmbeddings(model_id="amazon.titan-embed-text-v2:0")
llm.invoke("hi", config={"metadata": {"model_id": "x"}})
"#,
    );
    let import = |name: &str| found.imports.iter().find(|i| i.local == name).unwrap();
    assert_eq!(import("ChatBedrock").package, "langchain_aws");
    assert_eq!(import("ChatBedrock").sdk, Sdk::Aws);
    assert_eq!(import("Legacy").name.as_deref(), Some("ChatBedrock"));
    assert!(found.imports.iter().all(|i| i.local != "BedrockEmbeddings"));

    let llm = call_on(&found, 7);
    assert_eq!(llm.chain, "ChatBedrock");
    assert_eq!(llm.sdk, Sdk::Aws);
    assert_eq!(llm.package, "langchain_aws");
    assert!(!llm.request);
    assert_eq!(llm.routing, Routing::Bedrock);
    assert_eq!(
        value(llm, "model_id"),
        Some("anthropic.claude-3-5-sonnet-20240620-v1:0")
    );
    assert!(gaps_on(&found, 7).is_empty());
    assert_eq!(
        value(call_on(&found, 8), "model"),
        Some("us.amazon.nova-pro-v1:0")
    );
    assert_eq!(call_on(&found, 8).routing, Routing::Bedrock);
    assert_eq!(
        value(call_on(&found, 9), "model_id"),
        Some("meta.llama3-1-70b-instruct-v1:0")
    );
    // Given its model under both names, which one LangChain keeps is not
    // confirmed; next to an opaque `**`, the other name may be in it.
    assert_eq!(
        gaps_on(&found, 10),
        vec!["unresolved_model/reassigned".to_string()]
    );
    assert_eq!(gaps(&found, GapKind::UnresolvedModel)[0].key, None);
    let mut opaque = gaps_on(&found, 11);
    opaque.sort();
    assert_eq!(
        opaque,
        vec![
            "ambiguous_client/base_url_unresolved".to_string(),
            "unresolved_model/splat".to_string(),
        ]
    );
    assert_eq!(call_on(&found, 12).routing, Routing::CustomBaseUrl);
    assert!(gaps_on(&found, 12).is_empty());
    assert_eq!(call_on(&found, 13).routing, Routing::Unknown);
    assert!(gaps_on(&found, 13).contains(&"ambiguous_client/given_client".to_string()));
    // Only its own fields are read: no `model_name` or `engine`.
    let found_names = usages(
        "from langchain_aws import ChatBedrock\nllm = ChatBedrock(model_id=\"amazon.nova-pro-v1:0\", model_name=\"x\", engine=\"y\")\n",
    );
    let keys: Vec<&str> = call_on(&found_names, 2)
        .values
        .iter()
        .map(|v| v.key.as_str())
        .collect();
    assert_eq!(keys, vec!["model_id"]);
    // `model_id` is required: no default applies, so no gap.
    assert!(gaps_on(&found, 14).is_empty());
    // Embeddings are not tracked; run options are not read.
    assert!(found.calls.iter().all(|c| c.span.line != 15));
    let invoke = call_on(&found, 16);
    assert_eq!(invoke.routing, Routing::Bedrock);
    assert!(invoke.values.is_empty());

    // A call on the built model may name the model the Converse API sends
    // instead (`model_id=`, or `modelId=`); `model=` is not a parameter
    // there. `ChatBedrock` sends it only when it runs on the Converse API.
    let found = usages(
        r#"
from langchain_aws import ChatBedrock, ChatBedrockConverse
llm = ChatBedrockConverse(model_id="anthropic.claude-3-5-sonnet-20240620-v1:0")

def ask(q):
    return llm.invoke(q, model_id="anthropic.claude-v2")

def ask_any(q, **kw):
    return llm.invoke(q, **kw)

old = ChatBedrock(model_id="anthropic.claude-3-haiku-20240307-v1:0")
old.invoke("hi", modelId="amazon.nova-pro-v1:0")
llm.invoke("hi", model="amazon.nova-lite-v1:0")
"#,
    );
    assert_eq!(
        value(call_on(&found, 6), "model_id"),
        Some("anthropic.claude-v2")
    );
    assert!(gaps_on(&found, 6).is_empty());
    assert!(call_on(&found, 9).values.is_empty());
    assert_eq!(
        gaps_on(&found, 9),
        vec!["unresolved_model/splat".to_string()]
    );
    let splat = gaps(&found, GapKind::UnresolvedModel)
        .into_iter()
        .find(|g| g.span.unwrap().line == 9)
        .unwrap();
    assert_eq!(splat.key.as_deref(), Some("model_id"));
    assert_eq!(
        value(call_on(&found, 12), "modelId"),
        Some("amazon.nova-pro-v1:0")
    );
    assert_eq!(
        gaps_on(&found, 12),
        vec!["unresolved_model/conditional".to_string()]
    );
    assert!(call_on(&found, 13).values.is_empty());
    assert!(gaps_on(&found, 13).is_empty());

    // Its requests read `model_id` when they are sent.
    let found = usages(
        "from langchain_aws import ChatBedrock\nllm = ChatBedrock(model_id=\"amazon.nova-pro-v1:0\")\nllm.model_id = pick()\n",
    );
    let rewritten = gaps(&found, GapKind::UnresolvedModel);
    assert_eq!(rewritten.len(), 1, "{rewritten:?}");
    assert_eq!(rewritten[0].reason, "dynamic_feature");
    assert_eq!(rewritten[0].key.as_deref(), Some("model_id"));
}

#[test]
fn litellm_completion_calls_route_litellm() {
    let found = usages(
        r#"
import litellm
from litellm import completion, acompletion as achat
from litellm.main import completion as main_completion
from litellm import Router

completion(model="gpt-4o", messages=[])
litellm.completion("anthropic/claude-3-5-sonnet-20241022", messages=[])
achat(model="gemini/gemini-1.5-pro", messages=[])
main_completion(model="o3-mini", messages=[], fallbacks=[{"model": "gpt-4o-mini"}])
litellm.completion(messages=[])
litellm.completion(**params)
litellm.token_counter(model="gpt-4o", messages=[])
litellm.supports_function_calling(model="gpt-4o")
litellm.embedding(model="text-embedding-3-small", input=["x"])
Router(model_list=[]).completion(model="gpt-4o", messages=[])
"#,
    );
    let import = |name: &str| found.imports.iter().find(|i| i.local == name).unwrap();
    assert_eq!(import("litellm").sdk, Sdk::Litellm);
    assert_eq!(import("completion").kind, BindingKind::Symbol);
    assert_eq!(import("achat").name.as_deref(), Some("acompletion"));
    assert!(found.imports.iter().all(|i| i.local != "Router"));

    let lines: Vec<u32> = found.calls.iter().map(|c| c.span.line).collect();
    assert_eq!(lines, vec![7, 8, 9, 10, 11, 12]);
    for c in &found.calls {
        assert_eq!(c.sdk, Sdk::Litellm, "{}", c.chain);
        assert_eq!(c.package, "litellm");
        assert_eq!(c.routing, Routing::Litellm, "{}", c.chain);
        assert!(c.request, "{}", c.chain);
    }
    assert_eq!(value(call_on(&found, 7), "model"), Some("gpt-4o"));
    assert_eq!(call_on(&found, 7).receiver, Receiver::Symbol);
    let positional = call_on(&found, 8);
    assert_eq!(positional.receiver, Receiver::Module);
    assert_eq!(
        value(positional, "model"),
        Some("anthropic/claude-3-5-sonnet-20241022")
    );
    assert!(positional.values[0].positional);
    assert_eq!(
        value(call_on(&found, 9), "model"),
        Some("gemini/gemini-1.5-pro")
    );
    let fallbacks = call_on(&found, 10);
    assert_eq!(value(fallbacks, "model"), Some("o3-mini"));
    assert_eq!(value(fallbacks, "fallbacks.model"), Some("gpt-4o-mini"));
    // A missing model is a TypeError, not a default.
    assert!(gaps_on(&found, 11).is_empty());
    assert_eq!(
        gaps_on(&found, 12),
        vec!["unresolved_model/splat".to_string()]
    );
    assert_eq!(found.gaps.len(), 1, "{:?}", found.gaps);

    // `model_alias_map` may map the model, and a fallback's, to another one.
    let found = usages(
        "import litellm\nlitellm.model_alias_map = {\"fast\": \"gpt-4o-mini\"}\nlitellm.completion(model=\"gpt-4o\", messages=[], fallbacks=[{\"model\": \"fast\"}])\n",
    );
    assert_eq!(value(call_on(&found, 3), "model"), Some("gpt-4o"));
    assert_eq!(value(call_on(&found, 3), "fallbacks.model"), Some("fast"));
    let alias = gaps(&found, GapKind::UnresolvedModel);
    assert_eq!(alias.len(), 1, "{alias:?}");
    assert_eq!(alias[0].reason, "dynamic_feature");
    assert_eq!(alias[0].key, None);
    assert!(alias[0].detail.contains("line 2"), "{}", alias[0].detail);

    // A `model_list` (given, in a same-file dict or possibly in an opaque
    // `**`) maps the model name to a deployment; its entries are not
    // models the call sends.
    let found = usages(
        r#"
import litellm
CFG = {"model_list": [{"model_name": "gpt-4o", "litellm_params": {"model": "anthropic/claude-3-opus-20240229"}}]}
litellm.completion(model="gpt-4o", messages=[], model_list=[{"model_name": "gpt-4o", "litellm_params": {"model": "anthropic/claude-3-opus-20240229"}}])
litellm.completion(model="gpt-4o", messages=[], **CFG)
litellm.completion(model="gpt-4o", messages=[], **options)
litellm.completion(model="gpt-4o", messages=[])
"#,
    );
    for line in [4, 5, 6, 7] {
        let keys: Vec<&str> = call_on(&found, line)
            .values
            .iter()
            .map(|v| v.key.as_str())
            .collect();
        assert_eq!(keys, vec!["model"], "{line}");
    }
    assert_eq!(
        gaps_on(&found, 4),
        vec!["unresolved_model/model_list".to_string()]
    );
    assert_eq!(
        gaps_on(&found, 5),
        vec!["unresolved_model/model_list".to_string()]
    );
    assert_eq!(
        gaps_on(&found, 6),
        vec!["unresolved_model/splat".to_string()]
    );
    assert!(gaps_on(&found, 7).is_empty());
    assert!(
        found.gaps.iter().all(|g| g.key.is_none()),
        "{:?}",
        found.gaps
    );
}

#[test]
fn bedrock_and_litellm_members_replaced_or_hooked_are_dynamic() {
    for patch in [
        "client.converse = fake",
        "boto3.client = fake",
        "client.meta.events.register(\"before-call.*\", hook)",
        "boto3.Session().events.register(\"before-call.*\", hook)",
    ] {
        let source = format!(
            "import boto3\nclient = boto3.client(\"bedrock-runtime\")\nclient.converse(modelId=\"amazon.nova-pro-v1:0\", messages=[])\n{patch}\n"
        );
        let found = usages(&source);
        assert_eq!(
            gaps_on(&found, 3),
            vec!["ambiguous_client/dynamic_feature".to_string()],
            "{patch}"
        );
        let detail = &gaps(&found, GapKind::AmbiguousClient)[0].detail;
        assert!(detail.contains("line 4"), "{patch}: {detail}");
    }
    let found = usages(
        "import litellm\nlitellm.completion(model=\"gpt-4o\", messages=[])\nlitellm.completion = traced(litellm.completion)\n",
    );
    assert_eq!(
        gaps_on(&found, 2),
        vec!["ambiguous_client/dynamic_feature".to_string()]
    );

    // A handler registered where this file cannot follow it counts by its
    // botocore event name, and so does a default session set up from a
    // botocore session; a `register` call that names no botocore event
    // does not.
    for (setup, hooked) in [
        (
            "boto3._get_default_session().events.register(\"before-parameter-build.bedrock-runtime.Converse\", hook)",
            true,
        ),
        (
            "boto3.Session()._session.register(\"provide-client-params.bedrock-runtime.Converse\", hook)",
            true,
        ),
        (
            "botocore.session.get_session().register(event_name=\"before-call.*\", handler=hook)",
            true,
        ),
        ("boto3.setup_default_session(botocore_session=custom)", true),
        ("boto3.setup_default_session(profile_name=\"dev\")", false),
        ("atexit.register(cleanup)", false),
        ("admin.site.register(Model)", false),
        ("codecs.register(\"search\")", false),
    ] {
        let source = format!(
            "import atexit\nimport boto3\nimport botocore.session\n{setup}\nclient = boto3.client(\"bedrock-runtime\")\nclient.converse(modelId=\"amazon.nova-pro-v1:0\", messages=[])\n"
        );
        let found = usages(&source);
        let gaps = if hooked {
            vec!["ambiguous_client/dynamic_feature".to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(gaps_on(&found, 6), gaps, "{setup}");
        assert_eq!(call_on(&found, 6).routing, Routing::Bedrock, "{setup}");
    }
}

#[test]
fn bedrock_and_litellm_files_pass_the_prefilter() {
    use super::may_use_sdk;
    for source in [
        "import boto3\nclient = boto3.client(\"bedrock-runtime\")\n",
        "def run(client):\n    return client.invoke_model(modelId=M, body=b)\n",
        "from langchain_aws import ChatBedrock\n",
        "from litellm import completion\n",
        "import boto3\ndef ask(client):\n    return client.converse(modelId=M, messages=[])\n",
        "import boto3\ndef ask(client):\n    return client.converse_stream(modelId=M, messages=[])\n",
    ] {
        assert!(may_use_sdk(source), "{source}");
    }
    // A Bedrock call on a client passed in is a gap where boto3 is imported.
    let found = usages(
        "import boto3\ndef ask(client):\n    return client.converse_stream(modelId=\"amazon.nova-pro-v1:0\", messages=[])\n",
    );
    assert_eq!(
        gaps_on(&found, 3),
        vec!["unknown_client_factory/parameter".to_string()]
    );
    // Bedrock's and LiteLLM's request names are not another SDK's.
    let found = usages(
        "import openai\nfrom openai import OpenAI\nclient = OpenAI()\nclient.converse(model=\"gpt-4o\")\nopenai.acompletion(model=\"gpt-4o\")\n",
    );
    assert!(found.calls.iter().all(|c| !c.request), "{:?}", found.calls);
    // A file using boto3 for anything else is not parsed.
    assert!(!may_use_sdk(
        "import boto3\ns3 = boto3.client(\"s3\")\ns3.put_object(Bucket=\"b\", Key=\"k\", Body=b\"\")\n"
    ));
}

// ---------------------------------------------------------------------------
// Raw HTTP requests
// ---------------------------------------------------------------------------

#[test]
fn raw_http_posts_to_provider_apis_read_the_json_body() {
    let found = usages(
        r#"import json
import requests
import httpx as hx
from requests import post

OPENAI_URL = "https://api.openai.com/v1/chat/completions"

def ask(prompt):
    data = {"model": "claude-3-haiku-20240307", "max_tokens": 200, "messages": []}
    return requests.post("https://api.anthropic.com/v1/messages", headers=H, json=data)

def ask_inline(prompt, key):
    return requests.post(url="https://api.openai.com/v1/chat/completions", headers={"authorization": key}, json={"model": "gpt-3.5-turbo", "messages": []})

def ask_httpx(prompt):
    body = {"model": "gpt-4o", "input": prompt}
    return hx.post("https://api.openai.com/v1/responses/", content=json.dumps(body))

def ask_bare():
    return post(OPENAI_URL, data=json.dumps({"model": "gpt-4-turbo", "messages": []}), timeout=5)
"#,
    );
    assert!(found.imports.is_empty(), "{:?}", found.imports);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);

    let anthropic = call_on(&found, 10);
    assert_eq!(anthropic.chain, "requests.post");
    assert_eq!(anthropic.package, "requests");
    assert_eq!(anthropic.sdk, Sdk::Anthropic);
    assert_eq!(anthropic.routing, Routing::AnthropicDirect);
    assert_eq!(anthropic.member, "messages.create");
    assert!(anthropic.request);
    assert_eq!(anthropic.receiver, Receiver::Module);
    assert_eq!(anthropic.kwargs, ["headers", "json"]);
    assert_eq!(value(anthropic, "model"), Some("claude-3-haiku-20240307"));
    let captured = &anthropic.values[0];
    assert_eq!(captured.origin, ValueOrigin::LocalConstant);
    assert_eq!(captured.name.as_deref(), Some("data[\"model\"]"));
    assert_eq!(captured.model_family, Some(Sdk::Anthropic));

    let inline = call_on(&found, 13);
    assert_eq!(inline.routing, Routing::OpenaiDirect);
    assert_eq!(inline.member, "chat.completions.create");
    assert_eq!(value(inline, "model"), Some("gpt-3.5-turbo"));
    assert_eq!(inline.values[0].origin, ValueOrigin::Literal);
    assert_eq!(inline.values.len(), 1, "{:?}", inline.values);

    let responses = call_on(&found, 17);
    assert_eq!(responses.package, "httpx");
    assert_eq!(responses.chain, "hx.post");
    assert_eq!(responses.member, "responses.create");
    assert_eq!(value(responses, "model"), Some("gpt-4o"));

    let bare = call_on(&found, 20);
    assert_eq!(bare.receiver, Receiver::Symbol);
    assert_eq!(bare.member, "chat.completions.create");
    assert_eq!(value(bare, "model"), Some("gpt-4-turbo"));
}

#[test]
fn raw_http_to_other_urls_is_not_recorded() {
    for url in [
        "\"https://api.openai.com/v1/models\"",
        "\"https://openrouter.ai/api/v1/chat/completions\"",
        "\"https://myco.openai.azure.com/openai/deployments/d/chat/completions\"",
        "\"https://api.openai.com.evil.example/v1/chat/completions\"",
        "\"https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent\"",
        "f\"{BASE}/v1/chat/completions\"",
        "os.getenv(\"URL\", \"https://api.openai.com/v1/chat/completions\")",
        "url",
        "\"ftp://api.openai.com/v1/chat/completions\"",
    ] {
        let source = format!(
            "import os\nimport requests\ndef ask(url):\n    return requests.post({url}, json={{\"model\": \"gpt-4o\"}})\n"
        );
        let found = usages(&source);
        assert!(found.calls.is_empty(), "{url}: {:?}", found.calls);
        assert!(found.gaps.is_empty(), "{url}: {:?}", found.gaps);
    }
    // A query string, a fragment, a port and a trailing `/` keep the path.
    for url in [
        "https://api.openai.com/v1/chat/completions?x=1",
        "https://API.openai.com:443/v1/chat/completions/",
        "http://api.openai.com/v1/chat/completions#top",
    ] {
        let source =
            format!("import requests\nrequests.post(\"{url}\", json={{\"model\": \"gpt-4o\"}})\n");
        let found = usages(&source);
        assert_eq!(
            call_on(&found, 2).member,
            "chat.completions.create",
            "{url}"
        );
    }
}

#[test]
fn only_post_imported_from_an_http_library_is_followed() {
    for source in [
        // Not the library: a name assigned, a parameter, another module's
        // `post`, another method.
        "import requests\nrequests = Session()\nrequests.post(URL, json={\"model\": \"gpt-4o\"})\n",
        "def ask(requests):\n    return requests.post(URL, json={\"model\": \"gpt-4o\"})\n",
        "from mylib import post\npost(URL, json={\"model\": \"gpt-4o\"})\n",
        "from requests import get as post\npost(URL, json={\"model\": \"gpt-4o\"})\n",
        "import requests.adapters as requests\nrequests.post(URL, json={\"model\": \"gpt-4o\"})\n",
        "import requests\nrequests.put(URL, json={\"model\": \"gpt-4o\"})\n",
        "import requests\nrequests.Session().post(URL, json={\"model\": \"gpt-4o\"})\n",
        "from requests import *\npost(URL, json={\"model\": \"gpt-4o\"})\n",
    ] {
        let source = format!("URL = \"https://api.openai.com/v1/chat/completions\"\n{source}");
        let found = usages(&source);
        assert!(found.calls.is_empty(), "{source}: {:?}", found.calls);
    }
    for source in [
        "import requests.adapters\nrequests.post(URL, json={\"model\": \"gpt-4o\"})\n",
        "from requests import post as send\nsend(URL, json={\"model\": \"gpt-4o\"})\n",
        "def f():\n    import httpx\n    return httpx.post(URL, json={\"model\": \"gpt-4o\"})\n",
    ] {
        let source = format!("URL = \"https://api.openai.com/v1/chat/completions\"\n{source}");
        let found = usages(&source);
        assert_eq!(found.calls.len(), 1, "{source}");
        assert_eq!(value(&found.calls[0], "model"), Some("gpt-4o"), "{source}");
    }
}

#[test]
fn raw_http_bodies_that_do_not_read_are_gaps() {
    let found = usages(
        r#"import json
import requests
URL = "https://api.openai.com/v1/chat/completions"
MODELS = {"fast": "gpt-4o-mini", "slow": "gpt-4.1"}

def from_param(body):
    return requests.post(URL, json=body)

def mutated():
    data = {"model": "gpt-4o"}
    data["model"] = pick()
    return requests.post(URL, json=data)

def overridable(extra):
    return requests.post(URL, json={"model": "gpt-4o", **extra})

def no_model():
    return requests.post(URL, json={"messages": []})

def string_body(payload):
    return requests.post(URL, data=payload)

def splat(**kw):
    return requests.post(URL, json={"model": "gpt-4o"}, **kw)

def looked_up(kind):
    return requests.post(URL, json={"model": MODELS[kind]})

def batch():
    return requests.post("https://api.anthropic.com/v1/messages/batches", json={"requests": [{"custom_id": "a", "params": {"model": "claude-3-opus-20240229"}}]})

def form():
    return requests.post("https://api.openai.com/v1/audio/transcriptions", files=F, data={"model": "whisper-1"})
"#,
    );
    assert_eq!(gaps_on(&found, 7), ["unresolved_model/request_body"]);
    assert!(call_on(&found, 7).values.is_empty());
    // A mutated dict is not read.
    assert_eq!(gaps_on(&found, 12), ["unresolved_model/request_body"]);
    assert!(call_on(&found, 12).values.is_empty());
    assert_eq!(gaps_on(&found, 15), ["unresolved_model/splat"]);
    let overridable = &gaps(&found, GapKind::UnresolvedModel)
        .into_iter()
        .find(|g| g.span.is_some_and(|s| s.line == 15))
        .unwrap()
        .values;
    assert_eq!(overridable[0].value, "gpt-4o");
    assert_eq!(gaps_on(&found, 18), ["unresolved_model/missing_model"]);
    assert_eq!(gaps_on(&found, 21), ["unresolved_model/request_body"]);
    // `**kw` may hold `data=`, which replaces a `json=` body.
    assert_eq!(gaps_on(&found, 24), ["unresolved_model/splat"]);
    assert_eq!(value(call_on(&found, 24), "model"), Some("gpt-4o"));
    let splat = found
        .gaps
        .iter()
        .find(|g| g.span.is_some_and(|s| s.line == 24))
        .unwrap();
    assert_eq!(splat.key, None);
    // A model looked up with a run-time key lists the dict's values.
    assert_eq!(gaps_on(&found, 27), ["unresolved_model/subscript"]);
    let looked_up = found
        .gaps
        .iter()
        .find(|g| g.span.is_some_and(|s| s.line == 27))
        .unwrap();
    let listed: Vec<&str> = looked_up.values.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(listed, ["gpt-4o-mini", "gpt-4.1"]);
    // A Message Batch nests its models in its requests.
    let batch = call_on(&found, 30);
    assert_eq!(batch.member, "messages.batches.create");
    assert!(gaps_on(&found, 30).is_empty());
    assert_eq!(
        value(batch, "requests.params.model"),
        Some("claude-3-opus-20240229")
    );
    // Form data is not a JSON body.
    assert_eq!(gaps_on(&found, 33), ["unresolved_model/request_body"]);
    assert!(call_on(&found, 33).values.is_empty());
}

#[test]
fn a_dict_passed_to_requests_post_stays_readable() {
    // `requests.post(json=...)` only serializes the dict, so an SDK call
    // that expands it later still reads it.
    let found = usages(
        r#"import requests
from openai import OpenAI
client = OpenAI()
PARAMS = {"model": "gpt-4o", "messages": []}
requests.post("https://api.openai.com/v1/chat/completions", json=PARAMS)
client.chat.completions.create(**PARAMS)
"#,
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
    assert_eq!(value(call_on(&found, 6), "model"), Some("gpt-4o"));
    // Any other `post` may keep or change it.
    let found = usages(
        r#"import requests
from openai import OpenAI
client = OpenAI()
session = make_session()
PARAMS = {"model": "gpt-4o", "messages": []}
session.post("https://api.openai.com/v1/chat/completions", json=PARAMS)
client.chat.completions.create(**PARAMS)
"#,
    );
    assert!(call_on(&found, 7).values.is_empty());
    assert_eq!(gaps_on(&found, 7), ["unresolved_model/splat"]);
}

#[test]
fn a_replaced_post_or_a_dynamic_file_leaves_the_route_unconfirmed() {
    for (setup, reason) in [
        ("requests.post = fake_post", "dynamic_feature"),
        ("setattr(requests, \"post\", fake_post)", "dynamic_feature"),
        ("del requests.post", "dynamic_feature"),
        ("setattr(requests, name, fake)", "dynamic_feature"),
        ("exec(code)", "dynamic_feature"),
    ] {
        let source = format!(
            "import requests\n{setup}\nrequests.post(\"https://api.openai.com/v1/chat/completions\", json={{\"model\": \"gpt-4o\"}})\n"
        );
        let found = usages(&source);
        assert_eq!(
            gaps_on(&found, 3),
            [format!("ambiguous_client/{reason}")],
            "{setup}"
        );
        assert_eq!(call_on(&found, 3).routing, Routing::OpenaiDirect, "{setup}");
    }
}
