# SDK usages: what a report confirms

`arbor usages` and the MCP tool `find_api_usages` list where a Python,
TypeScript or JavaScript project calls the OpenAI, Anthropic and Gemini SDKs,
with each call's routing and model values. A consumer (a codemod, a reviewer) should treat the report this way:

- **A value or route with no gap at its call is confirmed.** It comes from one
  of the shapes in the supported grammar below, and nothing in the file can
  change it at run time.
- **Anything else carries a gap.** The gap's `reason` says why, and the call
  needs a person to confirm it first. A gap may still list the value the code
  names (a parameter default, an environment fallback, a class attribute that
  something else may overwrite), so it can be proposed for review. A call
  whose client is not resolved (`unknown_client_factory`) also lists its
  argument names in `args` (names only, never values, and `splat` when some
  come from `**`), so its arguments can be checked like a resolved call's.
- **A `**` expression is listed in `spreads`**, on a call and in a gap's
  `args`: its `name` as written (`PARAMS`, `kwargs`, `self.options`; omitted
  for a dict written inline, a conditional, or a credential-shaped name), the
  argument names it sends (`keys`) when it is a dict literal in the same file,
  and `complete` when those are all of them (no `**` or computed key inside,
  every key recorded, and the dict is never changed or passed on later in that
  file; a change made to it from another module is not seen). Names only,
  never values: a secret-sounding name such as `max_tokens` is listed, a key
  that is not an identifier or is shaped like a credential is not, and the
  dict is then not `complete`.

Python is too dynamic to resolve every shape soundly. Rather than adding a
special case for each one, the extractor resolves a small grammar exactly, and
turns everything outside it into a gap. That includes a list of dynamic
features that can change a resolved name, class or dict. A wrong value with no
gap inside this grammar is a bug. A shape outside it is reported as a gap.

