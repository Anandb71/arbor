# Find code that calls retiring AI models

AI providers retire models on fixed dates, and a request to a retired model fails.
OpenAI shuts down `gpt-4o`, `gpt-4o-mini`, `gpt-4.1` and `gpt-4.1-mini` on
**14 January 2027**. `arbor retirements` finds every call in your code that uses a
model its provider has retired or is retiring. For each call it shows the replacement
the provider lists and the other arguments that have to change with it.

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
                  also change: add thinking={"type": "between_tools"}
  note: A request that ends with an assistant turn (prefill) returns 400 on claude-sonnet-5-5.
  ...

3 calls use models OpenAI shuts down on 14 Jan 2027 (88 days)
  app/chat.py:12  gpt-4o (MODEL, line 8) → gpt-6-luna
                  also change: max_tokens → max_completion_tokens, add reasoning_effort="none" (for temperature, max_tokens)
  app/chat.py:21  gpt-4o-mini (line 22) → gpt-6-luna  fallback of an environment or config lookup: confirm the deployed value
                  check: stop and logit_bias are not supported on gpt-6-luna (400).
  app/chat.py:29  gpt-4.1 → gpt-6-luna
                  also change: add reasoning_effort="none" (for tools)
  note: gpt-6-luna is OpenAI's most efficient tier; GPT-4o and GPT-4.1 were flagship models, so answer quality may change.
  ...

Not checked: 1 call takes the model from somewhere Arbor cannot read (an environment variable, a function argument, another module); arbor usages lists it.
Not listed: 1 call reaches these models through Azure, a custom base_url or another cloud, where the provider's dates may not apply. Add --include-routed to list it.
Summary: 4 calls in 1 file use retired or retiring models: 1 already retired, 3 retiring (next: 14 Jan 2027, 88 days).
3 can move to the listed replacement with the changes shown; 1 needs a manual check.
Failing (exit 1): 1 call uses a model that is already retired.
```

(Run on 18 October 2026. Days are counted from the day you run it.)

For each affected call it reports:

| | |
|---|---|
| **Where** | `file:line`, and the line of the string to edit when the model comes from a constant or a default |
| **When** | Already retired, or the shutdown date and the days left. Google lists its Gemini dates as the earliest possible, so those read "on or after". |
| **Replacement** | The model the provider's deprecation page lists. Arbor does not pick a "better" one. When the provider lists several models, or none, the report says so. |
| **Also change** | Arguments that must change with the replacement, worked out from the arguments the call passes. For example, `max_tokens` becomes `max_completion_tokens` and `reasoning_effort="none"` is added when the call sets `temperature` or `tools`. |
| **Check** | What needs a person: parameters the replacement rejects (`stop`, `logit_bias`), fine-tuned models, embedding models (new vectors), several listed replacements. |
| **Confidence** | *Confirmed*: a literal or a constant in the same file. *Confirm first*: the fallback of `os.getenv(...)`, a parameter default, or one of the values a dict lookup, a branch or an in-file caller may give; the value at run time may differ. *Gap*: an SDK-shaped call whose client Arbor could not resolve (built by a factory, passed in). |
| **Routing** | A provider's dates apply to its own API. Calls through Azure OpenAI, a custom `base_url` (OpenRouter, LiteLLM, a proxy), Bedrock or Vertex are counted separately; `--include-routed` lists them. |

## Options and exit codes

| Flag | |
|---|---|
| `--json` | One JSON object on stdout, errors included (`{"version", "error"}`) |
| `--markdown` | A Markdown report, for a CI job summary |
| `--annotations` | GitHub Actions annotations, one warning per affected call |
| `--fail-within <days>` | Also fail when a model retires within this many days |
| `--include-routed` | List calls through Azure, a custom `base_url` or another cloud, and let them fail the check |
| `--quiet` | Only the calls and the summary |

| Exit code | Meaning |
|---|---|
| `0` | Nothing failing. Calls to models that retire later are still listed. |
| `1` | A call uses a model that is already retired, or, with `--fail-within N`, one that retires within N days |
| `2` | The check could not run (a missing path, a bad flag) |

## In CI: GitHub Action

```yaml
name: Model retirements
on: [pull_request]

jobs:
  retirements:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: Anandb71/arbor@vX.Y.Z   # the first release that includes arbor retirements
        with:
          mode: retirements
          # retirements-fail-on: retired   # or a number of days, e.g. 90
```

The job summary gets the Markdown report and each affected line gets a warning
annotation. The Action never fails the build unless you set `retirements-fail-on`
to `retired` or to a number of days. `retirements-path` picks a subdirectory, and
`retirements-include-routed: "true"` adds the routed calls. The check runs on the
runner and sends nothing anywhere.

## Scope today

- **Python only.** `.py` files and Python scripts. TypeScript and JavaScript are coming.
- **Providers:** OpenAI, Anthropic, the Gemini API and Gemini on Google Cloud (Vertex AI):
  283 model IDs from their deprecation pages, plus aliases and changelog entries those
  pages leave out, such as the four OpenAI aliases above.
- **SDK calls only.** It reads the `openai`, `anthropic`, `google-genai` and
  `google-generativeai` SDKs. Framework wrappers (LangChain, LlamaIndex, LiteLLM's own
  API) are not tracked yet. Model IDs in `.env`, YAML, JSON or notebooks are not read.
- **Values it can see.** A model passed in from an environment variable, a function
  argument or another module is counted under "Not checked"; `arbor usages` lists those
  calls. So is a call that passes no model, where the SDK's or the server's default applies. Argument checks look at the argument *names* a call passes, not their values,
  and not at parameters nested in config objects such as Gemini's
  `GenerateContentConfig`.
- **Exact matches.** A value matches only when the whole string equals a listed ID.
  Arbor never matches substrings and never changes case. Two exceptions are reported as
  such: Gemini's `models/` prefix, and OpenAI fine-tunes (`ft:<base>:...`), which follow
  the schedule of their base model.

## Where the data comes from

The model IDs, dates and replacements come from the providers' own deprecation pages,
and every one was checked against a saved copy of the page. The data was last read on
**18 October 2026**, and each report prints that date. The argument rules come from
the providers' migration guides and API references. Rules marked `live` in the JSON were
confirmed with real API calls on 18 October 2026. Those calls used Chat Completions; the
Responses API was not tested. Rules marked `unverified` are reported as checks, not
changes.

The data is bundled into Arbor at build time, so it changes only when you update Arbor.
When the data is more than 90 days old, the report reminds you to update. It is built by
[`scripts/retirements/generate_model_retirements.py`](../scripts/retirements/generate_model_retirements.py)
from Arbor's retirement recipes. The generator keeps provenance with the data: the
pages, the recipe versions and the date. A test fails `cargo test` if the file is edited
by hand.

## Privacy

`arbor retirements` reads source files on your machine and prints a report. It makes no
network requests and has no telemetry. Model values are captured only for model
arguments, and credential-shaped strings are never captured or printed.

The human report ends with one line offering automated fix pull requests. That line is
never printed with `--json`, `--markdown`, `--annotations`, `--quiet`, or when `CI` is set.
