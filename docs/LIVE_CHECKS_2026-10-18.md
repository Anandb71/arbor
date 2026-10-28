# Live checks, 18 October 2026

On 18 October 2026 we sent 72 requests to OpenAI's API to settle, with real calls, what
the provider's pages state only for other models or leave open: which arguments the
replacement models reject, and what the Models API says about the aliases. This page
publishes what came back: each request, the HTTP status and the API's error message,
word for word. The rules marked `live` in the data bundled with Arbor rest on these
calls (see [Where the data comes from](RETIREMENTS.md#where-the-data-comes-from)).

Every rule and entry named here is in
[`crates/arbor-core/data/model-retirements.json`](../crates/arbor-core/data/model-retirements.json)
as bundled in Arbor 3.1.0. A rule's `live_models` lists the only models its `live` label
covers, and `live_endpoints` the only API.

- [What was checked](#what-was-checked)
- [Results](#results)
- [The messages, verbatim](#the-messages-verbatim)
- [What these calls did not check](#what-these-calls-did-not-check)
- [Reproduce the checks](#reproduce-the-checks)

## What was checked

| | |
|---|---|
| **When** | 18 October 2026 (UTC+05:30). In UTC the 66 calls of the run went out on 17 October between 21:08:52 and 21:10:32, and the 6 retry attempts between 21:10:33 and 21:11:29. |
| **Where** | OpenAI's own host, `https://api.openai.com/v1`, called directly: no proxy, no Azure, no SDK. |
| **Endpoints** | `GET /v1/models/{model}` (the Models API), `POST /v1/chat/completions` (Chat Completions) and, for the four `-completions` IDs only, `POST /v1/completions` (legacy Completions). No request went to the Responses API. |
| **Models** | Models API: `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini`, `gpt-4o-2024-08-06`, `gpt-4o-2024-11-20`, `gpt-4o-mini-2024-07-18`, `gpt-4.1-2025-04-14`, `gpt-4.1-mini-2025-04-14`, `gpt-4-1106-preview`, `gpt-4o-2024-05-13`, `gpt-3.5-turbo`, `gpt-4`, and the four `-completions` IDs. Chat Completions: the four aliases (served snapshot only), `gpt-5.6-terra`, `gpt-5.6-sol`, `gpt-5.6-luna` and `gpt-6-luna`. |
| **Method** | One Python script using only the standard library (`urllib`), with one API key, sending one request at a time 0.4 s apart. Every Chat Completions request asked "Reply with the word OK." with a small output cap; the tool and function requests asked "What time is it?". The two `gpt-6-luna` penalty calls returned 500 and were each retried three more times. |
| **Recorded** | For every call: the request body, the HTTP status, and the error's `type`, `code`, `param` and `message`. For a success, a few named fields: from the Models API the model's `shutdown_date` and `owned_by`; from Chat Completions the served `model`, `finish_reason`, the first 80 characters of the reply, the names of tool or function calls, whether logprobs came back, and the completion and reasoning token counts; and the response's `id` and `created`. Never the key or a full response body. |
| **Left out here** | The completion IDs (`chatcmpl-...`), the response and per-call timestamps, and anything else that could identify the account or a request. Error messages are quoted whole and unchanged: none of them contained an account detail or a request ID. |

The runner labels each call with its check, `L-OAI-1` to `L-OAI-8`; there is no `L-OAI-7`
in this run.

## Results

Below, *@none* means the request set `"reasoning_effort": "none"`, and *default effort*
means it did not set `reasoning_effort`, so the model's default applied. Every Chat
Completions request also sent
`"messages": [{"role": "user", "content": "Reply with the word OK."}]`, except the
`tools` and `functions` requests, whose message was `"What time is it?"`. The exact
bodies are under [The messages, verbatim](#the-messages-verbatim).

### Models API: `GET /v1/models/{model}`

| Check | Model | Status | Recorded | Supports in the bundled data |
|---|---|---|---|---|
| L-OAI-1 | `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini` (aliases) | 200 | `"shutdown_date": "2027-01-14"` | The entries for the four aliases: retiring on 2027-01-14, replacement `gpt-6-luna`, `verified: live`. OpenAI's deprecations page lists only their dated snapshots. |
| L-OAI-1 | `gpt-4o-2024-08-06`, `gpt-4o-2024-11-20`, `gpt-4o-mini-2024-07-18`, `gpt-4.1-2025-04-14`, `gpt-4.1-mini-2025-04-14` | 200 | `"shutdown_date": "2027-01-14"` | Their entries (2027-01-14, `verified: docs`): the API agrees with the page. |
| L-OAI-1 | `gpt-4o-2024-05-13`, `gpt-3.5-turbo`, `gpt-4` | 200 | `"shutdown_date": "2026-10-23"` | Their entries (2026-10-23, `verified: docs`): the API agrees with the page. |
| L-OAI-1 | `gpt-4-1106-preview` | 404 | `The model 'gpt-4-1106-preview' does not exist` ([E8](#e8-gpt-4-1106-preview-does-not-exist)) | The `gpt-4-1106-preview` entry: `status: shut_down`, `verified: live`, note `OpenAI lists 23 Oct 2026, but the API already answers 404 (checked 18 Oct 2026).` |
| L-OAI-2 | `gpt-3.5-turbo-completions`, `gpt-4-0613-completions`, `gpt-4-completions`, `gpt-4-turbo-completions` | 404 | `The model '<id>' does not exist`, with each ID in place of `<id>` ([E9](#e9-the-four-completions-ids-are-not-callable)) | Their entries: `status: shut_down`, `action: report_only`, `verified: live`, note `Not a callable model ID: the API answers 404 (checked 18 Oct 2026).` |

### Legacy Completions: `POST /v1/completions`

| Check | Model | Sent | Status | Message (verbatim) | Supports in the bundled data |
|---|---|---|---|---|---|
| L-OAI-2 | the four `-completions` IDs | `"prompt": "Reply with the word OK."`, `"max_tokens": 5` | 404 | ``The model `<id>` does not exist or you do not have access to it.``, with each ID in place of `<id>` ([E9](#e9-the-four-completions-ids-are-not-callable)) | The same four entries (`report_only`). |

### Chat Completions: `POST /v1/chat/completions`

| Check | Model | Sent | Status | Message (verbatim) | Supports in the bundled data |
|---|---|---|---|---|---|
| L-OAI-1 | `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini` | `"max_tokens": 5` | 200 | none. Served `gpt-4o-2024-08-06`, `gpt-4o-mini-2024-07-18`, `gpt-4.1-2025-04-14` and `gpt-4.1-mini-2025-04-14` | The alias entries above: on 18 October 2026 each alias served a snapshot that OpenAI lists for 14 January 2027. |
| L-OAI-4, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | `"max_tokens": 50`, at default effort and @none | 400 | `Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.` ([E1](#e1-max_tokens-is-rejected)) | `OAI-MAX-TOKENS` (live: `gpt-5.6-terra`), `G6-MAX-TOKENS` (live: `gpt-6-luna`) |
| L-OAI-4, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | `"max_completion_tokens": 50`, default effort | 200 | none | `OAI-MAX-TOKENS`, `G6-MAX-TOKENS`: the renamed argument works. |
| L-OAI-3, L-OAI-8 | `gpt-5.6-terra`, `gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-6-luna` | `"temperature": 0.2`, `"max_completion_tokens": 200`, default effort | 400 | `Unsupported value: 'temperature' does not support 0.2 with this model. Only the default (1) value is supported.` ([E2](#e2-a-non-default-temperature-is-rejected)) | `OAI-SAMPLING` (live: `gpt-5.6-terra`, `gpt-5.6-sol`, `gpt-5.6-luna`), `G6-SAMPLING` (live: `gpt-6-luna`) |
| L-OAI-3, L-OAI-8 | the same four | @none with `"temperature": 0.2`, or `"top_p": 0.5`, or `"logprobs": true, "top_logprobs": 2` | 200 | none (logprobs came back) | `OAI-SAMPLING`, `G6-SAMPLING`: the *keep reasoning off* option keeps these arguments. |
| L-OAI-6, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | `tools` (one function, `get_time`), default effort | 400 | `Function tools with reasoning_effort are not supported for <model> in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.`, with the model in place of `<model>` ([E3](#e3-function-tools-need-reasoning_effort-none)) | `OAI-TOOLS-CC` (live: `gpt-5.6-terra`), `G6-TOOLS-CC` (live: `gpt-6-luna`) |
| L-OAI-6, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | the same `tools` @none | 200 | none. `finish_reason: tool_calls`, called `get_time` | `OAI-TOOLS-CC`, `G6-TOOLS-CC`: the *keep reasoning off* option keeps `tools`. |
| L-OAI-6, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | legacy `functions` (`get_time`) @none | 200 | none. `finish_reason: function_call`, called `get_time` | The messages of `OAI-LEGACY-FUNCTIONS` and `G6-LEGACY-FUNCTIONS`. Both rules stay `unverified`: `functions` was not sent without `reasoning_effort`, and `function_call` was not sent at all. |
| L-OAI-5, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | `"stop": ["\n"]` @none | 400 | `Unsupported parameter: 'stop' is not supported with this model.` ([E4](#e4-stop-is-rejected)) | `OAI-STOP` (live: `gpt-5.6-terra`), `G6-STOP-LOGIT-BIAS` (live: `gpt-6-luna`) |
| L-OAI-5, L-OAI-8 | `gpt-5.6-terra`, `gpt-6-luna` | `"logit_bias": {"1000": -100}` @none | 400 | `Unsupported parameter: 'logit_bias' is not supported with this model.` ([E5](#e5-logit_bias-is-rejected)) | `OAI-LOGIT-BIAS` (live: `gpt-5.6-terra`), `G6-STOP-LOGIT-BIAS` (live: `gpt-6-luna`) |
| L-OAI-5 | `gpt-5.6-terra` | @none with `"presence_penalty": 0.5`, `"frequency_penalty": 0.5`, `"n": 2` or `"seed": 42` (one per request) | 200 | none | `OAI-PENALTIES-N-SEED` (live: `gpt-5.6-terra`) |
| L-OAI-8 | `gpt-6-luna` | @none with `"presence_penalty": 0.5` or `"frequency_penalty": 0.5` | 500, on 8 of 8 attempts | `The server had an error while processing your request. Sorry about that!` ([E7](#e7-presence_penalty-and-frequency_penalty-return-a-server-error)) | `G6-PENALTIES` (live: `gpt-6-luna`) |
| L-OAI-8 | `gpt-6-luna` | @none with `"n": 2` or `"seed": 42` | 200 | none | `G6-N-SEED` (live: `gpt-6-luna`) |
| L-OAI-8 | `gpt-6-luna` | `"prediction": {"type": "content", "content": "OK"}` @none | 400 | `Unsupported parameter: 'prediction' is not supported with this model.` ([E6](#e6-prediction-is-rejected)) | `G6-PREDICTED-OUTPUTS` (live: `gpt-6-luna`) |

## The messages, verbatim

Each message below is the `error.message` field of the response, copied from the record
without any change. The request bodies are the ones the record holds, minus the
`Authorization` header.

### E1: max_tokens is rejected

`POST /v1/chat/completions` returned **400**, `type: invalid_request_error`,
`code: unsupported_parameter`, `param: max_tokens`, on 4 of 4 calls: `gpt-5.6-terra` and
`gpt-6-luna`, each with and without `reasoning_effort`.

```json
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "Reply with the word OK."}], "max_tokens": 50}
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "Reply with the word OK."}], "max_tokens": 50, "reasoning_effort": "none"}
```

(and the same two bodies with `"model": "gpt-5.6-terra"`)

```text
Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.
```

The same request with `"max_completion_tokens": 50` in place of `max_tokens` returned 200
on both models. Supports `G6-MAX-TOKENS` (`max_tokens is rejected on {target}: use
max_completion_tokens (same value).`) and `OAI-MAX-TOKENS`, whose message since the
28 October 2026 data refresh says that OpenAI's API reference deprecates `max_tokens`
in favour of `max_completion_tokens` and that only gpt-5.6-terra was called.

### E2: a non-default temperature is rejected

**400**, `type: invalid_request_error`, `code: unsupported_value`, `param: temperature`,
on 4 of 4 calls, one per model: `gpt-5.6-terra`, `gpt-5.6-sol`, `gpt-5.6-luna` and
`gpt-6-luna`, at the default effort.

```json
{"model": "gpt-5.6-sol", "messages": [{"role": "user", "content": "Reply with the word OK."}], "temperature": 0.2, "max_completion_tokens": 200}
```

```text
Unsupported value: 'temperature' does not support 0.2 with this model. Only the default (1) value is supported.
```

With `"reasoning_effort": "none"` (and `"max_completion_tokens": 20`), the same
temperature returned 200 on all four models, and so did `"top_p": 0.5` and
`"logprobs": true, "top_logprobs": 2`. Supports `OAI-SAMPLING` and `G6-SAMPLING`, whose
message is `temperature, top_p and logprobs are accepted on {target} at reasoning_effort "none"; at its default effort a non-default temperature returns 400.`

### E3: function tools need reasoning_effort none

**400**, `type: invalid_request_error`, `code: null`, `param: reasoning_effort`, on 2 of 2
calls. The requests did not set `reasoning_effort`, so the model's default effort
applied.

```json
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "What time is it?"}], "tools": [{"type": "function", "function": {"name": "get_time", "description": "Get the time", "parameters": {"type": "object", "properties": {}}}}]}
```

`gpt-5.6-terra`:

```text
Function tools with reasoning_effort are not supported for gpt-5.6-terra in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.
```

`gpt-6-luna`:

```text
Function tools with reasoning_effort are not supported for gpt-6-luna in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.
```

With `"reasoning_effort": "none"` added, both models returned 200 and called `get_time`.
Supports `OAI-TOOLS-CC` and `G6-TOOLS-CC`, whose message is
`Chat Completions function tools work on {target} only at reasoning_effort "none" (400 otherwise).`

### E4: stop is rejected

**400**, `type: invalid_request_error`, `code: unsupported_parameter`, `param: stop`, on 2
of 2 calls (`gpt-5.6-terra`, `gpt-6-luna`), at `reasoning_effort` `"none"`.

```json
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "Reply with the word OK."}], "reasoning_effort": "none", "max_completion_tokens": 20, "stop": ["\n"]}
```

```text
Unsupported parameter: 'stop' is not supported with this model.
```

Supports `OAI-STOP` on gpt-5.6-terra only (since the 28 October 2026 data refresh its
message says gpt-5.6-sol, gpt-5.6-luna and the default effort were not tested) and
`G6-STOP-LOGIT-BIAS` (`stop and logit_bias are not supported on {target} (400).`).

### E5: logit_bias is rejected

**400**, `type: invalid_request_error`, `code: unsupported_parameter`, `param: logit_bias`,
on 2 of 2 calls (`gpt-5.6-terra`, `gpt-6-luna`), at `reasoning_effort` `"none"`.

```json
{"model": "gpt-5.6-terra", "messages": [{"role": "user", "content": "Reply with the word OK."}], "reasoning_effort": "none", "max_completion_tokens": 20, "logit_bias": {"1000": -100}}
```

```text
Unsupported parameter: 'logit_bias' is not supported with this model.
```

Supports `OAI-LOGIT-BIAS` on gpt-5.6-terra only (since the 28 October 2026 data refresh
its message says gpt-5.6-sol, gpt-5.6-luna and the default effort were not tested) and
`G6-STOP-LOGIT-BIAS`.

### E6: prediction is rejected

**400**, `type: invalid_request_error`, `code: unsupported_parameter`, `param: prediction`,
on 1 of 1 call (`gpt-6-luna` only).

```json
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "Reply with the word OK."}], "reasoning_effort": "none", "prediction": {"type": "content", "content": "OK"}, "max_completion_tokens": 20}
```

```text
Unsupported parameter: 'prediction' is not supported with this model.
```

Supports `G6-PREDICTED-OUTPUTS`, whose message is
`prediction (Predicted Outputs) is not supported on {target} (400).`

### E7: presence_penalty and frequency_penalty return a server error

**500**, `type: server_error`, `code: null`, `param: null`, on 8 of 8 attempts on
`gpt-6-luna`, 4 per parameter: one in the run and three in the retry that followed it.

```json
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "Reply with the word OK."}], "reasoning_effort": "none", "max_completion_tokens": 20, "presence_penalty": 0.5}
{"model": "gpt-6-luna", "messages": [{"role": "user", "content": "Reply with the word OK."}], "reasoning_effort": "none", "max_completion_tokens": 20, "frequency_penalty": 0.5}
```

```text
The server had an error while processing your request. Sorry about that!
```

These are the run's request bodies. The retry was a one-off command that recorded only the
attempt, the parameter, the status and the message, so its bodies are not on record;
[`retry_penalties.py`](#retry_penaltiespy) below re-sends the run's bodies. With
`"model": "gpt-5.6-terra"` the same two requests returned 200. A 500 is the server
failing, not a documented rejection, so Arbor reports these arguments as a check, not a
change. Supports `G6-PENALTIES`, whose message is
`presence_penalty and frequency_penalty failed on {target} in every test (server error).`

### E8: gpt-4-1106-preview does not exist

`GET /v1/models/gpt-4-1106-preview` returned **404**, `type: invalid_request_error`,
`code: model_not_found`, `param: model`.

```text
The model 'gpt-4-1106-preview' does not exist
```

The message has no closing period. OpenAI's deprecations page, read the same day, listed
`gpt-4-1106-preview` for shutdown on 23 October 2026. Supports the `gpt-4-1106-preview`
entry, which Arbor reports as already shut down, with the note
`OpenAI lists 23 Oct 2026, but the API already answers 404 (checked 18 Oct 2026).`
Only the Models API was asked: no Chat Completions request was sent to this model.

### E9: the four completions IDs are not callable

`GET /v1/models/{model}` returned **404**, `type: invalid_request_error`,
`code: model_not_found`, `param: model`. Each line is one whole message:

```text
The model 'gpt-3.5-turbo-completions' does not exist
The model 'gpt-4-0613-completions' does not exist
The model 'gpt-4-completions' does not exist
The model 'gpt-4-turbo-completions' does not exist
```

`POST /v1/completions` returned **404**, `type: invalid_request_error`,
`code: model_not_found`, `param: null`, for this body with each ID as the model:

```json
{"model": "gpt-4-completions", "prompt": "Reply with the word OK.", "max_tokens": 5}
```

Each line is one whole message (the IDs are in backticks in the message itself):

```text
The model `gpt-3.5-turbo-completions` does not exist or you do not have access to it.
The model `gpt-4-0613-completions` does not exist or you do not have access to it.
The model `gpt-4-completions` does not exist or you do not have access to it.
The model `gpt-4-turbo-completions` does not exist or you do not have access to it.
```

OpenAI's deprecations page lists these four IDs among the models it shuts down on
23 October 2026. Supports their entries, which Arbor reports but never rewrites
(`action: report_only`), with the note
`Not a callable model ID: the API answers 404 (checked 18 Oct 2026).`

## What these calls did not check

- **The Responses API.** No request went to `/v1/responses`. Every `live` label in the
  bundled data covers Chat Completions only (`live_endpoints`); for the Responses API the
  same rules read `docs` or `unverified`.
- **`gpt-5.6-sol` and `gpt-5.6-luna` beyond sampling.** They received only the
  temperature, `top_p` and logprobs requests (E2). `max_tokens`, `stop`, `logit_bias`, the
  penalties, `n`, `seed`, `tools`, `functions` and `prediction` were not sent to them, so
  where a rule for those arguments is `live`, the label covers `gpt-5.6-terra` (or, for
  the `G6-` rules, `gpt-6-luna`) only.
- **`prediction` on GPT-5.6.** Predicted Outputs was sent to `gpt-6-luna` only.
- **Arguments at the default effort.** On the new models, only temperature,
  `max_tokens`, `max_completion_tokens` and `tools` were sent without
  `reasoning_effort`. `top_p`, logprobs, `stop`, `logit_bias`, the penalties, `n`,
  `seed`, `functions` and `prediction` were sent at `"none"` only.
- **Other values and efforts.** One temperature (0.2) and one value per other argument.
  No effort other than `"none"` and the model's default. Whether `"none"` means no
  reasoning step at all, and how much the default effort reasons on real prompts, was not
  measured: these prompts were trivial.
- **`function_call`.** Legacy `functions` was sent at `"none"` only, and `function_call`
  never, so `OAI-LEGACY-FUNCTIONS` and `G6-LEGACY-FUNCTIONS` stay `unverified`.
- **The shutdowns themselves.** `gpt-3.5-turbo`, `gpt-4` and `gpt-4o-2024-05-13` were
  checked only for the `shutdown_date` the Models API gives, five days before that date.
  `gpt-4-1106-preview` and the `-completions` IDs got no Chat Completions request. No
  model was called after its listed date.
- **The SDKs.** Requests were plain HTTPS, not the `openai` package, so what old or new
  SDK versions accept or reject on the client side was not tested.
- **Other features.** Streaming, structured outputs, images, audio, pro mode, tokenizer
  names and fine-tuned model IDs were not tried.
- **Other providers and hosts.** No live call went to Anthropic, the Gemini API, Vertex
  AI, Azure OpenAI or any other host. Their rules read `docs` or `unverified`.
- **Repeatability and quality.** This is one run, from one account, on one day. Status
  codes, messages and model access can differ between accounts and can change at any
  time, and the 500s may have been temporary. Nothing here says anything about answer
  quality.

## Reproduce the checks

The scripts need Python 3 and nothing else. The runner sends 66 short requests and the
retry 6, and your OpenAI account is billed for them.

1. Save [`run_openai.py`](#run_openaipy) and, if you want the retry,
   [`retry_penalties.py`](#retry_penaltiespy), from below.
2. Put your key in a file outside any repository, as one line: `OPENAI_API_KEY=`
   followed by your key. The scripts read that file in memory, send the key only in the
   `Authorization` header to `api.openai.com`, and never print or write it.
3. Run:

   ```bash
   python -I run_openai.py path/to/openai.env results.json
   python -I retry_penalties.py path/to/openai.env retry.json
   ```

   Each prints one line per call, with its status and the start of the message, and
   writes the record as JSON.
4. Compare each status and message with the tables above. The models and the messages
   may have changed since 18 October 2026.
5. Before you share a record, remove what can identify your account or a request: each
   success's `id` (`chatcmpl-...`) and `created`, and the `at` timestamps. Check every
   `message` for a request or organization ID too; none of the messages above contained
   one.

If your results differ, an issue with the redacted record is welcome.

### run_openai.py

The runner as it was used on 18 October 2026, unchanged.

<details>
<summary>run_openai.py</summary>

```python
"""Run the OpenAI live checks from recipes/LIVE_CHECKS.txt (L-OAI-1..6, 8).

Reads OPENAI_API_KEY from the given .env file in memory only. Never prints the key
or full response bodies: each call records status, error type/message and the few
fields the check names. Calls api.openai.com only.
"""
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

ENV_PATH, OUT_PATH = sys.argv[1], sys.argv[2]
BASE = "https://api.openai.com/v1"


def load_key(path):
    for raw in open(path, encoding="utf-8"):
        line = raw.strip()
        if line.startswith("OPENAI_API_KEY="):
            return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise SystemExit("OPENAI_API_KEY not found")


KEY = load_key(ENV_PATH)
OK_MSG = [{"role": "user", "content": "Reply with the word OK."}]
results = []


def call(check, label, method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method)
    req.add_header("Authorization", "Bearer " + KEY)
    data = None
    if body is not None:
        req.add_header("Content-Type", "application/json")
        data = json.dumps(body).encode()
    rec = {"check": check, "label": label, "method": method, "path": path,
           "request": body, "at": datetime.now(timezone.utc).isoformat()}
    try:
        with urllib.request.urlopen(req, data=data, timeout=120) as resp:
            payload = json.loads(resp.read() or b"{}")
            rec["status"] = resp.status
    except urllib.error.HTTPError as e:
        rec["status"] = e.code
        try:
            payload = json.loads(e.read() or b"{}")
        except ValueError:
            payload = {}
    except Exception as e:  # network errors: record the type only
        rec["status"] = None
        rec["transport_error"] = type(e).__name__
        payload = {}
    err = payload.get("error") or {}
    if err:
        rec["error"] = {k: err.get(k) for k in ("type", "code", "param", "message")}
    for k in ("id", "model", "shutdown_date", "owned_by", "created"):
        if k in payload:
            rec[k] = payload[k]
    ch = (payload.get("choices") or [{}])[0]
    if ch:
        msg = ch.get("message") or {}
        rec["finish_reason"] = ch.get("finish_reason")
        if "content" in msg:
            rec["content"] = (msg.get("content") or "")[:80]
        if msg.get("tool_calls"):
            rec["tool_calls"] = [t.get("function", {}).get("name") for t in msg["tool_calls"]]
        if msg.get("function_call"):
            rec["function_call"] = msg["function_call"].get("name")
        if "text" in ch:
            rec["content"] = (ch.get("text") or "")[:80]
        if ch.get("logprobs") is not None:
            rec["logprobs_returned"] = True
    if "usage" in payload:
        u = payload["usage"]
        rec["usage"] = {"completion_tokens": u.get("completion_tokens"),
                        "reasoning_tokens": (u.get("completion_tokens_details") or {}).get("reasoning_tokens")}
    if "output" in payload:
        rec["output_types"] = [o.get("type") for o in payload["output"]]
    results.append(rec)
    short = rec.get("error", {}).get("message") or rec.get("content") or rec.get("model") or ""
    print(f"{check:9} {label:42} {rec.get('status')}  {str(short)[:110]}")
    time.sleep(0.4)


# L-OAI-1: aliases and snapshots
for m in ["gpt-4o", "gpt-4o-mini", "gpt-4.1", "gpt-4.1-mini", "gpt-4o-2024-08-06", "gpt-4o-2024-11-20",
          "gpt-4o-mini-2024-07-18", "gpt-4.1-2025-04-14", "gpt-4.1-mini-2025-04-14",
          "gpt-4-1106-preview", "gpt-4o-2024-05-13", "gpt-3.5-turbo", "gpt-4"]:
    call("L-OAI-1", f"GET {m}", "GET", f"/models/{m}")
for m in ["gpt-4o", "gpt-4o-mini", "gpt-4.1", "gpt-4.1-mini"]:
    call("L-OAI-1", f"served model for {m}", "POST", "/chat/completions",
         {"model": m, "messages": OK_MSG, "max_tokens": 5})

# L-OAI-2: -completions IDs
for m in ["gpt-3.5-turbo-completions", "gpt-4-0613-completions", "gpt-4-completions", "gpt-4-turbo-completions"]:
    call("L-OAI-2", f"GET {m}", "GET", f"/models/{m}")
    call("L-OAI-2", f"v1/completions {m}", "POST", "/completions",
         {"model": m, "prompt": "Reply with the word OK.", "max_tokens": 5})


def sampling(check, M):
    call(check, f"{M} temp default-effort", "POST", "/chat/completions",
         {"model": M, "messages": OK_MSG, "temperature": 0.2, "max_completion_tokens": 200})
    call(check, f"{M} temp @none", "POST", "/chat/completions",
         {"model": M, "messages": OK_MSG, "reasoning_effort": "none", "temperature": 0.2, "max_completion_tokens": 20})
    call(check, f"{M} top_p @none", "POST", "/chat/completions",
         {"model": M, "messages": OK_MSG, "reasoning_effort": "none", "top_p": 0.5, "max_completion_tokens": 20})
    call(check, f"{M} logprobs @none", "POST", "/chat/completions",
         {"model": M, "messages": OK_MSG, "reasoning_effort": "none", "logprobs": True, "top_logprobs": 2,
          "max_completion_tokens": 20})


def max_tokens(check, M):
    call(check, f"{M} max_tokens", "POST", "/chat/completions", {"model": M, "messages": OK_MSG, "max_tokens": 50})
    call(check, f"{M} max_tokens @none", "POST", "/chat/completions",
         {"model": M, "messages": OK_MSG, "max_tokens": 50, "reasoning_effort": "none"})
    call(check, f"{M} max_completion_tokens", "POST", "/chat/completions",
         {"model": M, "messages": OK_MSG, "max_completion_tokens": 50})


def legacy(check, M):
    for name, extra in [("stop", {"stop": ["\n"]}), ("presence_penalty", {"presence_penalty": 0.5}),
                        ("frequency_penalty", {"frequency_penalty": 0.5}), ("logit_bias", {"logit_bias": {"1000": -100}}),
                        ("n=2", {"n": 2}), ("seed", {"seed": 42})]:
        body = {"model": M, "messages": OK_MSG, "reasoning_effort": "none", "max_completion_tokens": 20}
        body.update(extra)
        call(check, f"{M} {name} @none", "POST", "/chat/completions", body)


def tools(check, M):
    t = [{"type": "function", "function": {"name": "get_time", "description": "Get the time",
                                            "parameters": {"type": "object", "properties": {}}}}]
    q = [{"role": "user", "content": "What time is it?"}]
    call(check, f"{M} tools no-effort", "POST", "/chat/completions", {"model": M, "messages": q, "tools": t})
    call(check, f"{M} tools @none", "POST", "/chat/completions",
         {"model": M, "messages": q, "tools": t, "reasoning_effort": "none"})
    call(check, f"{M} legacy functions @none", "POST", "/chat/completions",
         {"model": M, "messages": q, "reasoning_effort": "none",
          "functions": [{"name": "get_time", "parameters": {"type": "object", "properties": {}}}]})


# L-OAI-3..6 on GPT-5.6
for M in ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.6-luna"]:
    sampling("L-OAI-3", M)
max_tokens("L-OAI-4", "gpt-5.6-terra")
legacy("L-OAI-5", "gpt-5.6-terra")
tools("L-OAI-6", "gpt-5.6-terra")

# L-OAI-8: repeat on gpt-6-luna + predicted outputs
sampling("L-OAI-8", "gpt-6-luna")
max_tokens("L-OAI-8", "gpt-6-luna")
legacy("L-OAI-8", "gpt-6-luna")
tools("L-OAI-8", "gpt-6-luna")
call("L-OAI-8", "gpt-6-luna prediction @none", "POST", "/chat/completions",
     {"model": "gpt-6-luna", "messages": OK_MSG, "reasoning_effort": "none",
      "prediction": {"type": "content", "content": "OK"}, "max_completion_tokens": 20})

json.dump({"run_at": datetime.now(timezone.utc).isoformat(), "endpoint": BASE, "results": results},
          open(OUT_PATH, "w", encoding="utf-8"), indent=1)
print(f"\nwrote {len(results)} results to {OUT_PATH}")
```

</details>

### retry_penalties.py

A re-runnable version of the retry (E7), written after the run: the original retry was a
one-off command that was not saved. Unlike it, this script records each request and a UTC
timestamp per attempt.

<details>
<summary>retry_penalties.py</summary>

```python
"""Re-run the retry of the gpt-6-luna penalty calls (recipes/LIVE_CHECKS.txt L-OAI-8).

A reconstruction, written on 2026-10-18 after the review: the retry behind
openai-2026-10-18-retry.json was a one-off inline command that was not saved. This sends the
run's own L-OAI-8 request (run_openai.py, legacy(): gpt-6-luna, reasoning_effort none,
max_completion_tokens 20, the parameter at 0.5) ATTEMPTS times per parameter. Unlike the
original, each attempt records the request and a UTC timestamp.

Run:  python -I retry_penalties.py <.env path> <output .json>

Reads OPENAI_API_KEY from the .env file in memory only and never prints or writes it. Records
status and the error message only (no response bodies). Calls api.openai.com only.
"""
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

ENV_PATH, OUT_PATH = sys.argv[1], sys.argv[2]
URL = "https://api.openai.com/v1/chat/completions"
MODEL = "gpt-6-luna"
ATTEMPTS = 3
OK_MSG = [{"role": "user", "content": "Reply with the word OK."}]


def load_key(path):
    for raw in open(path, encoding="utf-8"):
        line = raw.strip()
        if line.startswith("OPENAI_API_KEY="):
            return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise SystemExit("OPENAI_API_KEY not found")


def main():
    key = load_key(ENV_PATH)
    results = []
    for attempt in range(ATTEMPTS):
        for param in ("presence_penalty", "frequency_penalty"):
            body = {"model": MODEL, "messages": OK_MSG, "reasoning_effort": "none",
                    "max_completion_tokens": 20, param: 0.5}
            req = urllib.request.Request(URL, data=json.dumps(body).encode(), method="POST")
            req.add_header("Authorization", "Bearer " + key)
            req.add_header("Content-Type", "application/json")
            rec = {"check": "L-OAI-8", "attempt": attempt, "param": param, "request": body,
                   "at": datetime.now(timezone.utc).isoformat()}
            try:
                with urllib.request.urlopen(req, timeout=120) as resp:
                    rec["status"] = resp.status
            except urllib.error.HTTPError as e:
                rec["status"] = e.code
                try:
                    rec["message"] = (json.loads(e.read() or b"{}").get("error") or {}).get("message")
                except ValueError:
                    pass
            except Exception as e:  # network errors: record the type only
                rec["status"] = None
                rec["transport_error"] = type(e).__name__
            results.append(rec)
            print(f"attempt {attempt} {param:18} {rec.get('status')}  {str(rec.get('message') or '')[:100]}")
            time.sleep(1)
    with open(OUT_PATH, "w", encoding="utf-8") as f:
        json.dump({"run_at": datetime.now(timezone.utc).isoformat(), "endpoint": URL, "results": results}, f, indent=1)
    print(f"\nwrote {len(results)} results to {OUT_PATH}")


if __name__ == "__main__":
    main()
```

</details>
