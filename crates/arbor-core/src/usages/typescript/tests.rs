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
fn openai_v3_api_given_its_own_axios_instance_may_go_elsewhere() {
    // v3's third argument is the axios instance it sends through: the v3
    // form of a custom `fetch`.
    let found = usages_js(
        r#"
const axios = require("axios");
const { Configuration, OpenAIApi } = require("openai");
const http = axios.create();
http.interceptors.request.use((c) => { c.url = c.url.replace("api.openai.com", "gateway.example.com"); return c; });
const openai = new OpenAIApi(new Configuration({ apiKey: process.env.KEY }), undefined, http);
const plain = new OpenAIApi(new Configuration({ apiKey: process.env.KEY }), undefined, undefined);
exports.r = () => openai.createChatCompletion({ model: "gpt-3.5-turbo-0301", messages: [] });
"#,
    );
    let c = call(&found, "openai.createChatCompletion");
    assert_eq!(c.routing, Routing::Unknown);
    assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/endpoint_option"]);
    assert_eq!(routing_of(&found, "openai"), Routing::Unknown);
    assert_eq!(routing_of(&found, "plain"), Routing::OpenaiDirect);
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
fn many_computed_keys_in_object_patterns_are_linear() {
    // Every computed key of an object pattern, deep in nested scopes, over
    // names written many times: each key costs the same small step.
    let mut source =
        with_client("client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] });\n");
    for i in 0..60 {
        source.push_str(&format!("function f{i}(k) {{\n"));
    }
    for (name, init) in [("d", "new X()"), ("c", "d"), ("b", "c"), ("a", "b")] {
        source.push_str(&format!("let {name} = {init};\n"));
        for _ in 0..63 {
            source.push_str(&format!("{name} = {init};\n"));
        }
    }
    let keys = "[k]: v, ".repeat(100);
    for _ in 0..400 {
        source.push_str(&format!("({{ {keys} }} = a);\n"));
    }
    source.push_str(&"}\n".repeat(60));
    let found = usages_within(&source, 10);
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), Some("gpt-4-0613"));
    assert_eq!(c.routing, Routing::OpenaiDirect);
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
fn a_hash_private_method_is_out_of_reach_of_a_computed_member() {
    // `this[kind]` reads a property by a string or symbol: never `#chat`.
    for elsewhere in [
        "dispatch(kind: string, m: string) { return (this as any)[kind](m); }",
        "dispatch(m: string) { return (this as any)[m].call({ model: \"gpt-5.5\" }); }",
        "dispatch(m: string) { const { [m]: f } = this as any; return f(\"gpt-5.5\"); }",
    ] {
        let (value, origin, gaps) = model_row(&format!(
            "class Router {{
  #chat(model = \"gpt-4o\") {{
    return client.chat.completions.create({{ model, messages: [] }});
  }}
  route() {{ return this.#chat(); }}
  {elsewhere}
}}
new Router().route();"
        ));
        assert_eq!(value.as_deref(), Some("gpt-4o"), "{elsewhere}");
        assert_eq!(origin, Some(ValueOrigin::ParameterDefault), "{elsewhere}");
        assert!(gaps.is_empty(), "{elsewhere}: {gaps:?}");
        let (value, gaps) = model_of(&format!(
            "class Bot {{
  private model = \"gpt-4\";
  #ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }}
  run() {{ return this.#ask(); }}
  {elsewhere}
}}
new Bot().run();"
        ));
        assert_eq!(value.as_deref(), Some("gpt-4"), "{elsewhere}");
        assert!(gaps.is_empty(), "{elsewhere}: {gaps:?}");
    }
    // A `private` method is a plain property at run time: it is reached.
    let (_, _, gaps) = model_row(
        "class Router {
  private chat(model = \"gpt-4o\") {
    return client.chat.completions.create({ model, messages: [] });
  }
  route() { return this.chat(); }
  dispatch(kind: string, m: string) { return (this as any)[kind](m); }
}
new Router().route();",
    );
    assert_eq!(gaps, vec!["unresolved_model/unseen_callers"]);
}