The sections up to [TypeScript and JavaScript](#typescript-and-javascript)
describe the Python extractor; that section states the TypeScript one, which
keeps the same contract.

## Supported grammar

### Model values

These are reported with no gap (`model`, `engine`, `deployment_id`,
`model_name`):

1. **A string literal at the call.** This covers a keyword argument, the
   positional model argument (`GenerativeModel("gemini-pro")`), and a dict or
   list literal written in the call (`**{"model": ...}`,
   `extra_body={"fallbacks": [{"model": ...}]}`). A literal written at the call
   is never downgraded, even when a dynamic feature applies to the file.
   - An entry of a dict literal counts only when no `**` expansion and no
     computed key follows it in that literal. `{"model": "gpt-4o",
     **overrides}` is a `splat` gap that lists `gpt-4o` as a candidate;
     `{**base, "model": "gpt-4o"}` is confirmed. The same holds for
     `dict(model=..., **kw)` and for nested dicts in `extra_body`.
   - A key written twice counts once, by its last entry, as Python does.
2. **A name with one reaching assignment.** The assignment is a string literal,
   or another name or attribute that resolves under these rules. The name is a
   module constant, a function local or a local of an enclosing function.
   - The reaching assignments of a use are: the nearest assignment that
     dominates it in straight-line code; every assignment between that one and
     the use; and assignments later in a loop that runs the use again.
   - A use inside a nested function, lambda or generator also sees every
     assignment after its definition, because it may run after them.
   - `global` and `nonlocal` writes from other functions reach every use.
   - Loop targets, `with ... as`, `except ... as` and `except* ... as`,
     `match` captures, walrus targets, `globals()["NAME"] = ...`, a wildcard
     import from another module and `del NAME` are all assignments. So are a
     `def` or `class` of the name, an import of it (in a class body too),
     `type NAME = ...`, and `global` or `nonlocal` in a class body.
   - A walrus in a default argument (`def f(a=(M := ...))`) binds where the
     `def` or `lambda` runs, in the enclosing scope.
   - When more than one assignment reaches the use, the call gets a
     `reassigned` gap.
3. **A class attribute assigned once.** It is read by class name (`Bot.MODEL`)
   or through `self`/`cls` in the class's methods, and assigned once: in the
   class body, by `self.x = ...` or `cls.x = ...`, or by `Bot.x = ...`.
   - `self` and `cls` are the class only as the first parameter of a method
     written directly in its body (a `lambda` in the body counts only when
     the body assigns it to a name, `go = lambda self: ...`), and in the
     closures nested in it. A `staticmethod` parameter named `self`, a
     nested `def inner(self)`, a lambda passed in a class body
     (`map(lambda self: ..., items)`) and a method another class borrows
     (`B2.go = B.go`, `go = B.go`) do not count; a write through such a
     method's `self` is a write on another object.
   - A method that binds the name of its `self` or `cls` parameter in any
     other way, in its body or in any scope nested in it, reads nothing
     through that name: an assignment, `+=`, a `for`, `with`, `except` or
     walrus target, a `match` capture, `del`, an import, a `def` or `class`,
     and a closure's `nonlocal` or `global`. Every read through the name in
     that method is a `dynamic_feature` gap, and every write through it is a
     write on another object (below).
   - Read through `self` or `cls`, a value of a **public class** keeps a
     `subclass_override` gap, the way a public function's default keeps
     `parameter_default`: another module may subclass the class and set the
     attribute. A class is public when it is not `_private`, not inside a
     private class or a function, and not in a script. A `_private`
     module-level class that `__all__` lists, or any one when `__all__` is
     built from names (rule 4), counts as public, and so does a private class
     one of whose direct same-file subclasses is public. Reads by class name
     are confirmed.
   - Every binding of a name in a class body is an assignment of that class
     attribute, whatever else the class assigns: `=`, an annotation, `+=`,
     `def`, `async def`, `class`, an import, a `for`, `with` or `except`
     target, a walrus, `del` and a `match` capture. This holds for the
     reading class, its base and its subclasses alike: `class
     _Fast(_Base): from cfg import model` makes `self.model` in `_Base` a
     `reassigned` gap. Only a class's single literal assignment is
     confirmed.

   The class must meet all of these:
   - It has no decorator, metaclass or other class keyword, and at most one
     base (`object` and `ABC` do not count). That base is either a same-file
     class with no same-file base of its own, or a class from another module.
     With a base from another module, only `self.x = ...` assignments count: a
     class-body attribute there may be a field (see the dynamic features).
   - Through `self`, the instance may also be one of at most 16 direct
     same-file subclasses, none of which has subclasses, a decorator or more
     than one base. A subclass's own assignment of the attribute is one more
     reaching assignment, so the call gets a `reassigned` gap.
   - No dynamic feature below applies to the attribute.
4. **A parameter default, when every call of the function is in this file.**
   That holds for a function nested in another function, a `_private`
   function or method (not a dunder), the `__init__` of a private class, and
   any function in a script (an extensionless file or `__main__.py`). The
   function must also be undecorated and never passed, stored or rebound, and
   every in-file call must pass nothing for the parameter, or the same value.
   A `_private` function listed in `__all__` (assigned, extended or appended)
   is exported. An `__all__` built from names (`__all__ = _EXPORTS`,
   `__all__ += names`, `__all__.extend(names)`) may list any module-level
   name, so every `_private` module-level function and constructor counts as
   exported. A `__mangled` method called from outside under its mangled name
   (`_A()._A__go(...)`, `getattr(_A(), "_A__go")`) has callers this file
   does not show. All of these are treated as passed (`unseen_callers`).
   Otherwise the default is still reported, with one of these gaps:
   - `parameter_default`: the function is public, so another module may pass
     another value.
   - `unseen_callers`: the function is decorated, passed or stored.
   - `parameter_overridden`: an in-file call passes another value.
5. **An entry of a same-file dict literal read by a literal key**
   (`MODELS["fast"]`, `MODELS.get("fast", ...)`, `**PARAMS`). The dict's name
   must never be mutated, assigned to another name, stored in a container or
   an attribute, passed to anything but a read-only builtin (`len`, `print`,
   `json.dumps`...), or returned. A part that hands out a nested container
   counts the same way. As in rule 1, the entry counts only when no `**`
   expansion and no computed key follows it in the literal (the
   defaults-then-overrides idiom `params = {"model": "gpt-4o",
   **overrides}` is a `splat` gap), and a repeated key counts by its last
   entry.

The fallback of an environment or configuration lookup
(`os.getenv("MODEL", "gpt-4o")`) is captured with origin `env_default` and
always keeps its gap.

### Routes

A call's routing is confirmed when its receiver reaches it through the
value rules above (one kind of client per name), or through a parameter that
every in-file caller of a private function passes. The receiver must be one of
these:

- **A client from a tracked constructor** (`OpenAI`, `AzureOpenAI`,
  `Anthropic`, `AnthropicBedrock`, `AnthropicVertex`, `genai.Client`...). It
  must have no `base_url`, `base_url=None`, or a `base_url` that resolves to a
  string under the value rules, given as a keyword or as a dict entry that no
  later `**` expansion or computed key may replace. A client built or copied
  from an expansion that may hold a `base_url` routes `unknown` with a
  `base_url_unresolved` gap.
- **A Gemini client** (`genai.Client`) also needs no `vertexai=True`, no
  `http_options` that names a `base_url` or expands anything, and no `**`
  expansion. Any of those may send it to Vertex AI or another endpoint, which
  retire models on another schedule, so it routes `unknown` with an
  `endpoint_option` gap. `vertexai=False` and `HttpOptions(api_version=...)`
  stay confirmed. The legacy SDK's `configure(client_options=...)` or
  `configure(**options)` gives its calls the same gap, under any name the
  file calls it by (`from google.generativeai import configure as setup`,
  `cfg = genai.configure`).
- **The module-level API** (`openai.ChatCompletion.create`,
  `openai.chat.completions.create`). Its routing comes from the file's
  routing settings: `openai.api_type`, `openai.api_base`,
  `openai.base_url`, `openai.azure_endpoint`, `openai.azure_ad_token`,
  `openai.azure_ad_token_provider` and `openai.bedrock_*`. They count with
  no gap only when every one of them is in the file's **top-of-file setup
  block**:
  - The setup block is the module's statements from the top up to, not
    including, the first statement that is none of these: an `import` or
    `from ... import`, a string (the docstring), or an assignment (`=`,
    annotated or `+=`) with no `lambda`, comprehension, generator
    expression, walrus, `yield` or `await` in it. A `def`, a `class`, a
    decorator, a bare expression or call (`load_dotenv()`, `main()`,
    `setattr(...)`) and any compound statement (`if`, `try`, `for`,
    `while`, `with`, `match`) end it.
  - Each routing setting is a plain `=` statement of its own in the block
    (not `+=`, `setattr` or `del`), on a name that only `import` statements
    bind (`import openai as oai` counts; `o = openai` does not), and is
    assigned once.
  - A module-level call that runs at import inside the block
    (`R = openai.chat.completions.create(...)`) comes after every setting.

  Before the block ends, the file has defined no function, class or lambda,
  so none of its own code can run before the configuration is complete. The
  report does not follow import-time execution any further than that.

  Anything else gives every module-level call of the file a `module_config`
  gap that keeps the routing the settings suggest: a setting in a function,
  in an `if`, loop, `try`, `with` or class body, after the setup block, by
  `+=`, `setattr` or `del`, through an alias, or assigned twice; a routing
  attribute written on an object the file cannot identify; and an attribute
  written with a run-time name on the module or on such an object. A file
  that configures nothing is reported direct (see
  [Outside the file](#outside-the-file)), except that a call passing
  `engine` or `deployment_id`, which Azure deployments take, routes
  `unknown` with a `module_config` gap.

A client read through `self` (`self.client`) is confirmed under rule 3
without the `subclass_override` gap that a model value read the same way
gets. That is an assumption of the report: a subclass in another module
that replaces a public class's client is not seen.

An OpenAI or Anthropic client's `base_url` routes by its host (a Gemini
client's endpoint options are a gap, above). The URL itself is never
reported.

| `base_url` | routing | gap |
|---|---|---|
| absent or `None` | `openai_direct` / `anthropic_direct` | none |
| the provider's own host (`api.openai.com`, `api.anthropic.com`) | direct | none |
| any other host | `custom_base_url` | none |
| an environment lookup whose fallback is the provider's host | direct | `base_url_env` |
| an environment lookup with no fallback | `unknown` | `base_url_env` |
| an environment lookup with another host as its fallback | `custom_base_url` | `base_url_env` |
| anything that does not resolve to a string | `unknown` | `base_url_unresolved` |

The file must not do any of these:

- set the SDK's endpoint variable (`endpoint_env`): `OPENAI_BASE_URL`,
  `ANTHROPIC_BASE_URL`, `GOOGLE_GENAI_USE_VERTEXAI`,
  `GOOGLE_GEMINI_BASE_URL` or `GOOGLE_VERTEX_BASE_URL`, and for the
  module-level API also `OPENAI_API_BASE` and `OPENAI_API_TYPE`;
- write the environment with a name known only at run time (a
  `dynamic_feature`, below);
- assign a `base_url` to the client after it is built (`base_url_assigned`), or
  to an object the file cannot identify (`dynamic_feature`);
- replace an SDK member rather than configure it
  (`client.chat.completions.create = patched`, `openai.OpenAI = Sub`,
  `client.chat = ...`), which makes every route of that SDK a
  `dynamic_feature` gap. Settings such as `openai.api_key` or
  `client.max_retries` change nothing.

## Dynamic features

The collection pass looks for the following features in the same linear walk
that records bindings. When one can affect a resolved name, class or dict, the
value or route gets a `dynamic_feature` gap that names the feature and its
line. When it affects the whole file, every value not written at its call and
every route gets that gap.

**Whole file:**
- `exec`, `eval` or `compile`;
- `globals()`;
- `locals()` or `vars()` at module or class level, where it is that namespace
  (a function's `locals()` cannot rebind its names);
- `vars(obj)` or `x.__dict__`;
- `sys.modules`, also through `from sys import modules`;
- `import __main__`, or the module importing itself: `__import__(__name__)`,
  `importlib.import_module(__name__)`, or an import of its own module name
  (`import app` in `app.py`; not in a package's `__init__.py`, a
  `__main__.py` or a script, and not when the name is a tracked SDK's);
- `builtins` or `sys` imported under another name (`import sys as S`), used
  as a value (`s = sys`, `f(builtins)`), or imported by name
  (`__import__("builtins")`, `importlib.import_module("sys")`);
- a frame's namespace: `f_globals`, `f_locals` or `f_builtins`
  (`inspect.currentframe().f_globals[...]`, `sys._getframe(1).f_locals`);
- scopes (functions, classes, lambdas, comprehensions) nested more than 64
  deep. They are not analyzed, so what they bind or write is not seen; each
  is also a `truncated` gap.

These builtins count under any name the file reaches them by: aliased
(`run = exec`, `g = globals`), through `builtins` or `__builtins__`
(`builtins.exec`, `getattr(builtins, "exec")`, and `builtins` itself under
another name, above), or imported from `builtins`. A parameter or keyword
argument that the file names `eval` is not the builtin. A module imported
with a name known only at run time is an assumption of the report (see
[Outside the file](#outside-the-file)).

**Class attributes, read by class name or through `self`/`cls`:**
- a class decorator, a metaclass or other class keyword, or more than one base,
  on the class, on its base or on a subclass;
- inheritance more than one same-file level deep;
- a base name that more than one same-file class has;
- more than 16 direct subclasses;
- an attribute hook anywhere in the file (`__getattr__`, `__getattribute__`,
  `__setattr__`, `__delattr__`, `__init_subclass__`);
- a write of that attribute name on any object other than `self`, `cls` or the
  class itself (`obj.model = ...`, `setattr(obj, "model", ...)`,
  `configure(Settings)` writing `cfg.MODEL`), including one through a call
  or `__class__` (`type(self).model = ...`, `self.__class__.model = ...`),
  and one through the `self` or `cls` of a method that rebinds it (rule 3);
- a write whose attribute name is known only at run time (`setattr(obj, name,
  v)`, `delattr`, `obj.__setattr__(name, v)`): on `self` of the class or its
  base, or on any other object (then it affects every class);
- the class name used as a value: assigned, stored, passed (other than to
  `isinstance`, `issubclass`, `type`, `super` and printing), or returned;
- a method or property with the attribute's name;
- an attribute only subclasses assign;
- a class attribute of a class with a base from outside the file. A dataclass,
  pydantic, attrs or `NamedTuple` field may be set by the constructor, and an
  enum member is rewritten by its metaclass.

**Constructors:** an instance built without naming its class (`type(self)(...)`,
`self.__class__`, `cls(...)`). Then no class's constructor calls are all seen,
so a model default of `__init__` keeps an `unseen_callers` gap.

**The environment:** any of these may set the endpoint variable, so a direct
client built without a `base_url`, and the module-level API, route `unknown`
with a gap:
- `os.environ[key] = v` with a key that is not a literal;
- `os.environ |= {...}`;
- `os.environ.update(...)` with anything but literal keys;
- `os.environ.setdefault(key, v)`, `os.environ.__setitem__(key, v)` or
  `os.putenv(key, v)` with a key that is not a literal;
- `os.environ` aliased, passed, updated in place or replaced
  (`os.environ = {...}`, `setattr(os, "environ", {...})`).

`os.environb` counts as `os.environ` throughout: on POSIX it shares its data
with it. Writes are matched by the attribute name `environ` or `environb`
(and `putenv`) on any object, so `import os as _os; _os.environ[...] = ...`
counts. Looking `os`, `os.environ` or `os.environb` up by name
(`importlib.import_module("os")`, `getattr(os, "environ")`) and importing
`environ`, `environb` or `putenv` under another name (`from os import
environ as E`) alias the process environment. A bare `environ` or `environb`
counts only when it is imported from `os`; a WSGI `environ` parameter does
not count.

**Dicts and lists:** a container reached through more than its one name is not
read at all. That covers an alias, a container holding it, a loop over a tuple
of them, a mutating method, an item assignment, `|=` or `+=`. The call gets a
`splat` or `subscript` gap, and none of its candidates are listed as complete.

## `values_complete`

A gap may list the values the key can take, for triage. `values_complete` is
`true` only when all of the following hold:

- the list comes from one of these sources:
  - the branches that assign one name in its binding scope;
  - a literal list, tuple or set a loop runs over, directly or through a name
    bound once and never aliased, passed or mutated;
  - the two sides of a conditional;
  - the entries of a same-file dict looked up with a run-time key;
  - what every call of a private function passes for a parameter (rule 4),
    when every call is seen;
- every candidate resolved with no caveat, and no limit cut the list;
- no candidate comes from a write in another scope (`global`, `nonlocal`), or
  from an attribute assigned in more than one scope;
- no dynamic feature applies. In a file with a whole-file feature, the list
  counts only when every candidate is written at the call.

A comprehension in a class body runs its first iterable in the class scope, so
its items are not listed.

## Outside the file

Resolution is per file. Writes in other modules are not seen: another
module may assign this module's constants or write a class's attributes from
outside. Two kinds of outside change are part of a module's interface, so
they keep a gap instead: a public function's parameter default
(`parameter_default`, rule 4), and a model value a public class reads through
`self` or `cls`, which a subclass may override (`subclass_override`, rule 3).

The report assumes the rest. In particular:

- A client a public class reads through `self` is confirmed, although a
  subclass in another module may replace it (see [Routes](#routes)).
- The process environment at run time is not known: a direct client built
  without a `base_url` is reported direct, although the deployment (or a
  `.env` file loaded with `load_dotenv`) may set `OPENAI_BASE_URL`,
  `ANTHROPIC_BASE_URL` or `GOOGLE_GENAI_USE_VERTEXAI`.
- The module-level API's state is process-wide: in a file that does not
  configure it, a call is reported direct, although another module may
  configure it. A call that passes `engine` or `deployment_id` is the
  exception, with a `module_config` gap.
- A module imported with a name known only at run time
  (`importlib.import_module(name)`), or through an importer the file binds
  under another name (`from importlib import import_module as load`), is
  taken not to be `builtins`, `sys`, `os` or the file's own module.

## Bounds

Each lookup reads a bounded number of things:

| Lookup | Limit |
|---|---|
| definitions per reaching-definition scan | 64 |
| candidate values read per gap, assignments and loop items together | 64 |
| direct subclasses per `self.x` read | 16 |
| assignments per subclass | 4 |
| attribute assignments | 64 |
| callers read per function | 256 |
| value resolution depth | 16 |
| expression and name nesting | 64 |
| scope nesting (functions, classes, lambdas, comprehensions) | 64; a deeper scope is not analyzed: a `truncated` gap and a whole-file dynamic feature |

Lists, loop items, class lookups and every-definition candidate lists are read
once and memoized. The output is bounded as well:

| Output | Limit |
|---|---|
| nested values | 32 per call, 4,096 per file |
| listed candidates | 32 per gap, 4,096 per file |
| names | 200 bytes |
| records | 20,000 per file, then a `truncated` gap |
| imported names | 20,000 per file |

Files over 4 MiB are skipped.

Time and memory grow linearly with the file: every lookup above is bounded,
and every walk up the enclosing scopes stops at the 64-level scope cap. The
unit tests generate stress files of 0.3 to 2 MB for each superlinear shape
that reviews found, and fail in a debug build past 10 s. Release builds were
measured on a Windows laptop, on the reviewers' stress sets and on the
generators of each later review:

- up to 2.6 MB: at most 3.2 s and 580 MB peak;
- 2.6 MB to 4 MiB: the generated shapes at most 3.4 s and 660 MB (a
  400,000-key dict expanded at 10,000 calls), and the 3 MB scale files about
  3.3 s and 780 MB, bound by the tree-sitter parse;
- 4 MiB: about 5 s and 1 GB, bound by the parse.

## TypeScript and JavaScript

The same contract holds for `.ts`, `.tsx`, `.mts` and `.cts` files (language
`typescript`) and `.js`, `.jsx`, `.mjs` and `.cjs` files (language
`javascript`, parsed with the TSX grammar). Declaration files (`.d.ts`,
`.d.mts`, `.d.cts`) declare types and send no request, so they are skipped like
`.pyi` stubs. The records, routes and gap kinds are the ones above; this
section states the grammar the TypeScript extractor
(`crates/arbor-core/src/usages/typescript.rs`) confirms.

JavaScript scopes names lexically, and nothing but an assignment written in the
file rebinds one; `eval` and `with` are the exceptions (see
[Dynamic features](#dynamic-features-1)). A binding is visible in all of its
scope (hoisting and the temporal dead zone), so resolution is
flow-insensitive: a name reads the same everywhere it is in scope, and a name
written more than once is a `reassigned` gap that lists what it may hold.

### Packages and imports

The tracked packages are `openai`, `@anthropic-ai/sdk`,
`@anthropic-ai/bedrock-sdk`, `@anthropic-ai/vertex-sdk`,
`@anthropic-ai/foundry-sdk`, `@anthropic-ai/aws-sdk`, `@google/genai` and
`@google/generative-ai`, with any version (`openai@4`), a Deno `npm:`
specifier, `jsr:@openai/openai`, or a CDN URL (`esm.sh`, `cdn.skypack.dev`,
`esm.run`, `unpkg.com`, `cdn.jsdelivr.net/npm/`, `deno.land/x/openai`). A
subpath that exports the clients (`openai/index`, `openai/azure`,
`@google/genai/node`) counts as the package; any other subpath
(`openai/resources/chat`) binds types and helpers.

Every way a file binds one of them is an import record:

- ESM: `import OpenAI from`, `import { AzureOpenAI as Az } from`,
  `import * as oa from`, `import OpenAI, { toFile } from`, and `import type`
  (a type only: it names a class for an annotation, never a value);
- TypeScript's `import OpenAI = require("openai")`;
- CommonJS: `const OpenAI = require("openai")`, `const { OpenAI } =
  require("openai")`, `const { default: OpenAI } = require("openai")`,
  `require("openai").default`;
- dynamic `import()`: `const { default: OpenAI } = await import("openai")`,
  `const mod = await import("@anthropic-ai/sdk")`.

`require("openai")` and `require("@anthropic-ai/sdk")` are the module object,
which is also callable as the default client (`new (require("openai"))()`); a
default import of `@google/genai` or `@google/generative-ai`, which have no
default export, is the module object too.

### Model values

These are reported with no gap (`model`, and the other keys in `VALUE_KEYS`):

1. **A string literal at the call**, or a template literal with no
   substitution, with no escape sequence. TypeScript SDK methods take one
   options object, so the value is a property of it
   (`create({ model: "gpt-4o" })`), or of an object or array literal nested in
   it under identifier-shaped keys (`requests: [{ params: { model } }]` is
   `requests.params.model`). A literal written at the call is never
   downgraded.
   - A property counts only when no spread or computed key this file cannot
     read is written after it in that literal: `{ model: "gpt-4o",
     ...overrides }` is a `splat` gap that lists `gpt-4o`; `{ ...base, model:
     "gpt-4o" }` is confirmed. A key written twice counts by its last entry.
2. **A name bound once**: a `const`, or a `let` or `var` that the file assigns
   only in its declaration, whose value resolves under these rules. A
   destructured name reads the property it names (`const { model } = CONFIG`).
3. **A property of a same-file object literal that is only read**:
   `MODELS.fast`, `MODELS["fast"]`, `CONFIG.openai.model`, a spread of it
   (`{ ...PARAMS, messages }`), or the options object itself
   (`create(PARAMS)`). The object's name must be bound once to an object
   literal (`as const` and `satisfies` are fine), and every use of the name
   must read it: a static or computed member read, a spread, a
   destructuring, or passing it to an SDK request or a read-only builtin
   (`JSON.stringify`, `console.*`, `Object.keys`, `Object.values`,
   `Object.entries`, `Object.freeze`, `structuredClone`, `String`...). A
   write into it (`MODELS.fast = ...`, `delete MODELS.fast`,
   `Object.assign(MODELS, ...)`), a method call on it, passing it anywhere else,
   or handing out a nested object or array of it makes every read of it an
   `attribute` gap. A nested container is handed out when it is read into
   anything but such a call, when the object is spread into a literal that
   is not one (`const copy = { ...CONFIG }` shares `CONFIG.chat`), and when
   the result of `Object.values` or `Object.entries` (its entries) or of
   `Object.freeze` (the object itself) is used. A literal whose own methods
   write `this` (`toJSON() { this.model = ... }`) may change any property, and
   `__proto__: P` sets its prototype: a key the literal does not set may come
   from `P`. A spread of a read-only object is read in place; a property it
   sets replaces the ones written before the spread. A run-time key
   (`MODELS[tier]`) is a `subscript` gap that lists every entry, with
   `values_complete` when the literal holds nothing else.
4. **A member of a same-file `enum`** with a string value (`Model.Fast`), or
   one that names another member (`Default = Fast` is the member `Fast`, not
   an outer `Fast`). An enum written, cast (`(Model as any).Fast = ...`),
   aliased or passed to anything but a read-only builtin is `reassigned`,
   and so is one declared in two blocks.
5. **A class property assigned once**, read through `this` in the class's own
   code (a method, the constructor, a field initializer, a static block, and
   the arrow functions inside them, but not a `function` nested in them) or by
   class name for a static one (`Bot.MODEL`). The assignment is a field
   initializer (`private model = "gpt-4o"`), one `this.model = ...` anywhere in
   the class, or a constructor parameter property
   (`constructor(private model = "gpt-4o")`, which then follows rule 6). It
   holds when:
   - no decorator is written on the class or any of its members;
   - the class has no write of `this` under a name known only at run time
     (`this[key] = v`, `Object.assign(this, ...)`, `Object.defineProperty(this,
     ...)`), no computed member name, and no method, getter, setter or
     `accessor` with the property's name;
   - no other object in the file has a property of that name written
     (`bot.model = ...`, `other.config.model = ...`, `Object.assign(bot, {
     model })`, `Reflect.set(bot, "model", ...)`), and no object but a fresh
     literal is written reflectively under names known only at run time
     (`Object.assign(bot, overrides)`);
   - its base is a same-file class (in parentheses or a cast is fine), an
     import or a global; any other base (`extends make()`, a mixin,
     `extends holder.Base`, a name bound to anything but a class) may write
     any property, a `dynamic_feature` gap;
   - it has at most one same-file base, which has none of its own, and at
     most 16 direct same-file subclasses, which have none of their own. A
     subclass's own assignment is another write (`reassigned`), and so is a
     field a subclass declares with no value, which resets the property
     (`declare model: string` emits nothing and is not one). A static
     property read through `this` sees the subclasses' own, which `this`
     may be.

   Read through `this` in a class another module may extend (it is exported,
   anonymous, or passed or stored), the value keeps a `subclass_override` gap.
   Read in a method that may run with another object as `this` (its name is
   read without being called, as in `setTimeout(bot.ask)`, `{ ask: bot.ask }`
   or `const { ask } = bot`, or it is called through `call`, `apply` or
   `bind`), it keeps an `attribute` gap. A method reached under a name known
   only at run time and called through `call`, `apply` or `bind`
   (`this[m].call(other)`), or a computed key in an object pattern (`const {
   [m]: f } = this`), may be any method: then every method's reads of `this`
   keep the gap. Reads by class name are confirmed.
6. **A parameter default, when every call of the function is in this file**
   and none passes another value. The calls are all seen for:
   - a function declaration, or a `const` bound to an arrow function or
     function expression, that is not exported (`export`, `export default`,
     `export { f }`, `module.exports`, `exports.f`) and whose name is only ever
     called directly (`f(...)`, `(f)(...)`; `f.call`, `f.bind`, `new f()` or
     passing `f` anywhere make them unseen). Calls through a function
     expression's own name (`function retry() { retry() }`) are its calls
     too;
   - a `private` or `#private` method or class-field function whose name is
     never read without a call (`bot.ask` passed or stored, or any key of an
     object pattern, as in `const { ask } = this`): every call of its name in
     the file is listed. A method called under a name known only at run time
     (`this[kind](...)`, `r[name](...)`, `this[m].call(...)`) may be any
     method, as with Python's `getattr(self, kind)(...)`: then no private
     method's calls are all seen;
   - the constructor of a class that is not exported, is only constructed by
     name (`new Bot(...)`, or a class expression's own name), has no
     same-file subclass, no `new this()` and no `.constructor` used as a
     value (`new (this.constructor)(...)`; outside the class's own code, any
     `x.constructor` used as a value makes every constructor's calls
     unseen).

   A destructured parameter reads the property each call passes
   (`function ask({ model = "gpt-4o" } = {})`). A call that passes nothing or
   `undefined` takes the default; a spread argument at or before the position
   is unknown. Otherwise the default is reported with a gap:
   `parameter_overridden` (an in-file call passes another value, listed),
   `parameter_default` (the function is exported, or is a public method) or
   `unseen_callers` (it is passed, stored, used as a callback or a method of
   an object literal, or the file calls a method under a run-time name). A parameter written in its function (`model = "x"`, a
   `var` of its name, `??=`) is `reassigned`, listing the default, what
   callers pass and each write; in a function with a simple parameter list
   that writes, passes or stores `arguments`, which aliases the parameters
   in sloppy mode, no list is complete.

A namespace's export is a property of the namespace (`export let model` in
`namespace Cfg` compiles to `Cfg.model`): a write of `Cfg.model`, a cast or
alias of `Cfg`, or a second `namespace Cfg` block makes it `reassigned`.

The literal fallback of an environment lookup (`process.env.MODEL ??
"gpt-4o"`, `|| "gpt-4o"`) is captured with origin `env_default` and keeps its
`env_var` gap. So is the fallback of a property or parameter lookup
(`options.model ?? "gpt-4o"`, `const { model = "gpt-4o" } = options`), which
keeps the lookup's own gap (`attribute`, `parameter`). An environment lookup
is `process.env.X`, `process.env["X"]`, `process["env"].X`,
`import.meta.env.X`, `Bun.env.X`, `Deno.env.get("X")`,
`Deno.env.toObject().X`, the `env` of any name for `process` (`import process
from "node:process"`, `import * as p from "process"`, `globalThis.process`,
`require("process")`, `const p = process`), or a name destructured from or
bound to one of them (`const { MODEL = "gpt-4o" } = process.env`, whose default
is the fallback, `import { env } from "node:process"`, `const { env: { MODEL }
} = process`). The variable's name is never reported (`process.env[...]`),
not even as the name of a destructured binding.

Anything else is a typed gap: `f_string` (a template literal with a
substitution), `conditional` (a ternary, or `??`/`||` whose left side is not
known, with the known sides listed), `function_call`, `attribute`,
`subscript`, `reassigned` (a loop variable lists the items of a literal array
it runs over), `imported_name`, `unbound`, `parameter`, `non_literal`
(concatenation, an escape sequence), `credential`, `splat` and
`missing_model`. A credential-shaped value, a secret-named binding or key, and
anything reached through one are refused exactly as in Python.

### Routes

A call's routing is confirmed when its receiver reaches it through the value
rules above (one kind of client per name; `null` and `undefined` assigned next
to a client are ignored, so a lazily built client resolves), through `this` (a
class property under rule 5, without the `subclass_override` gap, as in
Python), or through a parameter that every in-file caller of a function whose
calls are all seen passes. For an exported function the routing those callers
pass keeps an `in_file_callers` gap. A parameter known only by a type
annotation that names a tracked class (`client: OpenAI`,
`private azure: AzureOpenAI`) routes by its class: Azure, Bedrock and Vertex
are confirmed; a direct class is `unknown` with an `annotation` gap, since its
options are not seen.

The receiver must be one of these:

- **A client from a tracked constructor**: `new OpenAI(...)` (and
  `require("openai")`), `AzureOpenAI`, `Anthropic`, `AnthropicBedrock`,
  `AnthropicVertex`, `GoogleGenAI`, `GoogleGenerativeAI`, and openai v3's
  `new OpenAIApi(new Configuration({ basePath }))` or `new OpenAIApi(config,
  basePath)`. `BedrockOpenAI`, `AnthropicBedrockMantle`, `AnthropicFoundry`,
  `AnthropicAws` and `new OpenAI({ provider })` route `unknown`. `baseURL` (and
  v3's `basePath`) routes by host with the table above; a spread or an options
  object built elsewhere that may hold one, or written after it (`{ baseURL,
  ...overrides }`), is `base_url_unresolved`; `baseURL: ""` is the provider's
  host. `client.withOptions({ baseURL })` re-routes the copy it returns. A
  custom `fetch` (in the constructor or `withOptions`) receives every
  request's URL: an `endpoint_option` gap, like OpenAI's `dataResidency`.
  So does an Anthropic client's `middleware`, which may change a request
  before passing it on or answer it itself, unless it is an array literal
  whose items are all calls of the SDK's own `betaRefusalFallbackMiddleware`
  (imported from `@anthropic-ai/sdk`, under any name): that one only sends
  the request again with a fallback model, so the route stays. The SDKs
  send their requests through the global `fetch`, so a `fetch` the file
  writes on an object it cannot identify (`globalThis.fetch = ...`,
  `(window as any).fetch = ...`, `Object.assign(globalThis, { fetch })`, a
  bare `fetch = ...`) gives the same gap to every call whose client would
  route to a provider's own API.
- **A Gemini client** also needs no `vertexai` or `enterprise` option other
  than `false`, `null` or `undefined`, no `httpOptions` that names a
  `baseUrl` (or is not an object literal), no spread, and no
  `setDefaultBaseUrls(...)` call in the file: each routes `unknown` with an
  `endpoint_option` gap, and `vertexai`/`enterprise` sets the construction's
  `vertexai` field. OpenAI's `dataResidency` option gets the same gap.
- **A model or chat built by a client**: `genAI.getGenerativeModel({ model },
  requestOptions)` (an `endpoint_option` gap on it and on its calls when the
  request options may name a `baseUrl`), `model.startChat()` and
  `ai.chats.create(...)` keep the client's routing.
- **One request's own options**: a legacy model or chat request given
  request options that may name a `baseUrl` (`generateContent(request, {
  baseUrl })`, `sendMessage`), and an `@google/genai` request whose `config`
  may hold `httpOptions` naming one (or is built elsewhere), route `unknown`
  with an `endpoint_option` gap. The model sent with it was given where the
  model or chat was built, so in such a file every `getGenerativeModel`,
  `startChat` or `chats.create` of that SDK gets the same gap.

The file must not:

- set the SDK's endpoint variable through `process.env` (`endpoint_env`):
  `OPENAI_BASE_URL`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_PROFILE`,
  `ANTHROPIC_CONFIG_DIR` (an Anthropic profile may name a `base_url`),
  `GOOGLE_GENAI_USE_VERTEXAI`, `GOOGLE_GENAI_USE_ENTERPRISE`,
  `GOOGLE_GEMINI_BASE_URL` or `GOOGLE_VERTEX_BASE_URL`;
- write the environment under a name known only at run time
  (`process.env[key] = v`, `Object.assign(process.env, ...)`, through an alias
  of `process.env`, `Deno.env.set(name, ...)`), or pass it to anything but a
  read-only builtin (`setup(process.env)`): a `dynamic_feature` gap;
- assign `baseURL` to the client after it is built (`client.baseURL = ...`,
  `Object.assign(client, { baseURL })`, `Reflect.set(client, "baseURL", ...)`:
  `base_url_assigned`), or to an object it cannot identify
  (`dynamic_feature`);
- replace an SDK member (`client.chat.completions.create = fn`,
  `OpenAI.prototype.x = ...`, `Object.assign(client.chat.completions, ...)`,
  `Object.setPrototypeOf(client, ...)`) or mock the SDK in a test
  (`jest.mock("openai")`, `vi.mock`, `vi.mock(import("openai"))`,
  `vi.spyOn(client.chat.completions, "create")`): every route of that package
  is a `dynamic_feature` gap. Settings such as `client.apiKey`, `maxRetries`
  or `timeout` change nothing;
- call `setDefaultBaseUrls` from `@google/genai`, under any name or through
  `require`: every Gemini client routes `unknown`.

### Calls

A call's `member` is its path after the tracked root
(`chat.completions.create`); optional chaining and `!` are dropped from it
and kept in `chain`. `request` is true when the last member is a request
method (`create`, `stream`, `parse`, `runTools`, `generateContent`,
`sendMessage`, `countTokens`, openai v3's `createChatCompletion`...).
`.withResponse()` and `.asResponse()` on a request's promise are not calls of
their own. openai v3's `createTranscription(file, model)` and
`createTranslation(file, model)` take the model by position, and OpenAI's
`runs.create(threadId, body)`, `runs.createAndPoll`, `runs.createAndStream`,
`runs.stream` and `assistants.update(id, body)` take their body second.

An SDK-shaped call on a receiver that is not a tracked client is an
`unknown_client_factory` gap, with the values written at the call listed. A
call is SDK-shaped when it ends in a request of the SDKs' own shape
(`.chat.completions.create`, `.models.generateContent`...), or in any member
that takes a model (`getGenerativeModel`, `chats.create`,
`assistants.create`, `fineTuning.jobs.create`...) with a model written at
the call. The receiver may be a client imported from another module (`imported_name`, the common
`import { openai } from "@/lib/openai"`), a parameter, a factory function
(`factory_call`), a class property never assigned (`unassigned_attribute`)
or an SDK call's result. `messages.create` counts only where the file imports
an Anthropic package (Twilio and Slack use the same shape). An SDK member
reached under a run-time name (`client[method](...)`) is a `dynamic_dispatch`
gap (`computed_member`); a request method stored or passed as a value
(`const create = client.chat.completions.create`, `retry(
client.chat.completions.create)`, `create.bind(...)`) is `method_reference`.

### Dynamic features

A direct call of the global `eval` (also in parentheses, `(eval)(...)`), a
`with` statement, and scopes nested more
than 64 deep (not analyzed, a `truncated` gap) make every value not written at
its call, and every route, a `dynamic_feature` gap. `new Function(...)` sees
only the global scope and changes no binding of a module.

### Outside the file

Resolution is per file. A client or constant imported from another module is
a gap at the call. The report assumes the rest, in particular:

- Other modules do not write this module's objects or the properties of its
  class instances, except as rules 5 and 6 say (a subclass, an exported
  function's callers).
- A write under a computed key (`obj[key] = v`) on anything but `this` does not
  set a class property, and an SDK request does not modify the options object
  it is passed.
- A member read under a computed key without being called (`const f =
  this[kind]`, `Reflect.apply(this[kind], ...)`) is a value, not a method:
  only a computed member that is called, or followed by `call`, `apply` or
  `bind`, counts as calling a method under a run-time name.
- A client a class reads through `this` is confirmed although a subclass in
  another module may replace it, as in Python.
- The process environment, a `.env` file loaded at start-up and an Anthropic
  profile file are not known: a direct client built without a `baseURL` is
  reported direct.
- A `fetch` replaced or intercepted outside the file (nock, msw, a polyfill
  installed at start-up) is not seen.
- TypeScript's `private` is checked by the compiler, not at run time: a
  `private` method's callers are taken to be the calls of its name in the
  file.
- A `var` read before its declaration runs holds `undefined`; such a request
  fails rather than using another model.
- Bundled or minified code outside `dist/`, `build/`, `out/` and `*.min.js`
  is scanned as project code.

Not tracked yet: wrappers (LangChain JS, the Vercel AI SDK's `@ai-sdk/*`
providers, LiteLLM clients), the realtime WebSocket clients
(`new OpenAIRealtimeWS({ model })` is recorded as an SDK object with an
`unknown` route), `@google-cloud/vertexai`, the legacy SDK's
`@google/generative-ai/server` managers (a model passed to
`cacheManager.create` is an `unknown_client_factory` gap), the `baseModel` of
a Gemini tuning job (`ai.tunings.tune`), and a same-file function that
returns a client (`factory_call`).

### Bounds

The TypeScript extractor's lookups are bounded the same way: expression depth
64, nested name and property resolutions 64, scope nesting 64 (deeper scopes
are a `truncated` gap and a whole-file dynamic feature), 64 writes read per
name or property, 16 direct subclasses, 256 callers read per function, 64 expanded spreads per object literal, object literals read
inside one another 64 deep (through spreads and member chains; past either
bound a spread is opaque), 4,096 distinct paths read off one name (past that
every read of the name is an `attribute` gap), member chains of 32 links (a
longer one is not read), model keys nested at most 4 literals below an
option's value, as in Python (deeper ones are not read), and 256 nested
entries read per call (past that a `truncated` gap). Each object
literal is read once, into an index of its keys, however many calls spread it.
The output limits are the same as Python's: nested values (32 per call, 4,096
per file), listed candidates (32 per gap, 4,096 per file), names (200 bytes),
records including imports (20,000 per file, then a `truncated` gap) and
imported names (20,000 per file).
