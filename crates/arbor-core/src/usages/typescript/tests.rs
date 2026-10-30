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
import PinnedOpenAI from "https://esm.sh/v135/openai@4";
import JsdOpenAI from "https://cdn.jsdelivr.net/npm/openai@4/+esm";
import DenoOpenAI from "https://deno.land/x/openai@v4.24.0/mod.ts";
"#,
    );
    for name in [
        "OpenAI",
        "Anthropic",
        "JsrOpenAI",
        "CdnOpenAI",
        "PinnedOpenAI",
        "JsdOpenAI",
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

// ---------------------------------------------------------------------------
// Parameters and their callers
// ---------------------------------------------------------------------------

/// The model value, its origin and the gaps of the `create` call in a file
/// built from `body`.
fn model_row(body: &str) -> (Option<String>, Option<ValueOrigin>, Vec<String>) {
    let found = usages(&with_client(body));
    let c = found
        .calls
        .iter()
        .find(|c| c.member == "chat.completions.create")
        .unwrap_or_else(|| panic!("no create call: {:?} {:?}", found.calls, found.gaps));
    (
        value(c, "model").map(str::to_string),
        c.values.first().map(|v| v.origin),
        gaps_at(&found, c),
    )
}

#[test]
fn a_parameter_default_is_confirmed_only_when_every_call_is_seen() {
    let ask = "async function ask(prompt: string, model = \"gpt-4o\") {\n  return client.chat.completions.create({ model, messages: [] });\n}\n";
    // Every call is in the file and passes nothing for it.
    let (got, origin, gaps) = model_row(&format!("{ask}ask(\"a\");\nask(\"b\", undefined);"));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(origin, Some(ValueOrigin::ParameterDefault));
    assert!(gaps.is_empty(), "{gaps:?}");
    // A caller passes another value.
    let (got, _, gaps) = model_row(&format!("{ask}ask(\"a\");\nask(\"b\", \"gpt-4o-mini\");"));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_overridden"]);
    // Exported: other modules may pass another value.
    let (got, _, gaps) = model_row(&format!("export {ask}"));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_default"]);
    // Passed as a value: callers this file does not show.
    let (got, _, gaps) = model_row(&format!("{ask}app.post(\"/\", ask);"));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(gaps, vec!["unresolved_model/unseen_callers"]);
    // CommonJS exports count as exports.
    let (_, _, gaps) = model_row(&format!("{ask}module.exports = {{ ask }};"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_default"]);
}

#[test]
fn parameter_overrides_are_listed() {
    let found = usages(&with_client(
        "function ask(model = \"gpt-4o\") {\n  return client.chat.completions.create({ model, messages: [] });\n}\nask(\"gpt-4o-mini\");\nask(\"o3-mini\");\n",
    ));
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.reason, "parameter_overridden");
    let listed: Vec<&str> = gap.values.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(listed, vec!["gpt-4o-mini", "o3-mini"]);
    assert!(gap.values_complete);
}

#[test]
fn a_parameter_with_no_default_lists_what_callers_pass() {
    let found = usages(&with_client(
        "const ask = async (model: string) => client.chat.completions.create({ model, messages: [] });\nask(\"gpt-4o\");\nask(\"gpt-4o-mini\");\n",
    ));
    let gap = &gaps(&found, GapKind::UnresolvedModel)[0];
    assert_eq!(gap.reason, "parameter");
    assert_eq!(gap.values.len(), 2);
    assert!(gap.values_complete);
    assert_eq!(found.calls[0].symbol.as_deref(), Some("ask"));
}

#[test]
fn destructured_parameter_defaults() {
    let body = "function ask({ model = \"gpt-4o\", prompt }: { model?: string; prompt: string } = { prompt: \"\" }) {\n  return client.chat.completions.create({ model, messages: [] });\n}\nask({ prompt: \"hi\" });\n";
    let (got, origin, gaps) = model_row(body);
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(origin, Some(ValueOrigin::ParameterDefault));
    assert!(gaps.is_empty(), "{gaps:?}");
    let (_, _, gaps) = model_row(&format!("{body}ask({{ prompt: \"x\", model: \"o1\" }});\n"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_overridden"]);
    let (_, _, gaps) = model_row(&format!("{body}ask(options);\n"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_overridden"]);
}

#[test]
fn method_and_constructor_parameter_defaults() {
    // A public method: other modules may call it with another value.
    let (_, _, gaps) = model_row(
        "class Bot {\n  ask(model = \"gpt-4o\") {\n    return client.chat.completions.create({ model, messages: [] });\n  }\n}\n",
    );
    assert_eq!(gaps, vec!["unresolved_model/parameter_default"]);
    // A private method's calls are all in the class.
    let (got, _, gaps) = model_row(
        "class Bot {\n  run() { return this.ask(); }\n  private ask(model = \"gpt-4o\") {\n    return client.chat.completions.create({ model, messages: [] });\n  }\n}\n",
    );
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert!(gaps.is_empty(), "{gaps:?}");
    // A constructor's calls are the class's `new` expressions.
    let ctor = "class Bot {\n  constructor(private model = \"gpt-4o\") {}\n  ask() {\n    return client.chat.completions.create({ model: this.model, messages: [] });\n  }\n}\n";
    let (got, _, gaps) = model_row(&format!("{ctor}new Bot().ask();\n"));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert!(gaps.is_empty(), "{gaps:?}");
    let (_, _, gaps) = model_row(&format!("{ctor}new Bot(\"gpt-4o-mini\").ask();\n"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_overridden"]);
    let (_, _, gaps) = model_row(&format!("export {ctor}"));
    assert!(
        gaps.contains(&"unresolved_model/parameter_default".to_string()),
        "{gaps:?}"
    );
}

// ---------------------------------------------------------------------------
// Class properties
// ---------------------------------------------------------------------------

#[test]
fn class_properties_assigned_once() {
    let field = "class Bot {\n  private model = \"gpt-4o\";\n  ask() {\n    return client.chat.completions.create({ model: this.model, messages: [] });\n  }\n}\nnew Bot().ask();\n";
    let (got, origin, gaps) = model_row(field);
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(origin, Some(ValueOrigin::ClassConstant));
    assert!(gaps.is_empty(), "{gaps:?}");
    // Assigned in the constructor instead.
    let (got, _, gaps) = model_row(
        "class Bot {\n  model: string;\n  constructor() { this.model = \"gpt-4o\"; }\n  ask() {\n    return client.chat.completions.create({ model: this.model, messages: [] });\n  }\n}\n",
    );
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert!(gaps.is_empty(), "{gaps:?}");
    // Exported: a subclass in another module may override it.
    let (got, _, gaps) = model_row(&format!("export {field}"));
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert_eq!(gaps, vec!["unresolved_model/subclass_override"]);
    // A static property read by class name is confirmed.
    let (got, _, gaps) = model_row(
        "export class Bot {\n  static MODEL = \"gpt-4o\";\n}\nclient.chat.completions.create({ model: Bot.MODEL, messages: [] });\n",
    );
    assert_eq!(got.as_deref(), Some("gpt-4o"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn class_properties_other_code_may_change_are_gaps() {
    let cases: &[(&str, &str)] = &[
        (
            "class Bot {\n  model = \"gpt-4o\";\n  fast() { this.model = \"gpt-4o-mini\"; }\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\n",
            "unresolved_model/reassigned",
        ),
        (
            "class Bot {\n  model = \"gpt-4o\";\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\nconst bot = new Bot();\nbot.model = other;\n",
            "unresolved_model/dynamic_feature",
        ),
        (
            "class Bot {\n  @observable model = \"gpt-4o\";\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\n",
            "unresolved_model/dynamic_feature",
        ),
        (
            "class Bot {\n  model = \"gpt-4o\";\n  constructor(options: object) { Object.assign(this, options); }\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\n",
            "unresolved_model/dynamic_feature",
        ),
        (
            "class Bot {\n  get model() { return pick(); }\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\n",
            "unresolved_model/dynamic_feature",
        ),
        (
            "class Bot {\n  model = \"gpt-4o\";\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\nclass Fast extends Bot {\n  model = \"gpt-4o-mini\";\n}\n",
            "unresolved_model/reassigned",
        ),
        (
            "class Bot {\n  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }\n}\n",
            "unresolved_model/attribute",
        ),
    ];
    for (body, expected) in cases {
        let (got, _, gaps) = model_row(body);
        assert_eq!(got, None, "{body}");
        assert_eq!(gaps, vec![expected.to_string()], "{body}");
    }
}

#[test]
fn this_in_a_plain_function_is_not_the_class() {
    let (got, _, gaps) = model_row(
        "class Bot {\n  model = \"gpt-4o\";\n  ask() {\n    return [1].map(function () { return client.chat.completions.create({ model: this.model, messages: [] }); });\n  }\n}\n",
    );
    assert_eq!(got, None);
    assert_eq!(gaps, vec!["unresolved_model/attribute"]);
}

// ---------------------------------------------------------------------------
// Dynamic features and secrets
// ---------------------------------------------------------------------------

#[test]
fn eval_and_with_make_values_and_routes_dynamic() {
    for feature in ["eval(code);", "with (scope) { run(); }"] {
        let found = usages(&with_client(&format!(
            "{feature}\nconst MODEL = \"gpt-4o\";\nclient.chat.completions.create({{ model: MODEL, messages: [] }});\nclient.chat.completions.create({{ model: \"gpt-4o-mini\", messages: [] }});"
        )));
        let calls: Vec<_> = found
            .calls
            .iter()
            .filter(|c| c.member == "chat.completions.create")
            .collect();
        // The constant is kept, with a gap; the literal at the call is
        // never downgraded.
        assert_eq!(value(calls[0], "model"), Some("gpt-4o"), "{feature}");
        assert_eq!(
            gaps_at(&found, calls[0]),
            vec![
                "unresolved_model/dynamic_feature",
                "ambiguous_client/dynamic_feature"
            ],
            "{feature}"
        );
        assert_eq!(value(calls[1], "model"), Some("gpt-4o-mini"));
        assert_eq!(
            gaps_at(&found, calls[1]),
            vec!["ambiguous_client/dynamic_feature"],
            "{feature}"
        );
    }
    // A local function named `eval` is not the builtin.
    let found = usages(&with_client(
        "function evaluate() {}\nconst eval2 = 1;\nclient.chat.completions.create({ model: \"gpt-4o\", messages: [] });",
    ));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn credentials_are_never_captured() {
    let found = usages(
        r#"
import OpenAI from "openai";
const API_KEY = "sk-proj-AbCdEf0123456789xyzABCDEFGH";
const client = new OpenAI({ apiKey: API_KEY, baseURL: "https://user:hunter2@gateway.example/v1" });
const SECRET_MODEL = "gpt-4o";
client.chat.completions.create({ model: SECRET_MODEL, messages: [] });
client.chat.completions.create({ model: "sk-ant-api03-AbCdEf0123456789", messages: [] });
client.chat.completions.create({ model: process.env.OPENAI_API_KEY, messages: [] });
client.chat.completions.create({ model: config.apiKey, messages: [] });
"#,
    );
    let json = serde_json::to_string(&found).unwrap();
    for secret in [
        "sk-proj",
        "hunter2",
        "sk-ant-api03",
        "OPENAI_API_KEY",
        "gateway.example",
    ] {
        assert!(!json.contains(secret), "{secret} leaked: {json}");
    }
    let reasons: Vec<&str> = gaps(&found, GapKind::UnresolvedModel)
        .iter()
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        vec!["credential", "credential", "env_var", "credential"]
    );
    assert!(found.calls.iter().all(|c| c.values.is_empty()));
}

// ---------------------------------------------------------------------------
// Symbols
// ---------------------------------------------------------------------------

#[test]
fn records_name_the_graph_symbol() {
    let found = usages(
        r#"
import OpenAI from "openai";
const client = new OpenAI();
function top() { return client.models.list(); }
export const arrow = async () => client.models.list();
class Bot {
  ask() {
    function inner() { return client.models.list(); }
    const local = () => client.models.list();
    return [inner, local];
  }
  static build() { return client.models.list(); }
  field = client.models.list();
}
export default {
  async fetch() { return client.models.list(); },
};
describe("bot", () => {
  it("lists models", async () => { await client.models.list(); });
  it("works", () => client.models.list());
  it("works", () => client.models.list());
});
const handler = function () { return client.models.list(); };
client.models.list();
"#,
    );
    let symbols: Vec<Option<&str>> = found.calls.iter().map(|c| c.symbol.as_deref()).collect();
    assert_eq!(
        symbols,
        vec![
            Some("top"),
            Some("arrow"),
            Some("Bot.inner"),
            Some("local"),
            Some("Bot.build"),
            Some("Bot"),
            Some("fetch"),
            Some("it: lists models"),
            Some("it: works (line 20)"),
            Some("it: works (line 21)"),
            None,
            None,
        ]
    );
}

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

fn usages_within(source: &str, limit_secs: u64) -> FileUsages {
    let started = std::time::Instant::now();
    let found = usages(source);
    let took = started.elapsed();
    assert!(
        took.as_secs() < limit_secs,
        "took {took:?} for {} bytes",
        source.len()
    );
    found
}

#[test]
fn a_file_with_too_many_records_is_truncated() {
    let mut source = with_client("");
    for _ in 0..25_000 {
        source.push_str("client.models.list();\n");
    }
    let found = usages_within(&source, 30);
    let records = found.imports.len() + found.calls.len() + found.gaps.len() + found.clients.len();
    assert_eq!(records, 20_001);
    let last = found.gaps.last().unwrap();
    assert_eq!(last.reason, "truncated");
    assert_eq!(last.chain, None);
}

#[test]
fn deeply_nested_scopes_are_not_analyzed() {
    let mut source = with_client("");
    for _ in 0..80 {
        source.push_str("{ ");
    }
    source.push_str("client.chat.completions.create({ model: \"gpt-4o\", messages: [] });");
    for _ in 0..80 {
        source.push_str(" }");
    }
    source.push('\n');
    let found = usages_within(&source, 10);
    let truncated: Vec<_> = found
        .gaps
        .iter()
        .filter(|g| g.reason == "truncated")
        .collect();
    assert_eq!(truncated.len(), 1);
    assert!(found
        .calls
        .iter()
        .all(|c| c.member != "chat.completions.create"));
}

#[test]
fn a_long_call_chain_is_linear() {
    let mut source = with_client("let x = client");
    for _ in 0..20_000 {
        source.push_str(".m()");
    }
    source.push_str(";\n");
    let found = usages_within(&source, 10);
    assert!(found.calls.len() <= 20_000);
}

#[test]
fn long_chains_of_names_are_linear() {
    let mut source = with_client("const m0 = \"gpt-4o\";\n");
    for i in 1..2_000 {
        source.push_str(&format!("const m{i} = m{} || m{};\n", i - 1, i - 1));
    }
    source.push_str("client.chat.completions.create({ model: m1999, messages: [] });\n");
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 1);
}

#[test]
fn many_callers_and_large_objects_stay_bounded() {
    let mut source = with_client(
        "function ask(model = \"gpt-4o\") { return client.chat.completions.create({ model, messages: [] }); }\n",
    );
    for i in 0..5_000 {
        source.push_str(&format!("ask(\"m{i}\");\n"));
    }
    source.push_str("const BIG = {");
    for i in 0..50_000 {
        source.push_str(&format!(" k{i}: \"v{i}\","));
    }
    source.push_str(" model: \"gpt-4o\" };\n");
    for _ in 0..2_000 {
        source.push_str("client.chat.completions.create({ ...BIG, messages: [] });\n");
    }
    let found = usages_within(&source, 30);
    let json = serde_json::to_string(&found).unwrap();
    assert!(json.len() < 8_000_000, "{} bytes", json.len());
}

#[test]
fn a_key_late_in_a_large_object_is_still_found() {
    let mut source = with_client("const BIG = {");
    for i in 0..1_000 {
        source.push_str(&format!(" k{i}: \"v{i}\","));
    }
    source.push_str(" model: \"gpt-4o\" };\nclient.chat.completions.create({ ...BIG, messages: [] });\nclient.chat.completions.create(BIG);\n");
    let found = usages(&source);
    for c in &found.calls {
        assert_eq!(value(c, "model"), Some("gpt-4o"));
        assert_eq!(c.values[0].name.as_deref(), Some("BIG.model"));
    }
    assert!(
        found.gaps.iter().all(|g| g.reason != "missing_model"),
        "{:?}",
        found.gaps
    );
}

#[test]
fn model_keys_in_positional_literals_are_captured() {
    let found = usages(
        r#"
import Anthropic, { betaRefusalFallbackMiddleware } from "@anthropic-ai/sdk";
const client = new Anthropic({
  middleware: [betaRefusalFallbackMiddleware([{ model: "claude-opus-4-8" }])],
});
await client.messages.create({ model: "claude-fable-5", max_tokens: 10, messages: [] }, { timeout: 5 });
"#,
    );
    let middleware = call(&found, "betaRefusalFallbackMiddleware");
    assert_eq!(value(middleware, "model"), Some("claude-opus-4-8"));
    assert!(middleware.values[0].positional);
    let create = call(&found, "client.messages.create");
    assert_eq!(create.values.len(), 1);
    assert_eq!(create.positional, 1);
}

#[test]
fn a_mocked_sdk_builds_no_real_client_and_v3_configuration_is_an_sdk_object() {
    let found = usages(
        "import OpenAI from \"openai\";
jest.mock(\"openai\");
const client = new OpenAI();
",
    );
    assert_eq!(routing_of(&found, "client"), Routing::Unknown);
    let found = usages_js(
        "const { Configuration, OpenAIApi } = require(\"openai\");
const openai = new OpenAIApi(new Configuration({ apiKey: key }));
",
    );
    let configuration = call(&found, "Configuration");
    assert_eq!(configuration.receiver, Receiver::Symbol);
    assert_eq!(configuration.routing, Routing::Unknown);
    assert_eq!(configuration.kwargs, vec!["apiKey"]);
    assert_eq!(routing_of(&found, "openai"), Routing::OpenaiDirect);
}

// ---------------------------------------------------------------------------
// Values the review showed may differ at run time
// ---------------------------------------------------------------------------

const CLIENT: &str = "import OpenAI from \"openai\";\nconst client = new OpenAI();\n";

/// The model value and gaps of the only `chat.completions.create` call in
/// `body`, after a client named `client`.
fn model_of(body: &str) -> (Option<String>, Vec<String>) {
    model(&format!("{CLIENT}{body}"))
}

fn assert_held(body: &str, gap: &str) {
    let (value, gaps) = model_of(body);
    assert!(
        gaps.iter().any(|g| g == gap),
        "expected {gap} for {value:?}, got {gaps:?} in:\n{body}"
    );
}

#[test]
fn a_nested_object_handed_out_by_a_copy_or_a_builtin_is_not_read_only() {
    for body in [
        "const AGENTS = { writer: { model: \"gpt-4\" } };
for (const a of Object.values(AGENTS)) a.model = \"gpt-4o\";
client.chat.completions.create({ model: AGENTS.writer.model, messages: [] });",
        "const AGENTS = { writer: { model: \"gpt-4\" } };
Object.entries(AGENTS).forEach(([, v]) => { v.model = \"gpt-4o\"; });
client.chat.completions.create({ model: AGENTS.writer.model, messages: [] });",
        "const CONFIG = { chat: { model: \"gpt-4\" } };
const local = { ...CONFIG };
local.chat.model = \"gpt-4o\";
client.chat.completions.create({ model: CONFIG.chat.model, messages: [] });",
        "const CONFIG = { openai: { chat: { model: \"gpt-4\" } } };
const o = { ...CONFIG.openai };
o.chat.model = \"gpt-4o\";
client.chat.completions.create({ model: CONFIG.openai.chat.model, messages: [] });",
        "const CONFIG = { chat: { model: \"gpt-4\" } };
const f = Object.freeze(CONFIG);
f.chat.model = \"gpt-4o\";
client.chat.completions.create({ model: CONFIG.chat.model, messages: [] });",
    ] {
        assert_held(body, "unresolved_model/attribute");
    }
    // A copy handed straight to a request, a freeze whose result is
    // dropped, and the values of an object holding no container stay
    // confirmed.
    for body in [
        "const PARAMS = { model: \"gpt-4\", response_format: { type: \"json_object\" } };
client.chat.completions.create({ ...PARAMS, messages: [] });",
        "const CONFIG = { chat: { model: \"gpt-4\" } };
Object.freeze(CONFIG);
client.chat.completions.create({ model: CONFIG.chat.model, messages: [] });",
        "const MODELS = { fast: \"gpt-4\", smart: \"gpt-4o\" };
console.log(Object.values(MODELS));
client.chat.completions.create({ model: MODELS.fast, messages: [] });",
    ] {
        let (value, gaps) = model_of(body);
        assert_eq!(value.as_deref(), Some("gpt-4"), "{body}");
        assert!(gaps.is_empty(), "{gaps:?} in:\n{body}");
    }
}

#[test]
fn a_reflective_write_on_an_instance_writes_its_class_property() {
    for write in [
        "Object.assign(bot, { model: \"gpt-4o\" });",
        "Object.defineProperty(bot, \"model\", { value: \"gpt-4o\" });",
        "Reflect.set(bot, \"model\", \"gpt-4o\");",
        "Object.assign(bots[0], { model: \"gpt-4o\" });",
        "Object.assign(bot, overrides);",
    ] {
        assert_held(
            &format!(
                "class Bot {{ model = \"gpt-4\"; ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
const bot = new Bot();
const bots = [bot];
{write}
bot.ask();"
            ),
            "unresolved_model/dynamic_feature",
        );
    }
}

#[test]
fn a_static_property_read_through_this_sees_its_subclasses() {
    assert_held(
        "class Provider { static model = \"gpt-4\";
  static complete() { return client.chat.completions.create({ model: this.model, messages: [] }); } }
class FastProvider extends Provider { static model = \"gpt-4o\"; }
FastProvider.complete();",
        "unresolved_model/reassigned",
    );
    assert_held(
        "class Provider { static model = \"gpt-4\";
  static complete() { return client.chat.completions.create({ model: this.model, messages: [] }); } }
class Sub extends Provider {}
Sub.model = \"gpt-4o\";
Sub.complete();",
        "unresolved_model/reassigned",
    );
    // Read by class name, the subclass's own property is another one.
    let (value, gaps) = model_of(
        "class Provider { static model = \"gpt-4\";
  static complete() { return client.chat.completions.create({ model: Provider.model, messages: [] }); } }
class FastProvider extends Provider { static model = \"gpt-4o\"; }
FastProvider.complete();",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_base_class_arbor_cannot_identify_may_write_any_property() {
    for base in ["holder.Base", "make()", "Tuned(Object)", "B"] {
        let (value, gaps) = model_of(&format!(
            "class Base {{ model = \"gpt-4o\"; }}
const holder = {{ Base }};
function make() {{ return Base; }}
function Tuned(C: any) {{ return class extends C {{}}; }}
const B = make();
class Bot extends {base} {{ init() {{ this.model = \"gpt-4\"; }}
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
new Bot().ask();"
        ));
        assert!(!gaps.is_empty(), "{base}: {value:?} with no gap");
    }
    // A same-file base in parentheses or a cast is that base.
    for base in ["(Base)", "(Base as any)"] {
        assert_held(
            &format!(
                "class Base {{ model = \"gpt-4o\"; }}
class Bot extends {base} {{ init() {{ this.model = \"gpt-4\"; }}
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
new Bot().ask();"
            ),
            "unresolved_model/reassigned",
        );
    }
    // An imported or global base is outside the file, as documented.
    for base in ["import { Base } from \"./base\";", "const Base = Error;"] {
        let (value, gaps) = model_of(&format!(
            "{base}
class Bot extends Base {{ model = \"gpt-4\";
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
new Bot().ask();"
        ));
        if base.starts_with("import") {
            assert_eq!(value.as_deref(), Some("gpt-4"));
            assert!(gaps.is_empty(), "{gaps:?}");
        } else {
            assert!(!gaps.is_empty(), "{base}: {value:?} with no gap");
        }
    }
}

#[test]
fn a_method_called_with_another_this_reads_another_object() {
    for call in [
        "new Bot().ask.call({ model: \"gpt-4o\" });",
        "Bot.prototype.ask.apply({ model: \"gpt-4o\" }, []);",
        "const bound = new Bot().ask.bind({ model: \"gpt-4o\" }); bound();",
        "({ model: \"gpt-4o\", ask: new Bot().ask }).ask();",
    ] {
        let (value, gaps) = model_of(&format!(
            "class Bot {{ model = \"gpt-4\"; ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
{call}"
        ));
        assert_eq!(value.as_deref(), Some("gpt-4"), "{call}");
        assert!(
            gaps.iter().any(|g| g == "unresolved_model/attribute"),
            "{call}: {gaps:?}"
        );
    }
    // An arrow function keeps the `this` it was written in.
    let (value, gaps) = model_of(
        "class Bot { model = \"gpt-4\"; ask = () => client.chat.completions.create({ model: this.model, messages: [] }); }
const b = new Bot();
setTimeout(b.ask);",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_method_called_through_a_computed_member_has_unseen_callers() {
    // Python's `getattr(self, kind)(model)`: the method is picked at run
    // time, so no private method's calls are all seen.
    let router = |route: &str, after: &str| {
        format!(
            "class Router {{
  private chat(model = \"gpt-4o\") {{
    return client.chat.completions.create({{ model, messages: [] }});
  }}
  route(kind: \"chat\", model: string) {{ {route} }}
}}
{after}"
        )
    };
    for (route, after) in [
        (
            "return this[kind](model);",
            "new Router().route(\"chat\", \"gpt-5.5\");",
        ),
        (
            "return this[kind]?.(model);",
            "new Router().route(\"chat\", \"gpt-5.5\");",
        ),
        // Read from `this` without a call, as `this.chat` would be.
        (
            "const { chat } = this; return chat(model);",
            "new Router().route(\"chat\", \"gpt-5.5\");",
        ),
        (
            "const { chat: run = () => null } = this; return run(model);",
            "new Router().route(\"chat\", \"gpt-5.5\");",
        ),
        // On an instance.
        (
            "return null;",
            "const r = new Router();
const name = process.argv[2] as \"chat\";
r[name](\"gpt-5.5\");",
        ),
        (
            "return null;",
            "const { chat } = new Router() as any;
chat(\"gpt-5.5\");",
        ),
        (
            "return null;",
            "let chat: any;
({ chat } = new Router() as any);
chat(\"gpt-5.5\");",
        ),
    ] {
        let body = router(route, after);
        let (value, origin, gaps) = model_row(&body);
        assert_eq!(value.as_deref(), Some("gpt-4o"), "{body}");
        assert_eq!(origin, Some(ValueOrigin::ParameterDefault), "{body}");
        assert_eq!(gaps, vec!["unresolved_model/unseen_callers"], "{body}");
    }
    // A key known in the file is a call by name: its value is listed.
    let (value, _, gaps) = model_row(&router(
        "return this[\"chat\"](model);",
        "new Router().route(\"chat\", \"gpt-5.5\");",
    ));
    assert_eq!(value.as_deref(), Some("gpt-4o"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_overridden"]);
    // No call through a computed member: every call is seen.
    let (value, _, gaps) = model_row(&router(
        "return this.chat();",
        "const MODELS: Record<string, string> = { chat: \"gpt-4o\" };
console.log(MODELS[process.argv[2]]);
new Router().route(\"chat\", \"gpt-5.5\");",
    ));
    assert_eq!(value.as_deref(), Some("gpt-4o"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_method_run_through_a_computed_member_with_another_this_reads_another_object() {
    for run in [
        "return this[m].call({ model: \"gpt-5.5\" });",
        "return this[m].apply({ model: \"gpt-5.5\" }, []);",
        "return this[m].bind({ model: \"gpt-5.5\" })();",
        "const { ask } = this; return ask.call({ model: \"gpt-5.5\" });",
        "const { [m]: f } = this; return f.call({ model: \"gpt-5.5\" });",
    ] {
        let body = format!(
            "class Bot {{
  private model = \"gpt-4\";
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }}
  run(m: \"ask\") {{ {run} }}
}}
new Bot().run(\"ask\");"
        );
        let (value, gaps) = model_of(&body);
        assert_eq!(value.as_deref(), Some("gpt-4"), "{run}");
        assert!(
            gaps.iter().any(|g| g == "unresolved_model/attribute"),
            "{run}: {gaps:?}"
        );
    }
    // Called through a computed member with its own `this`, the class
    // property is still the one read.
    let (value, gaps) = model_of(
        "class Bot {
  private model = \"gpt-4\";
  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }
  run(m: \"ask\") { return this[m](); }
}
new Bot().run(\"ask\");",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_member_in_parentheses_or_a_type_assertion_is_used_as_written() {
    // Parentheses keep the reference, and `!`, `as`, `satisfies` and `<T>`
    // compile to nothing: the chain inside is called, bound or written.
    for route in [
        "return this[kind]!(model);",
        "return (this[kind])(model);",
        "return (this[kind] as any)(model);",
        "return (<any>this[kind])(model);",
        "return (this[kind] satisfies any)(model);",
        "return ((this[kind] as any)!)?.(model);",
        "return (this[kind] as any).call(this, model);",
        "return (this[kind] as any)[\"apply\"](this, [model]);",
    ] {
        let body = format!(
            "class Router {{
  private chat(model = \"gpt-4o\") {{
    return client.chat.completions.create({{ model, messages: [] }});
  }}
  route(kind: \"chat\", model: string) {{ {route} }}
}}
new Router().route(\"chat\", \"gpt-5.5\");"
        );
        let (value, origin, gaps) = model_row(&body);
        assert_eq!(value.as_deref(), Some("gpt-4o"), "{route}");
        assert_eq!(origin, Some(ValueOrigin::ParameterDefault), "{route}");
        assert_eq!(gaps, vec!["unresolved_model/unseen_callers"], "{route}");
    }
    for run in [
        "return (this[m] as any).call({ model: \"gpt-5.5\" });",
        "return (this[m])!.bind({ model: \"gpt-5.5\" })();",
    ] {
        let (value, gaps) = model_of(&format!(
            "class Bot {{
  private model = \"gpt-4\";
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }}
  run(m: \"ask\") {{ {run} }}
}}
new Bot().run(\"ask\");"
        ));
        assert_eq!(value.as_deref(), Some("gpt-4"), "{run}");
        assert!(
            gaps.iter().any(|g| g == "unresolved_model/attribute"),
            "{run}: {gaps:?}"
        );
    }
    // A class property written through a wrapper is written.
    for write in [
        "(this.model as any) = \"gpt-5.5\";",
        "this.model! = \"gpt-5.5\";",
    ] {
        let (_, gaps) = model_of(&format!(
            "class Bot {{
  private model = \"gpt-4\";
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }}
  swap() {{ {write} }}
}}
new Bot().ask();"
        ));
        assert_eq!(gaps, vec!["unresolved_model/reassigned"], "{write}");
    }
    // A method with a known name called through a wrapper is a call: what
    // it is passed is listed.
    let (default, _, gaps) = model_row(
        "class Router {
  private chat(model = \"gpt-4o\") {
    return client.chat.completions.create({ model, messages: [] });
  }
  route() { return (this.chat as any)(\"gpt-5.5\"); }
}
new Router().route();",
    );
    assert_eq!(default.as_deref(), Some("gpt-4o"));
    assert_eq!(gaps, vec!["unresolved_model/parameter_overridden"]);
    // An SDK request called through a wrapper is a request, not a method
    // stored as a value.
    let found = usages(&format!(
        "{CLIENT}(client.chat.completions.create as any)({{ model: \"gpt-4o\", messages: [] }});"
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), Some("gpt-4o"));
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn an_enum_member_names_the_members_before_outer_names() {
    let (value, gaps) = model_of(
        "const Fast = \"gpt-4\";
enum Model { Fast = \"gpt-4o\", Default = Fast }
client.chat.completions.create({ model: Model.Default, messages: [] });",
    );
    assert_eq!(value.as_deref(), Some("gpt-4o"));
    assert!(gaps.is_empty(), "{gaps:?}");
    let (value, gaps) = model_of(
        "const Fast = \"gpt-4\";
enum Model { Fast = \"gpt-4o\", Default = Fast || \"x\" }
client.chat.completions.create({ model: Model.Default, messages: [] });",
    );
    assert_eq!(value, None);
    assert!(!gaps.is_empty());
}

#[test]
fn an_enum_written_through_a_cast_or_an_alias_is_not_read_only() {
    for write in [
        "(Model as any).Chat = \"gpt-4o\";",
        "const alias: any = Model; alias.Chat = \"gpt-4o\";",
        "mutate(Model);",
    ] {
        assert_held(
            &format!(
                "enum Model {{ Chat = \"gpt-4\" }}
{write}
client.chat.completions.create({{ model: Model.Chat, messages: [] }});"
            ),
            "unresolved_model/reassigned",
        );
    }
    let (value, gaps) = model_of(
        "enum Model { Chat = \"gpt-4\" }
console.log(Model);
client.chat.completions.create({ model: Model.Chat, messages: [] });",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_namespace_export_written_from_outside_is_reassigned() {
    for write in [
        "Cfg.model = \"gpt-4o\";",
        "(Cfg as any).model = \"gpt-4o\";",
        "namespace Cfg { model = \"gpt-4o\"; }",
    ] {
        assert_held(
            &format!(
                "namespace Cfg {{ export let model = \"gpt-4\";
  export function ask() {{ return client.chat.completions.create({{ model, messages: [] }}); }} }}
{write}
Cfg.ask();"
            ),
            "unresolved_model/reassigned",
        );
    }
    let (value, gaps) = model_of(
        "namespace Cfg { export const model = \"gpt-4\";
  export function ask() { return client.chat.completions.create({ model, messages: [] }); } }
Cfg.ask();",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_parenthesized_eval_is_a_direct_eval() {
    let found = usages_js(
        "const OpenAI = require(\"openai\");
const client = new OpenAI();
let model = \"gpt-4\";
(eval)(\"model = 'gpt-4o'\");
client.chat.completions.create({ model, messages: [] });
",
    );
    let c = call(&found, "client.chat.completions.create");
    assert!(
        gaps_at(&found, c)
            .iter()
            .any(|g| g == "unresolved_model/dynamic_feature"),
        "{:?}",
        found.gaps
    );
}

#[test]
fn an_object_whose_own_methods_write_this_is_not_read_only() {
    for body in [
        "const A = { model: \"gpt-4\", toJSON() { this.model = \"gpt-4o\"; return {}; } };
JSON.stringify(A);
client.chat.completions.create({ model: A.model, messages: [] });",
        "const A = { model: \"gpt-4\", toString: function () { this.model = \"gpt-4o\"; return \"\"; } };
console.log(A);
client.chat.completions.create({ model: A.model, messages: [] });",
        "const A = { inner: { model: \"gpt-4\", reset() { Object.assign(this, { model: \"gpt-4o\" }); } } };
client.chat.completions.create({ model: A.inner.model, messages: [] });",
    ] {
        let (value, gaps) = model_of(body);
        assert!(!gaps.is_empty(), "{value:?} with no gap in:\n{body}");
    }
    // A method that only reads `this` changes nothing.
    let (value, gaps) = model_of(
        "const A = { model: \"gpt-4\", describe() { return this.model; } };
client.chat.completions.create({ model: A.model, messages: [] });",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_parameter_written_in_its_function_lists_every_value() {
    for body in [
        "async function ask(model = \"gpt-4\") { model = \"gpt-4o\"; return client.chat.completions.create({ model, messages: [] }); }
ask();",
        "async function ask(model = \"gpt-4\") { if (Math.random() > 0.5) model = \"gpt-4o\"; return client.chat.completions.create({ model, messages: [] }); }
ask();",
        "async function ask(model = \"gpt-4\") { var model = \"gpt-4o\"; return client.chat.completions.create({ model, messages: [] }); }
ask();",
    ] {
        let found = usages(&format!("{CLIENT}{body}"));
        let gap = found
            .gaps
            .iter()
            .find(|g| g.reason == "reassigned")
            .unwrap_or_else(|| panic!("no reassigned gap: {:?}", found.gaps));
        let values: Vec<&str> = gap.values.iter().map(|v| v.value.as_str()).collect();
        assert!(values.contains(&"gpt-4") && values.contains(&"gpt-4o"), "{values:?}");
        assert!(gap.values_complete, "{body}");
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(value(c, "model"), None);
    }
    // `??=` writes a value the scan does not read.
    let found = usages(&format!(
        "{CLIENT}function ask({{ model = \"gpt-4\" }}: {{ model?: string }} = {{}}) {{ model ??= other(); return client.chat.completions.create({{ model, messages: [] }}); }}
ask();"
    ));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "reassigned")
        .unwrap();
    assert!(!gap.values_complete);
}

#[test]
fn a_sloppy_function_reading_arguments_may_rewrite_its_parameters() {
    let found = usages_js(
        "const OpenAI = require(\"openai\");
const client = new OpenAI();
function ask(model) { arguments[0] = \"gpt-4o\"; return client.chat.completions.create({ model, messages: [] }); }
ask(\"gpt-4\");
",
    );
    let gap = found
        .gaps
        .iter()
        .find(|g| g.key.as_deref() == Some("model"))
        .unwrap_or_else(|| panic!("no gap: {:?}", found.gaps));
    assert!(!gap.values_complete, "{gap:?}");
    // A parameter with a default is never an alias of `arguments`.
    let (value, gaps) = model_of(
        "function ask(model = \"gpt-4\") { console.log(arguments.length); return client.chat.completions.create({ model, messages: [] }); }
ask();",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn calls_through_a_function_or_class_expression_s_own_name_are_callers() {
    assert_held(
        "const ask = function retry(model = \"gpt-4\", tries = 0): any {
  if (tries === 0) return retry(\"gpt-4o\", 1);
  return client.chat.completions.create({ model, messages: [] });
};
ask();",
        "unresolved_model/parameter_overridden",
    );
    assert_held(
        "const Bot = class Inner { constructor(private model = \"gpt-4\") {}
  static fast() { return new Inner(\"gpt-4o\"); }
  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); } };
new Bot().ask();
Bot.fast().ask();",
        "unresolved_model/parameter_overridden",
    );
}

#[test]
fn an_instance_built_through_its_constructor_property_has_unseen_callers() {
    for build in [
        "withModel(m: string) { return new (this.constructor as typeof Bot)(m); }",
        "withModel(m: string) { return new (this.constructor)(m); }",
        "withModel(m: string) { const C: any = this.constructor; return new C(m); }",
    ] {
        assert_held(
            &format!(
                "class Bot {{ constructor(private model = \"gpt-4\") {{}}
  {build}
  ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
new Bot().withModel(\"gpt-4o\").ask();"
            ),
            "unresolved_model/unseen_callers",
        );
    }
    assert_held(
        "class Bot { constructor(private model = \"gpt-4\") {}
  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); } }
const b = new Bot();
new (b.constructor as any)(\"gpt-4o\").ask();",
        "unresolved_model/unseen_callers",
    );
    // Reading the constructor's name builds nothing.
    let (value, gaps) = model_of(
        "class Bot { constructor(private model = \"gpt-4\") {}
  label() { return this.constructor.name; }
  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); } }
new Bot().ask();",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_proto_key_sets_the_prototype_not_a_property() {
    let found = usages(&format!(
        "{CLIENT}client.chat.completions.create({{ model: \"gpt-5\", __proto__: {{ model: \"gpt-3.5-turbo\" }}, messages: [] }});"
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.values.len(), 1, "{:?}", c.values);
    assert_eq!(value(c, "model"), Some("gpt-5"));
    // A property it does not set may come from the prototype.
    let (value, gaps) = model_of(
        "const A = { __proto__: { model: \"gpt-4\" } };
client.chat.completions.create({ model: A.model, messages: [] });",
    );
    assert_eq!(value, None);
    assert!(!gaps.is_empty());
}

#[test]
fn a_declared_field_in_a_subclass_emits_nothing() {
    let (value, gaps) = model_of(
        "class Base { model = \"gpt-4\"; ask() { return client.chat.completions.create({ model: this.model, messages: [] }); } }
class Bot extends Base { declare model: string; }
new Bot().ask();",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

// ---------------------------------------------------------------------------
// Names bound to themselves, deep spreads, and the environment
// ---------------------------------------------------------------------------

#[test]
fn a_name_bound_to_itself_is_no_environment_and_no_crash() {
    for body in [
        "var a = a;\na.x = 1;\n",
        "const a = b;\nconst b = a;\nclient.chat.completions.create({ model: a.model, messages: [] });\n",
        "let env = env;\nenv[k] = 1;\n",
    ] {
        let found = usages(&format!("{CLIENT}{body}"));
        assert_eq!(routing_of(&found, "client"), Routing::OpenaiDirect, "{body}");
    }
    let mut source = String::from(CLIENT);
    source.push_str("const a0 = process.env;\n");
    for i in 1..15_000 {
        source.push_str(&format!("const a{i} = a{};\n", i - 1));
    }
    source.push_str("client.chat.completions.create({ model: a14999.MODEL, messages: [] });\n");
    let found = usages(&source);
    let c = call(&found, "client.chat.completions.create");
    assert!(!gaps_at(&found, c).is_empty());
}

#[test]
fn spreads_nested_past_the_bound_are_opaque_not_a_crash() {
    let mut inline = String::from("{ model: \"gpt-4\" }");
    for _ in 0..2_000 {
        inline = format!("{{ ...{inline} }}");
    }
    let found = usages(&format!(
        "{CLIENT}client.chat.completions.create({{ ...{inline}, messages: [] }});\n"
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), None);
    assert!(!gaps_at(&found, c).is_empty());

    let mut named = String::from(CLIENT);
    named.push_str("const a0 = { model: \"gpt-4\" };\n");
    for i in 1..2_000 {
        named.push_str(&format!("const a{i} = {{ ...a{} }};\n", i - 1));
    }
    named.push_str("client.chat.completions.create({ ...a1999, messages: [] });\n");
    named.push_str("client.chat.completions.create({ model: a1999.model, messages: [] });\n");
    named.push_str("new OpenAI({ ...a1999 });\n");
    let found = usages(&named);
    for c in found
        .calls
        .iter()
        .filter(|c| c.member == "chat.completions.create")
    {
        assert_eq!(value(c, "model"), None);
    }
    // Within the bound, a spread chain still resolves.
    let (value, gaps) = model_of(
        "const a0 = { model: \"gpt-4\" };
const a1 = { ...a0 };
const a2 = { ...a1, temperature: 0 };
client.chat.completions.create({ ...a2, messages: [] });",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn an_object_read_under_many_paths_stays_read_only() {
    let mut body = String::from("const CFG = {");
    for i in 0..200 {
        body.push_str(&format!(" k{i}: \"gpt-{i}\","));
    }
    body.push_str(" };\n");
    for i in 0..199 {
        body.push_str(&format!("console.log(CFG.k{i});\n"));
    }
    body.push_str("client.chat.completions.create({ model: CFG.k199, messages: [] });\n");
    let (value, gaps) = model_of(&body);
    assert_eq!(value.as_deref(), Some("gpt-199"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn no_environment_variable_name_is_reported() {
    for body in [
        "const { OPENAI_MODEL } = process.env;
client.chat.completions.create({ model: OPENAI_MODEL, messages: [] });",
        "const { env: { OPENAI_MODEL } } = process;
client.chat.completions.create({ model: OPENAI_MODEL, messages: [] });",
        "client.chat.completions.create({ model: globalThis.process.env.OPENAI_MODEL, messages: [] });",
        "client.chat.completions.create({ model: process[\"env\"].OPENAI_MODEL, messages: [] });",
        "client.chat.completions.create({ model: Deno.env.toObject().OPENAI_MODEL, messages: [] });",
        "import process from \"node:process\";
client.chat.completions.create({ model: process.env.OPENAI_MODEL, messages: [] });",
    ] {
        let found = usages(&format!("{CLIENT}{body}"));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(gaps_at(&found, c), vec!["unresolved_model/env_var"], "{body}");
        let json = serde_json::to_string(&found).unwrap();
        assert!(!json.contains("OPENAI_MODEL"), "{body}\n{json}");
    }
    // A destructured default is the lookup's fallback.
    let found = usages(&format!(
        "{CLIENT}const {{ OPENAI_MODEL = \"gpt-4o\" }} = process.env;
client.chat.completions.create({{ model: OPENAI_MODEL, messages: [] }});"
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), Some("gpt-4o"));
    assert_eq!(c.values[0].origin, ValueOrigin::EnvDefault);
    assert_eq!(gaps_at(&found, c), vec!["unresolved_model/env_var"]);
    assert!(!serde_json::to_string(&found)
        .unwrap()
        .contains("OPENAI_MODEL"));
}

#[test]
fn the_endpoint_variable_written_through_any_alias_of_process_is_seen() {
    for write in [
        "import process from \"node:process\";\nprocess.env.OPENAI_BASE_URL = proxy;",
        "import * as proc from \"process\";\nproc.env.OPENAI_BASE_URL = proxy;",
        "import { env } from \"node:process\";\nenv.OPENAI_BASE_URL = proxy;",
        "const { env } = process;\nenv.OPENAI_BASE_URL = proxy;",
        "const p = process;\np.env.OPENAI_BASE_URL = proxy;",
        "globalThis.process.env.OPENAI_BASE_URL = proxy;",
        "require(\"process\").env.OPENAI_BASE_URL = proxy;",
        "process[\"env\"].OPENAI_BASE_URL = proxy;",
        "Deno.env.set(\"OPENAI_BASE_URL\", proxy);",
    ] {
        let (routing, gaps) = route(
            &format!(
                "{OPENAI}{write}
const client = new OpenAI();
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
            ),
            "chat.completions.create",
        );
        assert_eq!(routing, Routing::Unknown, "{write}");
        assert_eq!(gaps, vec!["ambiguous_client/endpoint_env"], "{write}");
    }
}

#[test]
fn a_mock_of_an_imported_module_mocks_the_sdk() {
    let found = usages(
        "import OpenAI from \"openai\";
vi.mock(import(\"openai\"), () => ({ default: class {} }));
const client = new OpenAI();
",
    );
    assert_eq!(routing_of(&found, "client"), Routing::Unknown);
}

// ---------------------------------------------------------------------------
// Routes the review showed may go elsewhere
// ---------------------------------------------------------------------------

#[test]
fn a_spread_written_after_the_base_url_may_replace_it() {
    for options in [
        "{ baseURL: \"https://api.openai.com/v1\", ...overrides }",
        "{ baseURL: \"https://api.openai.com/v1\", [key]: value }",
        "{ baseURL: \"https://proxy.example.com/v1\", ...overrides }",
    ] {
        let (routing, gaps) = route(
            &format!(
                "{OPENAI}const client = new OpenAI({options});
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
            ),
            "chat.completions.create",
        );
        assert_eq!(routing, Routing::Unknown, "{options}");
        assert_eq!(
            gaps,
            vec!["ambiguous_client/base_url_unresolved"],
            "{options}"
        );
    }
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI().withOptions({{ baseURL: \"https://api.openai.com/v1\", ...extra }});
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert!(!gaps.is_empty());
    // Written before the base URL, a spread is replaced by it.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI({{ ...overrides, baseURL: \"https://api.openai.com/v1\" }});
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_reflective_write_on_an_sdk_object_reroutes_or_patches_it() {
    for write in [
        "Object.assign(client, { baseURL: \"https://gateway.example.com\" });",
        "Object.defineProperty(client, \"baseURL\", { value: \"https://gateway.example.com\" });",
        "Reflect.set(client, \"baseURL\", \"https://gateway.example.com\");",
    ] {
        let (routing, gaps) = route(
            &format!(
                "import Anthropic from \"@anthropic-ai/sdk\";
const client = new Anthropic();
{write}
client.messages.create({{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 1, messages: [] }});"
            ),
            "messages.create",
        );
        assert_eq!(routing, Routing::Unknown, "{write}");
        assert_eq!(gaps, vec!["ambiguous_client/base_url_assigned"], "{write}");
    }
    for write in [
        "Object.assign(OpenAI.prototype, { buildURL() { return \"x\"; } });",
        "Object.defineProperty(OpenAI.prototype, \"baseURL\", { get() { return \"x\"; } });",
        "Object.assign(client.chat.completions, { create: fake });",
        "Object.setPrototypeOf(client, other);",
    ] {
        let (routing, gaps) = route(
            &format!(
                "{OPENAI}const client = new OpenAI();
{write}
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
            ),
            "chat.completions.create",
        );
        assert_eq!(routing, Routing::Unknown, "{write}");
        assert_eq!(gaps, vec!["ambiguous_client/dynamic_feature"], "{write}");
    }
    // Settings change nothing.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI();
Object.assign(client, {{ maxRetries: 5 }});
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn gemini_request_options_may_name_an_endpoint() {
    let found = usages(
        "import { GoogleGenerativeAI } from \"@google/generative-ai\";
const genAI = new GoogleGenerativeAI(key);
const model = genAI.getGenerativeModel({ model: \"gemini-1.5-flash\" }, { baseUrl: \"https://gateway.example.com\" });
model.generateContent(\"hi\");
",
    );
    let routed = call(&found, "genAI.getGenerativeModel");
    assert_eq!(routed.routing, Routing::Unknown);
    assert_eq!(
        gaps_at(&found, routed),
        vec!["ambiguous_client/endpoint_option"]
    );
    let found = usages(
        "import { GoogleGenerativeAI } from \"@google/generative-ai\";
const genAI = new GoogleGenerativeAI(key);
const model = genAI.getGenerativeModel({ model: \"gemini-1.5-flash\" });
model.generateContent(\"hi\", { baseUrl: \"https://gateway.example.com\" });
model.startChat().sendMessage(\"hi\", options);
model.generateContent(\"hi\");
",
    );
    let calls: Vec<&SdkCall> = found
        .calls
        .iter()
        .filter(|c| c.member == "generateContent" || c.member == "sendMessage")
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].routing, Routing::Unknown);
    assert_eq!(calls[1].routing, Routing::Unknown);
    assert_eq!(calls[2].routing, Routing::GoogleGenerativeaiLegacy);
    assert!(gaps_at(&found, calls[2]).is_empty());

    for config in [
        "{ httpOptions: { baseUrl: \"https://gateway.example.com\" } }",
        "{ httpOptions: opts }",
        "settings",
    ] {
        let (routing, gaps) = route(
            &format!(
                "import {{ GoogleGenAI }} from \"@google/genai\";
const ai = new GoogleGenAI({{ apiKey: key }});
ai.models.generateContent({{ model: \"gemini-1.5-flash\", contents: \"hi\", config: {config} }});"
            ),
            "models.generateContent",
        );
        assert_eq!(routing, Routing::Unknown, "{config}");
        assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"], "{config}");
    }
    let (routing, gaps) = route(
        "import { GoogleGenAI } from \"@google/genai\";
const ai = new GoogleGenAI({ apiKey: key });
ai.models.generateContent({ model: \"gemini-1.5-flash\", contents: \"hi\", config: { temperature: 0 } });",
        "models.generateContent",
    );
    assert_eq!(routing, Routing::GoogleGenai);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn set_default_base_urls_through_require_or_an_alias_reroutes_gemini() {
    for source in [
        "const genai = require(\"@google/genai\");
genai.setDefaultBaseUrls({ geminiUrl: \"https://proxy.example.com\" });
const ai = new genai.GoogleGenAI({ apiKey: \"k\" });
ai.models.generateContent({ model: \"gemini-1.5-flash\", contents: \"x\" });
",
        "import { GoogleGenAI, setDefaultBaseUrls } from \"@google/genai\";
const configure = setDefaultBaseUrls;
const ai = new GoogleGenAI({ apiKey: \"k\" });
configure({ geminiUrl: \"https://proxy.example.com\" });
ai.models.generateContent({ model: \"gemini-1.5-flash\", contents: \"x\" });
",
    ] {
        let found = scan(source, "app.js");
        let c = call(&found, "ai.models.generateContent");
        assert_eq!(c.routing, Routing::Unknown, "{source}");
        assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/endpoint_option"]);
        assert_eq!(routing_of(&found, "ai"), Routing::Unknown);
    }
}

#[test]
fn a_custom_fetch_or_an_empty_base_url() {
    for source in [
        "import Anthropic from \"@anthropic-ai/sdk\";
const client = new Anthropic({ fetch: rewrite });
client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 1, messages: [] });",
        "import Anthropic from \"@anthropic-ai/sdk\";
const client = new Anthropic().withOptions({ fetch: rewrite });
client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 1, messages: [] });",
    ] {
        let (routing, gaps) = route(source, "messages.create");
        assert_eq!(routing, Routing::Unknown, "{source}");
        assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"], "{source}");
    }
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI({{ baseURL: \"\" }});
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_middleware_may_send_requests_elsewhere_unless_it_is_the_sdk_refusal_fallback() {
    // A middleware may change the request before passing it on, or answer
    // it itself: the same power as a custom `fetch`.
    let gateway = "const viaGateway = async (req: any, next: any) =>
  next({ ...req, url: String(req.url).replace(\"api.anthropic.com\", \"gateway.example.com\") });
";
    let request = "client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] });";
    let fallback = "betaRefusalFallbackMiddleware([{ model: \"claude-opus-4-8\" }])";
    for (imports, client) in [
        (
            "import Anthropic from \"@anthropic-ai/sdk\";",
            "new Anthropic({ middleware: [viaGateway] })".to_string(),
        ),
        (
            "import Anthropic from \"@anthropic-ai/sdk\";",
            "new Anthropic().withOptions({ middleware: [viaGateway] })".to_string(),
        ),
        // The SDK's fallback next to one of the file's own.
        (
            "import Anthropic, { betaRefusalFallbackMiddleware } from \"@anthropic-ai/sdk\";",
            format!("new Anthropic({{ middleware: [{fallback}, viaGateway] }})"),
        ),
        // A function of the same name that is not the SDK's.
        (
            "import Anthropic from \"@anthropic-ai/sdk\";
function betaRefusalFallbackMiddleware(fallbacks: any) { return viaGateway; }",
            format!("new Anthropic({{ middleware: [{fallback}] }})"),
        ),
        (
            "import Anthropic from \"@anthropic-ai/sdk\";
import { betaRefusalFallbackMiddleware } from \"./gateway\";",
            format!("new Anthropic({{ middleware: [{fallback}] }})"),
        ),
        // An array the file may change, or one read from elsewhere.
        (
            &format!(
                "import Anthropic, {{ betaRefusalFallbackMiddleware }} from \"@anthropic-ai/sdk\";
const MIDDLEWARE = [{fallback}];"
            ),
            "new Anthropic({ middleware: MIDDLEWARE })".to_string(),
        ),
        (
            "import Anthropic from \"@anthropic-ai/sdk\";",
            "new Anthropic({ middleware: loadMiddleware() })".to_string(),
        ),
        // An item read off the client being built is not the SDK's.
        (
            "import Anthropic from \"@anthropic-ai/sdk\";",
            "new Anthropic({ middleware: [client.hook()] })".to_string(),
        ),
    ] {
        let source = format!("{imports}\n{gateway}const client = {client};\n{request}");
        let (routing, gaps) = route(&source, "messages.create");
        assert_eq!(routing, Routing::Unknown, "{source}");
        assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"], "{source}");
    }

    // The SDK's own refusal fallback only sends the request again with a
    // fallback model: the route stays.
    for (imports, client) in [
        (
            "import Anthropic, { betaRefusalFallbackMiddleware } from \"@anthropic-ai/sdk\";",
            format!("new Anthropic({{ middleware: [{fallback}] }})"),
        ),
        (
            "import Anthropic, { betaRefusalFallbackMiddleware } from \"@anthropic-ai/sdk\";",
            format!("new Anthropic().withOptions({{ middleware: [{fallback}] }})"),
        ),
        (
            "import { Anthropic, betaRefusalFallbackMiddleware as onRefusal } from \"@anthropic-ai/sdk\";",
            "new Anthropic({ middleware: [onRefusal([{ model: \"claude-opus-4-8\" }])] })"
                .to_string(),
        ),
        (
            "import * as sdk from \"@anthropic-ai/sdk\";",
            format!("new sdk.Anthropic({{ middleware: [sdk.{fallback}] }})"),
        ),
    ] {
        let source = format!("{imports}\n{gateway}const client = {client};\n{request}");
        let (routing, gaps) = route(&source, "messages.create");
        assert_eq!(routing, Routing::AnthropicDirect, "{source}");
        assert!(gaps.is_empty(), "{source}: {gaps:?}");
    }
}

#[test]
fn a_global_fetch_written_in_the_file_is_a_custom_fetch() {
    // The SDKs send every request through the global `fetch` unless the
    // client is given its own: the in-file form of what nock and msw do.
    let request = "client.chat.completions.create({ model: \"gpt-4o\", messages: [] });";
    for write in [
        "globalThis.fetch = (url: any, init: any) => proxy(url, init);",
        "global.fetch = proxy;",
        "window.fetch = proxy;",
        "self.fetch = proxy;",
        "globalThis[\"fetch\"] = proxy;",
        "(globalThis as any).fetch = proxy;",
        "const g: any = globalThis;\ng.fetch = proxy;",
        "Object.assign(globalThis, { fetch: proxy });",
        "Reflect.set(globalThis, \"fetch\", proxy);",
        "Object.defineProperty(globalThis, \"fetch\", { value: proxy });",
        "function install() { fetch = proxy; }",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{write}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{write}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/endpoint_option"],
            "{write}"
        );
        assert_eq!(routing_of(&found, "client"), Routing::Unknown, "{write}");
    }
    // Anthropic's and Gemini's clients go through it too.
    for (client, request) in [
        (
            "import Anthropic from \"@anthropic-ai/sdk\";\nconst client = new Anthropic();",
            "client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 1, messages: [] });",
        ),
        (
            "import { GoogleGenAI } from \"@google/genai\";\nconst client = new GoogleGenAI({});",
            "client.models.generateContent({ model: \"gemini-1.5-flash\", contents: \"x\" });",
        ),
    ] {
        let source = format!("{client}\nglobalThis.fetch = proxy;\n{request}");
        let found = usages(&source);
        let c = &found.calls[0];
        assert_eq!(c.routing, Routing::Unknown, "{source}");
        assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/endpoint_option"]);
    }
    // Read, or written on an object literal of the file: nothing replaced.
    for other in [
        "const original = globalThis.fetch;",
        "const api = { fetch: proxy };\napi.fetch = other;",
        "let fetch = proxy;\nfetch = other;",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{other}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{other}");
        assert!(gaps_at(&found, c).is_empty(), "{other}");
    }
}

#[test]
fn an_unresolved_client_on_any_model_bearing_member_is_a_gap() {
    for (call_text, sdk) in [
        (
            "genAI.getGenerativeModel({ model: \"gemini-1.5-flash\" })",
            Sdk::Google,
        ),
        (
            "ai.chats.create({ model: \"gemini-1.5-flash\" })",
            Sdk::Google,
        ),
        (
            "ai.models.generateImages({ model: \"imagen-3.0-generate-002\", prompt })",
            Sdk::Google,
        ),
        (
            "openai.beta.assistants.create({ model: \"gpt-4\" })",
            Sdk::Openai,
        ),
        (
            "openai.fineTuning.jobs.create({ model: \"gpt-4o-mini-2024-07-18\", training_file })",
            Sdk::Openai,
        ),
        (
            "openai.moderations.create({ model: \"text-moderation-007\", input })",
            Sdk::Openai,
        ),
        (
            "anthropic.beta.messages.toolRunner({ model: \"claude-3-5-sonnet-20241022\" })",
            Sdk::Anthropic,
        ),
    ] {
        let found = usages(&format!(
            "import {{ genAI, ai, openai, anthropic }} from \"@/lib/clients\";
export async function run(prompt: string, input: string, training_file: string) {{
  return {call_text};
}}
"
        ));
        let gap = found
            .gaps
            .iter()
            .find(|g| g.kind == GapKind::UnknownClientFactory)
            .unwrap_or_else(|| panic!("no gap for {call_text}: {:?}", found.gaps));
        assert_eq!(gap.sdk, Some(sdk), "{call_text}");
        assert_eq!(gap.reason, "imported_name");
        assert_eq!(gap.values.len(), 1, "{call_text}");
    }
    // Without a model written, an unknown receiver's `chats.create` is not
    // taken to be an SDK.
    let found = usages(
        "import { slack } from \"./slack\";
import OpenAI from \"openai\";
slack.chats.create({ channel: \"x\" });
",
    );
    assert!(found.gaps.is_empty(), "{:?}", found.gaps);
}

#[test]
fn a_file_calling_only_images_or_audio_is_scanned() {
    let found = usages(
        "import { ai } from \"./lib/ai\";
export async function speak(text: string) { return ai.audio.speech.create({ model: \"tts-1\", voice: \"alloy\", input: text }); }
export async function draw(prompt: string) { return ai.images.generate({ model: \"dall-e-2\", prompt }); }
",
    );
    assert_eq!(
        gaps(&found, GapKind::UnknownClientFactory).len(),
        2,
        "{:?}",
        found.gaps
    );
}

#[test]
fn a_request_body_after_a_path_parameter_holds_the_model() {
    let found = usages(&format!(
        "{CLIENT}client.beta.threads.runs.create(\"thread_abc\", {{ assistant_id: \"asst_abc\", model: \"gpt-4\" }});
client.beta.threads.runs.createAndPoll(threadId, {{ assistant_id: \"asst_abc\" }});"
    ));
    let runs = call(&found, "client.beta.threads.runs.create");
    assert_eq!(value(runs, "model"), Some("gpt-4"));
    assert!(gaps_at(&found, runs).is_empty(), "{:?}", found.gaps);
    assert_eq!(runs.kwargs, vec!["assistant_id", "model"]);
    let poll = call(&found, "client.beta.threads.runs.createAndPoll");
    assert_eq!(
        gaps_at(&found, poll),
        vec!["unresolved_model/missing_model"]
    );
}

#[test]
fn a_per_request_endpoint_unconfirms_where_the_model_was_given() {
    let found = usages(
        "import { GoogleGenerativeAI } from \"@google/generative-ai\";
const genAI = new GoogleGenerativeAI(key);
const model = genAI.getGenerativeModel({ model: \"gemini-1.5-flash\" });
await model.generateContent(\"hi\", { baseUrl: \"https://proxy.example.com\" });
",
    );
    let given = call(&found, "genAI.getGenerativeModel");
    assert_eq!(value(given, "model"), Some("gemini-1.5-flash"));
    assert_eq!(given.routing, Routing::Unknown);
    assert_eq!(
        gaps_at(&found, given),
        vec!["ambiguous_client/endpoint_option"]
    );
    let found = usages(
        "import { GoogleGenAI } from \"@google/genai\";
const ai = new GoogleGenAI({ apiKey: key });
const chat = ai.chats.create({ model: \"gemini-1.5-flash\" });
await chat.sendMessage({ message: \"hi\", config: { httpOptions: { baseUrl: \"https://proxy.example.com\" } } });
",
    );
    let given = call(&found, "ai.chats.create");
    assert_eq!(given.routing, Routing::Unknown);
    assert_eq!(
        gaps_at(&found, given),
        vec!["ambiguous_client/endpoint_option"]
    );
}

#[test]
fn the_environment_handed_to_code_that_may_write_it_is_unknown() {
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}function setup(env: NodeJS.ProcessEnv) {{ env.OPENAI_BASE_URL = \"https://proxy.example.com/v1\"; }}
setup(process.env);
const client = new OpenAI();
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/dynamic_feature"]);
    // Reading it changes nothing.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}console.log(Object.keys(process.env), JSON.stringify(process.env));
const client = new OpenAI();
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn nested_model_keys_are_read_as_deep_as_in_python() {
    let found = usages(&format!(
        "{CLIENT}client.batches.create({{ requests: [{{ p2: {{ p1: {{ p0: {{ model: \"gpt-4\" }} }} }} }}] }});
client.batches.create({{ requests: [{{ p3: {{ p2: {{ p1: {{ p0: {{ model: \"gpt-4\" }} }} }} }} }}] }});"
    ));
    let calls: Vec<&SdkCall> = found
        .calls
        .iter()
        .filter(|c| c.member == "batches.create")
        .collect();
    assert_eq!(calls[0].values.len(), 1, "{:?}", calls[0].values);
    assert_eq!(calls[0].values[0].key, "requests.p2.p1.p0.model");
    assert!(calls[1].values.is_empty());
}

#[test]
fn a_request_on_an_object_an_sdk_call_returned_lists_its_model() {
    let found = usages(
        "import { GoogleAICacheManager } from \"@google/generative-ai/server\";
const cacheManager = new GoogleAICacheManager(key);
const cache = await cacheManager.create({ model: \"models/gemini-1.5-flash\", contents: [] });
await cacheManager.list();
",
    );
    let gaps = gaps(&found, GapKind::UnknownClientFactory);
    assert_eq!(gaps.len(), 1, "{:?}", found.gaps);
    assert_eq!(gaps[0].reason, "sdk_call_result");
    assert_eq!(gaps[0].values[0].value, "models/gemini-1.5-flash");
}
