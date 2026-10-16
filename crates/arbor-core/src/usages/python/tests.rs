use super::extract;
use crate::usages::{
    BindingKind, ClientScope, FileUsages, GapKind, Receiver, Routing, Sdk, SdkCall, UsageGap,
    ValueOrigin,
};

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
    // Every call is still a usage; only the value is missing.
    assert_eq!(found.calls.len(), 11);
    assert!(found.calls.iter().all(|c| c.values.is_empty()));
    let via = gaps(&found, GapKind::UnresolvedModel)[3];
    assert!(via.detail.contains("ENV_MODEL"), "{}", via.detail);
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
