use super::extract;
use crate::usages::{
    BindingKind, ClientScope, FileUsages, GapKind, Receiver, Routing, Sdk, SdkCall, UsageGap,
    ValueOrigin, VertexAi,
};

fn scan(source: &str, file: &str) -> FileUsages {
    let mut parser = tree_sitter::Parser::new();
    let language = if file.ends_with(".ts") {
        tree_sitter_typescript::language_typescript()
    } else {
        tree_sitter_typescript::language_tsx()
    };
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    extract(&tree, source, file)
}

fn usages(source: &str) -> FileUsages {
    scan(source, "app.ts")
}

fn usages_js(source: &str) -> FileUsages {
    scan(source, "app.js")
}

fn call<'a>(found: &'a FileUsages, chain: &str) -> &'a SdkCall {
    found
        .calls
        .iter()
        .find(|c| c.chain == chain)
        .unwrap_or_else(|| {
            let chains: Vec<&str> = found.calls.iter().map(|c| c.chain.as_str()).collect();
            panic!(
                "no call `{chain}`; calls: {chains:?}; gaps: {:?}",
                found.gaps
            )
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

/// `kind/reason` of every gap at `call`.
fn gaps_at(found: &FileUsages, call: &SdkCall) -> Vec<String> {
    found
        .gaps
        .iter()
        .filter(|g| g.span.map(|s| s.start_byte) == Some(call.span.start_byte))
        .map(|g| format!("{}/{}", g.kind, g.reason))
        .collect()
}

/// The routing and gaps of the only call to `member`.
fn route(source: &str, member: &str) -> (Routing, Vec<String>) {
    let found = usages(source);
    let c = found
        .calls
        .iter()
        .find(|c| c.member == member)
        .unwrap_or_else(|| panic!("no call to {member}: {:?} {:?}", found.calls, found.gaps));
    (c.routing, gaps_at(&found, c))
}

/// The model value and gaps of the only `chat.completions.create` call.
fn model(source: &str) -> (Option<String>, Vec<String>) {
    let found = usages(source);
    let c = found
        .calls
        .iter()
        .find(|c| c.member == "chat.completions.create")
        .unwrap_or_else(|| panic!("no create call: {:?} {:?}", found.calls, found.gaps));
    (value(c, "model").map(str::to_string), gaps_at(&found, c))
}

fn routing_of(found: &FileUsages, binding: &str) -> Routing {
    found
        .clients
        .iter()
        .find(|c| c.binding.as_deref() == Some(binding))
        .unwrap_or_else(|| panic!("no client bound to {binding}: {:?}", found.clients))
        .routing
}

const OPENAI: &str = "import OpenAI from \"openai\";\n";

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

#[test]
fn records_every_import_form() {
    let found = usages(
        r#"
import OpenAI from "openai";
import { AzureOpenAI as AZ, toFile, type ChatCompletion } from "openai";
import * as oa from "openai";
import type { Message } from "@anthropic-ai/sdk/resources/messages";
import Anthropic, { APIError } from "@anthropic-ai/sdk";
import AnthropicBedrock from "@anthropic-ai/bedrock-sdk";
import { AnthropicVertex } from "@anthropic-ai/vertex-sdk";
import { GoogleGenAI } from "@google/genai";
import { GoogleGenerativeAI } from "@google/generative-ai";
import OpenAI2 = require("openai");
import express from "express";
import { helper } from "./local";
"#,
    );
    let local = |name: &str| {
        found
            .imports
            .iter()
            .find(|i| i.local == name)
            .unwrap_or_else(|| panic!("no import bound to {name}: {:?}", found.imports))
    };
    assert_eq!(local("OpenAI").kind, BindingKind::ClientClass);
    assert_eq!(local("OpenAI").name.as_deref(), Some("default"));
    assert_eq!(local("AZ").kind, BindingKind::ClientClass);
    assert_eq!(local("AZ").name.as_deref(), Some("AzureOpenAI"));
    assert_eq!(local("toFile").kind, BindingKind::Symbol);
    assert_eq!(local("ChatCompletion").kind, BindingKind::Symbol);
    assert_eq!(local("oa").kind, BindingKind::Module);
    assert_eq!(local("oa").name, None);
    assert_eq!(local("Message").package, "@anthropic-ai/sdk");
    assert_eq!(local("Message").kind, BindingKind::Symbol);
    assert_eq!(local("Anthropic").sdk, Sdk::Anthropic);
    assert_eq!(local("Anthropic").kind, BindingKind::ClientClass);
    assert_eq!(local("APIError").kind, BindingKind::Symbol);
    assert_eq!(local("AnthropicBedrock").kind, BindingKind::ClientClass);
    assert_eq!(
        local("AnthropicBedrock").package,
        "@anthropic-ai/bedrock-sdk"
    );
    assert_eq!(local("AnthropicVertex").kind, BindingKind::ClientClass);
    assert_eq!(local("GoogleGenAI").sdk, Sdk::Google);
    assert_eq!(local("GoogleGenAI").package, "@google/genai");
    assert_eq!(local("GoogleGenerativeAI").package, "@google/generative-ai");
    assert_eq!(local("OpenAI2").kind, BindingKind::Module);
    assert!(found.imports.iter().all(|i| i.local != "express"));
    assert!(found.imports.iter().all(|i| i.local != "helper"));
    // 1-based span of the local name.
    assert_eq!(local("OpenAI").span.line, 2);
    assert_eq!(local("OpenAI").span.column, 8);
}

#[test]
fn commonjs_and_dynamic_imports_are_imports() {
    let found = usages_js(
        r#"
const OpenAI = require("openai");
const { OpenAI: Named, AzureOpenAI } = require("openai");
const Default = require("openai").default;
const { GoogleGenAI } = require("@google/genai");
const path = require("path");
async function load() {
  const { default: Lazy } = await import("openai");
  const mod = await import("@anthropic-ai/sdk");
  return [Lazy, mod];
}
"#,
    );
    let kind = |name: &str| {
        found
            .imports
            .iter()
            .find(|i| i.local == name)
            .unwrap_or_else(|| panic!("no import {name}: {:?}", found.imports))
            .kind
    };
    // `require("openai")` is the module object, which is also callable as
    // the default client.
    assert_eq!(kind("OpenAI"), BindingKind::Module);
    assert_eq!(kind("Named"), BindingKind::ClientClass);
    assert_eq!(kind("AzureOpenAI"), BindingKind::ClientClass);
    assert_eq!(kind("Default"), BindingKind::ClientClass);
    assert_eq!(kind("GoogleGenAI"), BindingKind::ClientClass);
    assert_eq!(kind("Lazy"), BindingKind::ClientClass);
    assert_eq!(kind("mod"), BindingKind::Module);
    assert!(found.imports.iter().all(|i| i.local != "path"));
    assert_eq!(found.language, "javascript");
}

#[test]
fn deno_and_cdn_specifiers_are_the_package() {
    let found = usages(
        r#"
import OpenAI from "npm:openai@^4.20.0";
import Anthropic from "npm:@anthropic-ai/sdk@0.20.1";
import JsrOpenAI from "jsr:@openai/openai";
import CdnOpenAI from "https://esm.sh/openai@4.28.0";
import DenoOpenAI from "https://deno.land/x/openai@v4.24.0/mod.ts";
"#,
    );
    for name in [
        "OpenAI",
        "Anthropic",
        "JsrOpenAI",
        "CdnOpenAI",
        "DenoOpenAI",
    ] {
        let import = found
            .imports
            .iter()
            .find(|i| i.local == name)
            .unwrap_or_else(|| panic!("no import {name}: {:?}", found.imports));
        assert_eq!(import.kind, BindingKind::ClientClass, "{name}");
    }
}

#[test]
fn files_that_mention_no_sdk_report_nothing() {
    let found = usages("const x = 1;\nfetch('https://example.com');\n");
    assert!(found.is_empty());
    assert_eq!(found.language, "typescript");
}

// ---------------------------------------------------------------------------
// Constructions and routing
// ---------------------------------------------------------------------------

#[test]
fn module_level_openai_client_and_call() {
    let found = usages(
        "import OpenAI from \"openai\";\n\
         const client = new OpenAI();\n\
         const resp = await client.chat.completions.create({ model: \"gpt-3.5-turbo\", messages: [] });\n",
    );
    let built = &found.clients[0];
    assert_eq!(built.constructor, "openai.OpenAI");
    assert_eq!(built.chain, "OpenAI");
    assert_eq!(built.binding.as_deref(), Some("client"));
    assert_eq!(built.scope, ClientScope::Module);
    assert_eq!(built.routing, Routing::OpenaiDirect);
    assert!(!built.is_async);

    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.sdk, Sdk::Openai);
    assert_eq!(c.package, "openai");
    assert_eq!(c.member, "chat.completions.create");
    assert!(c.request);
    assert_eq!(c.receiver, Receiver::Client);
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert_eq!(c.client_line, Some(2));
    assert_eq!(c.kwargs, vec!["model", "messages"]);
    assert_eq!(c.positional, 0);
    assert!(!c.splat);
    assert_eq!(c.symbol, None);
    assert_eq!((c.span.line, c.span.column), (3, 20));
    let model = &c.values[0];
    assert_eq!(model.value, "gpt-3.5-turbo");
    assert_eq!(model.origin, ValueOrigin::Literal);
    assert_eq!(model.model_family, Some(Sdk::Openai));
    assert!(!model.positional);
    // The literal's own span, quotes included.
    assert_eq!((model.span.line, model.span.column), (3, 60));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn base_url_routes_by_host() {
    let cases: &[(&str, Routing, &[&str])] = &[
        ("", Routing::OpenaiDirect, &[]),
        ("baseURL: undefined", Routing::OpenaiDirect, &[]),
        ("baseURL: null", Routing::OpenaiDirect, &[]),
        (
            "baseURL: \"https://api.openai.com/v1\"",
            Routing::OpenaiDirect,
            &[],
        ),
        (
            "baseURL: \"https://openrouter.ai/api/v1\"",
            Routing::CustomBaseUrl,
            &[],
        ),
        (
            "baseURL: `http://localhost:11434/v1`",
            Routing::CustomBaseUrl,
            &[],
        ),
        (
            "baseURL: process.env.OPENAI_BASE_URL",
            Routing::Unknown,
            &["ambiguous_client/base_url_env"],
        ),
        (
            "baseURL: process.env.OPENAI_BASE_URL ?? \"https://api.openai.com/v1\"",
            Routing::OpenaiDirect,
            &["ambiguous_client/base_url_env"],
        ),
        (
            "baseURL: process.env.PROXY || \"https://proxy.example/v1\"",
            Routing::CustomBaseUrl,
            &["ambiguous_client/base_url_env"],
        ),
        (
            "baseURL: `https://${host}/v1`",
            Routing::Unknown,
            &["ambiguous_client/base_url_unresolved"],
        ),
        (
            "baseURL: url || \"https://api.openai.com/v1\"",
            Routing::Unknown,
            &["ambiguous_client/base_url_unresolved"],
        ),
    ];
    for (options, routing, expected) in cases {
        let source = format!(
            "{OPENAI}export function ask(url: string, host: string) {{\n  const client = new OpenAI({{ {options} }});\n  return client.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n}}\n"
        );
        let (got, gaps) = route(&source, "chat.completions.create");
        assert_eq!(got, *routing, "{options}: {gaps:?}");
        assert_eq!(gaps, *expected, "{options}");
    }
}

#[test]
fn a_constant_base_url_is_read() {
    let found = usages(&format!(
        "{OPENAI}const GATEWAY = \"https://gateway.example/v1\";\nconst client = new OpenAI({{ baseURL: GATEWAY }});\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
    ));
    assert_eq!(routing_of(&found, "client"), Routing::CustomBaseUrl);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
    // The URL itself is never reported.
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("gateway.example"));
}

#[test]
fn platform_clients_route_by_class() {
    let found = usages(
        r#"
import OpenAI, { AzureOpenAI, BedrockOpenAI } from "openai";
import { bedrock } from "openai/providers/bedrock";
import Anthropic from "@anthropic-ai/sdk";
import AnthropicBedrock, { AnthropicBedrockMantle } from "@anthropic-ai/bedrock-sdk";
import { AnthropicVertex } from "@anthropic-ai/vertex-sdk";
import AnthropicFoundry from "@anthropic-ai/foundry-sdk";
import { AnthropicAws } from "@anthropic-ai/aws-sdk";

const azure = new AzureOpenAI({ endpoint: "https://x.openai.azure.com", apiVersion: "2024-10-21", deployment: "gpt-4o" });
const brOpenai = new BedrockOpenAI({ awsRegion: "us-east-1" });
const adapter = new OpenAI({ provider: bedrock({ region: "us-east-1" }) });
const resident = new OpenAI({ dataResidency: "eu" });
const claude = new Anthropic();
const bedrockClaude = new AnthropicBedrock({ awsRegion: "us-west-2" });
const mantle = new AnthropicBedrockMantle();
const vertexClaude = new AnthropicVertex({ region: "us-east5", projectId: "p" });
const foundry = new AnthropicFoundry({ resource: "r" });
const aws = new AnthropicAws();
"#,
    );
    assert_eq!(routing_of(&found, "azure"), Routing::AzureOpenai);
    assert_eq!(routing_of(&found, "brOpenai"), Routing::Unknown);
    assert_eq!(routing_of(&found, "adapter"), Routing::Unknown);
    assert_eq!(routing_of(&found, "resident"), Routing::Unknown);
    assert_eq!(routing_of(&found, "claude"), Routing::AnthropicDirect);
    assert_eq!(
        routing_of(&found, "bedrockClaude"),
        Routing::AnthropicBedrock
    );
    assert_eq!(routing_of(&found, "mantle"), Routing::Unknown);
    assert_eq!(routing_of(&found, "vertexClaude"), Routing::AnthropicVertex);
    assert_eq!(routing_of(&found, "foundry"), Routing::Unknown);
    assert_eq!(routing_of(&found, "aws"), Routing::Unknown);
    let built = found
        .clients
        .iter()
        .find(|c| c.binding.as_deref() == Some("bedrockClaude"))
        .unwrap();
    assert_eq!(
        built.constructor,
        "@anthropic-ai/bedrock-sdk.AnthropicBedrock"
    );
    assert_eq!(built.kwargs, vec!["awsRegion"]);
}

#[test]
fn data_residency_is_an_endpoint_option() {
    let (routing, gaps) = route(
        &format!("{OPENAI}const c = new OpenAI({{ dataResidency: \"eu\" }});\nc.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"]);
}

#[test]
fn gemini_clients_and_their_endpoint_options() {
    let source = |options: &str| {
        format!(
            "import {{ GoogleGenAI }} from \"@google/genai\";\nconst ai = new GoogleGenAI({{ {options} }});\nawait ai.models.generateContent({{ model: \"gemini-2.0-flash\", contents: \"hi\" }});\n"
        )
    };
    let cases: &[(&str, Routing, Option<VertexAi>, &[&str])] = &[
        ("apiKey: key", Routing::GoogleGenai, None, &[]),
        (
            "vertexai: false",
            Routing::GoogleGenai,
            Some(VertexAi::Off),
            &[],
        ),
        (
            "vertexai: true, project: \"p\", location: \"us-central1\"",
            Routing::Unknown,
            Some(VertexAi::On),
            &["ambiguous_client/endpoint_option"],
        ),
        (
            "enterprise: true",
            Routing::Unknown,
            Some(VertexAi::On),
            &["ambiguous_client/endpoint_option"],
        ),
        (
            "vertexai: useVertex",
            Routing::Unknown,
            Some(VertexAi::Unknown),
            &["ambiguous_client/endpoint_option"],
        ),
        (
            "httpOptions: { apiVersion: \"v1\" }",
            Routing::GoogleGenai,
            None,
            &[],
        ),
        (
            "httpOptions: { baseUrl: \"https://proxy.example\" }",
            Routing::Unknown,
            None,
            &["ambiguous_client/endpoint_option"],
        ),
        (
            "...options",
            Routing::Unknown,
            None,
            &["ambiguous_client/endpoint_option"],
        ),
    ];
    for (options, routing, vertexai, expected) in cases {
        let found = usages(&source(options));
        assert_eq!(found.clients[0].vertexai, *vertexai, "{options}");
        let c = call(&found, "ai.models.generateContent");
        assert_eq!(c.routing, *routing, "{options}");
        assert_eq!(gaps_at(&found, c), *expected, "{options}");
        assert_eq!(value(c, "model"), Some("gemini-2.0-flash"), "{options}");
    }
}

#[test]
fn legacy_gemini_models_and_chats() {
    let found = usages(
        r#"
import { GoogleGenerativeAI } from "@google/generative-ai";
const genAI = new GoogleGenerativeAI(process.env.GEMINI_API_KEY!);
const model = genAI.getGenerativeModel({ model: "gemini-pro" });
const routed = genAI.getGenerativeModel({ model: "gemini-pro" }, { baseUrl: "https://proxy.example" });
export async function run(prompt: string) {
  const result = await model.generateContent(prompt);
  const chat = model.startChat({ history: [] });
  await chat.sendMessage("hi");
  await routed.generateContent(prompt);
  return result;
}
"#,
    );
    assert_eq!(
        found.clients[0].constructor,
        "@google/generative-ai.GoogleGenerativeAI"
    );
    assert_eq!(found.clients[0].routing, Routing::GoogleGenerativeaiLegacy);
    let binding = found
        .calls
        .iter()
        .find(|c| c.member == "getGenerativeModel")
        .unwrap();
    assert!(!binding.request);
    assert_eq!(value(binding, "model"), Some("gemini-pro"));
    assert!(gaps_at(&found, binding).is_empty());

    let generate = call(&found, "model.generateContent");
    assert!(generate.request);
    assert_eq!(generate.routing, Routing::GoogleGenerativeaiLegacy);
    assert_eq!(generate.client_line, Some(4));
    assert_eq!(generate.positional, 1);
    // The prompt is not an options object: no model gap.
    assert!(gaps_at(&found, generate).is_empty(), "{:?}", found.gaps);

    let send = call(&found, "chat.sendMessage");
    assert_eq!(send.member, "sendMessage");
    assert_eq!(send.routing, Routing::GoogleGenerativeaiLegacy);
    assert!(gaps_at(&found, send).is_empty());

    let routed = call(&found, "routed.generateContent");
    assert_eq!(routed.routing, Routing::Unknown);
    assert_eq!(
        gaps_at(&found, routed),
        vec!["ambiguous_client/endpoint_option"]
    );
}

#[test]
fn gemini_chats_keep_their_client_routing() {
    let found = usages(
        r#"
import { GoogleGenAI } from "@google/genai";
const ai = new GoogleGenAI({});
const chat = ai.chats.create({ model: "gemini-2.5-flash" });
await chat.sendMessage({ message: "hi" });
"#,
    );
    let create = call(&found, "ai.chats.create");
    assert_eq!(value(create, "model"), Some("gemini-2.5-flash"));
    let send = call(&found, "chat.sendMessage");
    assert_eq!(send.routing, Routing::GoogleGenai);
    assert!(send.request);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn openai_v3_api_is_routed_by_its_configuration() {
    let found = usages(
        r#"
import { Configuration, OpenAIApi } from "openai";
const configuration = new Configuration({ apiKey: process.env.OPENAI_API_KEY });
const openai = new OpenAIApi(configuration);
const proxied = new OpenAIApi(new Configuration({ basePath: "https://proxy.example/v1" }));
export async function ask() {
  await openai.createChatCompletion({ model: "gpt-3.5-turbo", messages: [] });
  await proxied.createCompletion({ model: "text-davinci-003", prompt: "x" });
  await openai.createTranscription(file, "whisper-1");
}
"#,
    );
    assert_eq!(routing_of(&found, "openai"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "proxied"), Routing::CustomBaseUrl);
    let chat = call(&found, "openai.createChatCompletion");
    assert_eq!(chat.member, "createChatCompletion");
    assert!(chat.request);
    assert_eq!(value(chat, "model"), Some("gpt-3.5-turbo"));
    assert_eq!(chat.routing, Routing::OpenaiDirect);
    let completion = call(&found, "proxied.createCompletion");
    assert_eq!(completion.routing, Routing::CustomBaseUrl);
    let transcription = call(&found, "openai.createTranscription");
    assert_eq!(value(transcription, "model"), Some("whisper-1"));
    assert!(transcription.values[0].positional);
    // Only the configuration's apiKey name is recorded.
    assert_eq!(found.clients[0].kwargs, vec!["apiKey"]);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn commonjs_clients_resolve() {
    let found = usages_js(
        r#"
const OpenAI = require("openai");
const { AzureOpenAI } = require("openai");
const Anthropic = require("@anthropic-ai/sdk").default;
const { GoogleGenAI } = require("@google/genai");
const a = new OpenAI();
const b = new AzureOpenAI({ apiVersion: "2024-10-21" });
const c = new Anthropic();
const d = new (require("openai"))();
const e = new GoogleGenAI({ apiKey });
const f = new OpenAI.OpenAI();
"#,
    );
    assert_eq!(routing_of(&found, "a"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "b"), Routing::AzureOpenai);
    assert_eq!(routing_of(&found, "c"), Routing::AnthropicDirect);
    assert_eq!(routing_of(&found, "d"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "e"), Routing::GoogleGenai);
    assert_eq!(routing_of(&found, "f"), Routing::OpenaiDirect);
}

#[test]
fn namespace_and_dynamic_import_clients_resolve() {
    let found = usages(
        r#"
import * as oa from "openai";
const a = new oa.OpenAI();
const b = new oa.default();
export async function lazy() {
  const { default: OpenAI } = await import("openai");
  const c = new OpenAI();
  const mod = await import("@anthropic-ai/sdk");
  const d = new mod.Anthropic();
  return [c, d];
}
"#,
    );
    assert_eq!(routing_of(&found, "a"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "b"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "c"), Routing::OpenaiDirect);
    assert_eq!(routing_of(&found, "d"), Routing::AnthropicDirect);
}

#[test]
fn inline_and_exported_clients() {
    let found = usages(&format!(
        "{OPENAI}export const openai = new OpenAI();\nexport default openai;\nexport async function once() {{\n  return new OpenAI().chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n}}\n"
    ));
    assert_eq!(found.clients[0].binding.as_deref(), Some("openai"));
    assert_eq!(found.clients[0].scope, ClientScope::Module);
    assert_eq!(found.clients[1].binding, None);
    assert_eq!(found.clients[1].scope, ClientScope::Inline);
    let c = call(&found, "new OpenAI().chat.completions.create");
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert_eq!(c.symbol.as_deref(), Some("once"));
}

// ---------------------------------------------------------------------------
// Clients held in variables, classes and parameters
// ---------------------------------------------------------------------------

#[test]
fn class_held_clients_resolve_through_this() {
    let found = usages(&format!(
        r#"{OPENAI}import {{ AzureOpenAI }} from "openai";
class Bot {{
  private client = new OpenAI({{ baseURL: "https://openrouter.ai/api/v1" }});
  private azure: AzureOpenAI;
  constructor() {{
    this.azure = new AzureOpenAI({{ apiVersion: "2024-10-21" }});
  }}
  ask() {{
    return this.client.chat.completions.create({{ model: "gpt-4o", messages: [] }});
  }}
  askAzure = async () => this.azure.chat.completions.create({{ model: "gpt-4o", messages: [] }});
}}
"#
    ));
    assert_eq!(found.clients[0].binding.as_deref(), Some("this.client"));
    assert_eq!(found.clients[0].scope, ClientScope::Attribute);
    assert_eq!(found.clients[1].binding.as_deref(), Some("this.azure"));
    let ask = call(&found, "this.client.chat.completions.create");
    assert_eq!(ask.routing, Routing::CustomBaseUrl);
    assert_eq!(ask.symbol.as_deref(), Some("Bot.ask"));
    let azure = call(&found, "this.azure.chat.completions.create");
    assert_eq!(azure.routing, Routing::AzureOpenai);
    // A class field's arrow function has no graph node of its own.
    assert_eq!(azure.symbol.as_deref(), Some("Bot"));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn lazily_built_clients_resolve() {
    let found = usages(&format!(
        r#"{OPENAI}let client: OpenAI | null = null;
function getClient() {{
  if (!client) client = new OpenAI();
  return client;
}}
export async function ask() {{
  getClient();
  return client!.chat.completions.create({{ model: "gpt-4o", messages: [] }});
}}
"#
    ));
    let c = call(&found, "client!.chat.completions.create");
    assert_eq!(c.member, "chat.completions.create");
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn a_name_assigned_two_kinds_of_client_is_ambiguous() {
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}import {{ AzureOpenAI }} from \"openai\";\nlet client = new OpenAI();\nif (process.env.AZURE) client = new AzureOpenAI({{ apiVersion: \"x\" }});\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert_eq!(gaps, vec!["ambiguous_client/multiple_assignments"]);
}

#[test]
fn block_scoped_clients_do_not_leak() {
    let found = usages(&format!(
        r#"{OPENAI}import {{ AzureOpenAI }} from "openai";
const client = new OpenAI();
export function ask(flag: boolean) {{
  if (flag) {{
    const client = new AzureOpenAI({{ apiVersion: "x" }});
    client.models.list();
  }}
  return client.chat.completions.create({{ model: "gpt-4o", messages: [] }});
}}
"#
    ));
    assert_eq!(
        call(&found, "client.models.list").routing,
        Routing::AzureOpenai
    );
    assert_eq!(
        call(&found, "client.chat.completions.create").routing,
        Routing::OpenaiDirect
    );
}

#[test]
fn a_parameter_shadowing_an_import_is_not_the_sdk() {
    let found = usages(
        r#"
import openai from "openai";
export function ask(openai: any) {
  return openai.chat.completions.create({ model: "gpt-4o", messages: [] });
}
"#,
    );
    assert!(found.calls.is_empty(), "{:?}", found.calls);
    let gap = &gaps(&found, GapKind::UnknownClientFactory)[0];
    assert_eq!(gap.reason, "parameter");
    assert_eq!(gap.values[0].value, "gpt-4o");
}

#[test]
fn a_client_passed_by_every_in_file_caller_resolves() {
    let source = |export: &str| {
        format!(
            r#"{OPENAI}const client = new OpenAI({{ baseURL: "https://openrouter.ai/api/v1" }});
{export}async function ask(c, prompt: string) {{
  return c.chat.completions.create({{ model: "gpt-4o", messages: [] }});
}}
ask(client, "a");
ask(client, "b");
"#
        )
    };
    let (routing, gaps) = route(&source(""), "chat.completions.create");
    assert_eq!(routing, Routing::CustomBaseUrl);
    assert!(gaps.is_empty(), "{gaps:?}");
    let (routing, gaps) = route(&source("export "), "chat.completions.create");
    assert_eq!(routing, Routing::CustomBaseUrl);
    assert_eq!(gaps, vec!["ambiguous_client/in_file_callers"]);
}

#[test]
fn a_typed_parameter_is_known_by_its_class_only() {
    let found = usages(
        r#"
import OpenAI, { AzureOpenAI } from "openai";
export class Service {
  constructor(private readonly client: OpenAI, private azure: AzureOpenAI) {}
  ask() {
    return this.client.chat.completions.create({ model: "gpt-4o", messages: [] });
  }
  askAzure() {
    return this.azure.chat.completions.create({ model: "gpt-4o", messages: [] });
  }
}
"#,
    );
    let direct = call(&found, "this.client.chat.completions.create");
    assert_eq!(direct.routing, Routing::Unknown);
    assert_eq!(gaps_at(&found, direct), vec!["ambiguous_client/annotation"]);
    // The model value itself is written at the call.
    assert_eq!(value(direct, "model"), Some("gpt-4o"));
    let azure = call(&found, "this.azure.chat.completions.create");
    assert_eq!(azure.routing, Routing::AzureOpenai);
    assert!(gaps_at(&found, azure).is_empty());
}

#[test]
fn with_options_copies_and_reroutes() {
    let found = usages(&format!(
        r#"{OPENAI}const client = new OpenAI();
client.withOptions({{ maxRetries: 5 }}).chat.completions.create({{ model: "gpt-4o", messages: [] }});
client.withOptions({{ baseURL: "https://proxy.example/v1" }}).chat.completions.create({{ model: "gpt-4o", messages: [] }});
"#
    ));
    let calls: Vec<_> = found
        .calls
        .iter()
        .filter(|c| c.member == "chat.completions.create")
        .collect();
    assert_eq!(calls[0].routing, Routing::OpenaiDirect);
    assert_eq!(calls[1].routing, Routing::CustomBaseUrl);
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn a_client_reconfigured_after_it_is_built_is_unknown() {
    let source = format!(
        "{OPENAI}const client = new OpenAI();\nclient.baseURL = \"https://proxy.example/v1\";\nclient.apiKey = key;\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
    );
    let (routing, gaps) = route(&source, "chat.completions.create");
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/base_url_assigned"]);
}

#[test]
fn the_endpoint_variable_set_in_the_file_reroutes_direct_clients() {
    let set = format!(
        "{OPENAI}process.env.OPENAI_BASE_URL = \"https://proxy.example/v1\";\nconst client = new OpenAI();\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
    );
    let (routing, gaps) = route(&set, "chat.completions.create");
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/endpoint_env"]);

    let wild = format!(
        "{OPENAI}process.env[name] = value;\nconst client = new OpenAI();\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
    );
    let (routing, gaps) = route(&wild, "chat.completions.create");
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/dynamic_feature"]);

    // Another variable changes nothing.
    let other = format!(
        "{OPENAI}process.env.LOG_LEVEL = \"debug\";\nconst client = new OpenAI();\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
    );
    let (routing, gaps) = route(&other, "chat.completions.create");
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn replaced_or_mocked_sdk_members_are_dynamic() {
    for setup in [
        "client.chat.completions.create = jest.fn();",
        "jest.mock(\"openai\");",
        "vi.mock(\"openai\", () => ({ default: Fake }));",
        "vi.spyOn(client.chat.completions, \"create\");",
    ] {
        let source = format!(
            "{OPENAI}import {{ vi }} from \"vitest\";\nconst client = new OpenAI();\n{setup}\nclient.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});\n"
        );
        let (routing, gaps) = route(&source, "chat.completions.create");
        assert_eq!(routing, Routing::Unknown, "{setup}");
        assert_eq!(gaps, vec!["ambiguous_client/dynamic_feature"], "{setup}");
    }
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[test]
fn chains_are_rendered_as_written() {
    let found = usages(&format!(
        r#"{OPENAI}const client = new OpenAI();
await client?.chat.completions.create({{ model: "gpt-4o", messages: [] }});
await (await client.beta.chat.completions.parse({{ model: "gpt-4o-2024-08-06", messages: [] }}));
const stream = client.responses.stream({{ model: "gpt-4.1", input: "hi" }});
await client.chat.completions.create({{ model: "gpt-4o", messages: [] }}).withResponse();
"#
    ));
    let optional = call(&found, "client?.chat.completions.create");
    assert_eq!(optional.member, "chat.completions.create");
    let parse = call(&found, "client.beta.chat.completions.parse");
    assert_eq!(parse.member, "beta.chat.completions.parse");
    assert!(parse.request);
    assert_eq!(value(parse, "model"), Some("gpt-4o-2024-08-06"));
    assert_eq!(
        value(call(&found, "client.responses.stream"), "model"),
        Some("gpt-4.1")
    );
    // `.withResponse()` on the request's promise is not another call.
    assert_eq!(
        found
            .calls
            .iter()
            .filter(|c| c.member == "chat.completions.create")
            .count(),
        2
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn nested_model_keys_are_captured_with_their_path() {
    let found = usages(
        r#"
import Anthropic from "@anthropic-ai/sdk";
const anthropic = new Anthropic();
await anthropic.messages.batches.create({
  requests: [
    { custom_id: "a", params: { model: "claude-3-5-haiku-20241022", max_tokens: 10, messages: [] } },
    { custom_id: "b", params: { model: "claude-sonnet-4-5", max_tokens: 10, messages: [] } },
  ],
});
"#,
    );
    let batch = call(&found, "anthropic.messages.batches.create");
    let paths: Vec<(&str, &str)> = batch
        .values
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_str()))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("requests.params.model", "claude-3-5-haiku-20241022"),
            ("requests.params.model", "claude-sonnet-4-5"),
        ]
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn a_request_with_no_model_is_a_gap() {
    let found = usages(&format!(
        "{OPENAI}const client = new OpenAI();\nclient.chat.completions.create({{ messages: [] }});\nclient.chat.completions.create(params);\nclient.models.list();\n"
    ));
    let calls: Vec<_> = found
        .calls
        .iter()
        .filter(|c| c.member == "chat.completions.create")
        .collect();
    assert_eq!(
        gaps_at(&found, calls[0]),
        vec!["unresolved_model/missing_model"]
    );
    assert_eq!(gaps_at(&found, calls[1]), vec!["unresolved_model/splat"]);
    assert!(calls[1].splat);
    // A request that takes no model has no model gap.
    assert!(gaps_at(&found, call(&found, "client.models.list")).is_empty());
}

#[test]
fn sdk_shaped_calls_on_unknown_receivers_are_gaps() {
    let found = usages(
        r#"
import { openai } from "@/lib/openai";
import { anthropic } from "./clients";
import Anthropic from "@anthropic-ai/sdk";
function build() { return new Anthropic(); }
export async function run(client: any) {
  await openai.chat.completions.create({ model: "gpt-4o", messages: [] });
  await client.responses.create({ model: "gpt-4.1", input: "x" });
  await build().messages.create({ model: "claude-3-opus-20240229", max_tokens: 1, messages: [] });
  await anthropic.messages.create({ model: MODEL, max_tokens: 1, messages: [] });
}
"#,
    );
    let reasons: Vec<(&str, &str)> = gaps(&found, GapKind::UnknownClientFactory)
        .iter()
        .map(|g| (g.chain.as_deref().unwrap_or_default(), g.reason.as_str()))
        .collect();
    assert_eq!(
        reasons,
        vec![
            ("openai.chat.completions.create", "imported_name"),
            ("client.responses.create", "parameter"),
            ("build().messages.create", "factory_call"),
            ("anthropic.messages.create", "imported_name"),
        ]
    );
    let imported = &gaps(&found, GapKind::UnknownClientFactory)[0];
    assert_eq!(imported.sdk, Some(Sdk::Openai));
    assert_eq!(imported.values[0].value, "gpt-4o");
    assert!(!imported.values_complete);
}

#[test]
fn messages_create_counts_only_where_anthropic_is_imported() {
    let twilio = usages(
        r#"
import twilio from "twilio";
const client = twilio(sid, token);
client.messages.create({ body: "hi", from: "+1", to: "+2" });
"#,
    );
    assert!(twilio.gaps.is_empty(), "{:?}", twilio.gaps);
}

#[test]
fn an_unassigned_class_property_is_a_gap() {
    let found = usages(
        r#"
import OpenAI from "openai";
export class Bot {
  openai: OpenAI;
  ask() {
    return this.openai.chat.completions.create({ model: "gpt-4o", messages: [] });
  }
}
"#,
    );
    let gap = &gaps(&found, GapKind::UnknownClientFactory)[0];
    assert_eq!(gap.reason, "unassigned_attribute");
    assert_eq!(
        gap.chain.as_deref(),
        Some("this.openai.chat.completions.create")
    );
}

#[test]
fn sdk_methods_reached_dynamically_are_gaps() {
    let found = usages(&format!(
        r#"{OPENAI}const client = new OpenAI();
const create = client.chat.completions.create;
withRetry(client.chat.completions.create, {{ model: "gpt-4o" }});
const bound = client.chat.completions.create.bind(client.chat.completions);
await client.chat.completions[method]({{ model: "gpt-4o", messages: [] }});
await client["chat"].completions.create({{ model: "gpt-4o", messages: [] }});
"#
    ));
    let reasons: Vec<&str> = gaps(&found, GapKind::DynamicDispatch)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        vec![
            "method_reference",
            "method_reference",
            "method_reference",
            "computed_member"
        ]
    );
    // A computed key that is a string literal is a plain member.
    let literal = call(&found, "client[...].completions.create");
    assert_eq!(literal.member, "chat.completions.create");
    assert_eq!(literal.routing, Routing::OpenaiDirect);
}

#[test]
fn sdk_objects_built_from_a_model_are_calls() {
    let found = usages(
        r#"
import { OpenAIRealtimeWS } from "openai/realtime/ws";
const rt = new OpenAIRealtimeWS({ model: "gpt-4o-realtime-preview" });
"#,
    );
    let c = call(&found, "OpenAIRealtimeWS");
    assert_eq!(c.receiver, Receiver::Symbol);
    assert_eq!(c.routing, Routing::Unknown);
    assert_eq!(value(c, "model"), Some("gpt-4o-realtime-preview"));
}

// ---------------------------------------------------------------------------
// Model values
// ---------------------------------------------------------------------------

fn with_client(body: &str) -> String {
    format!("{OPENAI}const client = new OpenAI();\n{body}\n")
}

#[test]
fn constants_resolve_in_their_scope() {
    let cases: &[(&str, Option<&str>, ValueOrigin)] = &[
        (
            "const MODEL = \"gpt-4o\";\nclient.chat.completions.create({ model: MODEL, messages: [] });",
            Some("gpt-4o"),
            ValueOrigin::ModuleConstant,
        ),
        (
            "export const MODEL = 'gpt-4o-mini' as const;\nclient.chat.completions.create({ model: MODEL, messages: [] });",
            Some("gpt-4o-mini"),
            ValueOrigin::ModuleConstant,
        ),
        (
            "function ask() {\n  const model = `gpt-4.1`;\n  return client.chat.completions.create({ model, messages: [] });\n}",
            Some("gpt-4.1"),
            ValueOrigin::LocalConstant,
        ),
        (
            "let MODEL = \"gpt-4o\";\nclient.chat.completions.create({ model: MODEL, messages: [] });",
            Some("gpt-4o"),
            ValueOrigin::ModuleConstant,
        ),
        (
            "var MODEL = \"gpt-4o\";\nclient.chat.completions.create({ model: MODEL, messages: [] });",
            Some("gpt-4o"),
            ValueOrigin::ModuleConstant,
        ),
        (
            "const BASE = \"gpt-4o\";\nconst MODEL = BASE;\nclient.chat.completions.create({ model: MODEL!, messages: [] });",
            Some("gpt-4o"),
            ValueOrigin::ModuleConstant,
        ),
    ];
    for (body, expected, origin) in cases {
        let found = usages(&with_client(body));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(value(c, "model"), *expected, "{body}");
        assert_eq!(c.values[0].origin, *origin, "{body}");
        assert!(found.gaps.is_empty(), "{body}: {:?}", found.gaps);
    }
}

#[test]
fn a_name_written_more_than_once_lists_its_values() {
    let (got, at) = model(&with_client(
        "let model = \"gpt-4o\";\nif (fast) model = \"gpt-4o-mini\";\nclient.chat.completions.create({ model, messages: [] });",
    ));
    assert_eq!(got, None);
    assert_eq!(at, vec!["unresolved_model/reassigned"]);
    let found = usages(&with_client(
        "let model = \"gpt-4o\";\nif (fast) model = \"gpt-4o-mini\";\nclient.chat.completions.create({ model, messages: [] });",
    ));
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    let listed: Vec<&str> = gap.values.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(listed, vec!["gpt-4o", "gpt-4o-mini"]);
    assert!(gap.values_complete);
}

#[test]
fn a_loop_variable_lists_the_items() {
    let found = usages(&with_client(
        "for (const model of [\"gpt-4o\", \"gpt-4o-mini\"]) {\n  client.chat.completions.create({ model, messages: [] });\n}",
    ));
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.reason, "reassigned");
    assert_eq!(gap.values.len(), 2);
    assert!(gap.values_complete);
}

#[test]
fn object_literal_entries_resolve_when_only_read() {
    let cases: &[(&str, Option<&str>, &[&str])] = &[
        (
            "const MODELS = { fast: \"gpt-4o-mini\", smart: \"gpt-4o\" } as const;\nclient.chat.completions.create({ model: MODELS.fast, messages: [] });",
            Some("gpt-4o-mini"),
            &[],
        ),
        (
            "const MODELS = { fast: \"gpt-4o-mini\" };\nclient.chat.completions.create({ model: MODELS[\"fast\"], messages: [] });",
            Some("gpt-4o-mini"),
            &[],
        ),
        (
            "const CONFIG = { openai: { model: \"gpt-4o\" } };\nclient.chat.completions.create({ model: CONFIG.openai.model, messages: [] });",
            Some("gpt-4o"),
            &[],
        ),
        (
            "const { model } = { model: \"gpt-4o\" };\nclient.chat.completions.create({ model, messages: [] });",
            Some("gpt-4o"),
            &[],
        ),
        (
            "const MODELS = { fast: \"gpt-4o-mini\" };\nconsole.log(JSON.stringify(MODELS));\nclient.chat.completions.create({ model: MODELS.fast, messages: [] });",
            Some("gpt-4o-mini"),
            &[],
        ),
        (
            "const MODELS = { fast: \"gpt-4o-mini\" };\nMODELS.fast = other;\nclient.chat.completions.create({ model: MODELS.fast, messages: [] });",
            None,
            &["unresolved_model/attribute"],
        ),
        (
            "const MODELS = { fast: \"gpt-4o-mini\" };\nconfigure(MODELS);\nclient.chat.completions.create({ model: MODELS.fast, messages: [] });",
            None,
            &["unresolved_model/attribute"],
        ),
        (
            "const MODELS = { fast: \"gpt-4o-mini\" };\nObject.assign(MODELS, overrides);\nclient.chat.completions.create({ model: MODELS.fast, messages: [] });",
            None,
            &["unresolved_model/attribute"],
        ),
    ];
    for (body, expected, gaps) in cases {
        let (got, at) = model(&with_client(body));
        assert_eq!(got.as_deref(), *expected, "{body}");
        assert_eq!(at, *gaps, "{body}");
    }
}

#[test]
fn a_computed_key_lists_the_entries() {
    let found = usages(&with_client(
        "const MODELS = { fast: \"gpt-4o-mini\", smart: \"gpt-4o\" };\nexport function ask(tier: string) {\n  return client.chat.completions.create({ model: MODELS[tier], messages: [] });\n}",
    ));
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.reason, "subscript");
    let listed: Vec<&str> = gap.values.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(listed, vec!["gpt-4o-mini", "gpt-4o"]);
    assert!(gap.values_complete);
}

#[test]
fn options_objects_and_spreads() {
    let cases: &[(&str, Option<&str>, &[&str])] = &[
        (
            "const PARAMS = { model: \"gpt-4o\", temperature: 0 };\nclient.chat.completions.create({ ...PARAMS, messages: [] });",
            Some("gpt-4o"),
            &[],
        ),
        (
            "const PARAMS = { model: \"gpt-4o\", messages: [] };\nclient.chat.completions.create(PARAMS);",
            Some("gpt-4o"),
            &[],
        ),
        (
            "export function ask(rest: any) {\n  return client.chat.completions.create({ ...rest, model: \"gpt-4o\", messages: [] });\n}",
            Some("gpt-4o"),
            &[],
        ),
        (
            "export function ask(rest: any) {\n  return client.chat.completions.create({ model: \"gpt-4o\", ...rest, messages: [] });\n}",
            None,
            &["unresolved_model/splat"],
        ),
        (
            "export function ask(rest: any) {\n  return client.chat.completions.create({ messages: [], ...rest });\n}",
            None,
            &["unresolved_model/splat"],
        ),
        (
            "client.chat.completions.create({ model: \"gpt-4o\", model: \"gpt-4o-mini\", messages: [] });",
            Some("gpt-4o-mini"),
            &[],
        ),
    ];
    for (body, expected, gaps) in cases {
        let (got, at) = model(&with_client(body));
        assert_eq!(got.as_deref(), *expected, "{body}");
        assert_eq!(at, *gaps, "{body}");
    }
    let found = usages(&with_client(
        "const PARAMS = { model: \"gpt-4o\", messages: [] };\nclient.chat.completions.create(PARAMS);",
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.values[0].name.as_deref(), Some("PARAMS.model"));
    assert_eq!(c.values[0].origin, ValueOrigin::ModuleConstant);
    assert!(c.splat);
}

#[test]
fn enum_members_resolve() {
    let (got, at) = model(&with_client(
        "enum Model { Fast = \"gpt-4o-mini\", Smart = 'gpt-4o' }\nclient.chat.completions.create({ model: Model.Smart, messages: [] });",
    ));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert!(at.is_empty(), "{at:?}");
}

#[test]
fn environment_values_are_gaps_with_their_fallback() {
    let cases: &[(&str, Option<&str>, &[&str])] = &[
        (
            "process.env.OPENAI_MODEL",
            None,
            &["unresolved_model/env_var"],
        ),
        (
            "process.env[\"OPENAI_MODEL\"]",
            None,
            &["unresolved_model/env_var"],
        ),
        (
            "import.meta.env.VITE_MODEL",
            None,
            &["unresolved_model/env_var"],
        ),
        (
            "Deno.env.get(\"MODEL\")",
            None,
            &["unresolved_model/env_var"],
        ),
        (
            "process.env.OPENAI_MODEL ?? \"gpt-4o\"",
            Some("gpt-4o"),
            &["unresolved_model/env_var"],
        ),
        (
            "process.env.OPENAI_MODEL || \"gpt-4o\"",
            Some("gpt-4o"),
            &["unresolved_model/env_var"],
        ),
    ];
    for (expr, expected, gaps) in cases {
        let found = usages(&with_client(&format!(
            "client.chat.completions.create({{ model: {expr}, messages: [] }});"
        )));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(value(c, "model"), *expected, "{expr}");
        assert_eq!(gaps_at(&found, c), *gaps, "{expr}");
        if expected.is_some() {
            assert_eq!(c.values[0].origin, ValueOrigin::EnvDefault, "{expr}");
        }
        // The variable's name is never reported.
        let json = serde_json::to_string(&found).unwrap();
        assert!(!json.contains("OPENAI_MODEL"), "{expr}: {json}");
        assert!(!json.contains("VITE_MODEL"), "{expr}");
    }
    // Destructured from the environment.
    let (got, gaps) = model(&with_client(
        "const { MODEL } = process.env;\nclient.chat.completions.create({ model: MODEL, messages: [] });",
    ));
    assert_eq!(got, None);
    assert_eq!(gaps, vec!["unresolved_model/env_var"]);
}

#[test]
fn a_property_lookup_with_a_fallback_is_confirm_first() {
    let found = usages(&with_client(
        "export function ask(options: { model?: string }) {\n  return client.chat.completions.create({ model: options.model ?? \"gpt-4o\", messages: [] });\n}",
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), Some("gpt-4o"));
    assert_eq!(c.values[0].origin, ValueOrigin::EnvDefault);
    assert_eq!(gaps_at(&found, c), vec!["unresolved_model/attribute"]);
}

#[test]
fn shapes_outside_the_grammar_are_typed_gaps() {
    let cases: &[(&str, &str)] = &[
        ("`gpt-${version}`", "f_string"),
        ("fast ? \"gpt-4o-mini\" : \"gpt-4o\"", "conditional"),
        ("pickModel()", "function_call"),
        ("await pickModel()", "function_call"),
        ("settings.model", "attribute"),
        ("undefinedName", "unbound"),
        ("\"gpt-\" + version", "non_literal"),
        ("\"gpt\\u002d4o\"", "non_literal"),
    ];
    for (expr, reason) in cases {
        let (got, gaps) = model(&with_client(&format!(
            "client.chat.completions.create({{ model: {expr}, messages: [] }});"
        )));
        assert_eq!(got, None, "{expr}");
        assert_eq!(gaps, vec![format!("unresolved_model/{reason}")], "{expr}");
    }
    let found = usages(&with_client(
        "client.chat.completions.create({ model: fast ? \"gpt-4o-mini\" : \"gpt-4o\", messages: [] });",
    ));
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.values.len(), 2);
    assert!(gap.values_complete);
}

#[test]
fn a_name_imported_from_another_module_is_a_gap() {
    let (got, gaps) = model(&with_client(
        "import { MODEL } from \"./config\";\nclient.chat.completions.create({ model: MODEL, messages: [] });",
    ));
    assert_eq!(got, None);
    assert_eq!(gaps, vec!["unresolved_model/imported_name"]);
}
