# Provenance

Verbatim copies of example code from the providers' own SDK repositories,
fetched on 2026-10-16 at the commits below. Each directory keeps the
upstream `LICENSE` beside the files. Nothing was edited; line numbers in
`eval/expected/py-sdk-real-slice.json` refer to these exact files.

| Directory | Repository and commit | License | Files |
|---|---|---|---|
| `openai/` | [openai/openai-python@230a923](https://github.com/openai/openai-python/tree/230a923faba5574ebdc5b45a505a26b6bad79f61/examples) | Apache-2.0 | `async_demo.py`, `azure.py`, `azure_ad.py`, `bedrock.py`, `demo.py`, `module_client.py`, `streaming.py` |
| `anthropic/` | [anthropics/anthropic-sdk-python@bcd55fe](https://github.com/anthropics/anthropic-sdk-python/tree/bcd55fe1d3f0f1240765f8cef04295b67337f55a/examples) | MIT | `azure.py`, `bedrock.py`, `fallbacks.py`, `google_cloud.py`, `messages.py`, `messages_stream.py`, `vertex.py` |
| `gemini/` | [google-gemini/deprecated-generative-ai-python@7a7cc54](https://github.com/google-gemini/deprecated-generative-ai-python/tree/7a7cc5474ddaa0255a4410e05361028a24400abd/samples) (archived) | Apache-2.0 | `chat.py`, `configure_model_parameters.py`, `embed.py`, `system_instruction.py` |

Why these files: they are the shapes the providers tell developers to copy,
including clients this engine version does not classify
(`OpenAI(provider=bedrock(...))`, `AnthropicFoundry`, `AnthropicGoogleCloud`),
module-level configuration (`openai.base_url`), imports inside test methods,
and model IDs nested in arguments. Three of those shapes changed the engine
before this fixture was recorded: platform clients are tracked with
`unknown` routing instead of being missed, an SDK-shaped call on an
unrecognised SDK result is a gap instead of a silent drop, and `model` keys
inside dict arguments (`fallbacks=[{"model": ...}]`) are captured.
