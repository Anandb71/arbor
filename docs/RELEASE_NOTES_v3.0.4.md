# Arbor v3.0.4

A fix release. Every item here was a wrong or missing answer, and each one
ships with a regression test. No breaking API changes. Saved graphs rebuild
once on first use (`extract-3`).

Most of it came from a code review that ran Arbor on a private Tauri app and
reported four problems: `arbor diff` counted 566 changed symbols for about six
edited functions, it missed the real callers of `draft.run_started(...)`, its
output was buried in warnings, and it can't find logic bugs. The last is
by design: Arbor tells you where to look, not what is wrong. Each of the other
three was reproduced on that codebase and is fixed here.

## The review was running an old binary

The machine had an old `cargo install` of **1.9.0** in `~/.cargo/bin`, earlier
on `PATH` than the npm install of 3.0.3. The shell ran 1.9.0, so the review got
its file-level diff and its warnings, about 100,000 lines while indexing.

Nothing told anyone. Now:

- `arbor doctor` lists every `arbor` on `PATH`, fails when the one that runs
  is older than another, and prints how to remove it (`cargo uninstall
  arbor-graph-cli`, `npm uninstall -g @anandb71/arbor-cli`, `brew` or `scoop`).
- The npm postinstall, `install.sh` and `install.ps1` run `arbor --version`
  the way your shell resolves it after installing, and warn if a different
  version answers.

## Calls on method receivers were dropped

`draft.run_started()` names no type, so it was resolved by method name. Once
more than two types defined `run_started`, the call was dropped as ambiguous
and `arbor callers` reported none, including for the two call sites the review
had to check by hand.

The Rust extractor now reads the receiver's type where the code states it:

- parameters, through `&`, `Box`, `Arc`, `Rc`, `Mutex`, `RwLock`, `RefCell`,
  lock guards, `Option`, `Result`, collections, channels and Tauri's `State`
- `let x: T`, `T::new()` / `default()` / `from()` (with `?` or `unwrap`),
  struct literals and `app.state::<T>()`
- `if let Some(x)`, `let Some(x) = … else`, and `match` arms through nested
  `Ok`/`Some`
- typed closure parameters and `self.field`
- the return type of the function or method that produced the value, and
  `.await`

Struct nodes record their field types and functions their return types, so
`state.draft.lock().unwrap().run_started()` or `let Some(d) = prepare(h) else`
resolve even when the struct or the helper lives in another file. A trait
type (`dyn Observer`, `T: Observer`) reaches each implementor. A type the
project doesn't define is external, so `client.get()` no longer binds to a
project's own `get`. Anything uncertain falls back to name-based resolution.

On the reviewed codebase the method went from no callers to all three.

## `arbor diff` counted what didn't change

Three more sources of inflated counts, measured on eight pull requests of that
codebase:

- **Containers.** Editing a method made the class around it, or the `mod`
  block around a function, "modified" too. A module, class, trait, struct or
  enum now counts only when a changed line in it falls outside all of its
  members.
- **Comments and attributes.** A hunk that adds a doc comment, a `#[test]` and
  a blank line touched lines outside the new function. Blank lines and comments
  no longer count, and an attribute or decorator counts as a change to its item.
  A hunk that removes code still counts in full, so commenting code out is a
  change.
- **Callees.** Impact also included everything the edited code calls. Editing
  a function can't break what it calls, so impact, files likely to need updates
  and entry points now come from callers alone.

| Pull request | 1.9.0 (modified / impacted) | 3.0.3 | 3.0.4 |
|---|---|---|---|
| A (3 files) | 379 / 1,046 | 6 / 58 | **2 / 13** |
| B (12 files) | 844 / 3,309 | 11 / 464 | **10 / 321** |
| C (15 files) | 287 / 2,065 | 9 / 944 | **8 / 3** |
| D (35 files) | 1,698 / 4,343 | 47 / 1,979 | **39 / 371** |

Pull request A was checked by hand: one new test and two edited functions,
which is exactly what 3.0.4 reports.

The report now also says *which* symbols: `diff` and `check` list each
modified symbol with its file, line and direct caller count, most-called
first, in text, `--markdown` and `--json` (`modified`).

## Quieter output

One-shot commands (`diff`, `index`, `callers`, …) log warnings only. Servers
(`serve`, `bridge`, `viz`, `gui`) keep their lifecycle lines. `-v` shows debug
output, `RUST_LOG` overrides both, and logs always go to stderr.

## Also in this release

- **Linux binaries run on older distributions (#242):** built in
  manylinux_2_28, so they need glibc 2.28 rather than 2.39.
- **First run without git or a repository:** commands that need git say which
  of git, the folder or git's ownership check is the problem, before touching
  the index.
- **`arbor hook claude`** no longer replaces a `CLAUDE.md` or
  `.claude/settings.json` it can't read, warns when receipts can't work, and
  read-only projects get a path and a fix instead of "Access is denied".
- **`.arbor/` stays out of git (#233)** through its own `.gitignore`.
- **Receipts** no longer miss an edit made right after the snapshot on Linux
  (#246).
- **Upgrades** no longer fail with a sled lock error while clearing an
  outdated cache (#251).
- **MCP:** malformed protocol versions are rejected with `-32602`, HTTP
  connections no longer share negotiation state, stdio answers malformed input
  with `-32700`, and only implemented extensions are advertised (#254, #256).
- **WebSocket servers refuse foreign origins:** the RPC and sync servers behind
  `arbor bridge` and `arbor serve` now apply the MCP HTTP transport's
  `Origin`/`Host` policy.
- **Supply chain:** `SHA256SUMS` and build provenance for every archive, npm
  provenance and a signed GHCR attestation (#243, #247). Every binary runs a
  first-use smoke test on its platform before publishing, and installers are
  exercised on Linux, macOS and Windows in CI (#255).
- **Accuracy corpus:** a versioned evaluation corpus checks recall and
  precision against hand-derived truth (#257).

See the [changelog](../CHANGELOG.md) for details.

## Upgrading

```bash
npm install -g @anandb71/arbor-cli    # or: cargo install arbor-graph-cli
arbor doctor                           # confirms the arbor on PATH is 3.0.4
```
