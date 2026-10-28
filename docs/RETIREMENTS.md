# Find code that calls retiring AI models

AI providers retire models on fixed dates, and a request to a retired model fails.
OpenAI's deprecations page lists five dated snapshots for shutdown on
**14 January 2027**: `gpt-4o-2024-08-06`, `gpt-4o-2024-11-20`, `gpt-4o-mini-2024-07-18`,
`gpt-4.1-2025-04-14` and `gpt-4.1-mini-2025-04-14`. The page gives the bare `gpt-4o`,
`gpt-4o-mini`, `gpt-4.1` and `gpt-4.1-mini` no row, but OpenAI's
[API reference](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
lists each of them as "Deprecated. Announced shutdown date: 2027-01-14", and OpenAI's
models API returned that date for each when we checked on 18 October 2026.
`arbor retirements` finds the direct `openai`, `anthropic` and
Google SDK calls in your code that use a model its provider has retired or is
retiring. For each call it shows the replacement the provider lists, the other
arguments that have to change with it, and the choices your repository has to make
first.

```bash
npx @anandb71/arbor-cli retirements .
```

or, with Arbor installed, `arbor retirements .`. It needs no account and no API key,
and it runs locally: no code leaves your machine.

## What it prints

```text
$ arbor retirements .
Model retirements in . (1 Python file checked)

1 call uses a model Anthropic shut down on 28 Oct 2025
  app/chat.py:33  claude-3-5-sonnet-20241022 → claude-sonnet-5-5
                  decide: think only between tools (add thinking={"type": "between_tools"}) or let the model think (no change)
  decide once for this repo (Arbor does not choose): claude-sonnet-5-5 thinks by default (adaptive thinking) when the call sends no thinking; the model it replaces answered without thinking.
    ...
  note: A request that ends with an assistant turn (prefill) returns 400 on claude-sonnet-5-5.
  ...

3 calls use models OpenAI shuts down on 14 Jan 2027 (88 days)
  app/chat.py:12  gpt-4o (MODEL, line 8) → gpt-6-luna
                  also change: max_tokens → max_completion_tokens
                  decide: keep reasoning off (add reasoning_effort="none"; keep temperature) or let the model reason (remove temperature)
  app/chat.py:21  gpt-4o-mini (line 22) → gpt-6-luna  fallback of an environment or config lookup: confirm the deployed value
                  decide: keep reasoning off (add reasoning_effort="none") or let the model reason (no change)
                  check: stop and logit_bias are not supported on gpt-6-luna (400).
  app/chat.py:29  gpt-4.1 → gpt-6-luna
                  decide: keep reasoning off (add reasoning_effort="none"; keep tools) or let the model reason (remove tools)
  decide once for this repo (Arbor does not choose): gpt-6-luna runs at medium reasoning effort by default, so it can reason; the model it replaces answered without a reasoning step.
    keep reasoning off: Closest to the old non-reasoning model: the call keeps answering without a reasoning step. (not verified)
                        ...
    let the model reason: The model can reason (default effort medium). When it does, the call is slower, reasoning tokens are billed as output tokens, max_completion_tokens also has to cover them, and answers may change. How much it reasons on real prompts is not measured yet.
                          The model can reason (default effort medium), but the request no longer offers its tools, so the app's tool calls stop. ...
  note: If gpt-6-luna reasons, max_completion_tokens also bounds its reasoning tokens, so a small limit can be used up before any visible text.
  ...

Not checked: 1 call takes the model from somewhere Arbor cannot read (an environment variable, a function argument, another module); arbor usages lists it.
Not listed: 1 call reaches these models through Azure, another host in base_url or another cloud, where the provider's dates may not apply. Add --include-routed to list it.
Summary: 4 calls in 1 file use retired or retiring models: 1 already retired, 3 retiring (next: 14 Jan 2027, 88 days).
3 can move once this repo makes the choice shown under decide; 1 needs a manual check.
Failing (exit 1): 1 call uses a model that is already retired.
```

(Run on 18 October 2026. Days are counted from the day you run it, in UTC.)

For each affected call it reports:

| | |
|---|---|
| **Where** | `file:line`, and the line of the string to edit when the model comes from a constant or a default |
| **When** | Already retired, or the shutdown date and the days left. Google lists its Gemini dates as the earliest possible, so those read "on or after". |
| **Replacement** | The model the provider's deprecation page lists. Arbor does not pick a "better" one. When the listed model is itself retired or retiring, Arbor follows what the provider lists for it in turn and shows the first one that still works: `gpt-4-vision-preview → gpt-6-luna (via gpt-4o)`. Such a call always needs a manual check, and `--json` has the whole chain. When the provider lists several models, or none, the report says so. |
| **Also change** | Arguments that must change with the replacement, worked out from the arguments the call passes: for example `max_tokens` becomes `max_completion_tokens` on gpt-6-luna. Only changes the provider documents as required are listed here. |
| **Decide** | Choices your repository makes once, which Arbor never makes for you. Today that is the reasoning effort on OpenAI's new models: gpt-6-luna and GPT-5.6 run at medium reasoning effort by default, so they can reason, while gpt-4o and gpt-4.1 did not. You can keep reasoning off (`reasoning_effort="none"`; parameters such as `temperature` and `tools` keep working) or let the model reason (when it does, the call is slower and reasoning tokens are billed; those parameters have to go). The same goes for Claude's thinking on claude-sonnet-5-5, which thinks by default where the Sonnet models it replaces did not: you can send `thinking={"type": "between_tools"}` (no thinking before the answer) or let the model think (slower and costlier when it does, and a response can begin with thinking blocks). The report shows both options with their effect and how well each effect is established. A call with a decision moves only once the repository has chosen: its *also change* edits, such as the `max_tokens` rename, go in together with the chosen option. |
| **Check** | What needs a person: parameters the replacement rejects (`stop`, `logit_bias`), fine-tuned models, embedding models (new vectors), several listed replacements, a model set in a request parameter object such as a tool config, and a model nested in a call's arguments, such as a Message Batch's `requests[].params` or a tool's `model` in `tools=[...]`. The call's own argument checks do not apply to a nested model: only the checks about the model itself do, and the report says to check the parameters next to it by hand. |
| **Confidence** | *Confirmed*: a literal or a constant in the same file, at a call where the usage scan (`arbor usages`) left no gap for it. *Confirm first*: the fallback of `os.getenv(...)`, a parameter default, one of the values a dict lookup, a branch or an in-file caller may give, or any value the usage scan put a gap on: a class attribute a subclass in another module may override, a name something in the file may change at run time (`exec`, `globals()`, `setattr`...), a call that passes more nested model values than the scan reads. The value at run time may differ, and the report says why. *Gap*: an SDK-shaped call whose client Arbor could not resolve (built by a factory, passed in). |
| **Routing** | A provider's dates apply to its own API. A client whose `base_url` is the provider's own host (`https://api.openai.com/v1`) or `None` is a direct call. When the usage scan cannot confirm where a call goes, it is listed with *routing not confirmed* and the reason: a `base_url` read from the environment (even one that falls back to the provider's host, since the deployment may set the variable) or from code Arbor cannot read, a client assigned more than one way or passed in by callers in other modules, a file that sets `OPENAI_BASE_URL`, module-level configuration outside the file's setup block, a Gemini client given `http_options` or `vertexai=True`, or a file that may rebind it at run time. A Gemini client follows Google Cloud's (Vertex AI) schedule only when it is built with `vertexai=True`; with `vertexai=False`, `None` or no flag it follows the Gemini API's. When `vertexai=` is decided at run time, Arbor looks the model up on Vertex AI first, then on the Gemini API, and the check says which schedule it used. Calls through Azure OpenAI, another host in `base_url` (OpenRouter, LiteLLM, a proxy), Bedrock or Vertex for Claude are counted separately; `--include-routed` lists them. |

Calls through LangChain, LiteLLM's own API, LlamaIndex or other wrappers are not
checked yet, so a clean result says "No direct openai/anthropic/google SDK calls"
and nothing more.

Fine-tuned models (`ft:<base>:...`) follow their base model's schedule. Arbor names a
base model to retrain on only when the provider lists one that supports fine-tuning;
today none does (gpt-6-luna and the GPT-5.6 models do not support fine-tuning), so the
report says that no fine-tunable replacement is listed. A fine-tuning job on a retiring
base (`client.fine_tuning.jobs.create(model="gpt-4o-mini-2024-07-18", ...)`) gets the
same advice, never the base's listed replacement, and always needs a manual check.

Objects built from SDK types in code that sends nothing, such as
`ChatCompletion(model=...)` in a test fixture, Anthropic's `Message(...)`, or
`.model_validate(...)`, are skipped and counted on one "Skipped" line. Tool configs
and other request parameter objects (`ImageGeneration(model=...)`, and types named
`...Param`, `...Params`, `...Config` or `...Request` such as
`RealtimeSessionCreateRequest(model=...)`) are checked, since their model is sent with
a request. Where it goes is decided by the request that sends the object, which Arbor
does not follow, so these are listed with *routing not confirmed* and count toward
exit 1 only with `--count-unconfirmed`.

## Options and exit codes

| Flag | |
|---|---|
| `--json` | One JSON object on stdout, errors included (`{"version", "error"}`) |
| `--markdown` | A Markdown report, for a CI job summary |
| `--annotations` | GitHub Actions annotations: one warning per affected call, the calls that fail the check first and then the most urgent, up to `--max-annotations` (default 10, GitHub's limit per step), then one notice saying how many more there are |
| `--markdown-out <file>` | Also write the Markdown report to a file, from the same scan |
| `--fail-within <days>` | Also fail when a model retires within this many days |
| `--count-unconfirmed` | Also let unconfirmed calls fail the check (see below) |
| `--include-routed` | List calls through Azure, another host in `base_url` or another cloud, and let them fail the check |
| `--quiet` | Only the calls and the summary |

| Exit code | Meaning |
|---|---|
| `0` | Nothing failing. Calls to models that retire later, and unconfirmed calls, are still listed. |
| `1` | A confirmed call uses a model that is already retired, or, with `--fail-within N`, one that retires within N days |
| `2` | The check could not run (a missing path, a bad flag) |

By default only **confirmed** values at **direct** calls count toward exit 1. The
others are listed, and a "Not counted" line says how many there are:
environment or config fallbacks, parameter defaults, values a lookup, branch or caller
may give, calls whose client was not resolved, and calls whose routing is unknown.
`--count-unconfirmed` counts them too; `--include-routed` adds the routed calls.

A shutdown date counts in UTC, and **on the shutdown date itself the call counts as
retired**: the provider may switch the model off at any time that day. So a `gpt-4o`
call fails the check from 14 January 2027 (UTC) on, and reads "OpenAI shuts down
today" that day. Google's "earliest" dates are a possibility, not a
shutdown: from that day on the call, or a replacement in a chain, is listed as "may
already be retired" and does not fail.

## In CI: GitHub Action

```yaml
name: Model retirements
on: [pull_request]

jobs:
  retirements:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: Anandb71/arbor@v3.1.0
        with:
          mode: retirements
          # retirements-fail-on: retired   # or a number of days, e.g. 90
```

The Action scans once. The job summary gets the Markdown report, which lists every
call. Affected lines get warning annotations: GitHub shows at most 10 warnings per
step, so Arbor writes 10, the calls that fail the check first and then the most
urgent, and one notice saying how many more the summary lists. The Action fails the build only if you set `retirements-fail-on` to
`retired` or to a number of days, with one exception: when the check cannot run
(exit 2: a missing path, a bad `retirements-fail-on` value), the step fails even with
`retirements-fail-on: never`, so a missing report never passes quietly. A `mode` other
than `command` or `retirements` (a typo such as `retirement`) fails the step before
anything is installed.
`retirements-path` picks a subdirectory, `retirements-include-routed: "true"` adds the
routed calls, and `retirements-count-unconfirmed: "true"` lets unconfirmed calls fail
it too. The check runs on the runner and sends nothing anywhere.

## Scope today

- **Python only.** `.py` files and Python scripts. TypeScript and JavaScript are coming.
- **Providers:** OpenAI, Anthropic, the Gemini API and Gemini on Google Cloud (Vertex AI):
  308 entries (297 model IDs; 11 listed by both the Gemini API and Vertex; aliases
  included) from their deprecation pages, with the aliases and changelog entries those
  pages leave out, such as the four OpenAI aliases above.
- **Direct SDK calls only.** It reads the `openai`, `anthropic`, `google-genai` and
  `google-generativeai` SDKs. Framework wrappers (LangChain, LlamaIndex, LiteLLM's own
  API) are not checked yet. Model IDs in `.env`, YAML, JSON or notebooks are not read.
- **Values it can see.** A model passed in from an environment variable, a function
  argument or another module is counted under "Not checked"; `arbor usages` lists those
  calls. So is a call that passes no model, where the SDK's or the server's default
  applies. Argument checks look at the argument *names* a call passes, not their values,
  and not at parameters nested in config objects such as Gemini's
  `GenerateContentConfig`.
- **What it did not read.** Every part of the scan that could hide a call gets its own
  "Not checked" line, so a clean result never claims more than was read: parts of files
  the usage scan did not analyze (scopes nested more than 64 deep, more records than it
  reads in one file), SDK calls through a client Arbor could not resolve (built by a
  factory or passed in) whose model it could not read either, SDK methods reached
  through `getattr` or passed as values, files
  with syntax errors (calls inside a broken region may be missing), files over the size
  limit or not readable as UTF-8, and calls in hidden directories such as `.github/`.
  Vendored and virtual-environment directories are not project code and are skipped.
- **Exact matches.** A value matches only when the whole string equals a listed ID.
  Arbor never matches substrings and never changes case. Two exceptions are reported as
  such: Gemini's `models/` prefix, and OpenAI fine-tunes (`ft:<base>:...`), which follow
  the schedule of their base model.

## Where the data comes from

The model IDs, dates and replacements come from the providers' own deprecation pages,
and every one was checked against a saved copy of the page. Each report prints the day
each provider's pages were read (in the bundled data: 28 October 2026 for every
provider, when every page was read and checked again), and the data date in `--json`
is the oldest of them. Between 23 and 26 October, OpenAI's page started listing bare
IDs and dated snapshots together in its rows, which added 15 IDs: seven bare IDs, among
them `gpt-5` and `o3` (11 December 2026), and eight dated snapshots, such as
`gpt-5.1-2025-11-13` (1 April 2027). The argument rules come from the
providers' migration guides and API references. A rule marked `live` in the JSON was confirmed with real API calls on
18 October 2026; the label is shown only for the models those calls used and only on
Chat Completions, the one API they called. [Live checks, 18 October 2026](LIVE_CHECKS_2026-10-18.md)
lists each of those calls with its request, status and error message, word for word,
and what they did not check. Elsewhere the same rule reads `docs` (a
provider page states it) or `unverified`. Rules marked `unverified` are reported as
checks, not changes, and a decision option whose effect is not verified says so.

The data is bundled into Arbor at build time, so it changes only when you update Arbor.
When the data is more than 90 days old, the report reminds you to update. It is built by
[`scripts/retirements/generate_model_retirements.py`](../scripts/retirements/generate_model_retirements.py)
from Arbor's retirement recipes, which live in a private repository. The
generator keeps provenance with the data: the provider pages, each recipe's version,
the date its pages were read and the SHA-256 of the recipe file. Every fact names the
provider page it came from, so you can check it there. A test fails `cargo test` if
any part of the file except its write date is edited by hand, and releases regenerate
it (see [RELEASING.md](RELEASING.md)).

## Privacy

`arbor retirements` reads source files on your machine and prints a report. It makes no
network requests and has no telemetry. Model values are captured only for model
arguments, and credential-shaped strings are never captured or printed. For a client
given a `base_url`, Arbor reads that argument to tell the provider's own host from a
proxy; it reports only that class, never the URL.

The human report ends with one line offering automated fix pull requests. That line is
never printed with `--json`, `--markdown`, `--annotations`, `--quiet`, or when `CI` is set.
Its link (`https://getarbor.dev/retirements?utm_source=arbor-cli`) carries only that
`utm_source` tag, so a fix-PR signup from the page can be credited to the CLI; nothing
about your code or the run is in it, and Arbor opens nothing itself.
