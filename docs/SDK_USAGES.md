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
   - A walrus binds the whole expression after `:=`, a conditional included:
     `(model := "gpt-4o" if fast else "o3-mini")` gives `model` both
     candidates, as Python does (the parser reads it as
     `(model := "gpt-4o") if fast else "o3-mini"`, which this rule undoes).
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
value rules above (one kind of client per name, where a parameter is one
more value whatever its annotation), or through a parameter that every
in-file caller of a private function passes, annotated or not (the clients
the callers pass say more than the annotation). The receiver must be one of
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

  In the block, a string literal decides `openai.api_type`: `azure`,
  `azure_ad` or `azuread` routes to Azure, `openai` or `open_ai` direct,
  and any other (`amazon-bedrock`) `unknown` with no gap, a platform this
  version does not name. An `api_type` that is not a string literal
  (`os.environ[...]`, a name bound to one), and any `openai.bedrock_*`
  setting, give a `module_routing` gap. The route is `unknown`, unless an
  Azure setting is next to them: then `azure_openai` is kept as the
  candidate, under the gap, as `module_config` keeps the routing the
  settings suggest.

  A v0 resource call (`openai<1.0`: `openai.ChatCompletion.create`,
  `Completion`, `Embedding`, `Audio`, `Model` and the other resources) also
  takes its own `api_base` and `api_type`, which route that call whatever
  the settings say. A call that passes either (other than `None`), or a
  `**` expansion that may hold one, routes `unknown` with a `call_routing`
  gap. A routing argument passed by position is not read: v0 code passes
  them by name.

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

