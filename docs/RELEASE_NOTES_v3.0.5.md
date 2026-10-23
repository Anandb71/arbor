# Arbor v3.0.5

A fix release. No breaking API changes. Saved graphs rebuild once on first use
(`extract-4`).

## Tests in TypeScript and JavaScript count

Every Jest, Vitest and Mocha test lives in an anonymous callback:
`describe('decide', () => { it('accepts two', () => { ... }) })`. Arbor only
gave named functions a node, so calls inside those callbacks belonged to
nothing. `arbor callers` missed every test, and `arbor diff` and `check`
reported that no test exercised a change even when one did (#235).

Each `it`, `test`, `specify` and `bench` call, and each `beforeEach`,
`afterEach`, `beforeAll`, `afterAll`, `before` and `after` hook, is now a
function named after the test, such as `it: accepts two`, that owns the calls
in its callback:

- `.only`, `.skip` and `test.each(rows)("…", fn)` are recognised.
- `describe` blocks hold tests and get no node of their own, so each test is
  counted once.
- A title used twice in one file gets its line, such as `it: works (line 8)`.
- A call without a callback (`it('todo')`) and ordinary callbacks such as
  `forEach` or `setTimeout` create nothing.

On the repro from the issue, `arbor callers decide` reported 2 callers in
3.0.4 and reports 4 in 3.0.5:

```
Callers of 'decide' (4):
  function test: rejects zero
  function it: accepts two
  function namedHelper
  function run
```

## MCP tool output uses project-relative paths

Tool responses could include absolute paths from your machine. Every file
path in tool output is now relative to the project root (#261), and
`initialize` reads the namespaced `io.modelcontextprotocol/protocolVersion`
key before the flat ones.

## Also

- The evaluation corpus has two-state fixtures, a real-repository slice and a
  second language, and records indexing time per fixture (#262, #264).
- CI installs the npm package on clean runners for each platform (#265).
