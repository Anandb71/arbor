# SDK usages: what a report confirms

`arbor usages` and the MCP tool `find_api_usages` list where a Python project
calls the OpenAI, Anthropic and Gemini SDKs, with each call's routing and model
values. A consumer (a codemod, a reviewer) should treat the report this way:

- **A value or route with no gap at its call is confirmed.** It comes from one
  of the shapes in the supported grammar below, and nothing in the file can
  change it at run time.
- **Anything else carries a gap.** The gap's `reason` says why, and the call
  needs a person to confirm it first. A gap may still list the value the code
  names (a parameter default, an environment fallback, a class attribute that
  something else may overwrite), so it can be proposed for review.

Python is too dynamic to resolve every shape soundly. Rather than adding a
special case for each one, the extractor resolves a small grammar exactly, and
turns everything outside it into a gap. That includes a list of dynamic
features that can change a resolved name, class or dict. A wrong value with no
gap inside this grammar is a bug. A shape outside it is reported as a gap.

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
     written directly in its body (and in closures that do not rebind them).
     A `staticmethod` parameter named `self`, a nested `def inner(self)` and
     a method another class borrows (`B2.go = B.go`, `go = B.go`) do not
     count; a write through such a method's `self` is a write on another
     object.
   - Read through `self` or `cls`, a value of a **public class** keeps a
     `subclass_override` gap, the way a public function's default keeps
     `parameter_default`: another module may subclass the class and set the
     attribute. A class is public when it is not `_private`, not inside a
     private class or a function, and not in a script. A private class one of
     whose direct same-file subclasses is public counts as public too. Reads
     by class name are confirmed.

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
   is exported, and a `__mangled` method called from outside under its
   mangled name (`_A()._A__go(...)`) has callers this file does not show, so
   both are treated as passed (`unseen_callers`).
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
  stay confirmed. The legacy SDK's `configure(client_options=...)` gives its
  calls the same gap.
- **The module-level API** (`openai.ChatCompletion.create`,
  `openai.chat.completions.create`). Its routing comes from the file's
  `openai.api_type`, `openai.api_base`, `openai.base_url` and
  `openai.azure_endpoint` settings, which count with no gap only when:
  - each is a statement of its own directly in the module body: not in an
    `if`, loop, `try`, `with`, class body or function;
  - each routing setting is assigned once;
  - each comes before the call runs: a call that runs at import must come
    after them, and a call in a function must not be reachable from code of
    this file that runs at import before them (a call of a function or class
    defined in this file, such as `main()` or `Bot()`).

  Otherwise the call keeps the routing the settings suggest, with a
  `dynamic_feature` gap. A file that configures nothing is reported direct
  (see [Outside the file](#outside-the-file)), except that a call passing
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
  `__main__.py` or a script, and not when the name is a tracked SDK's).

These builtins count under any name: aliased (`run = exec`, `g = globals`),
reached through `builtins` or `__builtins__` (`builtins.exec`,
`getattr(builtins, "exec")`), or imported from `builtins`. A parameter or
keyword argument that the file names `eval` is not the builtin.

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
  or `__class__` (`type(self).model = ...`, `self.__class__.model = ...`);
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
- `os.environ` aliased, passed or updated in place.

Writes are matched by the attribute name `environ` (and `putenv`) on any
object, so `import os as _os; _os.environ[...] = ...` counts. Looking `os`
or `os.environ` up by name (`importlib.import_module("os")`,
`getattr(os, "environ")`) and importing `environ` or `putenv` under another
name (`from os import environ as E`) alias the process environment. A bare
`environ` counts only when it is imported from `os`; a WSGI `environ`
parameter does not count.

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

Time and memory grow linearly with the file. The unit tests generate stress
files of 0.3 to 2 MB for each superlinear shape that reviews found, and fail
in a debug build past 10 s. Release builds of the reviewers' stress sets were
measured on a Windows laptop:

- up to 2.6 MB: at most 3.2 s and 580 MB peak;
- 2.6 MB to 4 MiB: the generated shapes at most 2.3 s and 370 MB, and the
  3 MB scale files about 3.3 s and 780 MB, bound by the tree-sitter parse;
- 4 MiB: about 5 s and 1 GB, bound by the parse.
