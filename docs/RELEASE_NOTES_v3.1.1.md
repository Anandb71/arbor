# Arbor v3.1.1

A fix release. The model-retirement data behind `arbor retirements` was read
again on 28 October 2026 and now lists the bare `gpt-5` and `o3`; `npx` works on
npm 12; and a folder under your home folder is no longer resolved to the whole
home folder. No breaking changes. As after every upgrade, a saved graph is
rebuilt once on first use (`extract-6`).

## `arbor retirements`

### Bare `gpt-5` and `o3`

Between 23 and 26 October 2026, OpenAI's deprecations page started listing bare
IDs and dated snapshots together in its rows (`gpt-5` | `gpt-5-2025-08-07`).
3.1.0's data was built before that, so `model="gpt-5"` or `model="o3"` was not
listed at all. 3.1.1 has the 15 IDs the page added:

- `gpt-5`, `gpt-5-mini`, `gpt-5-nano`, `gpt-5-pro`, `o3` and `o3-pro`:
  11 December 2026, to GPT-5.6
- `gpt-image-1.5-2025-12-16`: 1 December 2026
- `gpt-4o-mini-tts`: 6 January 2027
- four audio and realtime snapshots: 20 January 2027
- `gpt-4o-mini-transcribe-2025-12-15`: 26 February 2027
- `gpt-5.1-2025-11-13` and `gpt-5.4-nano-2026-03-17`: 1 April 2027

Like the snapshots already listed for those dates, the 15 come with the date and
the replacement OpenAI lists (several models for `gpt-image-1.5-2025-12-16` and
`gpt-4o-mini-transcribe-2025-12-15`) but no argument advice, so each one is a
manual check.

### The data was read again on 28 October 2026

Every provider page behind the bundled data was read and checked again on
28 October 2026, and the data was regenerated from it. Each report now says
"OpenAI, Anthropic, Google and Google Cloud as of 28 Oct 2026" (3.1.0: OpenAI
18 October, Anthropic and Google 16 October). The data has 308 entries
(297 model IDs). What changes in the report:

- **The bare 14 January aliases.** OpenAI's deprecations page lists five dated
  snapshots for shutdown on 14 January 2027: `gpt-4o-2024-08-06`,
  `gpt-4o-2024-11-20`, `gpt-4o-mini-2024-07-18`, `gpt-4.1-2025-04-14` and
  `gpt-4.1-mini-2025-04-14`. It gives the bare `gpt-4o`, `gpt-4o-mini`,
  `gpt-4.1` and `gpt-4.1-mini` no row. For those, the report cites OpenAI's
  [API reference](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create),
  which lists each as "Deprecated. Announced shutdown date: 2027-01-14", and
  OpenAI's models API, which returned that date on 18 October 2026.
- **Claude Haiku 4.5 and sampling.** A call whose listed replacement is
  `claude-haiku-4-5-20251001` and that sends `temperature`, `top_p` or `top_k`
  was kept with "accepts temperature, top_p and top_k". It is now a check:
  Anthropic's API version 2026-10-21, sent by `anthropic` 2.0.0 and
  `@anthropic-ai/sdk` 1.0.0 and later, rejects them on every model.
- **GPT-5.6 checks say what was tested.** `stop`, `logit_bias` and `max_tokens`
  say that gpt-5.6-terra was the only GPT-5.6 model called, and `stop` and
  `logit_bias` read `unverified` on the other tiers. The sampling check for a
  reasoning model moving to GPT-5.6 (such as `o3-mini` with `temperature`)
  gives the live result it rests on.
- **gpt-6-luna on the Responses API.** The sampling check quotes OpenAI's
  "Using GPT-6" guide, so a Responses API call with `temperature` reads
  `check temperature` on both options. Arbor has not tested this with a live
  call.
- **gpt-6-luna's tier note** quotes OpenAI ("most efficient model for focused,
  high-volume tasks") instead of calling GPT-4o and GPT-4.1 flagship models.
- **Fine-tunes.** OpenAI removed its separate table of fine-tune replacements,
  so fine-tunes of `gpt-3.5-turbo`, `gpt-4` and `gpt-4.1-nano` no longer name a
  `gpt-5.6-*` model as their listed base. Fine-tunes of `babbage-002` and
  `davinci-002` follow their base model's date, 28 September 2026.
- **Google.** Ten retired Veo 2.0 and Imagen models on Google Cloud are added,
  Veo 3.0 is confirmed retired, four Gemini API Veo 3.1 and omni previews are
  marked shut down, `deep-research-pro-preview-12-2025` is unconfirmed (past
  its earliest date, but not greyed out on the page), and
  `gemini-2.5-flash-preview-09-2025` is a swap to its listed replacement.

### No more "(no change)" next to arguments that need reasoning off

