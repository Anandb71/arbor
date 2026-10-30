# Changelog

All notable changes to Arbor will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **LangChain chat models in `arbor usages` and `arbor retirements` (Python):** `ChatOpenAI`, `AzureChatOpenAI` and `ChatAnthropic`, from `langchain_openai`, `langchain_anthropic` and the deprecated `langchain_community.chat_models` and `langchain.chat_models`, are recorded as a construction and a call that binds the model, like `genai.GenerativeModel(...)`. Their `model` or `model_name` is read under the same grammar as an SDK call (both given is a `reassigned` gap, and one given next to a `**` expansion the file cannot read a `splat` gap, since the expansion may hold the other name; neither is a `missing_model` gap, except on `AzureChatOpenAI`, which serves its deployment), and they route like the client each one builds: `base_url`, `openai_api_base` or `anthropic_api_url` by host, and `OPENAI_API_BASE`, `ANTHROPIC_API_URL` and `LANGSMITH_GATEWAY` set in the file next to the SDK's own variables (`endpoint_env`). A deprecated `ChatOpenAI` in a file that configures the module-level OpenAI API routes `unknown` with a `langchain_module_config` gap, since on `openai<1.0` its requests go through it, and a chat model given an SDK client (`client=`, `root_client=`) routes `unknown` with a `given_client` gap. A file that may give a built model another model (a write of `model` or `model_name` on another object, an attribute written with a run-time name, `configurable_fields(model_name=...)` or `configurable_alternatives(...)`) gives its values a `dynamic_feature` gap. A `model` key in LangChain's run options (`config`, `metadata`, `tags`...) is not read. Calls on a built chat model (`llm.invoke(...)`) are recorded with its routing; a `model` such a call passes replaces the model's own and is read the same way, and a `**` expansion there that the file cannot read is a `splat` gap. No other LangChain name is tracked. `arbor retirements` lists these models with their date and replacement but applies only the checks about the model itself, with a check to review the parameters LangChain sends by hand, since LangChain takes its own arguments; a confirmed retired model there fails the check like any other. On the Track C real-code sample (166 hits, getArbor-dev/arbor-cloud#179) it finds 7 of the 8 LangChain misses (the 8th is `ChatBedrock`): recall on the 74 live calls rises by 7, and none of the 92 non-call files gains a value.
- **Amazon Bedrock and LiteLLM in `arbor usages` and `arbor retirements` (Python):** a `boto3` client built for the service `"bedrock-runtime"` (`boto3.client(...)`, `from boto3 import client`, `boto3.Session(...).client(...)`; the service name is read under the value rules, and no other service is followed) is a client routed `bedrock` for any model, and its requests (`converse`, `converse_stream`, `invoke_model`, `invoke_model_with_response_stream`...) carry their `modelId`, the only key read there (botocore rejects `model=`, and nothing nested in a Bedrock request names its model). An `endpoint_url` routes by host (a Bedrock runtime host of a region on AWS, a VPC endpoint included, stays `bedrock`; another host is `custom_base_url`; one passed by position or through `*args` is `unknown` with a gap), and `AWS_ENDPOINT_URL_BEDROCK_RUNTIME` or `AWS_ENDPOINT_URL` set in the file is an `endpoint_env` gap. Replacing a request or `boto3.client`, or registering a botocore event handler (on a client's or session's events, under any botocore event name, or through `setup_default_session(botocore_session=...)`), is a `dynamic_feature` gap on the file's Bedrock routes. In a file that imports boto3, a Bedrock-shaped call on an unknown receiver is an `unknown_client_factory` gap. LangChain's `ChatBedrock` and `ChatBedrockConverse` (`langchain_aws`) are chat models like `ChatOpenAI`: the constructor binds `model_id` (or its alias `model`) and routes like a `bedrock-runtime` client, with `LANGSMITH_GATEWAY` among its endpoint variables. `litellm.completion(...)` and `acompletion(...)` route `litellm` and carry their `model`, by keyword or position; LiteLLM's other functions are not followed. A model LiteLLM may map to another one keeps a gap that covers its nested fallbacks too: `dynamic_feature` in a file that mentions `litellm.model_alias_map`, and a new reason, `model_list`, for a call given a `model_list` (a `splat` gap when an opaque `**` may hold one). Records say `sdk: "aws"` or `sdk: "litellm"`, and `--sdk aws` (alias `bedrock`) and `--sdk litellm` filter them. A `model_id=` or `modelId=` given to a call on a built Bedrock chat model (`llm.invoke(q, model_id=...)`), which the Converse API sends instead of the model's own, is read too (on `ChatBedrock`, which uses the Converse API only when `beta_use_converse_api`, with a `conditional` gap). In `arbor retirements` both are routed, listed with `--include-routed`, and checked for the model only: a LiteLLM model is looked up by its router prefix or its shape (a Gemini model with no prefix on Vertex AI's schedule, where LiteLLM sends it), and nothing is looked up for a Bedrock call, since AWS's own Bedrock schedule is not in the data. On the Track C sample it recovers the 5 Bedrock and LiteLLM misses: h091 and h115 with the exact model ID, h049 and h085 as a call with a typed gap (h049's lists `o3-mini`), and h097 as an `unknown_client_factory` gap. No record of any other kind changes on the 166 files, and no non-call file gains a value.

### Changed
- **`arbor retirements` offers a readiness review:** when the human report lists a call to a retired or retiring model, its closing offer is a paid readiness review with a link that carries only the number of listed calls, how many are already retired and the next shutdown date (`&review=1&calls=4&retired=1&next=2027-01-14`), never code, paths, file, repository or model names. Arbor still makes no network requests: nothing is sent unless you open the link. Other findings keep the fix-PR line; `--json`, `--markdown`, `--annotations`, `--quiet` and CI print no offer, as before.

### Fixed
- **`arbor retirements` missed bare `gpt-5` and `o3`:** OpenAI's deprecations page started listing bare IDs and dated snapshots together in its rows between 23 and 26 October 2026 (`gpt-5` | `gpt-5-2025-08-07`), after 3.1.0's data was built, so `model="gpt-5"` or `model="o3"` was not listed at all. The data now has the 15 IDs the page added, checked against the page as read on 28 October 2026: `gpt-5`, `gpt-5-mini`, `gpt-5-nano`, `gpt-5-pro`, `o3` and `o3-pro` (11 December 2026, to GPT-5.6), `gpt-image-1.5-2025-12-16` (1 December), `gpt-4o-mini-tts` (6 January 2027), four 20 January audio and realtime snapshots, `gpt-4o-mini-transcribe-2025-12-15` (26 February), and `gpt-5.1-2025-11-13` and `gpt-5.4-nano-2026-03-17` (1 April). Like the snapshots already listed for those dates, they come with the date and replacement but no argument advice. 298 entries (291 model IDs). `docs/RETIREMENTS.md` now says which source gives the 14 January date: the deprecations page for the dated snapshots, and OpenAI's models API for the bare `gpt-4o`, `gpt-4o-mini`, `gpt-4.1` and `gpt-4.1-mini`.
- **`arbor retirements` said more than the providers' pages, or less:** every page behind the bundled data was read and checked again on 28 October 2026 (getArbor-dev/arbor-cloud#194), and the data is regenerated from it. The bare `gpt-4o`, `gpt-4o-mini`, `gpt-4.1` and `gpt-4.1-mini` now cite OpenAI's API reference, which lists each as "Deprecated. Announced shutdown date: 2027-01-14", as well as the models API (18 October); the old note said the deprecations page "does not list" them, but it names `gpt-4o` as a replacement in older rows. A call that moves to `claude-haiku-4-5-20251001` with `temperature`, `top_p` or `top_k` was kept with "accepts temperature, top_p and top_k"; it is now a check, because Anthropic's API version 2026-10-21 (sent by `anthropic` 2.0.0 and `@anthropic-ai/sdk` 1.0.0 and later) rejects them on every model, and Claude's sampling check names both API versions. `stop`, `logit_bias` and `max_tokens` on GPT-5.6 say that only gpt-5.6-terra was called, and `stop` and `logit_bias` read `unverified` on the other tiers. The sampling check for a reasoning model moving to GPT-5.6 (such as `o3-mini` with `temperature`) gives the live result it rests on, says `top_p` and logprobs were tested only at `"none"`, and reads `unverified` on the Responses API, which no live call reached and no GPT-5.6 page covers. The Responses API sampling check for gpt-6-luna now quotes OpenAI's "Using GPT-6" guide (`docs`, still not live-checked), and gpt-6-luna's tier note quotes OpenAI ("most efficient model for focused, high-volume tasks") instead of calling GPT-4o and GPT-4.1 flagships. OpenAI removed its separate table of fine-tune replacements, so fine-tunes of `gpt-3.5-turbo`, `gpt-4` and `gpt-4.1-nano` no longer name `gpt-5.6-*` as their listed base, and fine-tunes of `babbage-002` and `davinci-002` follow their base's 28 September date. Google Cloud's ten retired Veo 2.0 and Imagen models are added, Veo 3.0 is confirmed retired, four Gemini API Veo 3.1 and omni previews are marked shut down, `deep-research-pro-preview-12-2025` (past its earliest date, not greyed) is unconfirmed, and `gemini-2.5-flash-preview-09-2025` is a swap to its listed replacement. 308 entries (297 model IDs). The generator now refuses an alias note without the API reference's quote, and a message whose live-check date differs from the recipe's.
- **`arbor retirements` advised "let the model reason (no change)" for calls that need `reasoning_effort="none"` (#290):** for a call whose client Arbor could not resolve (a package's subclass of `openai.OpenAI` exported under the same name, a client passed in as a parameter or built by a factory), the check dropped the call's arguments, so a non-default `temperature`, `tools` or `max_tokens` went unchecked and the decision still read `(no change)`. On gpt-6-luna those return 400 unless the call sends `reasoning_effort="none"` (`max_tokens` at any effort tested; [live checks](docs/LIVE_CHECKS_2026-10-18.md) E1 to E3). `arbor usages` now records the argument names of an `unknown_client_factory` gap (`args`, names only, never values), and the retirement check applies the same argument rules to them as to a resolved client. When Arbor cannot read every argument a call sends (arguments spread in with `**`, arguments sent in `extra_body`, or a gap that recorded none), an option that adds nothing says `could not check the arguments from **kwargs` (or `in extra_body`, or `this call's arguments`) instead of `no change`, a check line says why, and the JSON decision carries `unchecked_args`. The clause names the `**` expression as the code does (`could not check the arguments from **PARAMS`), and a dict literal in the same file spread in with `**` is read, so its keys are checked like arguments written at the call (`arbor usages` lists each `**` expression in `spreads`: its name and, for a same-file dict, its keys; names only). No option reads `no change` next to a check line that contradicts it either: when a check line names an argument the call sends that the options neither remove nor keep, each option names it, `check temperature` where the line says it fails or changes on the replacement (Claude's non-default `temperature` on claude-sonnet-5-5, `stop` on gpt-6-luna) and `could not check temperature` where nothing verified it for that API (sampling arguments on the Responses API, which no live check called), in `--json` as `options[].check` and `options[].unverified`. A same-file dict counts as read only in full: its keys are recorded whatever they are called, so `**{"max_tokens": 512}` gets the `max_tokens → max_completion_tokens` rename like `max_tokens=512` at the call, and a key Arbor does not record (not an identifier, or shaped like a credential) leaves the dict unread; a dict changed or passed on later in its file is named, not read (a change made from another module is not seen). The `could not check the arguments ...` clause is on every option, since an unseen `max_tokens` or `stop` returns 400 with reasoning off too. `seed` and `n`, which the live checks sent to gpt-6-luna only at `reasoning_effort="none"`, read `could not check seed` for letting the model reason, and a check line that a live check saw on another API says so (`Seen on Chat Completions only; the Responses API was not checked.`).
- **Calls through aliased imports (#258):** after `from money import money_to_minor_units as to_minor`, a call to `to_minor()` recorded no edge, because the import kept only the local name. When another file defined a function named like the alias, the call was bound to that function at full confidence instead. Python's `from X import Y as Z` and TypeScript and JavaScript's `import { Y as Z }` now record the imported name, so the call reaches `Y` in the module the import names. Dotted modules (`pkg.mod`) are supported, a package's own `__init__.py` comes before its submodules, and relative modules (`.mod`, `../route`) are read from the importing file's directory, so a same-named module elsewhere is never the target. An alias imported from a module the project doesn't define (`numpy`) links nothing, unless a project file or directory, or a directory above the project, has the module's name (Node's `http` and a `src/lib/http.ts`): then the alias binds there, as the unaliased import already does. A function the caller's own file defines under the alias name still wins. Indexes rebuild once (`extract-5`).
- **`arbor refactor` exited 0 for a missing symbol (#195):** it now exits 1 and prints `error: Symbol '<name>' not found` to stderr after the "Did you mean" suggestions, matching `inspect` and `callers`.
- **`npx @anandb71/arbor-cli` failed on npm 12 (#281):** npm 12 skips install scripts by default, so the binary was never downloaded and the wrapper printed "Arbor binary is not installed yet." The wrapper now downloads the release archive for the platform on first run, checks it against the release's `SHA256SUMS`, extracts only the binary and runs it; postinstall uses the same code and a failure there no longer fails `npm install`. Download problems (offline, DNS, untrusted TLS certificate, proxy, HTTP status, checksum mismatch) exit 2 with the reason and the manual steps. Proxies set in `HTTPS_PROXY` or npm's `https-proxy` are used on Node.js 22.21+ and 24.5+.
- **Bare cross-file calls the caller cannot see (#177):** a bare call bound to any same-named definition in another file, and the import filter only lowered its confidence, so `callers` and PageRank still counted it: a project's own `len` picked up the builtin `len(...)` calls of every other file. In Python a bare call to another file now stays only when the calling file imports the name (`import`, or `from m import`; a star import keeps the lower confidence). In JavaScript and TypeScript a CommonJS `require`, TypeScript's `import x = require()` and an awaited `import()` count as imports too, and the parser now records them. Names destructured from a value a project module produced (a factory's or a React hook's return value) link to that module's own definition when it has one. Any other name the file binds itself (a variable, a parameter, a destructured value) keeps the lower confidence, since it may hold any function. A name the file doesn't bind at all is a global: it can reach a classic script's function, or a module's exported one (build tools such as Nuxt inject those imports), at the lower confidence, but never a module's private function, so that edge is dropped. A file is a module when it has a top-level `import`, `export` or CommonJS export, or requires anything. Jest's global `expect()`, for one, no longer binds to a script's own `expect` helper. In Go a bare call stays inside its package (the same package clause in the same directory). Other languages keep the lower confidence. Import nodes now get an id per statement, so two imports of one module in a file both survive the index cache, and the saved graph keeps each file's imports beside it, so `arbor index --changed-only` no longer loses the imports of the files it doesn't re-parse. Indexes rebuild once (`extract-6`).
- **A folder under the home folder indexed the whole home folder (#273):** `.arbor` is a project marker, and `~/.arbor` exists for reasons that have nothing to do with a project: it holds the global config (`~/.arbor/config.json`), `install.ps1` installs the binary into `~/.arbor/bin`, and `arbor init`, `index` or `setup` run in the home folder creates it. So a command run in a folder with no nearer marker (no `.git`, `Cargo.toml`, `package.json`, ...) used the home folder as the project, and `arbor index .` indexed all of it and wrote its graph and cache to `~/.arbor`. The search for the project now stops at the home folder: the home folder is the project only when Arbor runs in it or is given its path, and the search never goes above it. With no marker between the folder and home, the folder itself is the project. The home folder is the one the system reports, and `HOME` and `USERPROFILE` when set, compared without case on Windows and macOS and also as the folder on disk, so a junction, symlink, 8.3 short name or network-share spelling (`\\localhost\C$\Users\...`) of it counts too, also in a path that does not exist yet. Tests that ran `arbor` in a bare temp folder now give it its own marker, since on Windows the temp folder is under the home folder.

## [3.1.0] - 2026-10-20

A feature release: two commands for Python code that calls AI model APIs. See [docs/RELEASE_NOTES_v3.1.0.md](docs/RELEASE_NOTES_v3.1.0.md).

### Added
- **`arbor retirements` (#277):** lists the direct `openai`, `anthropic` and Google SDK calls in a Python project that use a model its provider has retired or is retiring, with the date and days left, the replacement the provider lists (followed through a chain to the first model that still works), the arguments that have to change with it, the per-repo choices it needs and what needs a manual check. The data is bundled at build time from the providers' deprecation pages (283 entries, 276 model IDs; OpenAI read on 18 October 2026, Anthropic and Google on 16 October 2026): OpenAI shuts down `gpt-4`, `gpt-4-turbo`, `gpt-3.5-turbo`, `o1`, `o3-mini`, `o4-mini` and others on 23 October 2026, and `gpt-4o`, `gpt-4o-mini`, `gpt-4.1` and `gpt-4.1-mini` on 14 January 2027. A finding is confirmed only for a literal or same-file constant at a call where `arbor usages` left no gap; environment fallbacks, parameter defaults, possible values, overridable class attributes and unresolved clients are listed as *confirm first* or counted under "Not checked". Calls through Azure, another `base_url` host, Bedrock or Vertex for Claude are listed apart (`--include-routed`). Changes whose effect differs by design (the reasoning effort on gpt-6-luna and GPT-5.6, thinking on claude-sonnet-5-5) are shown as decisions with both options, never made for you. Exit 0 nothing failing, 1 a confirmed direct call uses a retired model (or one retiring within `--fail-within <days>`; the shutdown day counts as retired, UTC), 2 the check could not run. `--count-unconfirmed`, `--json`, `--markdown`, `--markdown-out <file>`, `--annotations` and `--quiet`. Runs locally with no network requests or telemetry. Data is regenerated from the retirement recipes at release by `scripts/retirements/generate_model_retirements.py`, and a test fails on any hand edit ([docs/RETIREMENTS.md](docs/RETIREMENTS.md)).
- **GitHub Action `mode: retirements` (#277):** runs `arbor retirements` once, writes the Markdown report to the job summary and annotates up to 10 affected lines (GitHub's limit per step) with a notice for the rest. It warns by default; `retirements-fail-on: retired` or a number of days fails the build. A check that cannot run fails the step whatever that input says, and an unknown `mode` fails before anything is installed. `retirements-path`, `retirements-include-routed` and `retirements-count-unconfirmed` map to the CLI.
- **`arbor usages` and the MCP tool `find_api_usages` (#276):** where a Python project calls the OpenAI, Anthropic and Gemini SDKs (`google.genai` and the legacy `google.generativeai`). Each call has its file, line and column, enclosing function, keyword-argument names and the client's routing: `openai_direct`, `azure_openai`, `custom_base_url` (a `base_url`/`api_base` whose host is not the provider's own; `api.openai.com` and `api.anthropic.com` stay direct), `anthropic_direct`, `anthropic_bedrock`, `anthropic_vertex`, `google_genai`, `google_generativeai_legacy` or `unknown`. Values are captured only for `model`, `engine`, `deployment_id` and `model_name`, from string literals, same-file constants, same-file dict literals (`**PARAMS`, `MODELS["fast"]`) and parameter defaults. A value or route with no gap is confirmed only inside a small supported grammar ([docs/SDK_USAGES.md](docs/SDK_USAGES.md)); everything else keeps a gap, including anything a dynamic feature of the file may change (`exec`, `eval`, `globals()`, `sys.modules`, `__dict__`, `setattr`/`delattr` with a run-time name, attribute hooks, a class decorator or metaclass, multiple or deep inheritance, a class used as a value, environment writes with a run-time name, `builtins` or `sys` under another name, a frame's `f_globals`, scopes nested more than 64 deep, an SDK member replaced in the file), which is a `dynamic_feature` gap naming the feature and its line; the literal fallback of an environment or config lookup (`os.getenv("MODEL", "gpt-4o")`, `config(..., default=...)`) is captured with origin `env_default` and keeps its gap. No credential value is ever captured: a value is refused when any name it flows through looks like a secret or the value itself is credential-shaped (`sk-`, `AIza`, `ghp_`, `xox...`, long random tokens), and that refusal is an `unresolved_model` gap with reason `credential`. Each call site sees the assignment that reaches it (including `nonlocal`, `match` captures, `except ... as`, `globals()["NAME"]` and a wildcard import from another module, which may rebind any name before it), `self.x` also sees up to 16 direct same-file subclasses (a model value read through `self` or `cls` in a class other modules may subclass keeps a `subclass_override` gap), every binding of a name in a class body (`def`, `class`, an import, a loop target...) counts as an assignment of that attribute in the class, its base and its subclasses, a method that rebinds its `self` or `cls` reads nothing through it, a dict entry that a later `**` expansion or computed key may replace is a `splat` gap, and a client passed to a helper resolves when every in-file caller passes the same one (a public function keeps an `ambiguous_client` gap, `in_file_callers`, since other modules may pass another; a public function's client or model default keeps `parameter_default`). Constructor calls count as calls of `__init__`. A parameter default that an in-file caller overrides keeps a `parameter_overridden` gap, and one of a function whose callers cannot all be seen (decorated, passed to an executor, `functools.partial`, `map`, a task queue) keeps `unseen_callers`; a client constructor chosen at run time against anything else builds an ambiguous client; a client set up outside its constructor (`client.base_url = ...`, `setattr(client, "base_url", ...)`, `OPENAI_BASE_URL`/`ANTHROPIC_BASE_URL` set in the file, through `os.environ` or `os.environb`) routes as `unknown` with a gap, and so does a `base_url` read from the environment with no fallback (`base_url_env`) or not resolved (`base_url_unresolved`), while one read from the environment with a fallback keeps the fallback's routing with a `base_url_env` gap; a Gemini client given `vertexai=True`, an endpoint in `http_options` or a `**` expansion routes `unknown` with an `endpoint_option` gap; the module-level OpenAI API's configuration counts only as plain assignments in the top-of-file setup block, before the first `def`, `class`, decorator, bare expression or compound statement, each setting once and through a name only imports bind (anything else gives every module-level call a `module_config` gap), and a module-level call passing `engine` or `deployment_id` in a file that configures nothing is `unknown` with a `module_config` gap; a request that passes no model is a `missing_model` gap; and a gap for a run-time dict key, a branch-assigned name, a loop over a literal list or a conditional lists the values it may take, with `values_complete: true` only when that list comes from literals in one scope (or from every call of a private function) with no alias, mutation or dynamic feature. What cannot be resolved is a typed gap (`unresolved_model`, `unknown_client_factory`, `ambiguous_client`, `dynamic_dispatch`, `vendored_path`); `venv`, `site-packages`, `node_modules` and other vendored directories are excluded and counted, and so are hidden directories such as `.github/`. Extensionless scripts with a Python shebang are scanned, and files with syntax errors are listed. `--sdk` filters by provider (`gemini` is an alias for `google`) and `--json` is always valid JSON, argument errors included. Each lookup reads a bounded number of definitions (64), candidate values (64), subclasses (16) and callers (256), scopes nested more than 64 deep are not analyzed (a `truncated` gap), and lists, loops and class lookups are read once, so time and memory grow linearly with the file (in release builds, the review stress files up to 2.6 MB take at most 3.2 s and 580 MB; 4 MiB files about 5 s and 1 GB, bound by the parse; larger files are skipped). The report is bounded too: nested values (32 per call, 4,096 per file), listed candidate values (32 per gap, 4,096 per file), names and import paths (200 bytes), records including imports (20,000 per file, then a `truncated` gap) and imported names (20,000 per file, past which the file's calls are not analyzed) are bounded. Graph extraction is unchanged; as after every upgrade, a saved graph is rebuilt once on first use.

## [3.0.5] - 2026-10-09

A fix release. See [docs/RELEASE_NOTES_v3.0.5.md](docs/RELEASE_NOTES_v3.0.5.md).

### Fixed
- **MCP tool output leaked local paths (#261):** every serialized node `file` is now relative to the project root, and `initialize` reads the namespaced `io.modelcontextprotocol/protocolVersion` key before the flat ones.
- **Tests in TypeScript and JavaScript were invisible (#235):** calls inside `describe`/`it`/`test` callbacks belonged to no symbol, so `arbor callers` missed every Jest, Vitest and Mocha test and `diff`/`check` reported changes as having no tests exercising them. Each `it`, `test`, `specify` and `bench` call, and each `beforeEach`/`afterEach`/`beforeAll`/`afterAll`/`before`/`after` hook, is now a function named after the test, such as `it: accepts two`, that owns the calls in its callback. `.only`, `.skip` and `test.each(rows)(...)` are included; `describe` blocks are containers and get no node of their own. A title repeated in one file gets its line, such as `it: works (line 8)`. Indexes rebuild once (`extract-4`).

## [3.0.4] - 2026-10-03

A fix release. See [docs/RELEASE_NOTES_v3.0.4.md](docs/RELEASE_NOTES_v3.0.4.md).

### Fixed
- **First run without git or a repository:** `arbor diff`, `check`, `summary`, `agent review` and `agent guard` now say whether git is missing from PATH, the folder is outside a repository, or git refused the repository (quoting git, which names the fix, such as `safe.directory`). They check this before the index, so the reason is the real one and a failed run no longer creates `.arbor/`. The `.git` folder and bare repositories are no longer mistaken for work trees.
- **Receipts that could not record:** `arbor hook claude` warns at install time when receipts cannot work there, and `receipt begin` writes the reason to stderr instead of returning silently.
- **Read-only projects:** failing to create `.arbor/`, `CLAUDE.md` or `.claude/settings.json` names the path and what to do, instead of a bare "Access is denied".
- **`arbor hook claude` overwriting files:** an existing `CLAUDE.md` or `.claude/settings.json` that could not be read (for example, saved as UTF-16) was treated as empty and replaced. It is now left untouched and the install stops with an explanation.
- **Linux binaries on older distributions (#242):** v3.0.3's Linux binaries needed glibc 2.39 and would not start on Ubuntu 22.04, Debian 12 or RHEL 9. They are now built in manylinux_2_28 and need glibc 2.28, and the build fails if that floor rises.
- **Missed Rust callers on method receivers:** `draft.run_started()` was resolved by method name alone and dropped once more than two types defined `run_started`, so `arbor callers` reported none. The receiver's type is now read from the code: typed parameters (through `&`, `Box`, `Arc`, `Mutex`, guards, `Option`, `Result`, collections, channels and Tauri's `State`), `let` annotations, constructors, struct literals, `if let` / `let … else` / `match` arms, closures, `self.field`, and the return types of the functions and methods that produced the value, followed across files. A type the project doesn't define links nothing, so `client.get()` no longer binds to a project `get`. Indexes rebuild once (`extract-3`).
- **`arbor diff` counting containers and comments:** a module, class, trait, struct or enum no longer counts as modified when every changed line inside it belongs to one of its members, and a hunk that only adds comments, blank lines or an item's doc comment no longer implicates anything; an attribute or decorator counts as a change to its item. On a real three-file PR the report went from 6 modified symbols (four of them `mod` blocks) to the 2 functions it edited.
- **`arbor diff` impact counting callees:** impact, files likely to need updates and entrypoints now come from callers only. Editing a function can't break what it calls; counting those made 8 edited startup symbols report 944 impacted nodes instead of 3.
- **Noisy output:** one-shot commands log warnings only (servers keep their lifecycle lines, `-v` shows more, `RUST_LOG` overrides). Logs always go to stderr.
- **Receipts missing an edit on Linux (#246):** seeding the receipt snapshot from a copy of git's index lost git's racy-clean check on Linux, where the copy gets a new timestamp, so an edit made right after the snapshot could be missed. The snapshot now keeps that check.
- **First run after an upgrade (#251):** clearing an outdated graph cache re-opened the store right after dropping its handle, and sled could still hold the directory lock ("could not acquire lock … WouldBlock"). The cache is now checked, cleared and re-stamped through one handle.
- **MCP protocol (#254, #256):** `initialize` rejects a malformed `protocolVersion` with `-32602` naming the supported versions, and maps any dated version from 2026 on to the current protocol. On HTTP, one connection's negotiation no longer changes what another sees. Over stdio a malformed line gets a `-32700` response instead of leaving the client waiting. Declared extensions are intersected with the implemented ones.
- **`.arbor/` showing up in git (#233):** Arbor now writes `.arbor/.gitignore` whenever it creates or initialises the folder, including when a receipt creates it, so `git add -A` no longer picks up the graph or saved prompts. An existing `.arbor/.gitignore` is left alone. Files from `.arbor/` that you already committed stay tracked; remove them with `git rm -r --cached .arbor`.

### Added
- **Named symbols in `arbor diff` and `check`:** each modified symbol is listed with its file, line and direct caller count, most-called first (top 12 in the text report, a table in `--markdown`, all in `--json` as `modified`).
- **Shadowed installs:** `arbor doctor` fails when an older `arbor` earlier on PATH runs instead of a newer install, and says how to remove it. The npm postinstall, `install.sh` and `install.ps1` warn about the same thing after installing. A leftover `cargo install` of 1.9.0 had kept answering after npm installed 3.0.3, giving its file-level diff (377 symbols for a 3-file change) and 100,000 warning lines.
- **Release checks:** every binary runs a first-use smoke test on its own platform before anything is published, including Intel macOS, Linux arm64, AlmaLinux 8, Ubuntu 22.04 and Debian 12. Pull requests that change the release build or `Cargo.lock` run the same build and smoke test without publishing.
- **Provenance for npm and GHCR (#247):** the npm package is published with provenance and the container image carries a signed SLSA attestation.
- **Installer and accuracy checks in CI (#255, #257):** `install.sh` and `install.ps1` install a locally served release on Linux, macOS and Windows runners on every pull request, and a versioned evaluation corpus checks the graph's recall and precision against hand-derived truth.
- **Verifiable downloads:** releases publish `SHA256SUMS` and a signed build provenance attestation for each archive (`gh attestation verify <archive> --repo Anandb71/arbor`).

### Security
- **WebSocket servers refuse foreign origins:** the RPC and sync WebSocket servers, which `arbor bridge` always starts and `arbor serve` runs, accepted a handshake from any web page, because browsers apply no CORS to WebSockets. A page open while they ran could read the code graph. They now apply the MCP HTTP transport's policy: a present `Origin` must be loopback, and on a loopback-bound server `Host` must be too (403 otherwise). Desktop clients send no `Origin` and are unaffected; `--headless` still accepts remote hosts but checks `Origin`.

## [3.0.3] - 2026-09-29

A fix release. See [docs/RELEASE_NOTES_v3.0.3.md](docs/RELEASE_NOTES_v3.0.3.md).

### Fixed
- **Rust call edges (#226):** calls through paths (`crate::jobs::enqueue()`, `jobs::enqueue()`), associated functions (`Type::new()`), `self.method()` and calls inside macro arguments (`assert!`, `format!`) now resolve. Paths resolve through the module tree via the new `SymbolTable::resolve_path`; paths into external crates stay unresolved instead of binding to a same-named local function.
- **Per-symbol `arbor diff` (#226):** only modified symbols carry a blast radius; new symbols are listed separately. Tests that call the change are counted as `tests_exercising`, not as impact or entry points.
- **Change scopes (#226):** `arbor diff`, `check` and `summary` take `--base <ref>` (committed and uncommitted changes since the merge base, as a pull request shows them) and `--staged`. Whitespace-only edits and generated files are ignored.
- **Cross-language callers (#226):** resolution only considers definitions in the caller's language family. `arbor callers`/`callees` list each same-named definition separately, accept `module::name` and `file:name`, and say what an empty answer cannot see.
- **Stale graphs (#226, this release):** the saved graph records the commit and the extractor that built it. A `HEAD` move (checkout, commit, rebase) or an upgrade refreshes it instead of answering from the old graph. A leftover index cache no longer disables the staleness check.
- **Inheritance (#191):** classes emit `extends`/`implements` edges to their bases, and inherited methods stay reachable.
- **Call cycles (#190):** PageRank runs on the condensation of the call graph, so a closed ring no longer fills the top of the ranking. `CentralityScores::get` is still the percentile rank.
- **Vendor assets (#189):** `vendor/` and minified files are not indexed, `arbor agent onboard` computes centrality before ranking hotspots, and Android activities and Hilt annotations are entry points.
- **HTTP bridge (#222):** `arbor bridge --http` rejects non-loopback `Origin` and `Host` headers (cross-origin and DNS-rebinding requests), requires `application/json`, sends no CORS headers, and bounds request size, time and connections.
- **Bridge busy loop (#174):** the bridge's watcher skips ignored build and dependency directories, and graph patches run off the async runtime.
- **GitHub Action:** the prebuilt-binary download named assets that don't exist, so every run compiled from crates.io instead. It now downloads the release asset, and a pinned version builds that tag when no asset fits.
- **Packaging (#186):** v3.0.0 Homebrew and Scoop checksums corrected.

### Added
- **`arbor receipt` (#223, #224):** a plain-English receipt after each coding-agent turn: what changed, what was touched outside the request, and `arbor receipt undo` to put a turn back. `arbor hook claude` wires it into Claude Code; reading receipts needs no prompt, undo asks first.
- **cargo-deny in CI (#220):** licences, duplicate dependencies and advisories are checked on every pull request.

### Changed
- **Release workflow:** a crates.io or manifest-PR failure is reported on its job without failing the run, so GHCR, npm and the VS Code extension still publish (v3.0.0's expired crates.io token skipped all three). The Homebrew formula and Scoop manifest are written by the workflow after the build instead of being bumped by hand before the tag.
- **Docs (#219 and this release):** new brand mark; install instructions list only channels that work; MCP tool tables list all sixteen tools; `docs/GRAPH_SCHEMA.md` documents call resolution.
- Cached graphs and per-file caches from 3.0.0 are rebuilt on first use.

## [3.0.0] - 2026-08-10 "The Right Node"

### Fixed
- **Symbol resolution consults the importing file.** When a bare name matched several modules, resolution fell through to the same-directory rule and attached the edge to whichever definition sat beside the caller. The file's own imports are now checked first (`Resolution::ViaImport`, confidence 0.93).

### Breaking
- `Resolution` gains a `ViaImport` variant; an exhaustive match will not compile.
- Edges land on different nodes, so cached graphs, stored node ids and centrality baselines from 2.6.0 differ.

## [2.6.0] - 2026-08-03 "Ground Truth"

### Fixed
- Colliding symbols are kept instead of the second definition replacing the first.
- Resolution is deterministic across processes.
- Edges carry a confidence in `[0, 1]` from how they resolved.
- Exported TypeScript symbols are indexed once, not twice.
- Method calls on untyped receivers resolve.
- Markdown headings and shell comments are no longer indexed as code.
- Centrality is a percentile rank rather than a fraction of the maximum.

### Added
- Concept search on the library (`ArborGraph::search_ranked`).

## [2.5.0] - 2026-07-15

### Added
- **Parallel indexing:** `index_directory` fans the cache-check/parse phase out across all cores with rayon; results assemble in walk order so graph construction stays deterministic. Measured (median of 3, warm FS cache): Arbor itself 253ms → 95ms (2.7x, 123 files); tokio 2.7s → 1.6s (1.7x, 815 files / 178k LOC — serial graph assembly caps the gain, see `docs/BENCHMARKS.md`). Thread count is tunable via `RAYON_NUM_THREADS`.
- **Warm-start PageRank:** `compute_centrality_warm` seeds iteration from previous scores (with analytic rescaling of the max-normalized stored values back to fixed-point scale) — watcher/server graph patches now converge in a couple of rounds instead of the full iteration budget. Wired into the sync server's re-index and delete paths.
- **Convergence early-exit:** centrality iteration stops once no score moves more than 1e-9 between rounds.
- **Benchmarks:** `compute_centrality_10k` and `compute_centrality_10k_warm` on a realistic fan-in graph (~10k nodes).

### Changed
- **23x faster PageRank:** `compute_centrality` rewritten from per-iteration `get_callers`/string-ID lookups to a one-pass flat adjacency build plus dense Vec iteration — 149.8ms → 6.6ms on a 10k-node graph. Semantics preserved (Calls-edges only, 10% test-caller weight, [0,1] max-normalization).

## [2.4.0] - 2026-07-08 "The Agent-Native Leap"

### Added
- **MCP 2026-07-28 Protocol:** Stateless core with `server/discover`, `_meta` parsing, response caching (`ttlMs`/`cacheScope`), and dual-version fallback for `2025-03-26` clients
- **Tasks Extension:** `tasks/get`, `tasks/update`, `tasks/cancel` — long-running index/audit operations return task handles; fixes cold-start race during background indexing
- **MCP Apps (SEP-1865):** Interactive blast-radius graph (`ui://arbor/blast-radius`) and architecture map (`ui://arbor/architecture-map`) HTML templates rendered inside agent hosts
- **Streamable HTTP Transport:** `arbor bridge --http [--port 3333]` — stateless MCP over HTTP with `Mcp-Method`/`Mcp-Name` header routing, alongside stdio
- **Real `get_blast_radius`:** Git-diff-aware blast radius via shared `arbor-graph::compute_blast_radius` (replaces stub)
- **Pagination:** `offset`/`limit`/`hasMore` on `search_symbols` and `get_map`
- **Criterion Benchmarks:** `cargo bench -p arbor-graph` with CI workflow (`benchmarks.yml`)
- **Release docs:** `docs/ROADMAP_v2.4.0.md`, `docs/RELEASE_NOTES_v2.4.0.md`

### Changed
- **Async MCP stdio:** Replaced blocking `stdin.lock().lines()` with tokio async I/O
- **MCP tool annotations:** `analyze_impact` and `get_architecture_overview` declare `_meta.ui` for MCP Apps
- Workspace version bumped to **2.4.0** across all manifests

## [2.3.0] - 2026-06-28 "Agent Brain"

### Added
- **5 New MCP Tools (10 → 15 total):**
  - `get_blast_radius`: Diff-based impact analysis exposed to AI agents via MCP — returns affected nodes, risk level, and architectural impact
  - `explain_symbol`: Token-bounded architectural explanation of any symbol — role classification, centrality, callers/callees, significance
  - `audit_security`: Traces execution paths from source to sensitive sinks (DB, exec, file I/O, network) — security audit via MCP
  - `get_architecture_overview`: High-level codebase orientation — hotspots, modules, entry points, graph statistics — ideal for onboarding agents
  - `batch_query`: Multi-symbol query in a single call — reduces round-trips for bulk lookups, optional caller/callee inclusion
- **MCP Resources:** Implemented `resources/list` and `resources/read` exposing `arbor://graph/stats`, `arbor://graph/entry-points`, and `arbor://graph/hotspots` for passive agent context
- **MCP Tool Annotations:** All 15 tools annotated with `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint` per 2025-03-26 spec — signals trust/safety to agents
- **Built-in Agent Workflows (`arbor agent`):**
  - `arbor agent review`: Autonomous PR review — analyzes git changes for high-centrality modifications, untested paths, and architecture violations
  - `arbor agent onboard`: Codebase onboarding guide generator — entry points, hotspots, module map, suggested reading order
  - `arbor agent guard`: Architecture guard — validates changes against blast radius thresholds, flags entry point modifications
- **A2A Agent Card:** `agent-card.json` for Agent-to-Agent protocol discovery — enables other agents to find and delegate to Arbor
- **GitHub Action Pre-Built Binary:** Downloads pre-compiled binary from GitHub Releases instead of compiling from source (~5s vs ~3-5min CI step)
- **Benchmarks Document:** Performance claims with reproduction methodology
- **Launch Plan:** Structured go-to-market strategy for organic growth

### Changed
- **MCP Protocol Version:** Bumped from `2024-11-05` → `2025-03-26`
- Workspace version bumped to **2.3.0** across all manifests

## [2.2.0] - 2026-05-30 "PR Intelligence & Sponsorships"

### Added
- **arbor diff --markdown**: Native markdown formatting option for impact analysis reports. Perfect for PR comments, presenting a color-coded risk assessment, changed files list, direct/indirect caller metrics, affected API entrypoints, and actionable suggestions.
- **arbor check --markdown**: Markdown safety check output. Validates change impact against maximum blast radius thresholds and prints color-coded PASS/FAIL status.
- **arbor summary**: Auto-generates structured markdown Pull Request descriptions based on graph diff analysis, classifying changes, mapping scope areas, analyzing blast radius, and automatically recommending relevant reviewers.
- **Upgraded GitHub Action composite steps**: Updated `action.yml` with a new `comment-on-pr` parameter. When running in a PR workflow, it automatically executes the impact analysis, posts a markdown comment, and deduplicates comments by editing previous reports.
- **GitHub Native Sponsorships**: Configured standard `.github/FUNDING.yml` to support GitHub Sponsors, Ko-fi, and custom Stripe billing channels.

### Changed
- Aligned workspace and all package manager manifests to **v2.2.0** (Cargo, npm wrapper, VS Code extension, Scoop, Homebrew).
- Simplified `.github/workflows/arbor-pr-bot.yml` to reference the local upgraded composite action directly, reducing boilerplate logic.
- Ignored `.claude/` locally via `.gitignore`.

## [2.1.0] - 2026-05-15 "Agent-Native MCP Ecosystem Expansion"

### Added
- **MCP Server Upgrade**: Re-engineered model context protocol server to expose **10+ advanced agent-native tools** directly to AI clients.
- Added comprehensive tool schema declarations for AI engine integration.

## [2.0.1] - 2026-04-20 "Patch Stability & Automation Fixes"

### Fixed
- **PR Bot reliability**: switched to valid CLI-driven impact report generation (`arbor diff . --json`) and added PR base/head commit-range support via `ARBOR_DIFF_BASE` / `ARBOR_DIFF_HEAD`.
- **PR Bot formatting**: corrected Markdown code-fence rendering so JSON output appears as a proper fenced block in PR comments.
- **CLI regression coverage**: added integration test coverage for ranged diff mode to prevent regressions in PR-impact reporting.
- **Contributors automation**: hardened `contributors.yml` by skipping PR creation when README has no contributor changes and using `github.token` consistently.
- **Contributors script resilience**: updated GitHub auth header usage and graceful API failure handling to avoid flaky scheduled workflow failures.

### Changed
- Release-facing versions aligned to **2.0.1** across workspace/package-manager/editor manifests (Cargo workspace version, Homebrew, Scoop, npm wrapper, VS Code extension metadata).

## [2.0.0] - 2026-04-20 "Context-Driven OS + Stability Release"

### Added
- **MCP Tool Expansion**: `get_knowledge_path` returns real logic paths with Markdown [[links]] + causality explanations (Aha! for Lattice users). `analyze_impact` supports `format=markdown` for professional PR bot tables with **bold high-risk** files via ConfidenceExplanation + centrality (sorted_by_centrality).
- **Tauri Lattice Companion**: Desktop shell with system tray (Personal OS feel), graph/MCP integration (left stable for later iteration).
- **Parser v2 Registry**: Clean compile_queries helper, Dart fixes (class_definition etc.), Markdown fallback with NodeKind::Section (no dep conflicts).
- **Sled GraphStore**: Incremental persistence, mtime/versioning, centrality precompute comment (Priority 2).
- **PR Bot**: Enhanced action.yml + workflow for blast radius comments using MCP output.

### Changed
- All discussed features stabilized: parser eat-own-dog-food, MCP supercharged for agents (Priority 3), Markdown support, tests 58/58 passing with feedback loop, no mistakes.
- ROADMAP, PHILOSOPHY aligned (Consumer First = stable, Accessibility = registry, Affordability = sled).
- Versions bumped, docs updated, sequential commits on audit-and-testing-overhaul.
- Release automation hardened: fixed contributors workflow failures (tokened GitHub API + robust LF/CRLF marker replacement), fixed aarch64 Linux linker in release cross-compilation, replaced PR bot action mock output with real command execution.
- Distribution manifests fully aligned to 2.0.0 (Homebrew, Scoop, npm wrapper, VS Code extension metadata/lockfile, server serialization version fixture).

### Stable for v2.0 Release
- `v2.0.0` tagged and `v2.0` PR branch prepared; release workflows (release, GHCR, Marketplace, MCP notes) are aligned and stable.

## [1.7.0] - 2026-03-25 "Distribution & Reach"

> **Feature release focused on making Arbor available everywhere — every package manager, every editor, every CI pipeline.**

### Added

- **Automated release workflow** (`release.yml`) — Cross-platform binary builds (5 targets), crates.io publishing, and GitHub Release creation on tag push
- **Homebrew formula** (`packaging/homebrew/arbor.rb`) — macOS/Linux install via `brew install`
- **Scoop manifest** (`packaging/scoop/arbor.json`) — Windows install via `scoop install`
- **npm wrapper** (`packaging/npm/`) — Cross-platform install via `npx @arbor-graph/cli`
- **VS Code extension: 5 new commands** — `arbor.refactor`, `arbor.status`, `arbor.quickPick`, `arbor.diff`, `arbor.index`
- **VS Code extension: Quick-pick command menu** — `Ctrl+Shift+R` for all Arbor actions
- **VS Code extension: Walkthrough onboarding** — Get Started guide with step-by-step setup
- **VS Code extension: New settings** — `arbor.autoIndex`, `arbor.maxBlastRadius`
- **GitHub Sponsors** (`.github/FUNDING.yml`)
- **Academic citation** (`CITATION.cff`)
- **Docker Compose bridge service** for MCP container usage

### Changed

- **Dockerfile** updated to Rust 1.85 with OCI labels, git support
- **docker-compose.yml** modernized (removed deprecated `version` key)
- **VS Code extension categories** improved for marketplace discoverability
- **README badges** expanded (crates.io, GitHub Release, GHCR, Docker)
- **Install instructions** expanded with Homebrew, Scoop, npm, Docker options

### Removed

- **`vscode-publish.yml`** workflow (duplicate of `vscode-marketplace.yml`, caused CI failures)
- Stale `package-lock.json`, `crates/Cargo.lock`, `crates/test_output.txt`

## [1.6.2] - 2026-03-24 "Revival Release: Language Expansion + Developer Momentum"

> **Feature release focused on expanding parser reach, improving live sync coverage, and strengthening release momentum workflows.**

### Added

- **Fallback parser engine** in `arbor-core` for rapid support of additional ecosystems when a full Tree-sitter path is unavailable in all runtime surfaces
- **New language extension support (5+)** via fallback parsing:
  - Kotlin (`.kt`, `.kts`)
  - Swift (`.swift`)
  - Ruby (`.rb`)
  - PHP (`.php`, `.phtml`)
  - Shell (`.sh`, `.bash`, `.zsh`)
- **Regression tests** for fallback parsing in both legacy parser path and query parser v2 path

### Changed

- **Indexer support matrix** now includes fallback-language extensions in support checks
- **Bridge + visualizer sync watchers** now watch and re-index the newly added language extensions
- **CLI empty-graph hints** now include the expanded extension set
- **Workspace crate line bumped** to `1.6.2` and internal crate dependency versions aligned

### Documentation

- Updated release/status messaging and supported-language listings
- Added release notes for `v1.6.2`

## [1.6.1.1] - 2026-03-18 "Maintenance + Ecosystem Alignment"

> **Maintenance release focused on workflow reliability, MCP guidance, and ecosystem currency as of March 18, 2026.**

### Added

- **CLI: `arbor diff`** — Git-aware blast radius preview for changed files
  - Handles rename-aware changed-file detection
  - Ignores whitespace-only diffs
  - Filters generated/internal files for cleaner signal
- **CLI: `arbor check`** — CI-oriented risk gate over changed blast radius
  - Supports machine-readable JSON output for automation
- **CLI: `arbor open <symbol>`** — Opens symbol/file location in configured editor
- **CLI: `arbor index --changed-only`** — Incremental re-index path based on git changes
- **Binary graph snapshots** — `.arbor/graph.bin` read/write support for faster warm starts
- **Integration tests for diff heuristics** — rename, whitespace-only, generated-file scenarios
- **Workspace cleanup scripts** — `scripts/clean.ps1` and `scripts/clean.sh` to safely prune large generated artifacts before releases

### Changed

- **Branching guidance** documented for `main`, `release/v1.5`, and `release/v1.6`
- **Documentation refresh** across README, Quickstart, Install, Architecture, and MCP integration guides
- **Troubleshooting guidance** now includes a dedicated workflow for reclaiming multi-GB workspace bloat

### Maintenance

- Release channel and status messaging aligned to the `1.6.1.1` maintenance cut.
- Workspace crate version advanced to `1.6.1` (SemVer-compliant crate line for Cargo).
- MCP server metadata version now reports `1.6.1.1` for client-visible maintenance tracking.
- Release context refreshed against current ecosystem signals (Rust `1.94.0`, tree-sitter `0.26.7`, and broader MCP client/platform adoption).

### Documentation

- Added formal release notes for v1.6.0 in `docs/RELEASE_NOTES_v1.6.0.md`
- Added formal release notes for v1.6.1.1 in `docs/RELEASE_NOTES_v1.6.1.1.md`

## [1.6.0] - 2026-03-16

> See [Release Notes](https://github.com/Anandb71/arbor/blob/v1.6.0/docs/RELEASE_NOTES_v1.6.0.md) for full details.

## [1.5.0] - 2026-02-xx

> Maintenance release. See git history for details.

## [1.4.0] - 2026-02-xx "The Trust Update"

> See [Release Notes](https://github.com/Anandb71/arbor/blob/v1.4.0/docs/RELEASE_NOTES_v1.4.0.md) for full details.

## [1.3.0] - 2026-01-xx

> Stabilization and UX improvements. See git history for details.

## [1.2.0] - 2026-01-xx

> Incremental improvements. See git history for details.

## [1.1.0] - 2026-01-08 "The Sentinel Update"

> **Predict breakage. Give AI only the logic it needs.**

### Added

- **Impact Radius Simulator** (`impact.rs`) — Bidirectional BFS to predict all affected nodes before refactoring
  - Severity classification: direct (1 hop), transitive (2-3), distant (4+)
  - Entry edge tracking for explainability
  - Stable ordering for reproducible output
  - 8 unit tests including cycle detection
- **Dynamic Context Slicing** (`slice.rs`) — Token-bounded context extraction for LLM prompts
  - Pinning support for critical nodes
  - Explicit truncation reasons (budget vs depth)
  - 6 unit tests
- **MCP `analyze_impact` Tool** — Structured JSON output for AI agents
  - Input: `{ "node_id": "...", "max_depth": 5 }`
  - Returns: target, upstream, downstream, severity, hop_distance, entry_edge
- **CLI: `arbor refactor <target>`** — Preview blast radius before making changes
  - `--why` flag shows reasoning for each affected node
  - `--json` flag for scripting and CI integration
  - `--depth N` controls search depth
- **CLI: `arbor explain <target>`** — Graph-backed context for code explanations
  - `--why` flag shows path traced
  - `--json` flag for structured output
  - `--tokens N` controls context budget

### Changed

- MCP `analyze_impact` now uses real graph traversal (was placeholder)

## [1.0.0] - 2026-01-07

### Added

- **World Edges (Cross-File Resolution)** - Implemented `SymbolTable` and FQN-based linking for robust cross-file references.
- **Persistence Layer** - Integrated `sled` database for local graph storage (`GraphStore`).
- **ArborQL (MCP)** - Added `find_path` tool for finding shortest paths between nodes.
- **C# language support** - Methods, classes, interfaces, structs, constructors, properties
- **Control Flow edges** - `FlowsTo` edge kind for CFG (Control Flow Graph) analysis
- **Data Flow edges** - `DataDependency` edge kind for DFA (Data Flow Analysis)
- **Barnes-Hut QuadTree** - O(n log n) force simulation for visualizer scalability
- **Viewport culling** - Only render visible nodes/edges for 100k+ node support
- **LOD rendering** - Simplified node rendering at low zoom levels
- **Headless mode** - `--headless` CLI flag for remote/Docker/WSL deployment
- **Binary serialization** - `bincode` dependency for future binary wire protocol

### Changed

- Consolidated language parsers into query-based `parser_v2.rs`
- Upgraded supported languages to 10 (TypeScript, JavaScript, Rust, Python, Go, Java, C, C++, Dart, C#)
- Improved graph rendering performance for large codebases

### Fixed

- None

## [0.1.1] - 2026-01-06

### Added

- **Go language support** - Functions, methods, structs, interfaces, imports
- **Java language support** - Classes, interfaces, methods, constructors, fields
- **C language support** - Functions, structs, enums, typedefs, includes
- **C++ language support** - Classes, namespaces, structs, functions, templates
- **Dart language support** - Classes, mixins, extensions, methods, enums
- `Constructor` and `Field` node kinds for Java/OOP languages
- Updated set-topics workflow with 19 repository topics

### Changed

- Expanded supported languages from 4 to 9
- Updated README with new language support table

### Fixed

- None

## [0.1.0] - 2026-01-05

### Added

- Initial release
- Core AST parsing with tree-sitter
- TypeScript/JavaScript language support
- Rust language support
- Python language support
- Interactive force-directed graph visualizer (Flutter)
- WebSocket-based real-time updates
- MCP (Model Context Protocol) bridge for AI agents
- CLI with `parse`, `graph`, and `bridge` commands
- File watching with hot reload
