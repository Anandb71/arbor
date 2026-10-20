# Arbor v3.1.0

A feature release with two new commands for Python code that calls AI model
APIs: `arbor retirements` and `arbor usages`. No breaking changes. Graph
extraction and the cache are unchanged, so saved graphs are not rebuilt.

## `arbor retirements`: find code that calls retiring AI models

Providers shut models down on fixed dates, and a request to a shut-down model
fails. `arbor retirements` lists the direct `openai`, `anthropic` and Google SDK
calls in a Python project that use a model its provider has retired or is
retiring. For each call it shows the date, the replacement the provider lists,
the arguments that have to change with it and the choices the repository has to
make first.

```bash
npx @anandb71/arbor-cli retirements .
```

Two dates from the bundled data:

- **23 October 2026:** OpenAI shuts down `gpt-4`, `gpt-4-turbo`,
  `gpt-3.5-turbo`, `gpt-4.1-nano`, `gpt-4o-2024-05-13`, `gpt-image-1`, `o1`,
  `o1-pro`, `o3-mini` and `o4-mini`, with their dated snapshots. For the text
  models OpenAI lists a GPT-5.6 model (`gpt-4` → `gpt-5.6-sol`); for
  `gpt-image-1` it lists two image models.
- **14 January 2027:** OpenAI shuts down `gpt-4o`, `gpt-4o-mini`, `gpt-4.1` and
  `gpt-4.1-mini`, with their dated snapshots. The listed replacement is
  `gpt-6-luna`. OpenAI's deprecations page does not list the four aliases; its
  models API gives them that shutdown date (checked 18 October 2026).

The data has 283 entries (276 model IDs) from the deprecation pages of OpenAI,
Anthropic, the Gemini API and Gemini on Google Cloud (Vertex AI). OpenAI's pages
were read on 18 October 2026, Anthropic's and Google's on 16 October 2026. Every
entry names the provider page it came from. The data is bundled at build time,
so it changes only when you update Arbor, and the report says so once it is more
than 90 days old.

### On a real repository

The `examples/` folder of
[openai/openai-python](https://github.com/openai/openai-python) at `0e5794d`,
run on 20 October 2026 with this release's build (`CI=1`, so the closing
fix-PR line is not printed). Lines marked `...` were left out.

```text
$ arbor retirements examples
Model retirements in examples (68 Python files checked)

1 call uses a model OpenAI shut down on 12 May 2026
  examples/picture.py:14  dall-e-3 (model, line 9) → several listed (see check)
                          check: OpenAI lists more than one way forward: gpt-image-2, gpt-image-1, or gpt-image-1-mini.
...
1 call uses a model OpenAI shuts down on 23 Oct 2026 (3 days)
  examples/image_stream.py:13  gpt-image-1 (line 14) → several listed (see check)
                               check: OpenAI lists more than one way forward: gpt-image-2.5-sunburst or gpt-image-2.5-flare.
...
12 calls use models OpenAI shuts down on 14 Jan 2027 (86 days)
  examples/parsing.py:21                            gpt-4o-2024-08-06 (line 22) → gpt-6-luna
                                                    decide: keep reasoning off (add reasoning_effort="none") or let the model reason (no change)
  examples/parsing_tools.py:60                      gpt-4o-2024-08-06 (line 61) → gpt-6-luna
                                                    decide: keep reasoning off (add reasoning_effort="none"; keep tools) or let the model reason (remove tools)
  examples/responses/background.py:22               gpt-4o-2024-08-06 (line 24) → gpt-6-luna
                                                    decide: keep reasoning off (add reasoning={"effort": "none"}) or let the model reason (no change)
...
Not checked: 1 call takes the model from somewhere Arbor cannot read (an environment variable, a function argument, another module); arbor usages lists it.
Not checked: 2 calls pass no model, so the SDK's or the server's default applies.
Summary: 22 calls in 20 files use retired or retiring models: 2 already retired, 20 retiring (next: 23 Oct 2026, 3 days).
12 can move once this repo makes the choice shown under decide; 10 need a manual check.
Failing (exit 1): 2 calls use a model that is already retired.
```

### Confirmed, confirm first, gap

A finding is **confirmed** only when the model is a literal or a same-file
constant at a call where the usage scan (below) left no gap. It is **confirm
first** when the value at run time may differ, and the report says why: the
fallback of `os.getenv(...)`, a parameter default, one of several values a dict
lookup, branch or in-file caller may give, a class attribute a subclass in
another module may override, or a name something in the file may change at run
time. A call whose client Arbor could not resolve is a **gap**. Models Arbor
cannot read and code it did not analyze are counted on "Not checked" lines, so a
clean result never claims more than was read.

Calls through Azure OpenAI, another host in `base_url` (OpenRouter, LiteLLM, a
proxy), Bedrock or Vertex for Claude are listed apart, since the provider's
dates may not apply there; `--include-routed` lists them. A `base_url` on the
provider's own host is a direct call.

