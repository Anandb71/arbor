# Arbor v3.0.3

A fix release. Every item here was a wrong answer rather than a missing
feature, and each one ships with a regression test. No breaking API changes.

Most of it came from running v3.0.0 on a mixed Rust and TypeScript codebase
and checking its answers by hand (#225).

## Rust calls were invisible

The Rust extractor recorded only bare `name()` calls. These produced no edge:

- `crate::jobs::enqueue()` and `jobs::enqueue()`
- `Type::new()` and other associated functions
- `self.method()`
- any call inside a macro argument, which includes every `assert!(...)` and
  `format!(...)`

So a function called from dozens of places could report a handful of callers,
all of them wrong. Paths now resolve through the module tree
(`SymbolTable::resolve_path`): `jobs::enqueue` means the `enqueue` defined in
`jobs.rs` or `jobs/mod.rs`, a crate-name prefix is skipped, and a path into an
external crate stays unresolved instead of binding to a local function that
happens to share its last segment. Macro token trees are read as code.

## `arbor diff` blamed new code for old callers

A change was mapped to every symbol in each touched file. Adding one function
to a busy file reported the blast radius of everything already in it, so an
additive branch could come out Critical.

`arbor diff` now works per symbol. It compares each changed file against its
previous version: modified symbols carry a blast radius and new ones don't.
Tests that call the change are listed as worth running, not counted as impact
or as entry points.

It also gained scopes:

```bash
arbor diff --base origin/main   # this branch, as its pull request shows it
arbor diff --staged             # only what is staged
```

`check` and `summary` take the same flags. Whitespace-only edits and
generated files are ignored.

## Same name, different language

`arbor callers enqueue` could list TypeScript callers for a Rust function.
Resolution now stays within a language family, and when a name matches several
definitions `callers`/`callees` answer for each one separately. Narrow a query
with `jobs::enqueue`, `Type.method` or `src/jobs.rs:enqueue`. An empty answer
now says what the graph cannot see (trait objects, callbacks, reflection)
rather than implying the symbol is unused.

## Answers from the wrong branch

The graph was treated as fresh when no source file was newer than it. After
switching branches, the files that remained were older, so the graph built on
the other branch answered. It also survived upgrades: a graph built by an older
Arbor kept giving that version's answers until some file changed.

Saved graphs now record the commit and the extractor that built them. A moved
`HEAD` or a different Arbor version triggers a refresh through the per-file
cache. A running `arbor bridge` still owns its graph; the CLI tells you when
the bridge is behind `HEAD` rather than racing it.

## Also fixed

- **Inheritance** (#191): `class Middle(Base)` now produces an `extends`
  edge, and methods a subclass inherits stay reachable. Changing a base class
  used to report zero blast radius.
- **Call cycles** (#190): a closed ring of functions no longer fills the top
  of the centrality ranking. Each cycle is ranked once, as a component.
- **Vendor assets** (#189): `vendor/` and minified files are not indexed.
- **HTTP bridge** (#222): `arbor bridge --http` rejects cross-origin and
  DNS-rebinding requests, requires JSON, and bounds request size, time and
  connections.
- **Bridge busy loop** (#174): ignored build directories no longer trigger
  re-indexing.
- **GitHub Action:** the prebuilt download looked for asset names that were
  never published, so every run compiled from crates.io instead. Pin
  `Anandb71/arbor@v3.0.3` to get the fix.

## New: receipts

`arbor receipt` explains, after each coding-agent turn, what changed in plain
English and flags anything touched that you didn't ask for.
`arbor receipt undo` puts a turn back. `arbor hook claude` wires it into
Claude Code. See [RECEIPTS.md](RECEIPTS.md).

## Upgrading

Cached graphs from 3.0.0 are rebuilt automatically on first use. JSON output
only gained fields: `arbor diff --json` adds `compared`, `modified_symbols`,
`added_symbols`, `new_symbols`, `deleted_files` and `impact.tests_exercising`,
and `arbor callers --json` adds `matches` when a name is ambiguous and `note`
when the answer is empty.

## Install

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash -s -- --version v3.0.3

# From source
cargo install --git https://github.com/Anandb71/arbor --tag v3.0.3 arbor-graph-cli
```

Other channels: [INSTALL.md](INSTALL.md).
