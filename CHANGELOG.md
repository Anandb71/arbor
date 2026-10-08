# Changelog

All notable changes to Arbor will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
