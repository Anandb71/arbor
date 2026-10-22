# Provenance

Verbatim copies of example code from the providers' own SDK repositories,
fetched on 2026-10-21 at the commits below. Each directory keeps the
upstream `LICENSE` beside the files. Nothing was edited; line numbers in
`eval/expected/ts-sdk-real-slice.json` refer to these exact files. Files from
a package subdirectory are prefixed with the package (`bedrock-demo.ts` is
`packages/bedrock-sdk/examples/demo.ts`).

| Directory | Repository and commit | License | Files |
|---|---|---|---|
| `openai/` | [openai/openai-node@95e8fde](https://github.com/openai/openai-node/tree/95e8fdef313b59d1b49a74665b8c87196d17bd5c/examples) | Apache-2.0 | `chat-completions/demo.ts`, `chat-completions/parsing.ts`, `azure/chat.ts`, `bedrock/runtime-chat.ts`, `responses/stream.ts`, `fine-tuning/fine-tuning.ts`, `audio/speech-to-text.ts` |
| `anthropic/` | [anthropics/anthropic-sdk-typescript@9c65624](https://github.com/anthropics/anthropic-sdk-typescript/tree/9c6562472002aa451b6f672acafd6d454e31840f/examples) | MIT | `demo.ts`, `streaming.ts`, `fallbacks.ts`, `count-tokens.ts`, `thinking.ts`, `tools.ts`, `packages/bedrock-sdk/examples/demo.ts`, `packages/vertex-sdk/examples/vertex.ts` |
| `gemini/` | [googleapis/js-genai@e82a75a](https://github.com/googleapis/js-genai/tree/e82a75a8bfe56529ff2651645b19509199360cbc/sdk-samples) | Apache-2.0 | `generate_content_with_text.ts`, `generate_content_with_text_vertex_apikey.ts`, `chats.ts`, `embed_content.ts`, `count_tokens.ts`, `generate_content_with_model_configuration.ts` |
| `gemini-legacy/` | [google-gemini/generative-ai-js@22acc9c](https://github.com/google-gemini/generative-ai-js/tree/22acc9cd0d75d677c70776e1542034f6a6106677/samples) (archived) | Apache-2.0 | `chat.js`, `embed.js`, `model_configuration.js`, `system_instruction.js` |

Why these files: they are the shapes the providers tell developers to copy:
the default and named client imports, Azure's v1 endpoint through a plain
`OpenAI` client and Bedrock through `openai/providers/bedrock`, streaming helpers, fine-tuning jobs, Anthropic's
server-side and client-side model fallbacks, the Bedrock and Vertex SDK
packages, the current Gemini SDK with and without Vertex AI, and the archived
legacy Gemini SDK's models and chats.