A client known only from a type annotation (`def ask(client: OpenAI)`,
`client: OpenAI` in a class body) routes by its class when the class alone
decides the route (`AzureOpenAI`, `AnthropicBedrock`, `AnthropicVertex`). A
direct class (`OpenAI`, `Anthropic`, `genai.Client`) may have been given a
`base_url` or another endpoint, so its calls route `unknown` with an
`annotation` gap. An annotation that names several client classes
(`AzureOpenAI | OpenAI`, `Union[AnthropicBedrock, Anthropic]`) decides the
route only when every one of them does, and all the same way; otherwise its
calls get the same gap. Its other members (`None`, a class of the
project's own, as in `AzureOpenAI | Gateway`) are not read.

When the name or attribute is given other values too:

- A construction the file shows decides the route over a **declaration**
  (`client: OpenAI` in a class body with no value, or with `None` or
  another literal as in `client: Optional[OpenAI] = None`, and
  `self.client = OpenAI()` in `__init__`). A declaration with `None` or
  another literal that nothing else assigns holds that value, not a client.
  Read on its own (`self.client.chat.completions.create(...)`), it routes
  by its annotation, as above. Where another binding reads it next to any
  other value (a construction, a factory's client, a parameter, a name
  from another module), it is dropped, as an unannotated `client = None`
  is, and types none of them: `client = self.client or OpenAI()` routes as
  `OpenAI()` does, and `self.client or make_client()` gets the factory's
  gap, whatever the annotation. Several such declarations, read together
  (`self._a or self._b`) or made for one attribute (in the class body and
  in `__init__`), combine as typed values do (below), and are dropped
  together next to any other value. A member of one
  (`self.client.chat.completions`) is combined as members are (below): it
  is dropped next to the same member of a client, and next to anything
  else (`self.client.chat.completions if flag else
  make_client().chat.completions`) the call gets a `multiple_assignments`
  gap. Next to a value that a dynamic feature affects and that is not
  known to be a client, though, the declaration is kept, with that
  feature's gap. A bare declaration is set somewhere else, to any client,
  and counts as a typed value there.
- A class attribute that one of the class-attribute dynamic features may
  set (see "Class attributes" under [Dynamic features](#dynamic-features):
  a field of a dataclass, attrs class, `NamedTuple` or pydantic model, an
  attribute of a class with a base from another module, of a decorated
  class or of one used as a value, one written on another object...) is
  not one that nothing else assigns. A declaration with `None` there is a
  typed value (below) with a gap. Read on its own, it routes by its
  annotation with a `dynamic_feature` gap (a direct class keeps its
  `annotation` gap). Next to a construction its class decides the route of
  (`self.client or AzureOpenAI(...)` under `client:
  Optional[AzureOpenAI] = None`), the call takes that route and keeps the
  `dynamic_feature` gap. Next to any other construction (`self.client or
  OpenAI()`, also under `client: Optional[OpenAI] = None`) or a value that
  is not a client (`self.client or make_client()`), it gets a
  `multiple_assignments` gap. A dynamic feature of the whole file (`exec`)
  does not make a declaration one of these: it is dropped as above, and
  the route the call takes gets that feature's gap (`self.client or
  OpenAI()` is `openai_direct` with a `dynamic_feature` gap).
- A **typed value** (a parameter, or an annotated assignment with any other
  value, `self.client: OpenAI = make()`) is one more value. It agrees with a
  construction only when its class decides that construction's route
  (`AzureOpenAI` next to `AzureOpenAI(...)`), and then the call keeps any
  gap the typed value carries for another reason (a field's
  `dynamic_feature`); a direct class never does. Otherwise the call gets a
  `multiple_assignments` gap, as with an untyped
  parameter (`def ask(client: OpenAI | None = None): client = client or
  OpenAI()`). So does a typed value next to a value that is not a client,
  unless a declaration types the binding: `self.client = client or
  make_client()` under `client: AzureOpenAI`, or `self.client: AzureOpenAI =
  client or make_client()`, routes `azure_openai` when `client:
  AzureOpenAI`, whether the two values are combined in that expression, in a
  local default (`if client is None: client = make_client()`) or in two
  assignments.
- With no construction, the typed values and the binding's own
  declarations (not a `= None` declaration read next to another value,
  which is dropped, above) decide the route only when every one of them
  does, and all the same way, as in one annotation; a declaration says
  more than the values next to it that are not clients (`self.client =
  make_client()`). Otherwise the call gets the `annotation` gap (`client =
  azure if flag else direct`, with `azure: AzureOpenAI` and `direct:
  OpenAI`, in either order), or the gap one of them already carries when
  they route alike.
- A **member of a client** (`azure.chat.completions`,
  `client.embeddings.create`) follows the same rules when every value is
  that same member of a client: `chat = azure.chat.completions if flag else
  direct.chat.completions` gets the `annotation` gap in either order, and
  the member is kept. Different members, or a member next to a value that is
  not one, get a `multiple_assignments` gap. A call through a name bound to
  a member of a client the file does not resolve (`chat =
  make_client().chat.completions`, then `chat.create(...)`; the same with a
  parameter or an imported name, or `(self.client or
  make_client()).chat.completions` under a `= None` declaration) is not
  reported, not even as a gap: only a call that names the member
  (`make_client().chat.completions.create(...)`) is, with the client's gap.

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
- 4 MiB: about 5 s and 1 GB on ordinary code, bound by the parse.

A file made of little but calls costs more at the same size: the scan
indexes every call and evaluates each one's callee. Time and memory still
grow linearly with the number of calls (half the file takes half the time
and memory). Release builds on the same laptop:

| 4 MiB of | Calls | Time | Peak memory |
|---|---|---|---|
| bare calls, `f()` on each line | about 1M | about 9 s, 4 s of it the parse | about 1.4 GB, 0.96 GB of it the parse |
| chained calls, `f()()()...` 60 to a line | about 2M | about 20 s, over half of it naming each call's callee | about 1.8 GB |

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
[Dynamic features](#dynamic-features-1)), and so is a function declared
in a block of a script that is not in strict mode, which also assigns the
enclosing function's binding of its name (Annex B): that is not seen
([Outside the file](#outside-the-file-1)). A binding is visible in all of its
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
   downgraded by a dynamic feature.
   - A property counts only when no spread or computed key this file cannot
     read is written after it in that literal: `{ model: "gpt-4o",
     ...overrides }` is a `splat` gap that lists `gpt-4o`; `{ ...base, model:
     "gpt-4o" }` is confirmed. A key written twice counts by its last entry.
   - An OpenAI or Anthropic request sends a `body` given in its request
     options (the argument after the body) in place of the body written
     at the call, on every client of those packages but openai v3's
     `OpenAIApi`, whatever its routing (Azure, a custom `baseURL`, Bedrock,
     Vertex, Foundry and AWS too). When those options may hold one,
     because they set a `body` (`create({ model: "gpt-4o" }, { body: {
     model: "gpt-5.5" } })`) or cannot be read (a parameter, an import, a
     call's result; see [Routes](#routes-1)), the model written in the
     body is a `splat` gap that lists it, and each model nested in it
     (`requests.params.model`) is a `splat` gap that lists none. A model
     that already has a gap keeps it (a value listed with its own gap,
     such as `subclass_override`, gets this one next to it), and the
     call's `splat` is set ([Calls](#calls)). So a wrapper that forwards
     the request options it is given (`complete(prompt, options)`, `{
     timeout, ...opts }`) does not confirm the model written at its call.
     A model written in a `body` literal in the options at the call is
     captured as `body.model`.
2. **A name bound once**: a `const`, or a `let` or `var` that the file assigns
   only in its declaration, whose value resolves under these rules. A
   destructured name reads the property it names (`const { model } = CONFIG`).
3. **A property of a same-file object literal that is only read**:
   `MODELS.fast`, `MODELS["fast"]`, `CONFIG.openai.model`, a spread of it
   (`{ ...PARAMS, messages }`), or the options object itself
   (`create(PARAMS)`). The object's name must be bound once to an object
   literal (`as const` and `satisfies` are fine), and every use of the name
   must read it: a static or computed member read, a spread, a destructuring,
   a test (`if (MODELS)`, `!MODELS`, a comparison, `typeof`, a `switch` or
   `case` value, the left side of `&&`, an operand of `||` or `??` that is
   only tested, as in `if (MODELS || fallback)`), a substitution in a
   template that is not tagged (`` `${MODELS}` ``; a tagged one,
   `` tag`${MODELS}` ``, passes it to `tag`), a `for...in` over it, or
   passing it to an SDK request or a read-only builtin
   (`JSON.stringify`, `console.*`, `Object.keys`, `Object.values`,
   `Object.entries`, `Object.getOwnPropertyNames`, `Object.freeze`,
   `structuredClone`, `String`...), also inside parentheses or a cast
   (`(MODELS as any).fast`). A write into it (`MODELS.fast = ...`,
   `(MODELS as any).fast = ...`, `delete MODELS.fast`,
   `Object.assign(MODELS, ...)`), a method call on it, an alias
   (`const m = MODELS`), passing it anywhere else, or handing out a nested
   object or array of it makes every read of it an `attribute` gap. A nested
   container is handed out when it is read into
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
   an outer `Fast`). An enum written (also through a cast,
   `(Model as any).Fast = ...`), aliased or passed to anything but a read-only
   builtin is `reassigned`, and so is one declared in two blocks.
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
     model })`, `Reflect.set(bot, "model", ...)`, `Object.defineProperties(o,
     { model: ... })`; `Object.create(proto, { model: ... })` gives only
     the new object its properties, and counts through its prototype, as
     below), and no object but a fresh literal is written reflectively
     under names known only at run time (`Object.assign(bot, overrides)`);
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
   Read in a method that may run with another object as `this`, it keeps an
   `attribute` gap whose detail names one such use and its line, and the
   value's own origin (`class_constant`). Reads by class name are
   confirmed. A method may run with another `this` when:
   - its name is read without being called, as in `setTimeout(bot.ask)`,
     `{ ask: bot.ask }`, `const { ask } = bot` or `Reflect.get(bot,
     "ask")`, or it is run through `call`, `apply` or `bind` with another
     `this`. Run through them with the object it is read off as `this`
     (`this.ask.bind(this)`, `bot.ask.call(bot)`, `super.ask.call(this)`),
     a method reads that object, as `bot.ask()` does, though its calls are
     then not all seen (rule 6);
   - the file reads a member under a name known only at run time off
     `this`, `super`, a same-file class (`Bot[m]`, for its static
     methods), its prototype (`Bot.prototype[m]`), `new Bot()` or a name
     that may hold one of them (below), whatever it then does with the
     value (`const f = this[m]`, `(bot as any)[m]`, `Reflect.apply(this[m],
     other, [])`, `this[m].call(other)`, `(this[m] ?? fallback)()`), but
     not when it calls it right there (`this[m](...)`, which runs with that
     object, as `bot.ask()` does). So do `Reflect.get(x, k)` with a
     run-time `k` off any of those (it reads `x[k]`; a call of what it
     returns runs with no `this`), a member of any object read under a
     run-time name and run through `call`, `apply` or `bind`
     (`r[m].call(other)`), and a computed key of an object pattern (`const
     { [m]: f } = this`, below). The member may be any method but a
     `#name`, which no computed key reaches, so every other method's reads
     of `this` keep the gap;
   - another object borrows the prototype of the class or of a same-file
     subclass: the prototype used as a value other than to read or write a
     member of it by name or to test or compare it
     (`Object.create(Bot.prototype)`, `{ __proto__: Bot.prototype }`,
     `x.__proto__ = Bot.prototype`, `Object.setPrototypeOf(o,
     Bot.prototype)`, `Object.getOwnPropertyDescriptor(Bot.prototype,
     "ask")`, `const proto = Bot.prototype`), or an instance given to `new
     Proxy(...)` as its target, to `Object.create(...)`, or to
     `Object.setPrototypeOf` or `Reflect.setPrototypeOf` as the prototype,
     or made another object's prototype otherwise: written as the value of
     `__proto__` in an object literal, assigned to `x.__proto__` or
     `x["__proto__"]`, or given to `Reflect.set(x, "__proto__", ...)`. An
     instance is `this`, `new Bot()`, the result of a method that returns
     `this` or a name that may hold one of them (below). Every method of
     the class, `#name` ones included, is then taken to run with that
     object, and so are its static methods (an object that borrows the
     class itself, as `new Proxy(Bot, h)` does, runs them); where the file
     cannot tell the class (`new Proxy(this, h)` in a plain function), the
     methods of every class are.

   A name may hold an instance or the class when it is a `const`, `let` or
   `var` given `this`, `new Bot()` (or `Reflect.construct(Bot, args)`), the
   class, its prototype or the result of a call of a method that returns
   `this` (`withModel(m) { ...; return this; }`, in a method that is
   neither async nor a generator; the file does not tell whose, so it may
   be an instance of any class), also as either side of a conditional,
   `??` or `||`, the right side of `&&` or under `await`; a `for...of`
   variable over an array literal of them; or a parameter that a call in
   the file passes one (a call of the function by name, `new` of the class
   for a constructor, or any call of a method of that name, on whatever
   object), or whose default is one. A name written more than 64 times may
   hold an instance of any class. Anything else is taken to hold none of
   them ([Outside the file](#outside-the-file-1)).

   A computed key counts in every object pattern of the file, whatever the
   pattern destructures, as Python counts any `getattr` with a run-time
   name file wide: in a declaration, a parameter, a `for...of` or `catch`
   binding, or an assignment (`({ [m]: f } = this)`). Only a string, number
   or substitution-free template literal key (`{ ["ask"]: f }`) reads just
   the name it spells; a `const` key (`{ [ID_FIELD]: id }`) is not
   resolved. This over-approximates: the omit idiom `const { [key]: _,
   ...rest } = record`, which drops a field of a plain record and reaches
   no method, also keeps every method's reads of `this` a gap and leaves no
   `private` method's calls all seen (rule 6).
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
     never read without a call (`bot.ask` passed or stored, run through
     `call`, `apply` or `bind`, even with its own object as in
     `this.ask.bind(this)`, `Reflect.get(this, "ask")`, or any key of an
     object pattern, as in `const { ask } = this`): every call of its name
     in the file is listed. A member of any object called under a name
     known only at run time (`this[kind](...)`, `r[name](...)`) or run
     through `call`, `apply` or `bind` (`r[m].call(...)`), one read under
     such a name off `this` or anything else rule 5 lists, whatever is then
     done with it (`const handler = this[kind]`, `Reflect.apply(this[kind],
     this, [m])`), `Reflect.get(x, k)` with a run-time `k` off any of
     those, and a computed
     key of any object pattern (rule 5) may be any method, as with
     Python's `getattr(self, kind)`: then no `private` method's calls are
     all seen. A `#private` method's are, since no computed key reaches a
     `#name`;
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
   an object literal, or the file calls or reads a member under a run-time
   name as the `private` method bullet above says; for a method whose name
   is never read without a call, the gap's detail names that call or read
   and its line). A parameter written in its
   function (`model = "x"`, a `var` of its name, `??=`) is `reassigned`,
   listing the default, what callers pass and each write; in a function with
   a simple parameter list that writes, passes or stores `arguments`, which
   aliases the parameters in sloppy mode, no list is complete.

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
Python; read in a method that may run with another `this`, the route keeps a
`multiple_assignments` gap whose detail names the use that may rebind it and
its line), or through a parameter that every in-file caller of a function
whose calls are all seen passes. For an exported
function the routing those callers pass keeps an `in_file_callers` gap. A
parameter known only by a type annotation that names a tracked class
(`client: OpenAI`, `private azure: AzureOpenAI`) routes by its class:
Azure, Bedrock and Vertex are confirmed; a direct class is `unknown` with an
`annotation` gap, since its options are not seen.

The receiver must be one of these:

- **A client from a tracked constructor**: `new OpenAI(...)` (and
  `require("openai")`), `AzureOpenAI`, `Anthropic`, `AnthropicBedrock`,
  `AnthropicVertex`, `GoogleGenAI`, `GoogleGenerativeAI`, and openai v3's
  `new OpenAIApi(new Configuration({ basePath }))` or `new OpenAIApi(config,
  basePath)`. `BedrockOpenAI`, `AnthropicBedrockMantle`, `AnthropicFoundry`,
  `AnthropicAws` and `new OpenAI({ provider })` route `unknown`. `baseURL` (and
  v3's `basePath`) routes by host with the table above. A constructor reads
  only an object literal written at the call: options given any other way
  (a name, even one bound to a literal, a parameter, a conditional,
  `Object.freeze(...)`, a function's result), a spread in the literal that
  may hold one, or a spread written after it (`{ baseURL, ...overrides }`),
  are `base_url_unresolved`; `baseURL: ""` is the provider's host.
  `client.withOptions({ baseURL })` re-routes the copy it returns.
- **An OpenAI or Anthropic client** (`OpenAI`, `Anthropic`; an Azure,
  Bedrock or Vertex client is routed by its class whatever these options
  hold) also needs no custom `fetch`, and an Anthropic client no
  `middleware`, in the constructor or `withOptions`. A
  custom `fetch` receives every request's URL: the client routes `unknown`
  with an `endpoint_option` gap, as with OpenAI's `dataResidency`. So does
  an Anthropic client's `middleware`, which may change a request before
  passing it on or answer it itself, unless it is an array literal of at
  most 16 items, all calls of the SDK's own `betaRefusalFallbackMiddleware`
  (imported from `@anthropic-ai/sdk`, under any name): that one only sends
  the request again with a fallback model, so the route stays. The array
  must be written where nothing else can change it: in the options at the
  call (also in either side of a conditional, in a spread or in
  `Object.freeze(...)` there), in an object literal bound once to a name
  that is only read, or nested in one, or in a literal a same-file function
  returns. A constructor reads only the literal at the call (above), so
  there the array must be in that literal or in a spread of a literal written
  in it; the other forms keep the route for `withOptions` and for one
  request's options. Not exempt: an array held in a name, a call of a
  same-named function of the file or of another module, and an array reached
  through a class property, a parameter, a name assigned more than once or
  bound to anything but an object literal
  (`const OPTS = Object.freeze({ ... })` too), or a literal that is not only
  read. The options are read through every value the file gives them, as one
  request's options are (below). Options the file cannot read (an import, a
  computed key, a `__proto__`, or a parameter or a call's result the reader
  does not follow, below) may hold either: when a `baseURL` written after it
  names the provider's own host, which would confirm the route, the client
  gets the same gap, and another host's `baseURL` routes `custom_base_url`
  either way. `fetchOptions` is not read
  ([Outside the file](#outside-the-file-1)). openai v3's `OpenAIApi` sends
  through axios: given its own axios instance (`new OpenAIApi(config,
  basePath, http)`, anything but `undefined` or `null`, even the default
  `axios` import itself), it gets the same gap.
- **A Gemini client** also needs no `vertexai` or `enterprise` option other
  than `false`, `null` or `undefined`, no `httpOptions` that names a
  `baseUrl` (or is not an object literal), no spread, and no
  `setDefaultBaseUrls(...)` call in the file: each routes `unknown` with an
  `endpoint_option` gap, and `vertexai`/`enterprise` sets the construction's
  `vertexai` field.
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
  `startChat` or `chats.create` of that SDK gets the same gap. An OpenAI or
  Anthropic request gets the same gap when an options object passed after
  its body (after the path parameter and body of `runs.create(threadId,
  body)`) may hold a `path`, which replaces the client's URL when it is
  absolute, a `defaultBaseURL`, a `fetch`, or for Anthropic a `middleware`,
  which runs after the client's (the refusal fallback above keeps the route
  here too, written where nothing else can change it). A `body` there
  replaces the request body instead: the route stays, and the models
  written in the body are `splat` gaps ([Model values](#model-values-1),
  rule 1). A `path`, `defaultBaseURL`, `fetch` or `middleware` is read
  only for a client routed to the provider's own API (a custom `baseURL`
  included), as the client's own `fetch` and `middleware` are (above): an
  Azure, Bedrock or Vertex client keeps its route whatever its requests'
  options hold. The options are read for these keys through every value
  the file gives them:
  - an object literal's entries, and what each spread in it may hold;
  - both sides of a conditional (`signal ? { signal } : undefined`), of
    `??` and of `||`, and the right side of `&&`;
  - `Object.freeze(x)`, which is `x`;
  - what a same-file function called by name returns (`requestOptions(req)`):
    a function declaration, or a `const` bound to an arrow function or
    function expression, that the file never assigns again, read through
    each `return` in its own body (a bare `return` gives `undefined`) or an
    arrow function's expression body;
  - every value of a `const`, `let` or `var` (not destructured, not a loop
    variable) that the file only reads, each use being one of these:
    - a read of a property of it (`opts.timeout`, `opts[k]`);
    - a test: `if (opts)`, `!opts`, a comparison, `typeof`, a `switch` or
      `case` value, the left side of `&&`
      (`if (opts && opts.timeout > 0)`), or an operand of `||` or `??`
      that is only tested (`if (opts || fallback)`);
    - a substitution in a template that is not tagged (a tag function is
      passed it), a `for...in` over it, or a spread;
    - a destructuring declaration (`const { timeout } = opts`, also with a
      rest element or a computed key);
    - an assignment or `delete` of a property of it under a static key
      other than `fetch`, `middleware`, `path`, `defaultBaseURL`, `body` or
      `__proto__` (`options.signal = req.signal`; a reflective write such
      as `Object.assign(options, ...)` counts as any key);
    - binding it to another name that is only read: `const o = opts`, `o =
      opts`, or a parameter's default (`function chat(o = OPTS)`);
    - passing it to an SDK request, a read-only builtin, or a same-file
      function whose calls are all seen (rule 6: a function called by
      name, or a `private` method called through `this`) that does not
      read its `arguments` and whose parameter there is only read or is an
      object pattern, directly or as a side of a conditional, `??`, `||`
      or `&&` there (`create(body, signal ? { ...base, signal } : base)`).

    Each of these may be written inside parentheses, `as`, `satisfies` or
    `!` (`opts as OpenAI.RequestOptions`, `(opts as any).signal = s`). So
    `let opts = { timeout: 30_000 }; if (slow) opts = { timeout: 120_000 }`
    holds both. A parameter used only these ways, of a function whose
    calls are all seen (rule 6), holds what each call passes, or its
    default where a call passes nothing. Anything else makes the name
    unreadable: returning or storing it, an operand of a conditional,
    `??`, `||` or the right side of `&&` whose value goes anywhere else
    (`return o || opts`), a destructuring assignment (`({ timeout: t } =
    opts)`), a call of a method of it, passing it to any other function
    (an exported one included), or binding it to a name that is not only
    read;
  - a class or static property read through `this` or by class name
    (`this.requestOptions`, `Summarizer.OPTIONS`), through every
    assignment rule 5 reads, when every read of a property of that name in
    the file, on whatever object (`this.opts`, `svc.opts`, `Svc.OPTS`), is
    one of the uses above: a read of a property of it (`this.opts.timeout`,
    `this.opts!.timeout`), a test (`!this.opts`, `if (this.opts)`,
    `this.opts && this.opts.timeout > 0`), a substitution in a template
    that is not tagged, a `for...in` or a spread, a destructuring in a
    declaration that names each key it reads (`const { timeout } =
    this.opts`, not with a rest element or a computed key), an assignment
    or `delete` of a property of
    it under a static key other than those six (`this.opts.signal = s`,
    which is not an assignment of the property), binding it to a name that
    is only read (`const o = this.opts`, `const { opts } = this`), or
    passing it to an SDK request, a read-only builtin or a same-file
    function as above (also as a side of a conditional, `??`, `||` or `&&`
    there). The property itself may be in parentheses or a cast
    (`send(this.opts as any)`), but a member read or written through them
    (`(this.opts as any).timeout`, `(this.opts).signal = s`) is none of
    these. Any other read of the name makes every class property of that
    name unreadable, even where nothing changes it: returning or storing
    it, a call of a method of it (`this.opts.set(k, v)`), passing it to any
    other function, or a write into it under one of the six keys or a key
    known only at run time (`(this.opts as any)[k] = v`, `svc.opts[k] =
    v`). Nor can a class's property be read when the file may reach the
    properties of an instance of that class, of its same-file base or of a
    direct same-file subclass without naming them: through a member read
    off the instance (or off the class, for a static property) under a
    run-time name and used for more than a read, a test or a call
    (`(this as any)[section].path = url`, `Reflect.get(this,
    section).path = url`, `(Svc as any)[section].path = url`), or through
    the instance:
    - spread (`{ ...this }`), destructured with a rest element or a
      run-time key (`const { ...all } = this`, also in an assignment);
    - given to `Object.values`, `Object.entries` or `Object.freeze` (whose
      result is the instance) when the result is used, to
      `Object.getOwnPropertyDescriptor(s)`, as a source of
      `Object.assign` (`Object.assign({}, this)`), to `new Proxy(...)`,
      `Object.create(...)` or `Object.setPrototypeOf` (rule 5), or to a
      same-file function called by name (a `private` method called
      through `this`, or a constructor run by `new` with its class's
      name) whose parameter there is an object pattern with a rest
      element or a run-time key, or that reads its `arguments`. A
      parameter that is a name and is given an instance is one (rule 5),
      so its own uses count the same way, as in a same-file
      `deepMerge(target, source)` that reads `target[k]`.

    Stored, returned, exported or passed to any other function or
    constructor (`registry.push(this)`, `container.register("chat", svc)`,
    `makeAutoObservable(this)`, `fn.apply(this, args)`, an imported
    `deepMerge(this, src)`, `module.exports = { summarizer }`), the
    instance is not followed: other modules do not write what its
    properties hold, and code in the file that reaches them through those
    without naming them is not seen
    ([Outside the file](#outside-the-file-1)). An instance is what rule 5
    lists; where the file cannot tell the class (`this` in a plain
    function), no class's property can be read when such a use reaches
    it. Nor can a property written 64 times or more (its assignments and
    the writes into it) be read, or one read through `this` in a method
    that may run with another `this` (rule 5);
  - a literal nested in a same-file literal that is only read
    (`CONFIG.requests`).

  `undefined`, `null`, other primitives, arrays and functions hold none.
  Anything else cannot be read and may hold any of them: a parameter of a
  function whose calls are not all seen (with its default), an import, the
  result of any other call (a method, a function held in a `let`, an
  import), a property of another object, a key known only at run time, a
  `__proto__`, a literal whose methods write `this`, a name given no value
  in the file. Returning a name is not one of the uses above, so a
  function that returns one (`const opts = () => base`) cannot be read
  either. On a client routed to the provider's own API, the options right
  after the body of a request that takes a model also count when some
  value they may hold cannot be read; OpenAI's `threads.messages.create`,
  which takes no model, is not such a request. Since they may then hold
  a `body` too, the models written in the body are `splat` gaps (rule 1)
  on every OpenAI or Anthropic client but openai v3's, while a client
  routed elsewhere (Azure, another host, Bedrock) keeps its route. Any
  other argument counts only for what the file shows it holds: without
  the method's signature, it may be a path parameter or a body
  (`runs.retrieve(threadId, runId)`). Only that request is re-routed; the
  client keeps its route for its other calls.

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
  (`dynamic_feature`; a CommonJS export, `exports.baseURL = ...`, is not
  one, as for `fetch` below);
- replace or intercept the global `fetch` the SDKs send their requests
  through. Every client that would route to a provider's own API (OpenAI,
  Anthropic or Gemini) then routes `unknown`, and its calls get a
  `global_fetch` gap (`ambiguous_client`) whose detail names one such
  write or call in the file and its line. openai v3's `OpenAIApi` sends
  through axios, which in Node goes through the `http` module, not
  `fetch`: it gets the gap only when the file loads `nock`, `msw` or
  `@mswjs/interceptors` (the last item below), which intercept `http` too,
  and the detail then names Node's `http` module. The rest leave a v3
  client's route. That is:
  - a `fetch` written on an object the file cannot identify
    (`globalThis.fetch = ...`, `(window as any).fetch = ...`, a bare
    `fetch = ...`), also reflectively (`Object.assign(globalThis, { fetch })`,
    `Reflect.set(o, "fetch", f)`, `Object.defineProperty(o, "fetch", ...)`).
    The object may be the global one, so `options.fetch = ...` on a
    parameter, `this.http.fetch = ...` and `root.fetch = ...` on any name
    count too. A CommonJS export (`exports.fetch = ...`,
    `module.exports.fetch = ...`, where the file binds neither name) does
    not, unless the file sets `module.exports` to something other than an
    object literal (`module.exports = client`), assigns `exports` or
    `module` itself, whatever the value (`exports = client`), or uses
    `module` other than to read or write a member by name (`module.exports`,
    `module.hot`) or to test it (`typeof module`, `require.main ===
    module`): an alias (`const m = module; m.exports = client`), passing it
    (`Object.assign(module, { exports: client })`) or a key known only at
    run time (`module[k] = v`) may replace `module.exports` where the file
    does not show it. `Object.assign(module.exports, { fetch })` still
    counts;
  - the global object written under a key known only at run time that may
    be `"fetch"` (`(globalThis as any)[name] = v`,
    `Object.assign(globalThis, polyfills)`, `Reflect.set(globalThis, k,
    v)`, `jest.spyOn(globalThis, key)`, a loop variable in `for (const k of
    keys) g[k] = v`). The global object is `globalThis`, `global`, `window`
    or `self` where the file does not bind the name; a `const`, `let` or `var`
    given one in its declaration or in a plain assignment
    (`const g: any = globalThis`, `let g = {}; g = globalThis`, and
    `let g = globalThis; g = {}` too), or written where the file does not show
    the value it is given (`[root] = [globalThis]`, `({ root } = env)`,
    `for (root of roots)`) or more than 64 times; either side of a
    conditional, `??`, `||` or `&&`, written where the key is or held in such
    a name (`(typeof window !== "undefined" ? window : globalThis)[k] = v`,
    `((globalThis || window) as any)[k] = v`,
    `const root = typeof window !== "undefined" ? window : globalThis`); or
    one read off another inside parentheses, a cast or a reflective call
    (`(globalThis.window as any)[k] = v`,
    `Object.assign(globalThis.window, o)`). A key the file shows is not
    `"fetch"` does not count: a string or number literal, a template
    literal whose fixed text rules it out (`` `__${app}_started` ``), a
    symbol of its own (`Symbol("k")`, `Symbol.iterator`), `Symbol.for`
    given a string literal that does not start with `undici`
    (`Symbol.for("app.cache")`), a conditional whose sides are all such
    keys, a `const`, `let` or `var` whose every value is one (`const KEY
    = "__cache"`), a parameter of a function whose calls are all seen
    (rule 6) when each call passes such a key (or, passing none, the
    default is one), and a `for...of` variable that runs over the items of
    an array literal written in the loop, all such keys (`for (const k of
    ["TextEncoder", "URL"])`), or over the keys of an object literal none
    of whose keys is `fetch` (`Object.keys(o)`, the first item of each
    entry of `Object.entries(o)`, as in `for (const [k, v] of
    Object.entries(o))`, or `Object.getOwnPropertyNames(o)`). That literal
    is written in the loop or bound once to a name that is not exported
    and that the file only reads, as rule 3 of
    [Model values](#model-values-1) says (also inside parentheses or a
    cast, as in `(o as any)[k]`), and passes to nothing but read-only
    builtins, not even to an SDK request; and it has at most 256 entries,
    each a static key. A loop over anything else counts (`for (const k in
    o)`, a loop over a name bound to an array). Any other `Symbol.for` key
    counts, since undici keeps the global `fetch`'s dispatcher under
    `Symbol.for("undici.globalDispatcher.1")`.
    The key is read this way for a write (`g[k] = v`), `Reflect.set`,
    `Object.defineProperty`, `Reflect.defineProperty`,
    `Reflect.deleteProperty`, a test double's key and `stubGlobal`'s name;
    `Object.assign` and `Object.defineProperties` with a source that is not
    an object literal of static keys count whatever the source holds;
  - a test double of `fetch` on an object the file cannot identify, read by
    the call's last name, whatever import form reaches the runner
    (CommonJS, a sinon sandbox, a named import, `import.meta.vitest`):
    `spyOn` or `stub` given the object and the key (`jest.spyOn`,
    `vi.spyOn`, `sinon.stub`, `sandbox.stub`, `stub` from sinon or Deno's
    std, Bun's and Jasmine's `spyOn`); `replace`, `replaceGetter`,
    `replaceSetter` or `replaceProperty` given a replacement too
    (`jest.replaceProperty(globalThis, "fetch", f)`; testdouble.js's
    two-argument `td.replace(globalThis, "fetch")` is not one, see
    [Outside the file](#outside-the-file-1)); node:test's `method`,
    `getter`, `setter` or `property` on a `mock` (`mock.method`,
    `t.mock.method`, `test.mock.method`); and a `stubGlobal` method
    (`vi.stubGlobal`) given a name that may be `"fetch"`, read as above;
  - undici's `setGlobalDispatcher` given anything but a new `Agent`,
    `ProxyAgent` or `EnvHttpProxyAgent`, at the call or in a name given one
    in its declaration and never written again (`const agent = new
    Agent(...)`): those are taken to send each request to the host it
    names, while a `MockAgent` may answer it itself. A conditional counts,
    even of two such agents (`proxy ? new ProxyAgent(proxy) : new
    Agent()`); an `Agent` given a `factory` does not (see
    [Outside the file](#outside-the-file-1));
  - a package that intercepts or mocks `fetch`, loaded with `import` (not
    `import type`, though an `import` whose specifiers are each marked
    `type`, as in `import { type RequestHandler } from "msw"`, counts),
    `require(...)` or `import(...)`: `nock`, `msw`, `@mswjs/interceptors`,
    `fetch-mock`, `fetch-mock-jest`, `@fetch-mock/jest`,
    `@fetch-mock/vitest`, `jest-fetch-mock` or `vitest-fetch-mock`, or a
    subpath of one (`msw/node`). TypeScript's `import nock =
    require("nock")` is not read. `nock`, `msw` and `@mswjs/interceptors`
    (any subpath, `msw/browser` too) give an openai v3 client the gap as
    well;
- replace an SDK member (`client.chat.completions.create = fn`,
  `OpenAI.prototype.x = ...`, `Object.assign(client.chat.completions, ...)`,
  `Object.setPrototypeOf(client, ...)`) or mock the SDK in a test
  (`jest.mock("openai")`, `vi.mock`, `vi.mock(import("openai"))`, or one of
  the test doubles above given an SDK object, as in
  `vi.spyOn(client.chat.completions, "create")` or
  `sinon.stub(client.chat.completions, "create")`): every route of that
  package is a `dynamic_feature` gap. Settings such as `client.apiKey`,
  `maxRetries` or `timeout` change nothing;
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
`spreads` is never listed in these files: on a call and in an
`unknown_client_factory` gap's `args`, `kwargs` names the keys written in
the options object at the call, and `splat` means some property is not
written there: a spread, a computed key, a `__proto__`, or options that
are not an object literal at the call (`create(PARAMS)`, a parameter, a
conditional), for which `kwargs` is empty. It is also set on an OpenAI
or Anthropic request whose request options may hold a `body`, which the
SDK sends in place of every argument written (rule 1 of
[Model values](#model-values-1)).

An SDK-shaped call on a receiver that is not a tracked client is an
`unknown_client_factory` gap, with the values written at the call listed. A
call is SDK-shaped when it ends in a request of the SDKs' own shape
(`.chat.completions.create`, `.models.generateContent`...), or in any member
that takes a model (`getGenerativeModel`, `chats.create`,
`assistants.create`, `fineTuning.jobs.create`...) with a model written at
the call. The receiver may be a client imported from another module
(`imported_name`, the common `import { openai } from "@/lib/openai"`), a
parameter, a factory function (`factory_call`), a class property never
assigned (`unassigned_attribute`) or an SDK call's result. `messages.create`
counts only where the file imports an Anthropic package (Twilio and Slack use
the same shape). An SDK member reached under a run-time name
(`client[method](...)`) is a `dynamic_dispatch` gap (`computed_member`); a
request method stored or passed as a value (`const create =
client.chat.completions.create`, `retry(client.chat.completions.create)`,
`create.bind(...)`) is `method_reference`.

### Dynamic features

A direct call of the global `eval` (also in parentheses, `(eval)(...)`), a
`with` statement, and scopes nested more
than 64 deep (not analyzed, a `truncated` gap) make every value not written at
its call, and every route, a `dynamic_feature` gap. `new Function(...)` sees
only the global scope and changes no binding of a module.

### Outside the file

Resolution is per file. A client or constant imported from another module is
a gap at the call. The report assumes the rest, in particular:

- Other modules do not write this module's objects, the properties of its
  class instances or what those properties hold, except as rules 5 and 6
  say (a subclass, an exported function's callers): an instance passed to
  another module's code (`deepMerge(this, src)` imported, `register(svc)`)
  or exported changes nothing the report reads.
- A write under a computed key (`obj[key] = v`) on anything but `this` does not
  set a class property, and an SDK request does not modify the options object
  it is passed.
- A class instance is known only as rule 5 lists: `this`, `new Bot()`
  (or `Reflect.construct(Bot, args)`) with the class's own name, the class,
  its prototype, the result of a method that returns `this`, and names and
  parameters given one. A
  member read under a run-time name off any other value reads no method,
  so the class's methods' reads of `this` stay confirmed (`const f =
  (this.bot as any)[m]; f.call(other)`): an array item (`bots[0][m]`), a
  property of another object (`registry.bot[m]`), a call's result
  (`make()[m]`), a callback's parameter (`[bot].forEach((b) =>
  b[m]...)`), the result of an async method that returns `this`, or a
  parameter of a function that no call in the file passes an instance.
  Run through `call`, `apply` or `bind` (`registry.bot[m].call(other)`),
  it still counts whatever the object, and so does a call right there for the
  callers of a `private` method (rules 5 and 6). An instance built through
  another name for the class (`const cls = Svc; const s = new cls()`) is not
  known to be one either, so request options a property of it holds are read
  although `(s as any)[section].path = url` may write them. Nor is an
  instance followed once it is stored in a property, an array or a
  collection, returned, or passed to a function the file does not call by
  name (a method of an object literal, a callback), or read through
  `arguments` or an object-pattern parameter of a method: code in the file
  that reaches its properties that way without naming them (`for (const s
  of registry) Object.values(s)`) is not seen. `this` in an object
  literal's own method is taken to be that object, so a member read there
  under a run-time name counts for no class even when `call`, `apply` or
  `bind` runs the method with an instance as `this`
  (`helper.run.call(new Bot(), "ask")`).
- An object that borrows a same-file class's prototype through a value rule 5
  does not list is not seen either, and `this.model` in the class's methods
  keeps the class's value there: an instance reached as an array item or a
  call's result (`new Proxy(bots[0], h)`, `Object.setPrototypeOf(o,
  make())`), and a prototype reached without the class's name
  (`Object.getPrototypeOf(bot)`, `bot.constructor.prototype`).
- A client a class reads through `this` is confirmed although a subclass in
  another module may replace it, as in Python, and so are request options
  it reads through `this` (`this.requestOptions`).
- In a script that is not in strict mode, a function declared inside a
  block also assigns the enclosing function's binding of its name when the
  block runs (Annex B of the language standard). That assignment is not
  seen, so the outer declaration's returns and default parameters are read
  as if it were the only one.
- The process environment, a `.env` file loaded at start-up and an Anthropic
  profile file are not known: a direct client built without a `baseURL` is
  reported direct.
- A `fetch` replaced or intercepted outside the file is not seen: nock, msw
  or a test double set up in another module (a Jest or Vitest setup file),
  or a polyfill installed at start-up. In the file, these are not seen
  either:
  - a package that [Routes](#routes-1) does not list (Polly.js,
    `cross-fetch/polyfill`), or one loaded through TypeScript's `import
    nock = require("nock")`;
  - testdouble.js's two-argument `td.replace(globalThis, "fetch")`, which
    makes its own fake: a `replace` counts only when it is given a
    replacement, so that `String#replace` is not taken for one;
  - the global object written under a run-time key (a `fetch` written under
    its own name counts wherever it is written) when the file reaches it
    through a parameter (`function set(target, k, v) { target[k] = v }`,
    even when a call passes `globalThis`), through `this` in a plain
    function, which is the global object in sloppy mode, through a call's
    result or a property of another object (`getGlobal()[k] = v`,
    `env.root[k] = v`, and `root = getGlobal()`), through a name
    destructured in its declaration (`const [root] = [globalThis]`), past
    the bounds on names and conditionals ([Bounds](#bounds-1)), or as a
    global name read off another with no parentheses, cast or reflective
    call around it (`globalThis.window[k] = v`, `window.self[k] = v`). A
    `const`, `let` or `var`, or a conditional, that may hold it counts
    ([Routes](#routes-1)), and so does a name the file writes without
    showing the value (`[root] = [globalThis]`);
  - a `dispatcher` given in `fetchOptions`, which both SDKs pass on to
    `fetch` (`new OpenAI({ fetchOptions: { dispatcher: new MockAgent() }
    })`, or in one request's options): `fetchOptions` is not read. Nor is an
    undici `Agent` given a `factory`, which builds the dispatcher for each
    origin and may send its requests elsewhere (`setGlobalDispatcher(new
    Agent({ factory }))`).
- openai v3's `OpenAIApi` without an axios instance of its own sends through
  axios's default instance, taken as axios ships it: interceptors or an
  adapter set on it (`axios.interceptors.request.use(...)`,
  `axios.defaults.adapter = ...`) or `axios-mock-adapter`, in this file or
  another, are not seen, and neither is nock or msw set up in another
  module.
- A file with a syntax error is read as far as tree-sitter parses it, and is
  listed under `skipped.syntax_errors`: a write in a region it cannot parse
  is not seen. Its grammar rejects a destructuring target in parentheses or
  a type assertion, which JavaScript and TypeScript accept
  (`({ m: (this.model) } = o)`, `[(this.model as string)] = o`), so such a
  write is missed.
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
name or property, 16 direct subclasses, 256 callers read per function, 16
items of a `middleware` array checked for the SDK's refusal fallback (a
longer array keeps its `endpoint_option` gap), 64 expanded spreads per
object literal, object literals read inside one another 64 deep (through
spreads and member chains; past either bound a spread is opaque), 4,096
distinct paths read off one name (past that every read of the name is an
`attribute` gap), member chains of 32 links (a longer one is not read),
model keys nested at most 4 literals below an option's value, as in Python
(deeper ones are not read), and 256 nested entries read per call (past that
a `truncated` gap). A client's or a request's options are read for a
`fetch`, `middleware`, `path`, `defaultBaseURL` or `body`
([Routes](#routes-1)) in at most 1,024 steps (each expression, entry and
name followed), 32 values deep, keeping at most 16 entries that set one:
past any of these, they are options the file cannot read. Nor can options
be read from a name passed to more than 64 calls or bound to more than 64
other names, from a class property written 64 times or more (its
assignments and the writes into it), from any class property of a name
whose reads are passed to more than 64 calls in all or bound to more than
64 names (SDK requests count as calls), or from a function with more than
64 `return`s. Whether a name hands its value out is followed through
every name it is bound to and every parameter it is passed to, and
theirs. Each object literal, name, class property and function's set of
returns read this way is read once and kept, every name a question about
handing out reaches is answered at once and kept, and a class's property
writes are kept by name, so a file's reads stay linear. Whether a name
may hold a class instance or the class (rule 5) is likewise answered for
every name it is read through at once and kept, reading each of their
values in at most 64 steps (past them, it may hold an instance of any
class); a method's parameter is read through the calls of its name,
which every method of that name shares, read once. A parameter reads
what the first 256 calls of its function (or method name) pass: past
them, the other calls are not read, so this bound fails open.
Whether a key written on the global object may be `"fetch"` is read in
at most 16 steps; past them, it may be, and so may a key of an object
literal of more than 256 entries that a loop runs over. Whether an
object written under a run-time key is the global object is followed at
most 4 levels deep (each name, name read off it, or side of a
conditional, `??`, `||` or `&&` is a level) and in at most 16 steps; past
either, it is taken not to be, so the write gives no `global_fetch` gap:
this bound fails open, and a route it leaves direct is reported
confirmed. A name written more than 64 times may be the global object
(that bound fails closed). Each object literal is read once, into an
index of its keys, however many calls spread it.
The output limits are the same as Python's: nested values (32 per call, 4,096
per file), listed candidates (32 per gap, 4,096 per file), names (200 bytes),
records including imports (20,000 per file, then a `truncated` gap) and
imported names (20,000 per file).
