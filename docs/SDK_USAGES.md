# SDK usages: what a report confirms

`arbor usages` and the MCP tool `find_api_usages` list where a Python project
calls the OpenAI, Anthropic and Gemini SDKs, directly or through LangChain's
OpenAI and Anthropic chat models ([LangChain chat
models](#langchain-chat-models)), with each call's routing and model values. A
consumer (a codemod, a reviewer) should treat the report this way:

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

## LangChain chat models

LangChain's chat models call the OpenAI and Anthropic SDKs for you. The report
follows these classes:

| Class | Imported from | Routes like |
|---|---|---|
| `ChatOpenAI` | `langchain_openai`, `langchain_community.chat_models`, `langchain.chat_models` | `OpenAI()` |
| `AzureChatOpenAI` | the same | `AzureOpenAI()`: `azure_openai` |
| `ChatAnthropic` | `langchain_anthropic`, `langchain_community.chat_models`, `langchain.chat_models` | `Anthropic()` |

The partner packages (`langchain_openai`, `langchain_anthropic`) are followed
the way an SDK is: `from ... import`, `import langchain_openai` and a wildcard
import. The deprecated `langchain_community.chat_models` and
`langchain.chat_models` re-export other providers' models too, so they, and
their submodules (`langchain_community.chat_models.openai`), are followed only
through `from ... import` of one of these classes; `import
langchain_community.chat_models as cm` is not followed. No other LangChain name
is tracked: other providers' chat models (`ChatOllama`, `ChatBedrock`,
`ChatGoogleGenerativeAI`, `ChatVertexAI`), embeddings (`OpenAIEmbeddings`),
completion models (`langchain_openai.OpenAI`), `init_chat_model(...)` and
LangChain's helpers report nothing.

- **The constructor binds the model.** `ChatOpenAI(model="gpt-4o")` is a
  construction and a call (`request: false`) whose `model` or `model_name`
  value is read under the value rules above. LangChain takes the model under
  either name:
  - given both (by keyword or through a same-file dict expanded with `**`),
    a `reassigned` gap with no key keeps both values unconfirmed;
  - given one next to a `**` expansion the file cannot read, a `splat` gap
    on that name keeps it unconfirmed: the expansion may hold the other name,
    which is no `TypeError` as a second `model=` would be;
  - given neither, `ChatOpenAI` and `ChatAnthropic` get a `missing_model` gap
    (LangChain's default model applies). `AzureChatOpenAI` serves its
    deployment, so a missing `model` there is not a gap.

  The classes are pydantic models that take keyword arguments only, so a
  positional argument is not read. A built model's requests send its model
  field, read when they are sent, unless the call passes its own `model`
  (see the calls below). So a write of `model` or `model_name` on any
  object other than `self` or a same-file class (`llm.model_name = ...`,
  `settings.model = ...`), or of an attribute with a run-time name, gives
  every LangChain chat model's value in the file a `dynamic_feature` gap.
  So does making the model configurable at run time
  (`configurable_fields(model_name=...)`, `configurable_alternatives(...)`).
- **The route.** A chat model routes like the SDK client it builds, under the
  client rules in [Routes](#routes). Its endpoint is `base_url` or LangChain's
  own name for it (`openai_api_base`, `anthropic_api_url`), routed by host.
  Given none, LangChain reads `OPENAI_API_BASE` or `ANTHROPIC_API_URL`, the SDK
  it builds then reads `OPENAI_BASE_URL` or `ANTHROPIC_BASE_URL`, and the
  partner packages fall back to the LangSmith gateway (`LANGSMITH_GATEWAY`). A
  file that sets any of these is an `endpoint_env` gap, as for an SDK client,
  and the client rules' other gaps apply as well (an environment written with
  a run-time name, a `**` expansion that may hold the endpoint). Two writes
  after the model is built count too: an endpoint attribute
  (`llm.openai_api_base = ...`) is `base_url_assigned`, and replacing the SDK
  client it holds (`llm.client`, `root_client`, `_client` and their async
  versions) is a `dynamic_feature` gap. Passing one of those to the
  constructor (`ChatOpenAI(client=OpenAI(base_url=...).chat.completions)`)
  routes it `unknown` with a `given_client` gap.
- **The pre-1.0 SDK.** The deprecated `ChatOpenAI` also runs on `openai<1.0`,
  where a chat model given no endpoint sends its requests through the
  module-level API. In a file that configures the module-level API (any
  routing setting, anywhere in the file), such a model routes `unknown` with a
  `langchain_module_config` gap. `langchain_openai` needs `openai>=1`, so this
  does not apply to it.
- **Calls on a chat model** (`llm.invoke(...)`,
  `llm.with_structured_output(...)`) are recorded with its routing, as calls on
  a client are. What such a call returns, including a copy
  (`llm.copy(update=...)`, which may change any field), is not followed.
  LangChain sends a call's keyword arguments over the model's own parameters,
  so a call may pass its own `model` (`llm.invoke(q, model=...)`,
  `llm.bind(model=...)`), which is sent in place of the field: a literal is
  read under the value rules above, a parameter is a gap, and a `**` expansion
  the file cannot read is a `splat` gap on `model`, as for an SDK request.
  LangChain's run options (`config`, `configurable`, `metadata`, `tags`,
  `callbacks`, `run_name`) and a call's positional input never reach the
  provider's request, so a `model` key inside them is not read.

`arbor retirements` checks only the model of these calls: LangChain takes its
own arguments and builds the request, so the SDK's argument rules do not
describe them (see [RETIREMENTS.md](RETIREMENTS.md)).

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
  `ANTHROPIC_BASE_URL` or `GOOGLE_GENAI_USE_VERTEXAI`, and for a LangChain
  chat model `OPENAI_API_BASE`, `ANTHROPIC_API_URL` or `LANGSMITH_GATEWAY`.
- A LangChain chat model's field names and aliases are those of the current
  `langchain-openai` and `langchain-anthropic`; which LangChain version the
  project runs is not known. The deprecated classes are read the same way.
  Where they differ, that can only add a gap: the deprecated `ChatAnthropic`
  has no `base_url` alias and does not read `ANTHROPIC_BASE_URL`, so for it a
  `base_url=` or that variable is a gap or a custom route it does not need.
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