### Arbor does not decide for you

When the replacement behaves differently by design, the report shows both
options and what each costs, and changes nothing until you choose. Today that is
the reasoning effort on `gpt-6-luna` and the GPT-5.6 models, which can reason by
default where `gpt-4o` and `gpt-4.1` did not, and thinking on
`claude-sonnet-5-5`, which thinks by default where the Sonnet models it replaces
did not.

### Exit codes and flags

`0` nothing failing, `1` a confirmed direct call uses a model that is already
retired (or, with `--fail-within <days>`, one retiring inside that window), `2`
the check could not run. The shutdown day itself counts as retired, in UTC.
`--count-unconfirmed` lets unconfirmed calls fail the check too. Output formats:
`--json` (always valid JSON, errors included), `--markdown`, `--markdown-out
<file>`, `--annotations` and `--quiet`.

## GitHub Action: `mode: retirements`

```yaml
      - uses: Anandb71/arbor@v3.1.0
        with:
          mode: retirements
          retirements-fail-on: never   # or: retired, or a number of days such as 90
```

The Action scans once. The job summary lists every affected call, and warning
annotations go on the 10 most urgent lines (GitHub's limit per step), with one
notice saying how many more the summary lists. It warns by default and fails the
build only when `retirements-fail-on` is `retired` or a number of days. A check
that cannot run (exit 2) fails the step whatever that input says, and a `mode`
other than `command` or `retirements` fails before anything is installed.
`retirements-path`, `retirements-include-routed` and
`retirements-count-unconfirmed` map to the CLI's path and flags. See
[docs/RETIREMENTS.md](RETIREMENTS.md#in-ci-github-action).

## `arbor usages` and the MCP tool `find_api_usages`

`arbor usages` lists where a Python project calls the OpenAI, Anthropic and
Gemini SDKs (`google.genai` and the legacy `google.generativeai`). Each call has
its file, line and column, enclosing function, keyword-argument names and the
client's routing: `openai_direct`, `azure_openai`, `custom_base_url`,
`anthropic_direct`, `anthropic_bedrock`, `anthropic_vertex`, `google_genai`,
`google_generativeai_legacy` or `unknown`. `--sdk openai|anthropic|google`
filters by provider, and `--json` is always valid JSON.

The MCP tool `find_api_usages` returns the same report to agents, paginated with
`offset`, `limit` and `hasMore`. It reads files directly and needs no graph.

Values are captured only for `model`, `engine`, `deployment_id` and
`model_name`. A value or route with no gap is confirmed only inside a small
supported grammar: a string literal at the call, a same-file name with one
reaching assignment, a class attribute assigned once, a parameter default of a
function whose every call is in the file, and an entry of a same-file dict
literal, each under the rules in [docs/SDK_USAGES.md](SDK_USAGES.md). Everything
else keeps a typed gap (`unresolved_model`, `unknown_client_factory`,
`ambiguous_client`, `dynamic_dispatch` and others) with a reason, such as
`env_var`, `parameter_default`, `subclass_override` or `dynamic_feature`. Python
is too dynamic to resolve every shape, so a shape outside the grammar is a gap,
never a guess. No credential value is ever captured: a value is refused when a
name it flows through looks like a secret or the value itself is
credential-shaped.

## Scope today

- **Python only.** `.py` files and extensionless scripts with a Python shebang.
  TypeScript and JavaScript are not supported yet.
- **Direct SDK calls only.** The `openai`, `anthropic`, `google-genai` and
  `google-generativeai` SDKs. Calls through LangChain, LiteLLM's own API,
  LlamaIndex or other wrappers are not checked yet.
- **Code only.** Model IDs in `.env`, YAML, JSON or notebooks are not read.
- Vendored and virtual-environment directories (`venv`, `site-packages`,
  `node_modules`) and hidden directories such as `.github/` are skipped and
  counted, not reported.

## Local only, no telemetry

Both commands read source files on your machine and print a report. They make
no network requests and have no telemetry, and no code leaves your machine.
`base_url` values are read only to tell the provider's own host from another
one; the report names the class, never the URL.

The human `arbor retirements` report ends with one line offering automated fix
pull requests, with a link that carries only `utm_source=arbor-cli`. It is not
printed with `--json`, `--markdown`, `--annotations` or `--quiet`, or when `CI`
is set, and Arbor opens nothing itself.

## Install

```bash
npx @anandb71/arbor-cli retirements .   # no install
npm install -g @anandb71/arbor-cli
cargo install arbor-graph-cli --version 3.1.0
```

Binaries for Linux, macOS and Windows are attached to the GitHub release. See
[docs/INSTALL.md](INSTALL.md) for Scoop, Homebrew and the container image.

The previous release is [v3.0.5](RELEASE_NOTES_v3.0.5.md).