When Arbor could not resolve a call's client (a package's subclass of
`openai.OpenAI` exported under the same name, a client passed in as a
parameter or built by a factory), the check dropped the call's arguments. A
`temperature`, `tools` or `max_tokens` went unchecked, and "let the model
reason" still read `(no change)`, although on gpt-6-luna `temperature` and
`tools` return 400 unless the call sends `reasoning_effort="none"`, and
`max_tokens` returned 400 at every effort the live checks tried (#290).

Those arguments are now checked as for any other call, and an option that used
to read `no change` now says what still needs a look:

- Arguments Arbor cannot read, from a `**` expression such as `**kwargs` or
  sent in `extra_body`, make each option say
  `could not check the arguments from **kwargs` (or `in extra_body`), and a
  check line asks you to look at them. A dict literal in the same file spread
  in with `**` is read; a change made to it from another module is not seen.
- An argument a check line names is named on each option: `check stop` where
  the line says it fails or changes on the replacement, and
  `could not check temperature` where nothing verified it for that API, such
  as sampling arguments in a Responses API call moving to a GPT-5.6 model.

`--json` carries the same as `unchecked_args`, `options[].check` and
`options[].unverified`, and `arbor usages` records the argument names of an
unresolved client's call (names only, never values).

### On a call with `max_tokens`, `temperature` and `tools`

A scratch repository with one `client.chat.completions.create(...)` call to
`gpt-4o-2024-08-06` that sends `max_tokens=300`, `temperature=0.2` and
`tools=tools`, run with this release's build on 30 October 2026:

```text
$ arbor retirements .
Model retirements in . (1 Python file checked)

1 call uses a model OpenAI shuts down on 14 Jan 2027 (76 days)
  app.py:7  gpt-4o-2024-08-06 (line 8) → gpt-6-luna
            also change: max_tokens → max_completion_tokens
            decide: keep reasoning off (add reasoning_effort="none"; keep temperature, tools) or let the model reason (remove temperature, tools)
  decide once for this repo (Arbor does not choose): gpt-6-luna runs at medium reasoning effort by default, so it can reason; the model it replaces answered without a reasoning step.
    ...
  note: If gpt-6-luna reasons, max_completion_tokens also bounds its reasoning tokens, so a small limit can be used up before any visible text.
  note: OpenAI describes gpt-6-luna as its "most efficient model for focused, high-volume tasks", so answer quality may change compared with the model it replaces.

Summary: 1 call in 1 file uses retired or retiring models: 1 retiring (next: 14 Jan 2027, 76 days).
1 can move once this repo makes the choice shown under decide.
Retirement data from the providers' deprecation pages: OpenAI, Anthropic, Google and Google Cloud as of 28 Oct 2026. Days are counted in UTC. Python and direct SDK calls only for now. Runs locally: no code leaves this machine.

Want this call ready before 14 Jan 2027? Ask for a paid readiness review: we write the fix pull requests with you and list every call we find but can't check.
https://getarbor.dev/retirements?utm_source=arbor-cli&review=1&calls=1&retired=0&next=2027-01-14
Arbor has sent nothing. From this run, opening the link shares only how many calls are listed, how many are already retired and the next shutdown date: no code, paths, file names or model names. Your email reaches Arbor only if you submit the form there.
```

Arbor changes no code: the report says what to change, and the repository
makes the choice under *decide*.

### The readiness-review line

When the report lists at least one call to a retired or retiring model, its
closing line is now an offer of a paid readiness review, with a link (the last
three lines above). The link is built from the summary alone and carries three
values from the run: the number of calls listed, how many of them are already
retired and, when a call is retiring, the soonest shutdown date
(`&review=1&calls=1&retired=0&next=2027-01-14`). It never carries code, paths,
file names, repository names or model names.

- **Terminal only.** The offer is printed only in the human report. It is not
  printed with `--json`, `--markdown`, `--annotations` or `--quiet`, or when
  the `CI` environment variable is set (GitHub Actions sets it), and the file
  `--markdown-out` writes does not have it. The GitHub Action's summary and
  annotations do not show it.
- **Nothing is sent by Arbor.** `arbor retirements` still makes no network
  requests and has no telemetry. Opening the link is up to you: it sends those
  three values to getarbor.dev like any page visit, and your email only if you
  submit the form there.
- Other findings keep the fix-PR line with its plain link, and a clean report
  has no offer.

## `npx` on npm 12

npm 12 runs install scripts only for packages its `allow-scripts` setting
names, so the 3.1.0 package never downloaded its binary and
`npx @anandb71/arbor-cli` stopped with "Arbor binary is not installed yet."

From 3.1.1, `npx @anandb71/arbor-cli retirements .` works on npm 12 without
`--allow-scripts`. When the install script did not run, the first `arbor` run
downloads the release archive for your platform from the GitHub Release that
matches the package version, checks it against that release's `SHA256SUMS`
before extracting anything, and takes only the binary out of it. A second run
downloads nothing. On npm 10 and 11 the install script does the same download,
and a failure there no longer fails `npm install`.

- Progress goes to stderr, so `--json` output is unaffected.
- A failed download (offline, DNS, an untrusted TLS certificate, a proxy, an
  HTTP error, a checksum mismatch) exits 2 with the reason and the manual
  steps.
- Behind a proxy, Node.js 22.21+ and 24.5+ use `HTTPS_PROXY` and `NO_PROXY`, or
  npm's `https-proxy` and `noproxy` settings.
- `ARBOR_NPM_DOWNLOAD_BASE` points the download at a mirror that holds the
  release archives and `SHA256SUMS`, and `ARBOR_NPM_CACHE_DIR` sets the folder
  used when the package's own folder is not writable.

See [docs/INSTALL.md](INSTALL.md).

## The home folder is no longer taken as the project

`.arbor` marks a project, and `~/.arbor` exists for reasons that have nothing to
do with one: it holds the global config, `install.ps1` installs the binary into
`~/.arbor/bin`, and `arbor init`, `index` or `setup` run in the home folder
create it. So a command run in a folder with no nearer marker (no `.git`,
`Cargo.toml`, `package.json` and so on) used the home folder as the project, and
`arbor index .` indexed all of it (#273).

The search for the project now stops at the home folder. The home folder is the
project only when Arbor runs in it or is given its path, and the search never
goes above it. With no marker between a folder and home, the folder itself is
the project. Junctions, symlinks, 8.3 short names and network-share spellings of
the home folder count as the home folder too.

## Code graph

- **Aliased imports (#258).** After `from money import money_to_minor_units as
  to_minor`, a call to `to_minor()` recorded no edge, or was bound to an
  unrelated function named like the alias. Python's `from X import Y as Z` and
  TypeScript and JavaScript's `import { Y as Z }` now record the imported name,
  so the call reaches `Y` in the module the import names.
- **Bare cross-file calls (#177).** A bare call could bind to a same-named
  definition in any other file: a project's own `len` picked up the builtin
  `len(...)` calls of every other file. In Python a bare call to another file
  now counts only when the calling file imports the name. In JavaScript and
  TypeScript, `require`, `import x = require()` and an awaited `import()` count
  as imports too, and a module's private function is no longer reached through
  a global name. In Go a bare call stays inside its package.
- **`arbor refactor` (#195)** exits 1 and prints
  `error: Symbol '<name>' not found` for a missing symbol, like `inspect` and
  `callers`.

## Also

- `Cargo.lock` moves `rustls-webpki` to 0.103.16 (RUSTSEC-2026-0382) and
  `webpki-roots` to 1.0.10 (RUSTSEC-2026-0381). Only `arbor-gui` uses them; the
  `arbor` CLI does not.
- [Live checks, 18 October 2026](LIVE_CHECKS_2026-10-18.md) lists each live
  API call behind a rule marked `live`, with its request, status and error
  message.

## Upgrade notes

- **npm.** `npx @anandb71/arbor-cli@3.1.1 retirements .` works on npm 12 with
  its defaults. The first run needs to reach GitHub Releases (or your mirror);
  the binary is checked against `SHA256SUMS` before it is used.
- **GitHub Action.** `uses: Anandb71/arbor@v3.1.1`. The Action installs the
  latest release by default (`arbor-version: latest`), so a workflow that pins
  `@v3.1.0` runs the 3.1.1 binary once this release is out; set `arbor-version`
  to pin the binary. With 3.1.1's data the report lists calls that 3.1.0 did
  not, such as bare `gpt-5` and `o3` (11 December 2026), so a workflow that
  sets `retirements-fail-on` can fail where it passed before.
- **Saved graphs** are rebuilt once on first use (`extract-6`).
- **The home folder.** If an earlier run indexed your whole home folder, its
  graph and cache are still in `~/.arbor`, next to the global `config.json`
  (and, on Windows, `bin/`). 3.1.1 leaves them alone; removing them is up to
  you. The same goes for `arbor hook claude .` run in a folder under home with
  no nearer marker: it wrote `~/CLAUDE.md` and `~/.claude/settings.json`,
  Claude Code's user settings.

## Known limits

- **Python only, direct SDK calls only.** The `openai`, `anthropic`,
  `google-genai` and `google-generativeai` SDKs. Calls through LangChain,
  LiteLLM's own API, LlamaIndex or other wrappers are not checked yet, and
  model IDs in `.env`, YAML, JSON or notebooks are not read.
- **The 15 IDs OpenAI added** have a date and the replacement OpenAI lists,
  but no argument advice yet.
- **Live checks** are those of 18 October 2026, on Chat Completions only. This
  release adds none. On the Responses API, gpt-6-luna's sampling check rests on
  OpenAI's guide, and for a call moving to a GPT-5.6 model it is `unverified`.
- **Anthropic API version.** Arbor does not read which `anthropic` SDK version
  a project pins, so Claude's sampling check names both API versions
  (2023-06-01 and 2026-10-21).
- **The bare OpenAI aliases** have no row on OpenAI's deprecations page; their
  date comes from OpenAI's API reference and models API.
- **In the home folder itself**, `arbor index .` still indexes all of it, as
  before.

## Install

```bash
npx @anandb71/arbor-cli retirements .   # no install
npm install -g @anandb71/arbor-cli
cargo install arbor-graph-cli --version 3.1.1
```

Binaries for Linux, macOS and Windows are attached to the GitHub release, with
`SHA256SUMS`. See [docs/INSTALL.md](INSTALL.md) for Scoop, Homebrew and the
container image.

The previous release is [v3.1.0](RELEASE_NOTES_v3.1.0.md).