#[test]
fn a_computed_key_in_any_object_pattern_may_read_a_method() {
    // As Python's `getattr(obj, name)`, a computed key in an object pattern
    // counts file-wide, whatever the pattern destructures: the file cannot
    // tell an instance from a record (`new this()`, a `Bot | undefined`
    // parameter, an instance held in a property, a factory's result, an
    // untyped parameter, a parameter pattern).
    let bot = "class Bot {
  private model = \"gpt-4\";
  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }
  static run(m: string) { const inst = new this(); const { [m]: f } = inst as any; return f.call({ model: \"gpt-5.5\" }); }
}
";
    let router = "class Router {
  private chat(model = \"gpt-4o\") {
    return client.chat.completions.create({ model, messages: [] });
  }
  route() { return this.chat(); }
}
new Router().route();
";
    let class_free = "class Bot {
  private model = \"gpt-4\";
  ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }
}
";
    let (value, gaps) = model_of(bot);
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(
        gaps.iter().any(|g| g == "unresolved_model/attribute"),
        "{gaps:?}"
    );
    for helper in [
        "export function run(bot: Bot | undefined, m: string) { const { [m]: f } = bot as any; return f.call({ model: \"gpt-5.5\" }); }",
        "class Holder { private bot = new Bot(); run(m: string) { const { [m]: f } = this.bot as any; return f.call({ model: \"gpt-5.5\" }); } }",
        "class Holder { constructor(private readonly bot: Bot) {} run(m: string) { const { [m]: f } = this.bot as any; return f.call({ model: \"gpt-5.5\" }); } }",
        "function makeBot() { return new Bot(); }
export function run(m: string) { const { [m]: f } = makeBot() as any; return f.call({ model: \"gpt-5.5\" }); }",
        "export function run(bot: any, m: string) { const { [m]: f } = bot; return f.call({ model: \"gpt-5.5\" }); }",
        "const bots = { support: new Bot() };
export function run(m: string) { const { [m]: f } = bots.support as any; return f.call({ model: \"gpt-5.5\" }); }",
        "export function invoke(k: \"ask\", { [k]: f }: Bot) { return f.call({ model: \"gpt-5.5\" } as any); }",
        "export function run(m: string, alt?: any) { let f: any; ({ [m]: f } = alt ?? new Bot()); return f.call({}); }",
        // The omit/redact idiom over-approximates the same way.
        "export function redact<T extends object>(ticket: T, field: keyof T) {
  const { [field]: _hidden, ...safe } = ticket;
  return safe;
}",
    ] {
        let (value, gaps) = model_of(&format!("{class_free}{helper}"));
        assert_eq!(value.as_deref(), Some("gpt-4"), "{helper}");
        assert!(
            gaps.iter().any(|g| g == "unresolved_model/attribute"),
            "{helper}: {gaps:?}"
        );
        let (value, origin, gaps) = model_row(&format!("{router}{helper}"));
        assert_eq!(value.as_deref(), Some("gpt-4o"), "{helper}");
        assert_eq!(origin, Some(ValueOrigin::ParameterDefault), "{helper}");
        assert_eq!(gaps, vec!["unresolved_model/unseen_callers"], "{helper}");
    }
    // The `unseen_callers` detail names the pattern and its line.
    let found = usages(&with_client(&format!(
        "{router}export function redact(ticket: object, field: string) {{
  const {{ [field]: _hidden, ...safe }} = ticket as any;
  return safe;
}}"
    )));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "unseen_callers")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert!(
        gap.detail
            .contains("a computed key in an object pattern at line 11"),
        "{}",
        gap.detail
    );
    // A literal key reads that member only.
    let (value, gaps) = model_of(&format!(
        "{class_free}export function pick(row: any) {{ const {{ [\"id\"]: id, name }} = row; return [id, name]; }}"
    ));
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn unseen_callers_names_the_call_under_a_run_time_name() {
    // The call may be in code that has nothing to do with the method.
    let found = usages(&with_client(
        "class Summarizer {
  private summarize(text: string, model = \"gpt-4o\") {
    return client.chat.completions.create({ model, messages: [{ role: \"user\", content: text }] });
  }
  run(text: string) { return this.summarize(text); }
}
export function log(level: \"info\" | \"warn\", message: string) {
  console[level](message);
}",
    ));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "unseen_callers")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert!(
        gap.detail.contains("name known only at run time"),
        "{}",
        gap.detail
    );
    assert!(gap.detail.contains("at line 10"), "{}", gap.detail);
    assert!(!gap.detail.contains("passed, stored"), "{}", gap.detail);
    // Passed or stored, the function's own use is the reason.
    let found = usages(&with_client(
        "class Summarizer {
  private summarize(text: string, model = \"gpt-4o\") {
    return client.chat.completions.create({ model, messages: [{ role: \"user\", content: text }] });
  }
  run(texts: string[]) { return texts.map(this.summarize); }
}",
    ));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "unseen_callers")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert!(gap.detail.contains("passed, stored"), "{}", gap.detail);
    assert!(!gap.detail.contains("at line"), "{}", gap.detail);
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
    // Written before the base URL, a spread is replaced by it, but may still
    // hold a custom `fetch`.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI({{ ...overrides, baseURL: \"https://api.openai.com/v1\" }});
client.chat.completions.create({{ model: \"gpt-4\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"]);
    // A spread of a literal the file shows, with no `fetch` in it, does not.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const overrides = {{ timeout: 5000 }};
const client = new OpenAI({{ ...overrides, baseURL: \"https://api.openai.com/v1\" }});
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
fn a_request_given_middleware_or_another_url_may_go_elsewhere() {
    // A request's own options: Anthropic runs its `middleware` after the
    // client's (the SDK's examples/middleware.ts), and both SDKs build the
    // URL from its `path` and `defaultBaseURL`.
    let gateway = "const viaGateway = async (req: any, next: any) =>
  next({ ...req, url: String(req.url).replace(\"api.anthropic.com\", \"gateway.example.com\") });
";
    let params = "{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }";
    let fallback = "betaRefusalFallbackMiddleware([{ model: \"claude-opus-4-8\" }])";
    let anthropic =
        "import Anthropic, { betaRefusalFallbackMiddleware } from \"@anthropic-ai/sdk\";";
    for request in [
        format!("client.messages.create({params}, {{ middleware: [viaGateway] }});"),
        format!("client.messages.stream({params}, {{ middleware: [viaGateway] }});"),
        format!("client.beta.messages.create({params}, {{ middleware: [viaGateway] }});"),
        format!("client.messages.countTokens({params}, {{ middleware: [viaGateway] }});"),
        format!("const opts = {{ middleware: [viaGateway] }};\nclient.messages.create({params}, opts);"),
        format!("client.messages.create({params}, {{ middleware: [{fallback}, viaGateway] }});"),
        format!("client.messages.create({params}, {{ defaultBaseURL: \"https://gateway.example.com\" }});"),
        format!("client.messages.create({params}, {{ path: \"https://gateway.example.com/v1/messages\" }});"),
    ] {
        let found = usages(&format!(
            "{anthropic}\n{gateway}const client = new Anthropic();\n{request}"
        ));
        let c = found
            .calls
            .iter()
            .find(|c| c.member.ends_with("create") || c.member.ends_with("stream") || c.member.ends_with("countTokens"))
            .unwrap_or_else(|| panic!("no request in {request}: {:?}", found.calls));
        assert_eq!(c.routing, Routing::Unknown, "{request}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/endpoint_option"],
            "{request}"
        );
        // The value is still confirmed, and the client's own route stays.
        assert_eq!(value(c, "model"), Some("claude-3-5-sonnet-20241022"));
        assert_eq!(routing_of(&found, "client"), Routing::AnthropicDirect);
    }
    // Options this file does not show may hold any of them, and a `body`
    // that replaces the one written.
    for request in [
        format!("client.messages.create({params}, requestOptions);"),
        format!("client.messages.create({params}, {{ ...requestOptions, timeout: 1000 }});"),
        format!("client.messages.create({params}, {{ [key]: [viaGateway] }});"),
    ] {
        let found = usages(&format!(
            "{anthropic}\n{gateway}const client = new Anthropic();\n{request}"
        ));
        let c = call(&found, "client.messages.create");
        assert_eq!(c.routing, Routing::Unknown, "{request}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["unresolved_model/splat", "ambiguous_client/endpoint_option"],
            "{request}"
        );
        assert_eq!(value(c, "model"), None);
        assert_eq!(routing_of(&found, "client"), Routing::AnthropicDirect);
    }
    // OpenAI's request options take a `path` and a `defaultBaseURL` too.
    for (options, body) in [
        (
            "{ path: \"https://gateway.example.com/v1/chat/completions\" }",
            false,
        ),
        (
            "{ defaultBaseURL: \"https://gateway.example.com/v1\" }",
            false,
        ),
        ("requestOptions", true),
    ] {
        let (routing, gaps) = route(
            &format!(
                "{OPENAI}const client = new OpenAI();
client.chat.completions.create({{ model: \"gpt-4o\", messages: [] }}, {options});"
            ),
            "chat.completions.create",
        );
        assert_eq!(routing, Routing::Unknown, "{options}");
        let mut expected = vec!["ambiguous_client/endpoint_option"];
        if body {
            expected.insert(0, "unresolved_model/splat");
        }
        assert_eq!(gaps, expected, "{options}");
    }
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI();
client.beta.threads.runs.create(\"thread_abc\", {{ assistant_id: \"asst_abc\", model: \"gpt-4\" }}, {{ path: \"https://gateway.example.com/v1/runs\" }});"
        ),
        "beta.threads.runs.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"]);

    // Options that only set how the request is sent keep the route, as the
    // SDK's own refusal fallback does (the real slice's
    // `{ fallbackState: state }`).
    for request in [
        format!("client.messages.create({params}, {{ fallbackState: state }});"),
        format!("client.messages.create({params}, {{ middleware: [{fallback}] }});"),
        format!("client.messages.create({params}, {{ signal: controller.signal, timeout: 1000, maxRetries: 2, headers: {{ \"x-trace\": id }} }});"),
        format!("const opts = {{ timeout: 1000 }};\nclient.messages.create({params}, opts);"),
        format!("client.messages.create({params}, undefined);"),
        format!("client.messages.create({params});"),
    ] {
        let found = usages(&format!(
            "{anthropic}\n{gateway}const client = new Anthropic();\n{request}"
        ));
        let c = call(&found, "client.messages.create");
        assert_eq!(c.routing, Routing::AnthropicDirect, "{request}");
        assert!(gaps_at(&found, c).is_empty(), "{request}: {:?}", found.gaps);
    }
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI();
client.beta.threads.runs.create(threadId, {{ assistant_id: \"asst_abc\", model: \"gpt-4\" }});"
        ),
        "beta.threads.runs.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
    // Past a request that takes no model, an argument the file does not
    // show may be a path parameter or a body: only options written in the
    // file are read.
    for (request, member) in [
        (
            "client.beta.threads.runs.retrieve(threadId, run.id);",
            "beta.threads.runs.retrieve",
        ),
        (
            "client.beta.threads.messages.create(threadId, message);",
            "beta.threads.messages.create",
        ),
        (
            "client.beta.threads.runs.steps.retrieve(threadId, runId, stepId);",
            "beta.threads.runs.steps.retrieve",
        ),
    ] {
        let (routing, gaps) = route(
            &format!("{OPENAI}const client = new OpenAI();\n{request}"),
            member,
        );
        assert_eq!(routing, Routing::OpenaiDirect, "{request}");
        assert!(
            !gaps.iter().any(|g| g.starts_with("ambiguous_client/")),
            "{request}: {gaps:?}"
        );
    }
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI();
client.files.retrieve(fileId, {{ path: \"https://gateway.example.com/v1/files/x\" }});"
        ),
        "files.retrieve",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"]);
    // A client already routed elsewhere keeps its route for options it may
    // be given; options written in the file still count.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI({{ baseURL: \"http://localhost:11434/v1\" }});
client.chat.completions.create({{ model: \"gpt-4o\", messages: [] }}, requestOptions);"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::CustomBaseUrl);
    assert_eq!(gaps, vec!["unresolved_model/splat"]);
}

#[test]
fn request_options_are_read_through_every_value_the_file_gives_them() {
    // Options held in a class property, a static member or a reassigned
    // `let`, or picked by a conditional or a conditional spread of
    // literals: each value is read, and only one that may hold a
    // `middleware`, `path`, `defaultBaseURL` or `fetch` (or that cannot be
    // read) costs the route.
    let bot = |fields: &str, body: &str, options: &str| {
        format!(
            "import Anthropic from \"@anthropic-ai/sdk\";
const viaGateway = async (req: any, next: any) => next(req);
class SupportBot {{
  private readonly client = new Anthropic();
  private readonly model = \"claude-3-5-sonnet-20241022\";
{fields}
  async reply(text: string, signal?: AbortSignal, stream?: boolean, slow?: boolean) {{
{body}
    return this.client.messages.create(
      {{ model: this.model, max_tokens: 1024, messages: [{{ role: \"user\", content: text }}] }},
      {options},
    );
  }}
}}
export const supportBot = new SupportBot();
"
        )
    };
    let reply = |source: &str| {
        let found = usages(source);
        let c = call(&found, "this.client.messages.create");
        (
            c.routing,
            value(c, "model").map(str::to_string),
            gaps_at(&found, c),
        )
    };
    for (fields, body, options) in [
        (
            "  private readonly requestOptions = { timeout: 60_000, maxRetries: 3 };",
            "",
            "this.requestOptions",
        ),
        (
            "  private requestOptions: { timeout: number };\n  constructor() { this.requestOptions = { timeout: 60_000 }; }",
            "",
            "this.requestOptions",
        ),
        (
            "  static readonly OPTIONS = { timeout: 30_000, maxRetries: 2 };",
            "",
            "SupportBot.OPTIONS",
        ),
        ("", "", "signal ? { signal } : undefined"),
        ("", "", "signal ? { signal } : {}"),
        ("", "", "signal ? { signal } : null"),
        ("", "", "stream ? { timeout: 120_000 } : { timeout: 30_000 }"),
        ("", "", "{ maxRetries: 0, ...(signal ? { signal } : {}) }"),
        ("", "", "{ maxRetries: 0, ...(signal && { signal }) }"),
        (
            "",
            "    let opts = { timeout: 30_000 };\n    if (slow) opts = { timeout: 120_000 };",
            "opts",
        ),
        (
            "",
            "    let opts: any = { timeout: 30_000 };\n    if (signal) opts = { ...opts, signal };",
            "opts",
        ),
        (
            "",
            "    const base = { timeout: 1000 };\n    const opts = signal ? { ...base, signal } : { ...base };",
            "opts",
        ),
    ] {
        let source = bot(fields, body, options);
        let (routing, model, gaps) = reply(&source);
        assert_eq!(routing, Routing::AnthropicDirect, "{source}");
        assert_eq!(model.as_deref(), Some("claude-3-5-sonnet-20241022"));
        assert!(gaps.is_empty(), "{source}: {gaps:?}");
    }
    // A value that may hold one, or one the file cannot read, which may
    // also hold a `body` that replaces the one written.
    for (fields, body, options, read) in [
        (
            "  private readonly requestOptions = { middleware: [viaGateway] };",
            "",
            "this.requestOptions",
            true,
        ),
        (
            "  static readonly OPTIONS = { path: \"https://gateway.example.com/v1/messages\" };",
            "",
            "SupportBot.OPTIONS",
            true,
        ),
        ("", "", "signal ? { middleware: [viaGateway] } : undefined", true),
        (
            "",
            "",
            "{ maxRetries: 0, ...(signal ? { defaultBaseURL: \"https://gateway.example.com\" } : {}) }",
            true,
        ),
        (
            "",
            "    let opts: any = { timeout: 30_000 };\n    if (slow) opts = { path: \"https://gateway.example.com/v1/messages\" };",
            "opts",
            true,
        ),
        (
            "",
            "    let opts: any = { timeout: 1 };\n    if (slow) opts = { ...opts, middleware: [viaGateway] };",
            "opts",
            true,
        ),
        // Written by the file through the property, or given to a call
        // that may write it.
        (
            "  private readonly requestOptions: any = { timeout: 60_000 };\n  useGateway() { this.requestOptions.middleware = [viaGateway]; }",
            "",
            "this.requestOptions",
            false,
        ),
        (
            "  private readonly requestOptions: any = { timeout: 60_000 };\n  useGateway() { const o = this.requestOptions; o.middleware = [viaGateway]; }",
            "",
            "this.requestOptions",
            false,
        ),
        (
            "",
            "    const opts = { timeout: 1 };\n    configure(opts);",
            "opts",
            false,
        ),
        ("", "", "signal ? { signal } : extraOptions", false),
        ("", "", "{ ...(signal ? { signal } : defaults()) }", false),
    ] {
        let source = bot(fields, body, options);
        let (routing, model, gaps) = reply(&source);
        assert_eq!(routing, Routing::Unknown, "{source}");
        if read {
            assert_eq!(model.as_deref(), Some("claude-3-5-sonnet-20241022"));
            assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"], "{source}");
        } else {
            assert_eq!(model, None, "{source}");
            assert_eq!(
                gaps,
                vec!["unresolved_model/splat", "ambiguous_client/endpoint_option"],
                "{source}"
            );
        }
    }
    // Names that hold each other: each holds what the other may.
    for order in [["a", "b"], ["b", "a"]] {
        let found = usages(&format!(
            "import Anthropic from \"@anthropic-ai/sdk\";
const viaGateway = async (req: any, next: any) => next(req);
const client = new Anthropic();
let a: any = {{ timeout: 1 }};
const b = {{ ...a }};
if (process.argv[2]) a = {{ ...b, middleware: [viaGateway] }};
client.messages.create({{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }}, {});
client.messages.create({{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }}, {});",
            order[0], order[1]
        ));
        assert_eq!(found.calls.len(), 2);
        for c in &found.calls {
            assert_eq!(c.routing, Routing::Unknown, "{order:?}");
            assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/endpoint_option"]);
        }
    }
    // In a method that may run with another `this`, the property may be
    // another object's.
    let (routing, gaps) = route(
        "import Anthropic from \"@anthropic-ai/sdk\";
const client = new Anthropic();
class Bot {
  private readonly requestOptions = { timeout: 60_000 };
  reply() {
    return client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }, this.requestOptions);
  }
}
setTimeout(new Bot().reply);",
        "messages.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(
        gaps,
        vec!["unresolved_model/splat", "ambiguous_client/endpoint_option"]
    );
    // As for the client it reads through `this`, a subclass in another
    // module is taken not to replace an exported class's options.
    let source = bot(
        "  private readonly requestOptions = { timeout: 60_000 };",
        "",
        "this.requestOptions",
    )
    .replace("class SupportBot", "export class SupportBot");
    let (routing, _, gaps) = reply(&source);
    assert_eq!(routing, Routing::AnthropicDirect);
    assert!(
        !gaps.iter().any(|g| g.starts_with("ambiguous_client/")),
        "{gaps:?}"
    );
    // The SDK's refusal fallback keeps the route where nothing else may
    // change the array: not in a class property.
    let fallback = "[betaRefusalFallbackMiddleware([{ model: \"claude-opus-4-8\" }])]";
    for (fields, body, options, routing) in [
        (
            String::new(),
            format!("    const opts = {{ middleware: {fallback} }};"),
            "opts",
            Routing::AnthropicDirect,
        ),
        (
            format!("  private readonly requestOptions = {{ middleware: {fallback} }};"),
            String::new(),
            "this.requestOptions",
            Routing::Unknown,
        ),
    ] {
        let source = bot(&fields, &body, options).replace(
            "import Anthropic from",
            "import Anthropic, { betaRefusalFallbackMiddleware } from",
        );
        assert_eq!(reply(&source).0, routing, "{source}");
    }
    // The same reading for OpenAI's request options, and for a client's.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}const client = new OpenAI();
export const ask = (signal?: AbortSignal) =>
  client.chat.completions.create({{ model: \"gpt-4o\", messages: [] }}, signal ? {{ signal }} : undefined);"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::OpenaiDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
    let (routing, gaps) = route(
        "import Anthropic from \"@anthropic-ai/sdk\";
const debug = process.argv.includes(\"--debug\");
const client = new Anthropic({ ...(debug ? { logLevel: \"debug\" } : {}), baseURL: \"https://api.anthropic.com\" });
client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] });",
        "messages.create",
    );
    assert_eq!(routing, Routing::AnthropicDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn reading_request_options_through_their_values_is_linear() {
    // Many requests given options that spread names that are only read,
    // each reassigned many times with conditionals of spreads of the one
    // before: a name read in full is kept, and one read takes a bounded
    // number of steps.
    let mut source = String::from(
        "import Anthropic from \"@anthropic-ai/sdk\";\nconst client = new Anthropic();\nconst o0 = { timeout: 1 };\n",
    );
    for i in 1..200 {
        let prev = i - 1;
        source.push_str(&format!(
            "let o{i}: any = {{ ...o{prev}, maxRetries: 1 }};\n"
        ));
        for _ in 0..30 {
            source.push_str(&format!(
                "o{i} = process.argv[1] ? {{ ...o{prev} }} : {{ ...o{prev}, timeout: 2 }};\n"
            ));
        }
    }
    for _ in 0..3_000 {
        source.push_str("client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 1, messages: [] }, { ...o199 });\n");
    }
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 3_000);
}

#[test]
fn options_that_may_hold_a_fetch_or_middleware_unconfirm_a_provider_base_url() {
    // The `baseURL` written after a spread or a computed key decides the
    // host, but what they hold may still be a `fetch` or a `middleware`.
    let gateway = "const viaGateway = async (req: any, next: any) =>
  next({ ...req, url: String(req.url).replace(\"api.anthropic.com\", \"gateway.example.com\") });
";
    let request = "client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] });";
    for (setup, client) in [
        (
            "const useGateway = process.argv.includes(\"--gateway\");",
            "new Anthropic({\n  ...(useGateway ? { middleware: [viaGateway] } : {}),\n  baseURL: \"https://api.anthropic.com\",\n})",
        ),
        (
            "const shared = { middleware: [viaGateway] };",
            "new Anthropic({ ...shared, baseURL: \"https://api.anthropic.com\" })",
        ),
        (
            "import { clientOptions } from \"./config\";",
            "new Anthropic({ ...clientOptions, baseURL: \"https://api.anthropic.com\" })",
        ),
        (
            "declare const key: string;",
            "new Anthropic({ [key]: [viaGateway], baseURL: \"https://api.anthropic.com\" })",
        ),
        (
            "declare const key: string;",
            "new Anthropic().withOptions({ [key]: [viaGateway], baseURL: \"https://api.anthropic.com\" })",
        ),
        (
            "import { clientOptions } from \"./config\";",
            "new Anthropic().withOptions({ ...clientOptions, baseURL: \"https://api.anthropic.com\" })",
        ),
    ] {
        let source = format!(
            "import Anthropic from \"@anthropic-ai/sdk\";\n{gateway}{setup}\nconst client = {client};\n{request}"
        );
        let (routing, gaps) = route(&source, "messages.create");
        assert_eq!(routing, Routing::Unknown, "{source}");
        assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"], "{source}");
    }
    // The `fetch` arm: OpenAI.
    let (routing, gaps) = route(
        &format!(
            "{OPENAI}import {{ clientOptions }} from \"./config\";
const client = new OpenAI({{ ...clientOptions, baseURL: \"https://api.openai.com/v1\" }});
client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }});"
        ),
        "chat.completions.create",
    );
    assert_eq!(routing, Routing::Unknown);
    assert_eq!(gaps, vec!["ambiguous_client/endpoint_option"]);
    // A spread the file shows that holds neither keeps the route.
    let (routing, gaps) = route(
        &format!(
            "import Anthropic from \"@anthropic-ai/sdk\";
const shared = {{ maxRetries: 2, timeout: 1000 }};
const client = new Anthropic({{ ...shared, baseURL: \"https://api.anthropic.com\" }});
{request}"
        ),
        "messages.create",
    );
    assert_eq!(routing, Routing::AnthropicDirect);
    assert!(gaps.is_empty(), "{gaps:?}");
    // Next to another host's `baseURL`, the request leaves the provider
    // either way: the route stays.
    for client in [
        "new OpenAI({ ...clientOptions, baseURL: \"http://localhost:11434/v1\" })",
        "new OpenAI().withOptions({ ...clientOptions, baseURL: \"http://localhost:11434/v1\" })",
    ] {
        let source = format!(
            "{OPENAI}import {{ clientOptions }} from \"./config\";
const client = {client};
client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }});"
        );
        let (routing, gaps) = route(&source, "chat.completions.create");
        assert_eq!(routing, Routing::CustomBaseUrl, "{client}");
        assert!(gaps.is_empty(), "{client}: {gaps:?}");
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
        // Test doubles replace it too.
        "jest.spyOn(globalThis, \"fetch\").mockImplementation(proxy);",
        "vi.spyOn(global, \"fetch\");",
        "sinon.stub(window, \"fetch\").callsFake(proxy);",
        "vi.stubGlobal(\"fetch\", proxy);",
        "vi.stubGlobal(name, proxy);",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{write}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{write}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/global_fetch"],
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
        assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/global_fetch"]);
    }
    // openai v3 sends through axios: its route stays, while a v4 client in
    // the same file is re-routed.
    let found = usages(
        "import OpenAI, { Configuration, OpenAIApi } from \"openai\";
global.fetch = require(\"node-fetch\");
const legacy = new OpenAIApi(new Configuration({ apiKey: key }));
const client = new OpenAI();
legacy.createChatCompletion({ model: \"gpt-4o\", messages: [] });
client.chat.completions.create({ model: \"gpt-4o\", messages: [] });",
    );
    let v3 = call(&found, "legacy.createChatCompletion");
    assert_eq!(v3.routing, Routing::OpenaiDirect);
    assert!(gaps_at(&found, v3).is_empty(), "{:?}", found.gaps);
    assert_eq!(routing_of(&found, "legacy"), Routing::OpenaiDirect);
    let v4 = call(&found, "client.chat.completions.create");
    assert_eq!(v4.routing, Routing::Unknown);
    assert_eq!(gaps_at(&found, v4), vec!["ambiguous_client/global_fetch"]);
    assert_eq!(routing_of(&found, "client"), Routing::Unknown);
    // Read, or written on an object literal of the file: nothing replaced.
    for other in [
        "const original = globalThis.fetch;",
        "vi.stubGlobal(\"location\", { href: \"/\" });",
        "jest.spyOn(console, \"log\");",
        "const api = { fetch: proxy };\napi.fetch = other;",
        "let fetch = proxy;\nfetch = other;",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{other}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{other}");
        assert!(gaps_at(&found, c).is_empty(), "{other}");
    }
    // A CommonJS module exporting a helper named `fetch` (or a `baseURL`
    // setting) writes its own export object, never the global or a client.
    for export in [
        "exports.fetch = async function (url) { return (await globalThis.fetch(url)).json(); };",
        "module.exports.fetch = (url) => globalThis.fetch(url);",
        "exports.baseURL = \"https://gateway.example.com/v1\";",
        "module.exports = { summarize };\nmodule.exports.fetch = (url) => globalThis.fetch(url);",
    ] {
        let source = format!(
            "const OpenAI = require(\"openai\");\nconst client = new OpenAI();\n{export}\n{request}"
        );
        let found = usages_js(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{export}");
        assert!(gaps_at(&found, c).is_empty(), "{export}: {:?}", found.gaps);
    }
    // Once the module exports a client, a property written on the export
    // object may be that client's.
    let found = usages_js(&format!(
        "const OpenAI = require(\"openai\");\nconst client = new OpenAI();\nmodule.exports = client;\nmodule.exports.fetch = proxy;\n{request}"
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.routing, Routing::Unknown);
    assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/global_fetch"]);
    // So does a module that assigns `exports` itself: `exports.x` then
    // writes what it was given.
    for (write, gap) in [
        (
            "exports = client;\nexports.baseURL = \"https://gateway.example.com/v1\";",
            "ambiguous_client/dynamic_feature",
        ),
        (
            "exports = client;\nexports.fetch = proxy;",
            "ambiguous_client/global_fetch",
        ),
        (
            "module = { exports: client };\nmodule.exports.fetch = proxy;",
            "ambiguous_client/global_fetch",
        ),
    ] {
        let found = usages_js(&format!(
            "const OpenAI = require(\"openai\");\nconst client = new OpenAI();\n{write}\n{request}"
        ));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{write}");
        assert_eq!(gaps_at(&found, c), vec![gap], "{write}");
    }
}

#[test]
fn the_global_object_written_under_a_run_time_name_may_replace_fetch() {
    // A name held in a binding, a spread, or a key known only at run time:
    // like `vi.stubGlobal(name, ...)`, any of them may be `fetch`.
    let request = "client.chat.completions.create({ model: \"gpt-4o\", messages: [] });";
    for write in [
        "const mocks = { fetch: proxy };\nObject.assign(globalThis, mocks);",
        "Object.assign(globalThis, { ...polyfills });",
        "const name = \"fetch\";\n(globalThis as any)[name] = proxy;",
        "function setGlobal(name: string, value: unknown) {\n  (globalThis as any)[name] = value;\n}\nsetGlobal(\"fetch\", proxy);",
        // Another module may call an exported helper with `"fetch"`.
        "export function setGlobal(k: string, v: unknown) {\n  Reflect.set(globalThis, k, v);\n}",
        "export function setGlobal(k: string, v: unknown) {\n  Object.defineProperty(globalThis, k, { value: v });\n}",
        "function setGlobal(k: string, v: unknown) {\n  Reflect.set(globalThis, k, v);\n}\nsetGlobal(process.argv[2], proxy);",
        "for (const [k, v] of Object.entries({ fetch: proxy })) (globalThis as any)[k] = v;",
        "const g: any = globalThis;\ng[name] = proxy;",
        "(window as any)[name] = proxy;",
        // A name or an expression that may be the global object.
        "let g: any = globalThis;\ng = {};\ng[name] = proxy;",
        "let g: any = {};\nif (process.argv[2]) g = globalThis;\ng[name] = proxy;",
        "const root: any = typeof window !== \"undefined\" ? window : globalThis;\nroot[name] = proxy;",
        "(typeof window !== \"undefined\" ? window : (globalThis as any))[name] = proxy;",
        "((globalThis || window) as any)[name] = proxy;",
        "const k = \"fetch\";\njest.spyOn(globalThis, k).mockImplementation(proxy);",
        // A key the file shows may be `fetch`.
        // undici keeps the global dispatcher under the registry's symbol.
        "(globalThis as any)[Symbol.for(\"undici.globalDispatcher.1\")] = new MockAgent();",
        "const DISPATCHER = Symbol.for(\"undici.globalDispatcher.1\");\n(globalThis as any)[DISPATCHER] = agent;",
        "(globalThis as any)[Symbol.for(name)] = agent;",
        "let key = \"__cache\";\nkey = \"fetch\";\n(globalThis as any)[key] = proxy;",
        "const key = process.argv[2] ? \"__cache\" : \"fetch\";\n(globalThis as any)[key] = proxy;",
        "(globalThis as any)[`fe${\"tch\"}`] = proxy;",
        "function Symbol(k: string) { return k; }\n(globalThis as any)[Symbol(\"fetch\")] = proxy;",
        "declare const KEY: string;\nReflect.set(globalThis, KEY, proxy);",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{write}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{write}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/global_fetch"],
            "{write}"
        );
    }
    // Run-time names written on anything else are ordinary code.
    for other in [
        "const headers: Record<string, string> = {};\nheaders[name] = value;",
        "export function merge(options: any, defaults: any) { return Object.assign(options, defaults); }",
        "export function set(target: any, k: string, v: unknown) { target[k] = v; }",
        // A symbol, or a string the file shows is not `fetch`: a global
        // cache or singleton.
        "const CACHE_KEY = Symbol.for(\"app.summaryCache\");\nconst g = globalThis as unknown as Record<symbol, Map<string, string> | undefined>;\ng[CACHE_KEY] ??= new Map<string, string>();",
        "const KEY = \"__summaryCache\";\n(globalThis as any)[KEY] = (globalThis as any)[KEY] || new Map();",
        "const APP = \"summaries\";\n(globalThis as any)[`__${APP}_started`] = Date.now();",
        "(globalThis as any)[Symbol(\"started\")] = Date.now();",
        "(window as any)[Symbol.iterator] = function* () {};",
        "const key = process.argv[2] ? \"__a\" : \"__b\";\n(globalThis as any)[key] = 1;",
        "Reflect.set(globalThis, Symbol.for(\"app.cache\"), new Map());",
        "const K = \"__cache\";\nObject.defineProperty(globalThis, K, { value: new Map() });",
        "const NAME = \"ResizeObserver\";\nvi.stubGlobal(NAME, vi.fn());",
        "const KEY = \"setTimeout\";\njest.spyOn(globalThis, KEY);",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{other}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{other}");
        assert!(gaps_at(&found, c).is_empty(), "{other}");
    }
}

#[test]
fn a_test_double_of_fetch_is_seen_however_the_runner_is_reached() {
    // The runner is read by the call's last name, not by how its object is
    // bound: CommonJS, a sinon sandbox, node:test's context, named imports,
    // Vitest's in-source `import.meta.vitest`.
    let request = "client.chat.completions.create({ model: \"gpt-4o\", messages: [] });";
    for (setup, js) in [
        (
            "const sinon = require(\"sinon\");\nsinon.stub(global, \"fetch\").resolves(new Response(\"{}\"));",
            true,
        ),
        (
            "const { jest } = require(\"@jest/globals\");\njest.spyOn(global, \"fetch\");",
            true,
        ),
        (
            "const { mock } = require(\"node:test\");\nmock.method(globalThis, \"fetch\", async () => new Response(\"{}\"));",
            true,
        ),
        (
            "import sinon from \"sinon\";\nconst sandbox = sinon.createSandbox();\nbeforeEach(() => { sandbox.stub(globalThis, \"fetch\"); });",
            false,
        ),
        (
            "import sinon from \"sinon\";\nsinon.createSandbox().stub(globalThis, \"fetch\");",
            false,
        ),
        (
            "import test from \"node:test\";\ntest(\"x\", (t) => { t.mock.method(globalThis, \"fetch\", async () => new Response(\"{}\")); });",
            false,
        ),
        (
            "import test from \"node:test\";\ntest.mock.method(globalThis, \"fetch\");",
            false,
        ),
        (
            "import { stub } from \"sinon\";\nstub(globalThis, \"fetch\").resolves(new Response(\"{}\"));",
            false,
        ),
        (
            "import { stub } from \"jsr:@std/testing/mock\";\nconst fetchStub = stub(globalThis, \"fetch\", () => Promise.resolve(new Response(\"{}\")));",
            false,
        ),
        (
            "import { spyOn } from \"bun:test\";\nspyOn(globalThis, \"fetch\").mockImplementation(proxy);",
            false,
        ),
        (
            "if (import.meta.vitest) {\n  const { it, vi } = import.meta.vitest;\n  it(\"x\", () => { vi.stubGlobal(\"fetch\", vi.fn()); });\n}",
            false,
        ),
        (
            "const { vi } = import.meta.vitest;\nvi.spyOn(globalThis, \"fetch\");",
            false,
        ),
        (
            "import { vi as v } from \"vitest\";\nv.stubGlobal(\"fetch\", v.fn());",
            false,
        ),
        (
            "import * as vitest from \"vitest\";\nvitest.vi.stubGlobal(\"fetch\", vitest.vi.fn());",
            false,
        ),
        ("jest.replaceProperty(globalThis, \"fetch\", proxy);", false),
        // Jasmine's global `spyOn`.
        ("spyOn(window, \"fetch\").and.returnValue(response);", false),
    ] {
        let (imports, found) = if js {
            let imports = "const OpenAI = require(\"openai\");\n";
            (
                imports,
                usages_js(&format!(
                    "{imports}{setup}\nconst client = new OpenAI();\n{request}"
                )),
            )
        } else {
            (
                OPENAI,
                usages(&format!(
                    "{OPENAI}{setup}\nconst client = new OpenAI();\n{request}"
                )),
            )
        };
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{imports}{setup}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/global_fetch"],
            "{setup}"
        );
    }
    // The same forms on an SDK object replace its member.
    for setup in [
        "const sinon = require(\"sinon\");\nsinon.stub(client.chat.completions, \"create\").resolves({});",
        "const sandbox = require(\"sinon\").createSandbox();\nsandbox.stub(client.chat.completions, \"create\");",
        "test(\"x\", (t) => { t.mock.method(client.chat.completions, \"create\", async () => ({})); });",
    ] {
        let found = usages_js(&format!(
            "const OpenAI = require(\"openai\");\nconst client = new OpenAI();\n{setup}\n{request}"
        ));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{setup}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/dynamic_feature"],
            "{setup}"
        );
    }
    // Calls that only share a name stay ordinary code.
    for other in [
        "const label = name.replace(\"get\", \"fetch\");",
        "export const rename = (name: string, prefix: string) => name.replace(prefix, \"fetch\");",
        "http.method(server, \"fetch\");",
        "const api = { fetch: proxy };\nsinon.stub(api, \"fetch\");",
        "sinon.stub(globalThis, \"setTimeout\");",
        "jest.spyOn(console, \"log\");",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{other}\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{other}");
        assert!(gaps_at(&found, c).is_empty(), "{other}");
    }
}

#[test]
fn a_library_that_intercepts_fetch_set_up_in_the_file_is_a_global_fetch() {
    let request = "client.chat.completions.create({ model: \"gpt-4o\", messages: [] });";
    for setup in [
        "import nock from \"nock\";\nnock(\"https://api.openai.com\").post(\"/v1/chat/completions\").reply(200, {});",
        "import { setupServer } from \"msw/node\";\nconst server = setupServer();\nbeforeAll(() => server.listen());",
        "import fetchMock from \"jest-fetch-mock\";\nfetchMock.enableMocks();",
        "require(\"jest-fetch-mock\").enableMocks();",
        "import fetchMock from \"fetch-mock\";\nfetchMock.mockGlobal().post(\"https://api.openai.com/v1/chat/completions\", {});",
        "const { default: nock } = await import(\"nock\");",
        "import { MockAgent, setGlobalDispatcher } from \"undici\";\nconst agent = new MockAgent();\nsetGlobalDispatcher(agent);",
    ] {
        let source = format!("{OPENAI}{setup}\nconst client = new OpenAI();\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{setup}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/global_fetch"],
            "{setup}"
        );
    }
    // undici's own fetch, a proxy agent, and types only change nothing.
    for other in [
        "import { fetch as undiciFetch } from \"undici\";",
        "import { ProxyAgent, setGlobalDispatcher } from \"undici\";\nsetGlobalDispatcher(new ProxyAgent(proxyUrl));",
        "import { Agent, setGlobalDispatcher } from \"undici\";\nconst agent = new Agent({ keepAliveTimeout: 10_000 });\nsetGlobalDispatcher(agent);",
        "import type { HttpHandler } from \"msw\";",
    ] {
        let source = format!("{OPENAI}{other}\nconst client = new OpenAI();\n{request}");
        let found = usages(&source);
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{other}");
        assert!(gaps_at(&found, c).is_empty(), "{other}");
    }
}

#[test]
fn openai_v3_is_intercepted_through_node_http_not_fetch() {
    // nock and msw intercept Node's `http` module, which axios sends v3's
    // requests through; the `fetch` mocks and undici's dispatchers do not.
    let v3 = "const { Configuration, OpenAIApi } = require(\"openai\");
const openai = new OpenAIApi(new Configuration({ apiKey: \"test\" }));
";
    let request = "openai.createChatCompletion({ model: \"gpt-3.5-turbo-0301\", messages: [] });";
    for setup in [
        "const nock = require(\"nock\");\nnock(\"https://api.openai.com\").post(\"/v1/chat/completions\").reply(200, { choices: [] });",
        "const { setupServer } = require(\"msw/node\");\nconst server = setupServer();\nserver.listen();",
        "const { http } = require(\"msw\");",
        "const { ClientRequestInterceptor } = require(\"@mswjs/interceptors/ClientRequest\");",
    ] {
        let found = usages_js(&format!("{setup}\n{v3}{request}"));
        let c = call(&found, "openai.createChatCompletion");
        assert_eq!(c.routing, Routing::Unknown, "{setup}");
        assert_eq!(
            gaps_at(&found, c),
            vec!["ambiguous_client/global_fetch"],
            "{setup}"
        );
        assert_eq!(routing_of(&found, "openai"), Routing::Unknown, "{setup}");
        let gap = found.gaps.iter().find(|g| g.reason == "global_fetch").unwrap();
        assert!(gap.detail.contains("Node's `http` module"), "{}", gap.detail);
        assert!(gap.detail.contains("at line 1"), "{}", gap.detail);
    }
    // ESM, next to a v4 client that gets the `fetch` gap.
    let found = usages(&format!(
        "import nock from \"nock\";\nimport OpenAI, {{ Configuration, OpenAIApi }} from \"openai\";
const openai = new OpenAIApi(new Configuration({{ apiKey: \"test\" }}));
const client = new OpenAI();
{request}
client.chat.completions.create({{ model: \"gpt-4o\", messages: [] }});"
    ));
    for chain in [
        "openai.createChatCompletion",
        "client.chat.completions.create",
    ] {
        let c = call(&found, chain);
        assert_eq!(c.routing, Routing::Unknown, "{chain}");
        assert_eq!(gaps_at(&found, c), vec!["ambiguous_client/global_fetch"]);
    }
    for setup in [
        "const fetchMock = require(\"jest-fetch-mock\");\nfetchMock.enableMocks();",
        "const fetchMock = require(\"fetch-mock\");\nfetchMock.mockGlobal();",
        "const { MockAgent, setGlobalDispatcher } = require(\"undici\");\nsetGlobalDispatcher(new MockAgent());",
        "global.fetch = require(\"node-fetch\");",
    ] {
        let found = usages_js(&format!("{setup}\n{v3}{request}"));
        let c = call(&found, "openai.createChatCompletion");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{setup}");
        assert!(gaps_at(&found, c).is_empty(), "{setup}: {:?}", found.gaps);
    }
}

#[test]
fn a_global_fetch_gap_names_what_the_file_does() {
    // A client with no options is not told it has one, nor sent to Vertex
    // AI.
    let found = usages(&format!(
        "{OPENAI}const client = new OpenAI();
globalThis.fetch = myFetch;
client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }});"
    ));
    let gap = &found.gaps[0];
    assert_eq!(gap.reason, "global_fetch");
    assert!(gap.detail.contains("line 3"), "{}", gap.detail);
    let text = crate::retirements::gap_text(gap.kind, &gap.reason).unwrap();
    for detail in [gap.detail.as_str(), text] {
        assert!(detail.contains("global `fetch`"), "{detail}");
        assert!(!detail.contains("option"), "{detail}");
        assert!(!detail.contains("Vertex"), "{detail}");
    }
    // Anthropic's `middleware` is an option, which sends no request to
    // Vertex AI.
    let found = usages(
        "import Anthropic from \"@anthropic-ai/sdk\";
const client = new Anthropic({ middleware: [logRequests] });
client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] });",
    );
    let gap = &found.gaps[0];
    assert_eq!(gap.reason, "endpoint_option");
    let text = crate::retirements::gap_text(gap.kind, &gap.reason).unwrap();
    for detail in [gap.detail.as_str(), text] {
        assert!(detail.contains("another endpoint"), "{detail}");
        assert!(!detail.contains("Vertex"), "{detail}");
    }
    // One request's options are that request's, not the client's.
    for (request, option) in [
        (
            "{ middleware: [logRequests] }",
            "request options with `middleware`",
        ),
        (
            "{ defaultBaseURL: \"https://gateway.example.com\" }",
            "request options with a `defaultBaseURL`",
        ),
        (
            "requestOptions",
            "request options that may hold `middleware`, a `path` or a `defaultBaseURL`",
        ),
    ] {
        let found = usages(&format!(
            "import Anthropic from \"@anthropic-ai/sdk\";
const client = new Anthropic();
client.messages.create({{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }}, {request});"
        ));
        let gap = found
            .gaps
            .iter()
            .find(|g| g.reason == "endpoint_option")
            .unwrap_or_else(|| panic!("{:?}", found.gaps));
        assert_eq!(
            gap.detail,
            format!(
                "`client.messages.create` is given {option}, which may send it to another endpoint"
            )
        );
        assert_eq!(routing_of(&found, "client"), Routing::AnthropicDirect);
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

/// The routing and gaps of the one `client.chat.completions.create` call
/// in `source` (TypeScript).
fn create_route(source: &str) -> (Routing, Vec<String>) {
    let found = usages(source);
    let c = call(&found, "client.chat.completions.create");
    (c.routing, gaps_at(&found, c))
}

#[test]
fn class_held_request_options_are_read_only_when_no_read_of_the_name_hands_them_out() {
    // The object a class property holds may be reached through any read of
    // a property of that name, on any object: a cast, parentheses, an
    // alias, a destructuring, an instance, the class name or a helper's
    // parameter. Written there under a run-time key, a `path` or a `fetch`,
    // it may send the request elsewhere.
    let summarizer = |method: &str, after: &str| {
        format!(
            "{OPENAI}const client = new OpenAI();
export class Summarizer {{
  private requestOptions: any = {{ timeout: 30_000, maxRetries: 2 }};
  {method}
  summarize(text: string) {{
    return client.chat.completions.create(
      {{ model: \"gpt-4-0613\", messages: [{{ role: \"user\", content: text }}] }},
      this.requestOptions,
    );
  }}
}}
{after}
"
        )
    };
    // Options the file cannot read may also hold a `body` that replaces
    // the one written.
    let gapped = vec![
        "unresolved_model/splat".to_string(),
        "ambiguous_client/endpoint_option".to_string(),
    ];
    for (method, after) in [
        (
            "setOption(key: string, value: unknown) { (this.requestOptions as any)[key] = value; }",
            "",
        ),
        (
            "setOption(key: string, value: unknown) { (this.requestOptions)[key] = value; }",
            "",
        ),
        (
            "setOption(key: string, value: unknown) { this.requestOptions[key] = value; }",
            "",
        ),
        (
            "setOption(key: string, value: unknown) { const o = this.requestOptions; o[key] = value; }",
            "",
        ),
        (
            "setOption(key: string, value: unknown) { const { requestOptions } = this; requestOptions[key] = value; }",
            "",
        ),
        (
            "useGateway() { this.requestOptions.path = \"https://gateway.example.com/v1/chat/completions\"; }",
            "",
        ),
        (
            "useGateway() { this.requestOptions.__proto__ = { path: \"https://gateway.example.com/v1/chat/completions\" }; }",
            "",
        ),
        (
            "useGateway() { this.requestOptions.__proto__.path = \"https://gateway.example.com/v1/chat/completions\"; }",
            "",
        ),
        (
            "useProxy(proxy: any) { put(this.requestOptions, \"fetch\", proxy); }",
            "function put(o: any, k: string, v: unknown) { o[k] = v; }",
        ),
        (
            "",
            "export const summarizer = new Summarizer();\nexport function setOption(k: string, v: unknown) { (summarizer as any).requestOptions[k] = v; }",
        ),
        ("options() { return this.requestOptions; }", ""),
        ("merge(extra: any) { _.merge(this.requestOptions, extra); }", ""),
        ("set(k: string, v: unknown) { this.requestOptions.set(k, v); }", ""),
    ] {
        let source = summarizer(method, after);
        assert_eq!(
            create_route(&source),
            (Routing::Unknown, gapped.clone()),
            "{source}"
        );
    }
    // The same through a static property read by class name.
    let statics = format!(
        "{OPENAI}const client = new OpenAI();
class Svc {{
  static OPTS: any = {{ timeout: 1 }};
  static ask() {{ return client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, Svc.OPTS); }}
}}
export function configure(k: string, v: unknown) {{ Svc.OPTS[k] = v; }}
"
    );
    assert_eq!(create_route(&statics), (Routing::Unknown, gapped.clone()));
    // Reads that cannot change what it holds keep the route, whatever the
    // file writes on other objects.
    for (method, after) in [
        ("", ""),
        ("setTimeoutMs(ms: number) { this.requestOptions.timeout = ms; }", ""),
        ("describe() { return `${this.requestOptions.timeout} ms`; }", ""),
        ("log() { console.log(this.requestOptions); }", ""),
        ("has() { return !this.requestOptions || \"path\" in this.requestOptions; }", ""),
        ("copy() { return { ...this.requestOptions }; }", ""),
        (
            "",
            "export function prefixed(route: any) { route.path = \"/api\" + route.path; return route; }",
        ),
        (
            "",
            "export function defaults(target: any, extra: any) { (target as any)[\"path\"] ??= extra; }",
        ),
        (
            "",
            "class Other { path = \"\"; at(p: string) { this.path = p; } }\nnew Other().at(\"/x\");",
        ),
    ] {
        let source = summarizer(method, after);
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{source}"
        );
    }
    // Passed through a conditional to the request.
    let source = summarizer("", "").replace(
        "      this.requestOptions,\n",
        "      text ? this.requestOptions : undefined,\n",
    );
    assert_eq!(
        create_route(&source),
        (Routing::OpenaiDirect, vec![]),
        "{source}"
    );
}

#[test]
fn a_method_bound_to_its_own_object_still_runs_with_it() {
    // `ctrl.chat.bind(ctrl)` and `this.send = this.send.bind(this)` run the
    // method with the object it is read off, as a call of it does: its
    // reads of `this` keep their value. Bound to anything else, it may
    // run with another `this`.
    let controller = |bind: &str| {
        format!(
            "{OPENAI}import express from \"express\";
const client = new OpenAI();
class ChatController {{
  private readonly requestOptions = {{ timeout: 30_000, maxRetries: 2 }};
  private readonly model = \"gpt-4-0613\";
  async chat(req: any, res: any) {{
    const r = await client.chat.completions.create({{ model: this.model, messages: req.body.messages }}, this.requestOptions);
    res.json(r);
  }}
}}
const controller = new ChatController();
const router = express.Router();
{bind}
"
        )
    };
    for bind in [
        "router.post(\"/chat\", controller.chat.bind(controller));",
        "router.post(\"/chat\", (req, res) => controller.chat.call(controller, req, res));",
    ] {
        let found = usages(&controller(bind));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::OpenaiDirect, "{bind}");
        assert_eq!(value(c, "model"), Some("gpt-4-0613"), "{bind}");
        assert!(gaps_at(&found, c).is_empty(), "{bind}: {:?}", found.gaps);
    }
    for bind in [
        "router.post(\"/chat\", controller.chat.bind({ requestOptions: { path: \"https://gateway.example.com/v1/chat/completions\" } }));",
        "router.post(\"/chat\", controller.chat);",
        "const other = new ChatController();\nrouter.post(\"/chat\", controller.chat.bind(other));",
    ] {
        let found = usages(&controller(bind));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{bind}");
        assert!(
            gaps_at(&found, c).contains(&"ambiguous_client/endpoint_option".to_string()),
            "{bind}: {:?}",
            found.gaps
        );
    }
    // A class component binding its handler in the constructor.
    let component = format!(
        "{OPENAI}const client = new OpenAI({{ dangerouslyAllowBrowser: true, apiKey: \"x\" }});
export class Chat {{
  private requestOptions = {{ timeout: 20_000 }};
  constructor() {{
    this.send = this.send.bind(this);
  }}
  async send() {{
    return client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, this.requestOptions);
  }}
}}
"
    );
    assert_eq!(create_route(&component), (Routing::OpenaiDirect, vec![]));
}

#[test]
fn request_options_written_under_other_keys_stay_read() {
    // `options.signal = req.signal` cannot add a `path`, `defaultBaseURL`,
    // `fetch` or `middleware`; a write of one of those, or under a key
    // known only at run time, can.
    let handler = |writes: &str| {
        format!(
            "{OPENAI}const client = new OpenAI();
export async function POST(req: Request) {{
  const options: OpenAI.RequestOptions = {{ timeout: 30_000 }};
{writes}
  return client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, options);
}}
"
        )
    };
    for writes in [
        "  if (req.signal) options.signal = req.signal;",
        "  options.headers = { \"x-request-id\": \"1\" };\n  options.maxRetries = 0;",
        "  (options.headers as any)[\"x-trace\"] = \"1\";",
        "  delete options.timeout;",
    ] {
        let source = handler(writes);
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{writes}"
        );
    }
    for writes in [
        "  options.path = \"https://gateway.example.com/v1/chat/completions\";",
        "  options.__proto__ = { path: \"https://gateway.example.com/v1/chat/completions\" };",
        "  options[\"defaultBaseURL\"] = \"https://gateway.example.com\";",
        "  for (const [k, v] of Object.entries(overrides)) options[k] = v;",
        "  (options as any)[key] = value;",
        "  Object.assign(options, { signal: req.signal });",
    ] {
        let source = handler(writes);
        assert_eq!(
            create_route(&source),
            (
                Routing::Unknown,
                vec![
                    "unresolved_model/splat".to_string(),
                    "ambiguous_client/endpoint_option".to_string()
                ]
            ),
            "{writes}"
        );
    }
}

#[test]
fn frozen_request_options_are_read() {
    let ask = |options: &str| {
        format!(
            "const OpenAI = require(\"openai\");
const client = new OpenAI();
const REQUEST_OPTIONS = {options};
exports.ask = (q) => client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [{{ role: \"user\", content: q }}] }}, REQUEST_OPTIONS);
"
        )
    };
    let found = usages_js(&ask("Object.freeze({ timeout: 30000, maxRetries: 2 })"));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(c.routing, Routing::OpenaiDirect);
    assert!(gaps_at(&found, c).is_empty(), "{:?}", found.gaps);
    for options in [
        "Object.freeze({ path: \"https://gateway.example.com/v1/chat/completions\" })",
        "Object.freeze(...[{ timeout: 1 }])",
        "Object.freeze(loadOptions())",
        "freeze({ timeout: 1 })",
    ] {
        let found = usages_js(&ask(options));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{options}");
    }
}

#[test]
fn request_options_reached_through_a_conditional_or_a_same_file_function_are_read() {
    // A name used as one side of a conditional passed to the request, a
    // same-file helper's returned literal, and a non-exported wrapper's
    // parameter (every call of it is in the file) are read.
    let file = |body: &str| format!("{OPENAI}const client = new OpenAI();\n{body}\n");
    let create = "client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }";
    for body in [
        format!("const base = {{ timeout: 30_000 }};\nexport async function ask(signal?: AbortSignal) {{\n  return {create}, signal ? {{ ...base, signal }} : base);\n}}"),
        format!("const base = {{ timeout: 30_000 }};\nexport async function ask(signal?: AbortSignal) {{\n  return {create}, signal && base);\n}}"),
        format!("function requestOptions(req: Request) {{\n  return {{ signal: req.signal, timeout: 60_000 }};\n}}\nexport async function POST(req: Request) {{\n  return {create}, requestOptions(req));\n}}"),
        format!("const requestOptions = (req: Request) => ({{ signal: req.signal }});\nexport async function POST(req: Request) {{\n  return {create}, requestOptions(req));\n}}"),
        format!("function load(slow: boolean) {{ if (slow) {{ const t = 120_000; return {{ timeout: t }}; }} return; }}\nexport async function POST() {{\n  return {create}, load(!!process.env.SLOW));\n}}"),
        format!("function complete(prompt: string, options?: OpenAI.RequestOptions) {{\n  return {create}, options);\n}}\nexport const ask = (p: string) => complete(p, {{ timeout: 1 }});\nexport const fast = (p: string) => complete(p);"),
        format!("function complete(prompt: string, options = {{ timeout: 1 }}) {{\n  return {create}, options);\n}}\nexport const ask = (p: string) => complete(p, undefined);"),
    ] {
        let source = file(&body);
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{source}"
        );
    }
    // Options the file cannot read may also hold a `body` that replaces the
    // one written.
    for (body, read) in [
        // Another module may call it, or pass another value.
        (format!("export function complete(prompt: string, options?: OpenAI.RequestOptions) {{\n  return {create}, options);\n}}"), false),
        (format!("function complete(prompt: string, options?: OpenAI.RequestOptions) {{\n  return {create}, options);\n}}\nexport const ask = complete;"), false),
        (format!("function complete(prompt: string, options?: any) {{\n  return {create}, options);\n}}\nexport const ask = (p: string, o: any) => complete(p, o);"), false),
        (format!("function complete(prompt: string, options?: any) {{\n  return {create}, options);\n}}\nexport const ask = (p: string) => complete(p, {{ path: \"https://gateway.example.com/v1/chat/completions\" }});"), true),
        // The helper returns something it cannot read, or may be replaced.
        (format!("function requestOptions(o: any) {{ return {{ ...o }}; }}\nexport async function POST(o: any) {{\n  return {create}, requestOptions(o));\n}}"), false),
        (format!("let requestOptions = () => ({{ timeout: 1 }});\nexport function swap(f: any) {{ requestOptions = f; }}\nexport async function POST() {{\n  return {create}, requestOptions());\n}}"), false),
        (format!("function requestOptions() {{ return {{ fetch: proxy }}; }}\nexport async function POST() {{\n  return {create}, requestOptions());\n}}"), true),
        // The name used as a side is passed elsewhere too.
        (format!("const base: any = {{ timeout: 30_000 }};\nconfigure(base);\nexport async function ask(signal?: AbortSignal) {{\n  return {create}, signal ? {{ ...base, signal }} : base);\n}}"), false),
    ] {
        let source = file(&body);
        let (routing, gaps) = create_route(&source);
        assert_eq!(routing, Routing::Unknown, "{source}");
        let mut expected = vec!["ambiguous_client/endpoint_option"];
        if !read {
            expected.insert(0, "unresolved_model/splat");
        }
        assert_eq!(gaps, expected, "{source}");
    }
}

#[test]
fn module_exports_reached_through_module_itself_may_be_a_client() {
    // `module` aliased, or written reflectively, may set `module.exports`
    // to the client; a `baseURL` written through `module.exports` is then
    // the client's.
    let file = |setup: &str| {
        format!(
            "const OpenAI = require(\"openai\");
const client = new OpenAI();
{setup}
module.exports.baseURL = \"https://gw.example.com/v1\";
client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }});
"
        )
    };
    for setup in [
        "const m = module;\nm.exports = client;",
        "Object.assign(module, { exports: client });",
        "Reflect.set(module, \"exports\", client);",
        "module[process.argv[2]] = client;",
    ] {
        let found = usages_js(&file(setup));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(c.routing, Routing::Unknown, "{setup}");
    }
    // A member of `module` read or written by name leaves the export alone.
    for setup in [
        "if (module.hot) module.hot.accept();",
        "if (typeof module === \"object\") console.log(module.id);",
        "require.main === module && main();",
    ] {
        let found = usages_js(&file(setup));
        let c = call(&found, "client.chat.completions.create");
        assert_eq!(
            c.routing,
            Routing::OpenaiDirect,
            "{setup}: {:?}",
            found.gaps
        );
    }
}

#[test]
fn global_keys_from_in_file_callers_and_literal_loops_are_read() {
    // A key that is a parameter of a function every call of which is in the
    // file, or a loop variable over a literal's keys or an array literal,
    // is read through what those give it.
    let request = "client.chat.completions.create({ model: \"gpt-4o\", messages: [] });";
    let many = format!(
        "const many = {{ {} }};\nfor (const name of Object.keys(many)) (globalThis as any)[name] = 1;",
        (0..300).map(|i| format!("k{i}: {i}")).collect::<Vec<_>>().join(", ")
    );
    for write in [
        "function singleton<T>(name: string, make: () => T): T {\n  const g = globalThis as any;\n  return (g[name] ??= make());\n}\nexport const cache = singleton(\"cache\", () => new Map());\nexport const db = singleton(\"db\", () => ({}));",
        "function setGlobal(k = \"__app\", v: unknown = 1) { (globalThis as any)[k] = v; }\nsetGlobal();\nsetGlobal(undefined, 2);",
        "const polyfills: Record<string, unknown> = { TextEncoder, TextDecoder };\nfor (const name of Object.keys(polyfills)) {\n  if (!(globalThis as any)[name]) (globalThis as any)[name] = polyfills[name];\n}",
        "const browserGlobals = { ResizeObserver: class {}, IntersectionObserver: class {} };\nfor (const [key, value] of Object.entries(browserGlobals)) {\n  Object.defineProperty(globalThis, key, { value, writable: true });\n}",
        "for (const name of [\"ResizeObserver\", \"matchMedia\"]) (window as any)[name] = vi.fn();",
        "const stubs = { ResizeObserver: class {}, IntersectionObserver: class {} };\nfor (const [name, impl] of Object.entries(stubs)) vi.stubGlobal(name, impl);",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{write}\n{request}");
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{write}"
        );
    }
    for write in [
        "function singleton<T>(name: string, make: () => T): T {\n  const g = globalThis as any;\n  return (g[name] ??= make());\n}\nexport const f = singleton(\"fetch\", () => proxy);",
        "export function singleton(name: string, v: unknown) { (globalThis as any)[name] = v; }",
        "function setGlobal(k = \"fetch\", v: unknown = 1) { (globalThis as any)[k] = v; }\nsetGlobal();",
        "const polyfills = { TextEncoder, fetch: proxy };\nfor (const name of Object.keys(polyfills)) (globalThis as any)[name] = (polyfills as any)[name];",
        "const polyfills: any = { TextEncoder };\npolyfills.fetch = proxy;\nfor (const name of Object.keys(polyfills)) (globalThis as any)[name] = polyfills[name];",
        "const polyfills: any = { TextEncoder };\nextend(polyfills);\nfor (const name of Object.keys(polyfills)) (globalThis as any)[name] = polyfills[name];",
        "export const polyfills: any = { TextEncoder };\nexport function install() { for (const name of Object.keys(polyfills)) (globalThis as any)[name] = polyfills[name]; }",
        "for (const name of [\"ResizeObserver\", ...extra]) (window as any)[name] = vi.fn();",
        "for (const name in polyfills) (globalThis as any)[name] = polyfills[name];",
        // More keys than are read.
        many.as_str(),
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{write}\n{request}");
        assert_eq!(
            create_route(&source),
            (
                Routing::Unknown,
                vec!["ambiguous_client/global_fetch".to_string()]
            ),
            "{write}"
        );
    }
}

#[test]
fn many_class_properties_read_as_request_options_are_linear() {
    // One class with many fields and many properties its constructor
    // writes, and requests whose options spread many of them or read one
    // as the model: each lookup reads only the writes of its own name.
    let props = 15_000;
    let mut source = with_client("class Q {");
    for j in 0..props {
        source.push_str(&format!("  a{j} = {{}};\n  m{j} = \"gpt-4-0613\";\n"));
    }
    source.push_str("  constructor() {\n");
    for j in 0..props {
        source.push_str(&format!("    this.b{j} = {{}};\n"));
    }
    source.push_str("  }\n  run() {\n");
    for start in (0..props).step_by(250) {
        let spreads: Vec<String> = (start..start + 250)
            .map(|j| format!("...this.a{j}, ...this.b{j}"))
            .collect();
        source.push_str(&format!(
            "    client.chat.completions.create({{ model: this.m{start}, messages: [] }}, {{ {} }});\n",
            spreads.join(", ")
        ));
    }
    source.push_str("  }\n}\n");
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), props / 250);
}

#[test]
fn a_rebound_this_gap_names_its_cause_and_the_value_origin() {
    // Read in a method that may run with another `this`, the gaps say why
    // and where, and the value keeps its own origin.
    let found = usages(
        "import Anthropic from \"@anthropic-ai/sdk\";

class Bot {
  private client = new Anthropic();
  private model = \"claude-3-5-sonnet-20241022\";
  ask() {
    return this.client.messages.create({ model: this.model, max_tokens: 10, messages: [] });
  }
  run(m: \"ask\") { return this[m].call({ model: \"claude-sonnet-5-5\" }); }
}
new Bot().run(\"ask\");
",
    );
    let c = call(&found, "this.client.messages.create");
    let model = found
        .gaps
        .iter()
        .find(|g| g.reason == "attribute")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert_eq!(
        model.detail,
        "`model` for `this.client.messages.create` is `this.model`, read in a method that may run with another object as `this` (a computed member run with another `this` at line 9), so it may be that object's property; its value is captured with origin `class_constant`"
    );
    let client = found
        .gaps
        .iter()
        .find(|g| g.reason == "multiple_assignments")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert_eq!(
        client.detail,
        "the receiver of `this.client.messages.create` is read through `this` in a method that may run with another object as `this` (a computed member run with another `this` at line 9), so it may be another object's property"
    );
    assert!(gaps_at(&found, c).contains(&"ambiguous_client/multiple_assignments".to_string()));
    // A method name read without being called, named at its first read.
    let found = usages(
        "import Anthropic from \"@anthropic-ai/sdk\";
class Bot {
  private client = new Anthropic();
  ask() {
    return this.client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] });
  }
}
const bot = new Bot();
setTimeout(bot.ask, 10);
",
    );
    let client = found
        .gaps
        .iter()
        .find(|g| g.reason == "multiple_assignments")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert!(
        client
            .detail
            .contains("(a method read without being called at line 9)"),
        "{}",
        client.detail
    );
}

#[test]
fn a_class_property_written_more_often_than_read_holds_any_request_options() {
    // Past the writes read per property, a later one may hold a `path`.
    let mut writes = String::new();
    for i in 0..70 {
        writes.push_str(&format!(
            "    if (n === {i}) this.requestOptions = {{ timeout: {i} }};\n"
        ));
    }
    let source = format!(
        "{OPENAI}const client = new OpenAI();
export class Svc {{
  private requestOptions: any;
  constructor(n: number) {{
{writes}    if (n < 0) this.requestOptions = {{ path: \"https://gateway.example.com/v1/chat/completions\" }};
  }}
  ask() {{
    return client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, this.requestOptions);
  }}
}}
"
    );
    assert_eq!(
        create_route(&source),
        (
            Routing::Unknown,
            vec![
                "unresolved_model/splat".to_string(),
                "ambiguous_client/endpoint_option".to_string()
            ]
        )
    );
}

/// The model value, its origin and the gaps of the only Anthropic
/// `messages.create` call in `source`.
fn messages_row(source: &str) -> (Option<String>, Option<ValueOrigin>, Vec<String>) {
    let found = usages(source);
    let c = found
        .calls
        .iter()
        .find(|c| c.member == "messages.create")
        .unwrap_or_else(|| panic!("no create call: {:?} {:?}", found.calls, found.gaps));
    (
        value(c, "model").map(str::to_string),
        c.values.iter().find(|v| v.key == "model").map(|v| v.origin),
        gaps_at(&found, c),
    )
}

const ANTHROPIC: &str =
    "import Anthropic from \"@anthropic-ai/sdk\";\nconst client = new Anthropic();\n";

/// A class whose method sends its class property as the model.
const ANTHROPIC_BOT: &str = "class Bot {
  private model = \"claude-3-5-sonnet-20241022\";
  ask() { return client.messages.create({ model: this.model, max_tokens: 10, messages: [] }); }
";

#[test]
fn a_member_read_under_a_run_time_name_may_be_any_method() {
    // `this[m]` read rather than called right there is Python's
    // `getattr(self, m)`: the method it reads may be called from anywhere,
    // and with another `this`. So is the same read on a name holding an
    // instance, on the class (its static methods) and on its prototype,
    // and `Reflect.get` with a run-time name.
    for run in [
        "run(m: \"ask\") { const f = this[m]; return f.call({ model: \"claude-sonnet-5-5\" }); }\n}\nnew Bot().run(\"ask\");",
        "run(m: \"ask\") { return Reflect.apply(this[m], { model: \"claude-sonnet-5-5\" }, []); }\n}\nnew Bot().run(\"ask\");",
        "run(m: \"ask\") { const f = (this as any)[m]; return f.call({ model: \"claude-sonnet-5-5\" }); }\n}\nnew Bot().run(\"ask\");",
        "run(m: \"ask\") { return Reflect.get(this, m).call({ model: \"claude-sonnet-5-5\" }); }\n}\nnew Bot().run(\"ask\");",
        "}\nconst bot = new Bot();\nexport function run(m: string) { const f = (bot as any)[m]; return f.call({ model: \"claude-sonnet-5-5\" }); }",
        "}\nexport function run(m: string) { const f = (Bot.prototype as any)[m]; return f.call({ model: \"claude-sonnet-5-5\" }); }",
        "}\nfunction pick(b: Bot, m: string) { return (b as any)[m]; }\nexport const run = (m: string) => pick(new Bot(), m).call({ model: \"claude-sonnet-5-5\" });",
        "me() { return this; }\n}\nconst b = new Bot().me();\nexport function run(m: string) { const f = (b as any)[m]; return f.call({ model: \"claude-sonnet-5-5\" }); }",
        "}\nexport function run(m: string) { for (const b of [new Bot()]) { const f = (b as any)[m]; return f.call({ model: \"claude-sonnet-5-5\" }); } }",
    ] {
        let source = format!("{ANTHROPIC}{ANTHROPIC_BOT}  {run}\n");
        let (value, origin, gaps) = messages_row(&source);
        assert_eq!(value.as_deref(), Some("claude-3-5-sonnet-20241022"), "{source}");
        assert_eq!(origin, Some(ValueOrigin::ClassConstant), "{source}");
        assert_eq!(gaps, vec!["unresolved_model/attribute"], "{source}");
    }
    // A static method read off the class.
    let (_, _, gaps) = messages_row(&format!(
        "{ANTHROPIC}class Bot {{
  static model = \"claude-3-5-sonnet-20241022\";
  static ask() {{ return client.messages.create({{ model: this.model, max_tokens: 10, messages: [] }}); }}
}}
export function run(m: string) {{ const f = (Bot as any)[m]; return f.call({{ model: \"claude-sonnet-5-5\" }}); }}
"
    ));
    assert!(
        gaps.contains(&"unresolved_model/attribute".to_string()),
        "{gaps:?}"
    );
    // A private method's parameter default: its calls are not all seen.
    let dispatcher = |route: &str| {
        format!(
            "class Svc {{
  private summarize(model = \"gpt-4o\") {{
    return client.chat.completions.create({{ model, messages: [] }});
  }}
  run(task: \"summarize\", model: string) {{ {route} }}
}}
new Svc().run(\"summarize\", \"gpt-5.5\");"
        )
    };
    for route in [
        "const handler = this[task]; return handler.call(this, model);",
        "const handler = (this as any)[`${task}`]; if (typeof handler !== \"function\") throw new Error(task); return handler.call(this, model);",
        "return Reflect.apply(this[task], this, [model]);",
        "return Reflect.get(this, task).call(this, model);",
        "return Reflect.get(this, \"summarize\").call(this, model);",
        "return (0, this[task])(model);",
        "return [this[task]][0](model);",
        "return Function.prototype.call.call(this[task], this, model);",
        "if (typeof this[task] !== \"function\") return null; return this.summarize();",
    ] {
        let (value, origin, gaps) = model_row(&dispatcher(route));
        assert_eq!(value.as_deref(), Some("gpt-4o"), "{route}");
        assert_eq!(origin, Some(ValueOrigin::ParameterDefault), "{route}");
        assert_eq!(gaps, vec!["unresolved_model/unseen_callers"], "{route}");
    }
    // The `unseen_callers` detail names the read and its line.
    let found = usages(&with_client(&dispatcher(
        "const handler = this[task]; return handler.call(this, model);",
    )));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "unseen_callers")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert!(
        gap.detail
            .contains("a member read under a name known only at run time at line 7"),
        "{}",
        gap.detail
    );
    // An object literal looked up by key is no instance, and neither is
    // an object literal's own `this`; `#name` is out of reach of a key.
    for elsewhere in [
        "const MODELS: Record<string, string> = { fast: \"gpt-4o-mini\" };\nexport const pick = (k: string) => MODELS[k];",
        "export const store = { items: {} as any, get(k: string) { return this.items[k] ?? this[k as \"items\"]; } };",
        "export function row(r: any, k: string) { return r[k]; }",
    ] {
        let (value, gaps) = model_of(&format!(
            "class Bot {{ private model = \"gpt-4\"; ask() {{ return client.chat.completions.create({{ model: this.model, messages: [] }}); }} }}
new Bot().ask();
{elsewhere}"
        ));
        assert_eq!(value.as_deref(), Some("gpt-4"), "{elsewhere}");
        assert!(gaps.is_empty(), "{elsewhere}: {gaps:?}");
    }
    let (value, gaps) = model_of(
        "class Bot {
  private model = \"gpt-4\";
  #ask() { return client.chat.completions.create({ model: this.model, messages: [] }); }
  run() { return this.#ask(); }
  pick(m: string) { const f = (this as any)[m]; return f.call({ model: \"gpt-5.5\" }); }
}
new Bot().run();",
    );
    assert_eq!(value.as_deref(), Some("gpt-4"));
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn a_body_in_request_options_replaces_the_model_written() {
    // Both SDKs send `{ body, ...options }`: a `body` in the request
    // options replaces the one written, so its model is only a candidate.
    let found = usages(&with_client(
        "client.chat.completions.create({ model: \"gpt-4o\", messages: [] }, { body: { model: \"gpt-5.5\", messages: [] } });",
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), None);
    assert_eq!(value(c, "body.model"), Some("gpt-5.5"));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "splat")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert_eq!(gap.key.as_deref(), Some("model"));
    assert_eq!(
        gap.values
            .iter()
            .map(|v| v.value.as_str())
            .collect::<Vec<_>>(),
        vec!["gpt-4o"]
    );
    assert!(
        gap.detail.contains(
            "is written in the request body, which request options that may hold a `body` replace"
        ),
        "{}",
        gap.detail
    );
    for request in [
        format!("{ANTHROPIC}client.messages.create(\n  {{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }},\n  {{ body: {{ model: \"claude-sonnet-5-5\", max_tokens: 10, messages: [] }} }},\n);"),
        format!("{ANTHROPIC}const opts = {{ timeout: 1, body: {{ model: \"claude-sonnet-5-5\" }} }};\nclient.messages.create({{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }}, opts);"),
        format!("{ANTHROPIC}export class Bot {{\n  private opts = {{ body: {{ model: \"claude-sonnet-5-5\" }} }};\n  ask() {{ return client.messages.create({{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }}, this.opts); }}\n}}"),
        "import { AnthropicVertex } from \"@anthropic-ai/vertex-sdk\";\nconst client = new AnthropicVertex();\nexport const ask = (o: any) => client.messages.create({ model: \"claude-3-5-sonnet-20241022\", max_tokens: 10, messages: [] }, o);".to_string(),
    ] {
        let (value, _, gaps) = messages_row(&request);
        assert_eq!(value, None, "{request}");
        assert!(
            gaps.contains(&"unresolved_model/splat".to_string()),
            "{request}: {gaps:?}"
        );
    }
    // Every argument written may be replaced: the call's `splat` says so.
    assert!(c.splat);
    // A value that already has a gap keeps it, with the body's next to it;
    // one that is not found keeps its own.
    let found = usages(&with_client(
        "export class Bot {
  private model = \"gpt-4-0613\";
  ask(o: any) { return client.chat.completions.create({ model: this.model, messages: [] }, o); }
}",
    ));
    let c = call(&found, "client.chat.completions.create");
    assert_eq!(value(c, "model"), Some("gpt-4-0613"));
    let reasons: Vec<&str> = found
        .gaps
        .iter()
        .filter(|g| g.key.as_deref() == Some("model"))
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(reasons, vec!["subclass_override", "splat"]);
    let splat = found
        .gaps
        .iter()
        .find(|g| g.reason == "splat")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert!(
        splat
            .detail
            .ends_with("its value is captured with origin `class_constant`"),
        "{}",
        splat.detail
    );
    let found = usages(&with_client(
        "import { CONFIG } from \"./config\";
export const ask = (o: any) => client.chat.completions.create({ model: CONFIG.model, messages: [] }, o);",
    ));
    let reasons: Vec<&str> = found
        .gaps
        .iter()
        .filter(|g| g.key.as_deref() == Some("model"))
        .map(|g| g.reason.as_str())
        .collect();
    assert_eq!(reasons, vec!["attribute"]);
    assert!(call(&found, "client.chat.completions.create").splat);
    // A model nested in the body is replaced too.
    let found = usages(&format!(
        "{ANTHROPIC}client.messages.batches.create({{ requests: [{{ custom_id: \"a\", params: {{ model: \"claude-3-5-sonnet-20241022\", max_tokens: 1, messages: [] }} }}] }}, {{ body: {{}} }});"
    ));
    assert!(
        found
            .gaps
            .iter()
            .any(|g| g.reason == "splat" && g.key.as_deref() == Some("requests.params.model")),
        "{:?}",
        found.gaps
    );
    // Options that set only how the request is sent keep it.
    for options in [
        "{ headers: { \"x-a\": \"1\" }, timeout: 1000, maxRetries: 1 }",
        "{ signal: AbortSignal.timeout(1000) }",
        "undefined",
    ] {
        let (value, gaps) = model(&with_client(&format!(
            "client.chat.completions.create({{ model: \"gpt-4o\", messages: [] }}, {options});"
        )));
        assert_eq!(value.as_deref(), Some("gpt-4o"), "{options}");
        assert!(gaps.is_empty(), "{options}: {gaps:?}");
    }
}

#[test]
fn a_method_run_on_an_object_that_borrows_its_class_reads_that_object() {
    // A `Proxy` of an instance, an object made from an instance or the
    // class's prototype, and a method taken from the prototype's
    // descriptor all run the class's methods with another `this`.
    for (after, gap) in [
        (
            "const p = new Proxy(new Bot(), { get(t: any, k) { return k === \"model\" ? \"claude-sonnet-5-5\" : t[k]; } });\np.ask();",
            "attribute",
        ),
        (
            "const o = Object.create(new Bot(), { model: { value: \"claude-sonnet-5-5\" } });\no.ask();",
            "attribute",
        ),
        (
            "const o: any = { model: \"claude-sonnet-5-5\", __proto__: Bot.prototype };\no.ask();",
            "attribute",
        ),
        (
            "const o: any = Object.setPrototypeOf({ model: \"claude-sonnet-5-5\" }, Bot.prototype);\no.ask();",
            "attribute",
        ),
        (
            "Object.getOwnPropertyDescriptor(Bot.prototype, \"ask\")!.value.call({ model: \"claude-sonnet-5-5\" });",
            "attribute",
        ),
        (
            "const o: any = { model: \"claude-sonnet-5-5\", __proto__: new Bot() };\no.ask();",
            "attribute",
        ),
        (
            "const bot = new Bot();\nconst o: any = Object.setPrototypeOf({ model: \"claude-sonnet-5-5\" }, bot);\no.ask();",
            "attribute",
        ),
        (
            "class Spy extends Bot {}\nconst o: any = Object.setPrototypeOf({ model: \"claude-sonnet-5-5\" }, Spy.prototype);\no.ask();",
            "attribute",
        ),
        // A name that holds an instance, made a prototype every way the
        // instance itself can be.
        (
            "const bot = new Bot();\n({ model: \"claude-sonnet-5-5\", __proto__: bot } as any).ask();",
            "attribute",
        ),
        (
            "const bot = new Bot();\nconst o: any = { model: \"claude-sonnet-5-5\" };\no.__proto__ = bot;\no.ask();",
            "attribute",
        ),
        (
            "const bot = new Bot();\nconst o: any = { model: \"claude-sonnet-5-5\" };\no[\"__proto__\"] = bot;\no.ask();",
            "attribute",
        ),
        (
            "const bot = new Bot();\nconst o: any = { model: \"claude-sonnet-5-5\" };\nReflect.set(o, \"__proto__\", bot);\no.ask();",
            "attribute",
        ),
        (
            "const o: any = { model: \"claude-sonnet-5-5\" };\nReflect.set(o, \"__proto__\", new Bot());\no.ask();",
            "attribute",
        ),
    ] {
        let source = format!("{ANTHROPIC}{ANTHROPIC_BOT}}}\n{after}\n");
        let (value, _, gaps) = messages_row(&source);
        if gap == "attribute" {
            assert_eq!(value.as_deref(), Some("claude-3-5-sonnet-20241022"), "{source}");
        }
        assert_eq!(gaps, vec![format!("unresolved_model/{gap}")], "{source}");
    }
    // `Reflect.construct(Bot, [])` builds an instance (passing the class
    // also lets another module extend it).
    let (_, _, gaps) = messages_row(&format!(
        "{ANTHROPIC}{ANTHROPIC_BOT}}}\nconst s = Reflect.construct(Bot, []);\nconst p = new Proxy(s, {{ get: (t: any, k) => (k === \"model\" ? \"claude-sonnet-5-5\" : t[k]) }});\np.ask();\n"
    ));
    assert!(
        gaps.contains(&"unresolved_model/attribute".to_string()),
        "{gaps:?}"
    );
    // A `Proxy` of the class itself runs its static methods with the proxy
    // as `this`.
    let (_, _, gaps) = messages_row(&format!(
        "{ANTHROPIC}class Bot {{
  static model = \"claude-3-5-sonnet-20241022\";
  static ask() {{ return client.messages.create({{ model: this.model, max_tokens: 10, messages: [] }}); }}
}}
const P = new Proxy(Bot, {{ get: (t: any, k) => (k === \"model\" ? \"claude-sonnet-5-5\" : t[k]) }});
P.ask();
"
    ));
    assert!(
        gaps.contains(&"unresolved_model/attribute".to_string()),
        "{gaps:?}"
    );
    // The gap names the use and its line, and the value keeps its origin.
    let found = usages(&format!(
        "{ANTHROPIC}{ANTHROPIC_BOT}}}\nconst o: any = Object.setPrototypeOf({{ model: \"claude-sonnet-5-5\" }}, Bot.prototype);\no.ask();\n"
    ));
    let gap = found
        .gaps
        .iter()
        .find(|g| g.reason == "attribute")
        .unwrap_or_else(|| panic!("{:?}", found.gaps));
    assert_eq!(
        gap.detail,
        "`model` for `client.messages.create` is `this.model`, read in a method that may run with another object as `this` (the class's prototype used as a value at line 7), so it may be that object's property; its value is captured with origin `class_constant`"
    );
    // The prototype compared, or a method read by name off it, is no
    // object that borrows it; `Object.create`'s descriptors give only the
    // new object its properties, and a prototype that is no instance
    // lends no method.
    for after in [
        "new Bot().ask();\nexport const make = (proto: object) => Object.create(proto, { model: { value: \"claude-sonnet-5-5\" } });",
        "new Bot().ask();\nconst base = { kind: \"a\" };\nexport const o = Object.create(base, { model: { value: \"x\", enumerable: true } });",
        "new Bot().ask();\nconst o: any = {};\nconst p: any = { model: \"x\" };\no.__proto__ = p;\nReflect.set(o, \"__proto__\", p);",
        "new Bot().ask();\nexport const isBot = (x: object) => Object.getPrototypeOf(x) === Bot.prototype;",
        "new Bot().ask();\nconsole.log(Bot.prototype.ask.length);",
        "new Bot().ask();\nexport const plain = Object.create(null, { x: { value: 1 } });",
    ] {
        let source = format!("{ANTHROPIC}{ANTHROPIC_BOT}}}\n{after}\n");
        let (value, _, gaps) = messages_row(&source);
        assert_eq!(value.as_deref(), Some("claude-3-5-sonnet-20241022"), "{source}");
        assert!(gaps.is_empty(), "{source}: {gaps:?}");
    }
}

#[test]
fn class_held_request_options_reached_without_their_name_are_not_read() {
    // A class property's object reached through a member read under a
    // run-time name, through the instance's own entries, or through an
    // instance handed to code that may change it.
    let summarizer = |method: &str, after: &str| {
        format!(
            "{OPENAI}const client = new OpenAI();
export class Summarizer {{
  private requestOptions = {{ timeout: 30_000, maxRetries: 2 }};
  {method}
  summarize(text: string) {{
    return client.chat.completions.create(
      {{ model: \"gpt-4-0613\", messages: [{{ role: \"user\", content: text }}] }},
      this.requestOptions,
    );
  }}
}}
{after}
"
        )
    };
    let gapped = vec![
        "unresolved_model/splat".to_string(),
        "ambiguous_client/endpoint_option".to_string(),
    ];
    for (method, after) in [
        (
            "configure(section: string, path: string) { const target = (this as any)[section]; target.path = path; }",
            "",
        ),
        (
            "",
            "function override(obj: any, section: string, url: string) { obj[section].path = url; }\nconst s = new Summarizer();\noverride(s, \"requestOptions\", \"https://gateway.example.com/v1/chat/completions\");",
        ),
        (
            "route(url: string) { for (const v of Object.values(this)) (v as any).path = url; }",
            "",
        ),
        (
            "route(url: string) { for (const k in this) { const v: any = (this as any)[k]; if (v && typeof v === \"object\") v.path = url; } }",
            "",
        ),
        // A same-file merge that walks the instance's keys.
        (
            "constructor(extra: any) { deepMerge(this, extra); }",
            "function deepMerge(target: any, source: any) { for (const k of Object.keys(source)) { if (typeof source[k] === \"object\") deepMerge(target[k], source[k]); else target[k] = source[k]; } }",
        ),
        (
            "route(url: string) { const { ...all } = this as any; for (const v of Object.values(all)) (v as any).path = url; }",
            "",
        ),
        (
            "route(url: string) { let all: any; ({ ...all } = this as any); for (const v of Object.values(all)) (v as any).path = url; }",
            "",
        ),
        (
            "route(url: string) { const copy: any = { ...this }; for (const v of Object.values(copy)) (v as any).path = url; }",
            "",
        ),
        (
            "route(url: string) { const copy: any = Object.assign({}, this); for (const v of Object.values(copy)) (v as any).path = url; }",
            "",
        ),
        (
            "route(url: string) { for (const d of Object.values(Object.getOwnPropertyDescriptors(this))) d.value.path = url; }",
            "",
        ),
        // A same-file function that reads its argument's properties
        // without naming them.
        (
            "route(url: string) { walk(this, url); }",
            "function walk({ ...all }: any, url: string) { for (const v of Object.values(all)) (v as any).path = url; }",
        ),
        (
            "route(url: string) { walk(this, url); }",
            "function walk() { for (const v of Object.values(arguments[0])) (v as any).path = arguments[1]; }",
        ),
        (
            "route(url: string) { return new Walker(this, url); }",
            "class Walker { constructor({ ...all }: any, url: string) { for (const v of Object.values(all)) (v as any).path = url; } }",
        ),
        (
            "configure(section: string, path: string) { Reflect.get(this, section).path = path; }",
            "",
        ),
        (
            "",
            "const s = Reflect.construct(Summarizer, []);\nexport function set(k: string, u: string) { (s as any)[k].path = u; }",
        ),
        // Through a name that holds one.
        (
            "",
            "const s = new Summarizer();\nfor (const v of Object.values(s)) (v as any).path = \"https://gateway.example.com/v1/chat/completions\";",
        ),
    ] {
        let source = summarizer(method, after);
        assert_eq!(
            create_route(&source),
            (Routing::Unknown, gapped.clone()),
            "{source}"
        );
    }
    // Options held in a static property, read by class name: a member of
    // the class read under a run-time name may be them.
    for configure in [
        "const target = (Svc as any)[section]; target.path = url;",
        "(Svc as any)[section].path = url;",
        "Object.values(Svc).forEach((v: any) => { v.path = url; });",
    ] {
        let source = format!(
            "{OPENAI}const client = new OpenAI();
export class Svc {{
  static OPTIONS = {{ timeout: 30_000 }};
  ask() {{ return client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, Svc.OPTIONS); }}
}}
export function configure(section: string, url: string) {{ {configure} }}
"
        );
        assert_eq!(
            create_route(&source),
            (Routing::Unknown, gapped.clone()),
            "{source}"
        );
    }
    // Uses that keep it: its own method bound to it, a builtin that only
    // reads it, a method that returns it to its caller, an export, a
    // same-file function that reads it by name, a test, and the instance
    // stored, returned or passed to another module's code, which does not
    // write what its properties hold (Outside the file).
    for (method, after) in [
        ("constructor(extra: any) { deepMerge(this, extra); }", ""),
        (
            "",
            "export const s = new Summarizer();\ndeepMerge(s, JSON.parse(process.env.OVERRIDES ?? \"{}\"));",
        ),
        ("register() { registry.push(this); }", ""),
        (
            "withTimeout(ms: number) { this.ms = ms; return this; }\n  private ms = 0;",
            "register(new Summarizer().withTimeout(1));",
        ),
        ("self() { return Promise.resolve(this); }", ""),
        ("async self() { return this; }", ""),
        ("constructor() { makeAutoObservable(this); autoBind(this); bus.emit(\"ready\", this); }", ""),
        (
            "private static instance?: Summarizer;\n  static get() { if (!Summarizer.instance) Summarizer.instance = new Summarizer(); return Summarizer.instance; }",
            "",
        ),
        ("", "export function createSession() { return new Summarizer(); }"),
        ("", "const jobs = [\"a\", \"b\"].map(() => new Summarizer());\nconst queue: Summarizer[] = [];\nqueue.push(jobs[0]);\nconst sessions = new Map<string, Summarizer>();\nsessions.set(\"x\", jobs[1]);"),
        ("", "class Controller { constructor(private svc: Summarizer) {} }\nexport const c = new Controller(new Summarizer());"),
        ("", "const svc = new Summarizer();\napp.locals.chat = svc;\ncontainer.register(\"chat\", svc);\nexpect(svc).toBeDefined();\nvi.spyOn(svc, \"summarize\");"),
        ("", "const summarizer = new Summarizer();\nmodule.exports = { summarizer, other: new Summarizer() };"),
        ("", "const summarizer = new Summarizer();\nexport default { summarizer };\nexport const services = { summarizer: new Summarizer() };"),
        // `this` in a plain function forwarded by a helper.
        (
            "",
            "export function debounce(fn: any, ms: number) { let t: any; return function (this: unknown, ...args: any[]) { clearTimeout(t); t = setTimeout(() => fn.apply(this, args), ms); }; }\nexport function once(fn: any) { let v: any; return function (this: unknown) { v ??= fn.apply(this, arguments); return v; }; }",
        ),
        ("", "function show(...items: any[]) { return items.length; }\nshow(new Summarizer());"),
        ("constructor() { this.summarize = this.summarize.bind(this); }", ""),
        ("log() { console.log(this); return JSON.stringify(this); }", ""),
        ("withTimeout(ms: number) { this.ms = ms; return this; }\n  private ms = 0;", "export const s = new Summarizer().withTimeout(1);"),
        ("", "export default new Summarizer();"),
        (
            "",
            "function describe(s: Summarizer) { return s.summarize.name; }\ndescribe(new Summarizer());",
        ),
        ("is(other: unknown) { return other === this || !this; }", ""),
        ("keys() { return Object.keys(this); }", ""),
        // Another class's instances handed out reach no property of this
        // one.
        (
            "",
            "class Route { constructor(public path: string) {} }\nexport const routes = [new Route(\"/x\")];\nregister(new Route(\"/y\"));\nfunction prefixed(r: Route) { r.path = \"/api\" + r.path; return r; }\nprefixed(new Route(\"/z\"));",
        ),
    ] {
        let source = summarizer(method, after);
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{source}"
        );
    }
}

#[test]
fn request_options_only_read_keep_the_route() {
    // A test, a condition, the left side of `&&`, a destructuring, a
    // `for...in`, a cast, an alias that is only read, and a same-file
    // function's parameter that is only read change nothing the options
    // hold.
    let class = |method: &str, options: &str| {
        format!(
            "{OPENAI}const client = new OpenAI();
export class Svc {{
  private opts = {{ timeout: 30_000, maxRetries: 2 }};
  private slow = {{ timeout: 120_000 }};
  {method}
  ask(q: string, slow?: boolean) {{
    return client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [{{ role: \"user\", content: q }}] }}, {options});
  }}
}}
"
        )
    };
    for (method, options) in [
        ("check() { if (this.opts && this.opts.timeout > 0) console.log(\"timeout set\"); }", "this.opts"),
        ("timeout() { return (this.opts && this.opts.timeout) || 60_000; }", "this.opts"),
        ("retries() { while (this.opts && \"maxRetries\" in this.opts) return 1; }", "this.opts"),
        ("pick() { return this.opts && this.opts.timeout ? this.opts.timeout : 1000; }", "this.opts"),
        ("check() { if (this.opts || this.slow) console.log(\"set\"); }", "this.opts"),
        ("fields() { const { timeout, maxRetries } = this.opts; return timeout + maxRetries; }", "this.opts"),
        ("fields() { const { timeout = 0 } = this.opts; return timeout; }", "this.opts"),
        ("keys() { const out: string[] = []; for (const k in this.opts) out.push(k); return out; }", "this.opts"),
        ("own() { const { opts } = this; return opts.timeout; }", "this.opts"),
        (
            "relay() { return this.send(this.opts); }\n  private send(o: OpenAI.RequestOptions) { return client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }, o); }",
            "this.opts",
        ),
        ("", "this.opts as OpenAI.RequestOptions"),
        ("", "this.opts!"),
        ("", "(this.opts)"),
    ] {
        let source = class(method, options);
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{source}"
        );
    }
    // An alias that is only read, also through a conditional.
    let source = class("", "this.opts").replace(
        "    return client",
        "    const options = slow ? this.slow : this.opts;\n    return client",
    );
    let source = source.replace("}, this.opts);", "}, options);");
    assert_eq!(
        create_route(&source),
        (Routing::OpenaiDirect, vec![]),
        "{source}"
    );
    // The same for names, through a wrapper whose calls are all seen.
    let file = |body: &str| format!("{OPENAI}const client = new OpenAI();\n{body}\n");
    let create = "client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }";
    for body in [
        format!("const opts = {{ timeout: 1 }};\nif (opts && opts.timeout > 0) console.log(\"t\");\nexport const ask = () => {create}, opts);"),
        format!("const opts = {{ timeout: 1 }};\nif (opts) console.log(\"t\");\nexport const ask = () => {create}, opts);"),
        format!("const opts = {{ timeout: 1 }};\nif (opts || process.env.DEBUG) console.log(\"t\");\nexport const ask = () => {create}, opts);"),
        format!("const OPTS = {{ timeout: 1 }};\nconst {{ timeout }} = OPTS;\nconsole.log(timeout);\nexport const ask = () => {create}, OPTS);"),
        format!("const OPTS = {{ timeout: 1 }};\nconst opts = OPTS;\nexport const ask = () => {create}, opts);"),
        format!("const DEFAULTS = {{ timeout: 1 }};\nfunction chat(q: string, opts = DEFAULTS) {{ return {create}, opts); }}\nexport const ask = (q: string) => chat(q);"),
        format!("const LONG = {{ timeout: 120_000 }};\nfunction chat(q: string, opts?: OpenAI.RequestOptions) {{ return {create}, opts); }}\nexport const ask = (q: string) => chat(q, LONG);"),
        format!("function ask(text: string, opts?: {{ timeout?: number }}) {{\n  if (opts) console.log(\"timeout\", opts.timeout);\n  return {create}, opts);\n}}\nask(\"hi\", {{ timeout: 5000 }});\nask(\"hi\");"),
        format!("export async function POST() {{\n  const options = {{ timeout: 1 }};\n  return {create}, options as OpenAI.RequestOptions);\n}}"),
        format!("export async function POST(req: Request) {{\n  const options = {{ timeout: 1 }};\n  (options as any).signal = req.signal;\n  return {create}, options);\n}}"),
    ] {
        let source = file(&body);
        assert_eq!(
            create_route(&source),
            (Routing::OpenaiDirect, vec![]),
            "{source}"
        );
    }
    // An alias or a parameter that hands it out does not.
    for body in [
        format!("const OPTS: any = {{ timeout: 1 }};\nconst opts = OPTS;\nopts.path = \"https://gateway.example.com/v1/chat/completions\";\nexport const ask = () => {create}, OPTS);"),
        format!("const OPTS: any = {{ timeout: 1 }};\nfunction tweak(o: any) {{ o.path = \"https://gateway.example.com/v1/chat/completions\"; }}\ntweak(OPTS);\nexport const ask = () => {create}, OPTS);"),
        format!("const OPTS: any = {{ timeout: 1 }};\nexport function tweak(o: any) {{ return o; }}\ntweak(OPTS);\nexport const ask = () => {create}, OPTS);"),
        format!("const OPTS = {{ timeout: 1 }};\n(OPTS as any).path = \"https://gateway.example.com/v1/chat/completions\";\nexport const ask = () => {create}, OPTS);"),
        format!("const OPTS: any = {{ timeout: 1 }};\nconst {{ ...copy }} = OPTS;\nexport const ask = () => {create}, OPTS);\nexport {{ OPTS as shared }};"),
    ] {
        let source = file(&body);
        let (routing, gaps) = create_route(&source);
        if body.contains("...copy") {
            // A rest copy shares no own key: still read.
            assert_eq!((routing, gaps), (Routing::OpenaiDirect, vec![]), "{source}");
            continue;
        }
        assert_eq!(routing, Routing::Unknown, "{source}");
        assert!(
            gaps.contains(&"ambiguous_client/endpoint_option".to_string()),
            "{source}: {gaps:?}"
        );
    }
}

#[test]
fn a_global_object_name_given_an_unknown_value_may_be_the_global_object() {
    let request = "client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] });";
    for setup in [
        "let root: any = {};\n[root] = [globalThis];\nexport function install(name: string, v: unknown) { root[name] = v; }",
        "let root: any = {};\n({ root } = { root: globalThis });\nexport function install(name: string, v: unknown) { root[name] = v; }",
    ] {
        let source = format!("{OPENAI}const client = new OpenAI();\n{setup}\n{request}");
        assert_eq!(
            create_route(&source),
            (
                Routing::Unknown,
                vec!["ambiguous_client/global_fetch".to_string()]
            ),
            "{setup}"
        );
    }
    // The keys of a literal through `Object.getOwnPropertyNames`, which
    // reads it.
    let source = format!(
        "{OPENAI}const client = new OpenAI();
const o = {{ TextEncoder: 1 }};
for (const k of Object.getOwnPropertyNames(o)) (globalThis as any)[k] = 1;
{request}"
    );
    assert_eq!(create_route(&source), (Routing::OpenaiDirect, vec![]));
}

#[test]
fn long_chains_of_aliases_and_wrappers_of_request_options_are_linear() {
    // Names bound to one another, and options passed down wrappers whose
    // calls are all seen, each passed to a request: each name is read
    // once, and past the bound every name read may hand them out.
    let n = 4_000;
    let mut source = with_client("const o0: any = { timeout: 1 };");
    for i in 1..n {
        source.push_str(&format!("const o{i} = o{};\n", i - 1));
    }
    for i in 0..n {
        source.push_str(&format!(
            "export const r{i} = () => client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, o{i});\n"
        ));
    }
    let found = usages_within(&source, 10);
    assert!(!found.calls.is_empty());
    let mut source = with_client("");
    for i in 0..n - 1 {
        source.push_str(&format!(
            "function f{i}(o: any) {{ client.chat.completions.create({{ model: \"gpt-4-0613\", messages: [] }}, o); return f{}(o); }}\n",
            i + 1
        ));
    }
    source.push_str(&format!("function f{}(o: any) {{ return o; }}\n", n - 1));
    for i in 0..n {
        source.push_str(&format!(
            "export const r{i} = () => f{i}({{ timeout: {i} }});\n"
        ));
    }
    let found = usages_within(&source, 10);
    assert!(!found.calls.is_empty());
}

#[test]
fn a_tagged_template_passes_its_substitutions_to_its_tag() {
    // `` tag`${x}` `` calls `tag` with `x`: a substitution there is an
    // argument, not a read. In a template that is not tagged it is read.
    let models = |use_: &str| {
        with_client(&format!(
            "const MODELS = {{ fast: \"gpt-4-0613\" }};\nfunction tag(_s: TemplateStringsArray, m: any) {{ m.fast = \"gpt-5.5\"; return \"\"; }}\n{use_};\nclient.chat.completions.create({{ model: MODELS.fast, messages: [] }});"
        ))
    };
    let (value, gaps) = model(&models("tag`${MODELS}`"));
    assert_eq!(value, None);
    assert_eq!(gaps, vec!["unresolved_model/attribute"]);
    let (value, gaps) = model(&models("console.log(`${MODELS}`)"));
    assert_eq!(value.as_deref(), Some("gpt-4-0613"));
    assert!(gaps.is_empty(), "{gaps:?}");
    // An enum given to a tag may be written.
    let (value, gaps) = model(&with_client(
        "enum Model { Fast = \"gpt-4-0613\" }\nfunction tag(_s: TemplateStringsArray, m: any) { m.Fast = \"gpt-5.5\"; return \"\"; }\ntag`${Model}`;\nclient.chat.completions.create({ model: Model.Fast, messages: [] });",
    ));
    assert_eq!(value, None);
    assert_eq!(gaps, vec!["unresolved_model/reassigned"]);
    // Request options given to a tag may be given a `path`, held in a
    // name or in a class property.
    for body in [
        "const OPTS: any = { timeout: 5 };\nfunction tag(_s: TemplateStringsArray, o: any) { o.path = \"https://gateway.example.com/v1/chat/completions\"; return \"\"; }\ntag`${OPTS}`;\nexport const ask = () => client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }, OPTS);",
        "function tag(_s: TemplateStringsArray, o: any) { o.path = \"https://gateway.example.com/v1/chat/completions\"; return \"\"; }\nexport class Svc {\n  private opts: any = { timeout: 5 };\n  log() { return tag`${this.opts}`; }\n  ask() { return client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }, this.opts); }\n}",
    ] {
        let source = with_client(body);
        let (routing, gaps) = create_route(&source);
        assert_eq!(routing, Routing::Unknown, "{source}");
        assert!(
            gaps.contains(&"ambiguous_client/endpoint_option".to_string()),
            "{source}: {gaps:?}"
        );
    }
    // A class's prototype given to a tag may be another object's.
    let (_, _, gaps) = messages_row(&format!(
        "{ANTHROPIC}{ANTHROPIC_BOT}}}\nfunction tag(_s: TemplateStringsArray, p: any) {{ return Object.setPrototypeOf({{ model: \"claude-sonnet-5-5\" }}, p).ask(); }}\ntag`${{Bot.prototype}}`;\n"
    ));
    assert_eq!(gaps, vec!["unresolved_model/attribute"]);
}

#[test]
fn reflect_get_reads_off_the_values_a_computed_member_reads_off() {
    // `Reflect.get(x, k)` with a run-time `k` counts as `x[k]` does: off
    // `this`, an instance or a name that may hold one, a method may be read
    // to run anywhere. A `Proxy` handler's target or a lazily built client
    // is none of them.
    let bot = |after: &str| format!("{ANTHROPIC}{ANTHROPIC_BOT}}}\n{after}\n");
    for after in [
        "const bot = new Bot();\nexport function run(m: string) { const f = Reflect.get(bot, m); return f.call({ model: \"claude-sonnet-5-5\" }); }",
        "export function run(m: string) { const f = Reflect.get(Bot.prototype, m); return f.call({ model: \"claude-sonnet-5-5\" }); }",
    ] {
        let source = bot(after);
        let (value, _, gaps) = messages_row(&source);
        assert_eq!(value.as_deref(), Some("claude-3-5-sonnet-20241022"), "{source}");
        assert_eq!(gaps, vec!["unresolved_model/attribute"], "{source}");
    }
    for after in [
        "new Bot().ask();\nexport const env = new Proxy(process.env, { get(target, prop) { const v = Reflect.get(target, prop); if (v === undefined) throw new Error(String(prop)); return v; } });",
        "new Bot().ask();\nlet lazy: any;\nexport const shared = new Proxy({} as any, { get(_t, prop, recv) { lazy ??= new Anthropic(); return Reflect.get(lazy, prop, recv); } });",
        "new Bot().ask();\nexport function run(o: any, m: string) { return Reflect.get(o, m); }",
    ] {
        let source = bot(after);
        let (value, _, gaps) = messages_row(&source);
        assert_eq!(value.as_deref(), Some("claude-3-5-sonnet-20241022"), "{source}");
        assert!(gaps.is_empty(), "{source}: {gaps:?}");
    }
    // Request options a class holds stay read beside such a handler.
    let source = with_client(
        "export const env = new Proxy(process.env, { get(target, prop) { return Reflect.get(target, prop); } });\nexport class Summarizer {\n  private requestOptions = { timeout: 30_000 };\n  run(text: string) { return client.chat.completions.create({ model: \"gpt-4-0613\", messages: [{ role: \"user\", content: text }] }, this.requestOptions); }\n}\nexport const s = new Summarizer();",
    );
    assert_eq!(create_route(&source), (Routing::OpenaiDirect, vec![]));
}

#[test]
fn names_past_256_are_answered_exactly() {
    // Whether a name hands request options out, and whether it may hold an
    // instance, is answered for every name it reaches, however many.
    let mut source = with_client(
        "const o0: any = { timeout: 1 };\nexport const ask = () => client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }, o0);\n",
    );
    for i in 1..300 {
        source.push_str(&format!("const o{i} = o{};\n", i - 1));
    }
    source.push_str("console.log(o299.timeout);\n");
    assert_eq!(create_route(&source), (Routing::OpenaiDirect, vec![]));
    // One of them that hands them out does, however far.
    let handed = source.replace(
        "console.log(o299.timeout);",
        "o299.path = \"https://gateway.example.com/v1/chat/completions\";",
    );
    let (routing, gaps) = create_route(&handed);
    assert_eq!(routing, Routing::Unknown);
    assert!(
        gaps.contains(&"ambiguous_client/endpoint_option".to_string()),
        "{gaps:?}"
    );
    let mut source =
        format!("{ANTHROPIC}{ANTHROPIC_BOT}}}\nnew Bot().ask();\nconst a0: any = {{}};\n");
    for i in 1..300 {
        source.push_str(&format!("const a{i} = a{};\n", i - 1));
    }
    source.push_str(
        "export function run(m: string) { const f = a299[m]; return f.call({ model: \"x\" }); }\n",
    );
    let (value, _, gaps) = messages_row(&source);
    assert_eq!(value.as_deref(), Some("claude-3-5-sonnet-20241022"));
    assert!(gaps.is_empty(), "{gaps:?}");
    // An instance at the end of the chain is one.
    let source = source.replace("const a0: any = {};", "const a0: any = new Bot();");
    let (_, _, gaps) = messages_row(&source);
    assert_eq!(gaps, vec!["unresolved_model/attribute"]);
}

#[test]
fn whether_names_hold_an_instance_is_read_once_per_file() {
    // Many reads under a run-time key, each off a parameter passed down a
    // long chain of functions to an instance: the chain is read once, not
    // once per read.
    let (chain, roots) = (1_500, 1_500);
    let mut source = with_client(
        "class Bot { private opts = { timeout: 1 }; run() { return client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }, this.opts); } }\n",
    );
    source.push_str("function g0(p: any) {");
    for i in 0..roots {
        source.push_str(&format!(" f{i}(p, \"a\");"));
    }
    source.push_str(" }\n");
    for j in 1..chain {
        source.push_str(&format!("function g{j}(p: any) {{ g{}(p); }}\n", j - 1));
    }
    source.push_str(&format!("g{}(new Bot());\n", chain - 1));
    for i in 0..roots {
        source.push_str(&format!(
            "function f{i}(x: any, k: string) {{ if (x[k]) return 1; return 0; }}\n"
        ));
    }
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 1);
    // A method's parameter is read through the calls of its name, which
    // every class with a method of that name shares.
    let mut source = with_client(
        "class Bot { private opts = { timeout: 1 }; run() { return client.chat.completions.create({ model: \"gpt-4-0613\", messages: [] }, this.opts); } }\nconst bot = new Bot();\n",
    );
    for i in 0..16_000 {
        source.push_str(&format!(
            "class C{i} {{ m(x: any, k: string) {{ return x[k]; }} }}\n"
        ));
    }
    for i in 0..1_000 {
        source.push_str(&format!("new C{i}().m(bot, \"k\");\n"));
    }
    let found = usages_within(&source, 10);
    assert_eq!(found.calls.len(), 1);
}
