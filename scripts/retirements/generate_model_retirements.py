#!/usr/bin/env python3
"""Build crates/arbor-core/data/model-retirements.json from Arbor's retirement recipes.

The recipes are Arbor's research files: one YAML file per provider announcement, every
model ID, date and replacement copied from the provider's own deprecation pages and
checked against saved copies of those pages. They live in Arbor's research repository.
This script keeps only the public facts `arbor retirements` needs:

- each retired or retiring model ID, its provider, status, date and the replacement the
  provider lists (or none, or several);
- the parameter rules that can be checked from a call's keyword-argument names (for
  example "max_tokens is rejected on gpt-6-luna"), with how each was verified and, for
  a live check, which models and which API it called;
- the choices a recipe leaves to each repository (`report_choice`: for example whether
  gpt-6-luna should keep reasoning off), with every option's edits and effect. These
  are reported as decisions, never as automatic changes;
- provenance: the provider pages each recipe was built from, the date they were read,
  the recipe versions and the SHA-256 of each recipe file.

It writes one JSON file that `crates/arbor-core` embeds at build time.

    python3 scripts/retirements/generate_model_retirements.py \\
        --recipes PATH/TO/recipes --source "getArbor-dev/arbor-cloud@<commit>:recipes/"

    python3 scripts/retirements/generate_model_retirements.py --recipes PATH/TO/recipes --check

`--check` regenerates in memory and exits 1 when the committed file differs, naming the
recipes whose version or hash changed. Recipe hashes are taken over the file with CRLF
line endings turned into LF, so a Windows checkout hashes the same as a Linux one. On
every `cargo test`, the Rust test in `crates/arbor-core/src/retirements/mod.rs`
recomputes the file's digest (every key except `digest` and `generated_at`) and checks
its invariants, so a hand edit to the JSON fails CI.

Any recipe field the generator cannot translate (a new rule id, action, option, edit
kind or condition) stops generation with an error instead of being dropped.

The recipes live in Arbor's research repository, which is private, so `source` names a
commit outsiders cannot open. Every fact the data keeps carries the provider page it
came from (`source_urls`, `source_url`), so each one can be checked against the
provider's own page without the recipes.

Nothing here reaches the network. Requires PyYAML.
"""

from __future__ import annotations

import argparse
import copy
import datetime as dt
import hashlib
import json
import keyword
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - environment problem, not a code path
    sys.exit("PyYAML is required: pip install pyyaml")

SCHEMA = 1
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = REPO_ROOT / "crates" / "arbor-core" / "data" / "model-retirements.json"
GENERATOR = "scripts/retirements/generate_model_retirements.py"

# Recipe files, in the order their entries are written.
RECIPE_FILES = [
    "openai-2026-10-23.yaml",
    "openai-2027-01-14.yaml",
    "openai-past-shutdowns.yaml",
    "anthropic-retirements.yaml",
    "gemini-model-retirements.yaml",
]

# Which lookup table a recipe fills: the provider endpoint its dates apply to.
TABLES = {
    "openai-2026-10-23": "openai",
    "openai-2027-01-14": "openai",
    "openai-past-shutdowns": "openai",
    "anthropic-retirements": "anthropic",
    "gemini-api-model-retirements": "gemini_api",
    "vertex-gemini-model-retirements": "vertex",
}

DATE_KINDS = {
    "hard": "hard",
    "earliest_possible": "earliest",
    "not_earlier_than_listed": "earliest",
}

PROVIDER_NAMES = {
    "openai": "OpenAI",
    "anthropic": "Anthropic",
    "gemini_api": "Google",
    "vertex": "Google Cloud",
}

# ---------------------------------------------------------------------------
# Parameter rules
#
# Every recipe rule id must appear here, either translated or skipped with a reason, so a
# new rule upstream fails generation instead of being dropped. Conditions (endpoint,
# keyword arguments, source family, target), the effect and the verification level are
# read from the recipe; this table only adds the sentence shown to people (public
# phrasing, no internal notes) and, where a recipe condition cannot be seen from keyword
# names (a parameter's value, the last message's role), the closest condition that can.
# `{target}` in a message is replaced by the replacement model.
# ---------------------------------------------------------------------------

RULES = {
    # -- OpenAI, shutdown 2026-10-23 (GPT-5.6 targets) --
    # report_choice rules: the message says why the call needs the decision; the
    # options (edits, effect, status) are read from the recipe.
    "OAI-TOOLS-CC": {"message": "Chat Completions function tools work on {target} only at reasoning_effort \"none\" (400 otherwise)."},
    "OAI-TOOLS-CC-REASONING-SOURCE": {"message": "On {target}, Chat Completions tools work only without reasoning, and this reasoning model's call relies on it: move the call to the Responses API or review it by hand."},
    "OAI-LEGACY-FUNCTIONS": {"message": "Legacy functions/function_call were checked only on gpt-5.6-terra, where they work at reasoning_effort \"none\"; on other models, and without that setting, they were not tested."},
    "OAI-REASONING-DEFAULT": {"message": "{target} runs at medium reasoning effort by default, so it can reason; the model it replaces answered without a reasoning step."},
    "OAI-SAMPLING": {"message": "temperature, top_p and logprobs are accepted on {target} at reasoning_effort \"none\"; at its default effort a non-default temperature returns 400."},
    "OAI-SAMPLING-RESPONSES": {"message": "Whether {target} accepts temperature, top_p or logprobs on the Responses API is not verified."},
    # 28 Oct audit (RECIPE-DRIFT §7): the live check sent top_p and logprobs only at "none".
    "OAI-SAMPLING-REASONING-SOURCE": {"message": "On {target}, a non-default temperature returned 400 at the default reasoning effort and worked at reasoning_effort \"none\", which would turn off this reasoning model's reasoning (live check, 18 Oct 2026); top_p and logprobs were tested only at \"none\"."},
    "OAI-MAX-TOKENS": {
        # 28 Oct audit (RECIPE-DRIFT item 15): the API reference says "deprecated"; only
        # gpt-5.6-terra was called live.
        "message": "max_tokens is deprecated in favour of max_completion_tokens (OpenAI's API reference) and returned 400 on gpt-5.6-terra (live check, 18 Oct 2026; gpt-5.6-sol and gpt-5.6-luna not tested): use max_completion_tokens (same value).",
        "choice_note": "If {target} reasons, max_completion_tokens also bounds its reasoning tokens, so a small limit can be used up before any visible text.",
    },
    # 28 Oct audit (RECIPE-DRIFT item 16 and §7): no page says stop or logit_bias fails on
    # any GPT-5.6 model; the only evidence is one live call on gpt-5.6-terra at "none".
    "OAI-STOP": {
        "message": "stop returned 400 on gpt-5.6-terra at reasoning_effort \"none\" (live check, 18 Oct 2026); it was not tested on gpt-5.6-sol, gpt-5.6-luna or at the default effort. OpenAI's API reference names only o3 and o4-mini as not supporting stop.",
        "otherwise_verified": "unverified",
        "why": "the cited API reference says stop is unsupported on o3 and o4-mini only, so no page backs the rule where the live call did not reach",
    },
    "OAI-LOGIT-BIAS": {
        "message": "logit_bias returned 400 on gpt-5.6-terra at reasoning_effort \"none\" (live check, 18 Oct 2026); it was not tested on gpt-5.6-sol, gpt-5.6-luna or at the default effort.",
        "otherwise_verified": "unverified",
        "why": "no page says logit_bias is unsupported on any GPT-5.6 model, so nothing backs the rule where the live call did not reach",
    },
    "OAI-PENALTIES-N-SEED": {
        "message": "presence_penalty, frequency_penalty, n and seed are accepted on gpt-5.6-terra at reasoning_effort \"none\".",
        "else_message": "presence_penalty, frequency_penalty, n and seed were verified on gpt-5.6-terra only, not on {target}.",
    },
    "OAI-PREDICTED-OUTPUTS": {"message": "prediction (Predicted Outputs) is not verified on {target}."},
    "OAI-PRO-MODE": {
        "message": "o1-pro's replacement is {target} in pro mode: add reasoning={\"mode\": \"pro\"} (Responses API).",
        "else_message": "o1-pro's replacement is {target} in pro mode, which needs the Responses API with reasoning.mode \"pro\".",
    },
    "OAI-LEGACY-COMPLETIONS-ENDPOINT": {"message": "{target} does not support the legacy Completions endpoint: move the call to Chat Completions or Responses."},
    "OAI-ASSISTANTS": {"message": "The Assistants API is shut down and {target} does not support it."},
    "OAI-TOKENIZER-AND-REGISTRIES": {"skip": "needs to follow the model string beyond the call (tokenizers, pricing tables)"},
    "OAI-MULTIMODAL": {"skip": "depends on message content parts, not on keyword arguments"},
    "OAI-CACHE": {"message": "Replace prompt_cache_retention with prompt_cache_options.ttl on {target}."},
    "OAI-SAFEGUARDS": {"message": "{target} has real-time safeguards that can refuse or pause some cyber and biology output."},
    # -- OpenAI, shutdown 2027-01-14 (gpt-6-luna target) --
    "G6-TOOLS-CC": {"message": "Chat Completions function tools work on {target} only at reasoning_effort \"none\" (400 otherwise)."},
    "G6-LEGACY-FUNCTIONS": {"message": "Legacy functions/function_call work on {target} at reasoning_effort \"none\"; without it they were not tested."},
    "G6-REASONING-DEFAULT": {"message": "{target} runs at medium reasoning effort by default, so it can reason; the model it replaces answered without a reasoning step."},
    "G6-SAMPLING": {"message": "temperature, top_p and logprobs are accepted on {target} at reasoning_effort \"none\"; at its default effort a non-default temperature returns 400."},
    # 28 Oct audit (RECIPE-DRIFT item 18, decision F4): the "Using GPT-6" guide states the
    # rule for the Responses API too; still no Responses call was made.
    "G6-SAMPLING-RESPONSES": {"message": "OpenAI's \"Using GPT-6\" guide says to remove temperature, top_p and top_logprobs on {target} unless reasoning effort is \"none\", and on the Responses API to drop message.output_text.logprobs from include. Arbor has not tested this on the Responses API."},
    "G6-MAX-TOKENS": {
        "message": "max_tokens is rejected on {target}: use max_completion_tokens (same value).",
        "choice_note": "If {target} reasons, max_completion_tokens also bounds its reasoning tokens, so a small limit can be used up before any visible text.",
    },
    "G6-STOP-LOGIT-BIAS": {"message": "stop and logit_bias are not supported on {target} (400)."},
    "G6-PENALTIES": {"message": "presence_penalty and frequency_penalty failed on {target} in every test (server error)."},
    "G6-N-SEED": {"message": "n and seed are accepted on {target} at reasoning_effort \"none\"."},
    "G6-PREDICTED-OUTPUTS": {"message": "prediction (Predicted Outputs) is not supported on {target} (400)."},
    "G6-CACHE": {"message": "Replace prompt_cache_retention with prompt_cache_options.ttl on {target}."},
    "G6-LEGACY-COMPLETIONS-ENDPOINT": {"message": "{target} does not support the legacy Completions endpoint: move the call to Chat Completions or Responses."},
    "G6-ASSISTANTS": {"message": "The Assistants API is shut down and {target} does not support it."},
    "G6-REGISTRIES": {"skip": "needs to follow the model string beyond the call (tokenizers, pricing tables)"},
    # 28 Oct audit (RECIPE-DRIFT item 19): OpenAI's own words; no page calls GPT-4.1 a flagship.
    "G6-TIER-CHANGE": {"message": "OpenAI describes {target} as its \"most efficient model for focused, high-volume tasks\", so answer quality may change compared with the model it replaces."},
    # -- Anthropic --
    # 28 Oct audit (RECIPE-DRIFT items 23 and 28, decision F3): API version 2026-10-21
    # removes the three parameters on every model; the SDK majors that send it are named
    # on Anthropic's migration page.
    "ANT-SAMPLING": {"message": "temperature, top_p or top_k return 400 on {target}: at any value under API version 2026-10-21 (sent by anthropic Python >= 2.0.0 and @anthropic-ai/sdk >= 1.0.0), and at a non-default value under 2023-06-01. Removing them is the documented fix and changes behaviour."},
    "ANT-SAMPLING-HAIKU45": [
        {"message": "{target} is not documented as rejecting temperature, top_p or top_k on API version 2023-06-01; on 2026-10-21 (sent by anthropic Python >= 2.0.0 and @anthropic-ai/sdk >= 1.0.0) they return 400 on every model."},
        {
            "suffix": "both",
            "effect": "review",
            "all_kwargs": ["temperature", "top_p"],
            "any_kwargs": [],
            "verified": "unverified",
            "message": "Whether {target} accepts temperature and top_p together is not stated.",
            "why": "recipe caveat: temperature and top_p sent together are unverified for this target",
        },
    ],
    # report_choice rules (the lead's 20 Oct decision applies the founder's 18 Oct
    # one to Claude's thinking): the options are read from the recipe.
    "ANT-THINKING-DEFAULT": {"message": "{target} thinks by default (adaptive thinking) when the call sends no thinking; the model it replaces answered without thinking."},
    "ANT-THINKING-DISABLED": {
        "any_kwargs": ["thinking"],
        "message": "This call sets thinking, and Arbor cannot read its value: if it is {\"type\": \"disabled\"}, the call returns 400 on {target}.",
        "why": "param_equals on thinking.type: the value is not visible, so the parameter's presence brings up the choice",
    },
    "ANT-THINKING-BUDGET": {
        "any_kwargs": ["thinking"],
        "message": "Check thinking: \"enabled\" with budget_tokens returns 400 on {target}; it takes adaptive thinking with an effort level instead.",
        "why": "param_equals on thinking.type: the value is not visible, so the parameter's presence triggers a check",
    },
    "ANT-PREFILL": {
        "effect": "note",
        "message": "A request that ends with an assistant turn (prefill) returns 400 on {target}.",
        "why": "messages_end_with_role is not visible from keyword names: shown as a note",
    },
    "ANT-FORCED-TOOL": {
        "any_kwargs": ["tool_choice"],
        "message": "Check tool_choice: \"any\" and \"tool\" return 400 on {target}.",
        "why": "param_in on tool_choice.type: the value is not visible, so the parameter's presence triggers a check",
    },
    "ANT-TOKENS": {
        "targets": ["claude-sonnet-5-5", "claude-opus-4-8"],
        "message": "{target} uses more tokens for the same text; check max_tokens headroom.",
        "why": "the evidence covers Sonnet 5.5 and Opus 4.7 and later",
    },
    "ANT-OUTPUT-FORMAT-AND-BETAS": {
        "any_kwargs": ["output_format", "betas"],
        "targets": ["claude-sonnet-5-5"],
        "message": "output_format and several beta flags changed on {target} (output_config.format replaces output_format).",
        "why": "the anthropic-beta header is not a keyword name; the evidence is the Sonnet 5.5 guide",
    },
    "ANT-TOOL-VERSIONS": {
        "effect": "note",
        "any_kwargs": ["tools"],
        "targets": ["claude-sonnet-5-5"],
        "message": "Anthropic-defined tools changed versions on {target} (text_editor_20250728, code_execution_20260521, computer use).",
        "why": "tool type values are not visible: shown as a note on calls with tools",
    },
    "ANT-STOP-REASONS-AND-SAFETY": {"message": "Handle stop_reason \"refusal\" and \"model_context_window_exceeded\"."},
    "ANT-SDK-1X": {"skip": "a policy for automated fixes (never bump the SDK), not a property of a call"},
    "ANT-TEXT-COMPLETIONS": {"message": "Whether {target} is served on the legacy Text Completions API is not verified."},
    # -- Gemini Developer API --
    "GEM-LEGACY-SDK": {"message": "The google-generativeai SDK is deprecated; whether its calls work with {target} is not verified."},
    "GEM-SAMPLING": {"message": "temperature, top_p and top_k are ignored on {target}."},
    "GEM-SAMPLING-OTHER-TARGETS": {"message": "temperature, top_p and top_k on {target} are not verified."},
    "GEM-PREFILL": {
        "effect": "note",
        "targets": ["gemini-3.6-flash", "gemini-3.8-flash"],
        "message": "A request whose last turn is a model turn returns 400 on {target}.",
        "why": "contents_end_with_role is not visible from keyword names: shown as a note for 3.6+ targets",
    },
    "GEM-CANDIDATE-COUNT": {"message": "candidate_count is not supported on Gemini 3 and later."},
    "GEM-THINKING-BUDGET": {"message": "Replace thinking_budget with thinking_level on {target}."},
    "GEM-THOUGHT-SIGNATURES": {"message": "Function calling on {target} needs thought signatures kept across turns; not verified for this call."},
    "GEM-25-ACCESS": {"skip": "applied to entries instead: a note on replacements that are gemini-2.5 models"},
    # -- Gemini on Google Cloud --
    "VTX-SHARED-GEMINI3": {
        "effect": "note",
        "message": "Gemini 3.x parameter changes are documented for the Gemini API and not re-checked on Vertex AI.",
        "why": "report_only with no condition: shown as a note so it does not hide the replacement",
    },
}

# Notes attached to entries that a recipe ties to a conflict, in public phrasing.
CONFLICT_NOTES = {
    "CONFLICT-HAIKU-REPLACEMENT": "Anthropic's Haiku 5.5 migration guide recommends claude-haiku-5-5 instead.",
    "CONFLICT-DEEP-RESEARCH": "Google's changelog also offers deep-research-max-preview-04-2026.",
}

ACTION_EFFECTS = {
    "add_param": "add",
    "rewrite_param": "rename",
    "report_only": "review",
    "keep": "keep",
    "disclose": "note",
    "founder_decision": "note",
    "report_choice": "choice",
}

# Every rule action a recipe may use. A new one stops generation until it is mapped
# here (or given an explicit effect in RULES).
KNOWN_ACTIONS = set(ACTION_EFFECTS) | {"add_param_or_report", "constraint"}

# Actions that change a call without asking. The founder's 18 Oct decision makes the
# reasoning effort a per-repository choice, and the lead's 20 Oct decision does the
# same for Claude's thinking, so none of these may add or name them (the recipe
# validator's rules C1 and C2 say the same; this keeps the data honest if it slips).
AUTOMATIC_ACTIONS = {"add_param", "add_param_or_report", "rewrite_param", "keep", "remove_param"}
EFFORT_NAMES = ("reasoning_effort", "effort")
PER_REPO_PARAMS = EFFORT_NAMES + ("thinking",)
# Effects that edit a call in the data (a choice's options are never applied).
AUTOMATIC_EFFECTS = {"add", "rename", "keep"}

WHEN_KEYS = {"endpoint", "request_has_any", "request_lacks", "source_family", "target", "source_ids", "client"}

# How each option of a report_choice reads to people. Option ids are the recipe's; a
# new one stops generation until it has a label here. "<choice>/<id>" labels an option
# of one choice only (Claude thinks, where OpenAI's models reason).
OPTION_LABELS = {
    "reasoning_none": "keep reasoning off",
    "model_default": "let the model reason",
    "thinking_between_tools": "think only between tools",
    "ANT-THINKING-DEFAULT/model_default": "let the model think",
}
OPTION_KEYS = {"id", "edits", "effect", "evidence", "status"}
EDIT_KINDS = {"add_param", "remove_param", "keep"}
OPTION_STATUSES = {"confirmed": "confirmed", "unverified": "unverified"}

# Recipe text copied into the data is shown to people; it must not carry internal
# references (live-check ids, decisions, the memo).
INTERNAL_WORDS = re.compile(r"\b(L-[A-Z]+-\d+|founder|decision|memo|Anandb71)\b", re.IGNORECASE)
# A live-check id in brackets on its own, such as "(L-OAI-12)", only points to the
# record: it is dropped. Any other internal reference still stops generation.
LIVE_CHECK_REF = re.compile(r"\s*\(L-[A-Z]+-\d+(?:(?:,\s*|\s+and\s+)L-[A-Z]+-\d+)*\)")

# Every live check recorded in the recipes called Chat Completions (the recipe headers
# and live-checks/ records say so); no Responses API call was made. A live result
# therefore vouches only for chat.completions calls to the models it lists.
LIVE_ENDPOINTS = ["chat.completions"]

# Fine-tunes: whether the base model a recipe lists for fine-tunes can itself be
# fine-tuned, and the public sentence that says so. Each recipe's fine_tuned.reason
# must contain `reason_has`, so a change upstream stops generation here.
FINE_TUNED = {
    "openai-2026-10-23": {
        "fine_tunable": False,
        "reason_has": "list Fine-tuning as Not supported",
        "note": "OpenAI's GPT-5.6 model pages list fine-tuning as not supported, so the fine-tune cannot be recreated on the listed base.",
    },
    "openai-2027-01-14": {
        "fine_tunable": False,
        "reason_has": "gpt-6-luna lists Fine-tuning as Not supported",
        # No rows: the recipe matches fine-tunes of its snapshots and lists no base.
        "bases": "snapshots",
        "note": "gpt-6-luna, the replacement OpenAI lists for these models, does not support fine-tuning.",
    },
}


class GenerationError(Exception):
    pass


# ---------------------------------------------------------------------------
# Loading
# ---------------------------------------------------------------------------


class UniqueKeyLoader(yaml.SafeLoader):
    """A SafeLoader that rejects a mapping key given twice (PyYAML keeps the last one)."""


def _construct_mapping(loader, node, deep=False):
    seen = set()
    for key_node, _ in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in seen:
            raise GenerationError(f"duplicate key {key!r} at line {key_node.start_mark.line + 1}")
        seen.add(key)
    return yaml.SafeLoader.construct_mapping(loader, node, deep)


UniqueKeyLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _construct_mapping)


def load_recipes(recipes_dir: Path):
    """Returns [(recipe_dict, file_name, sha256)] in file order."""
    out = []
    for name in RECIPE_FILES:
        path = recipes_dir / name
        if not path.is_file():
            raise GenerationError(f"missing recipe file: {path}")
        # LF line endings before hashing: a checkout with core.autocrlf (Windows)
        # holds the same recipe with CRLF, and must hash the same.
        raw = path.read_bytes().replace(b"\r\n", b"\n")
        sha = hashlib.sha256(raw).hexdigest()
        text = raw.decode("utf-8")
        for doc in yaml.load_all(text, Loader=UniqueKeyLoader):
            if not doc:
                continue
            if doc.get("type") != "arbor.dev/recipe/v1alpha1":
                raise GenerationError(f"{name}: unexpected type {doc.get('type')!r}")
            out.append((doc, name, sha))
    ids = [doc["id"] for doc, _, _ in out]
    if sorted(ids) != sorted(TABLES):
        raise GenerationError(f"recipe ids {ids} do not match the known tables {sorted(TABLES)}")
    return out


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def iso(value) -> str | None:
    """A YAML date (or ISO string) as YYYY-MM-DD; None for text such as 'N/A'."""
    if isinstance(value, dt.datetime):
        return value.date().isoformat()
    if isinstance(value, dt.date):
        return value.isoformat()
    if isinstance(value, str) and re.fullmatch(r"\d{4}-\d{2}-\d{2}", value):
        dt.date.fromisoformat(value)
        return value
    return None


def clean_text(text) -> str | None:
    """A page cell without Markdown code marks; None for 'none listed' cells."""
    if text is None:
        return None
    text = str(text).replace("`", "").strip()
    if not text or text in {"---", "(none listed)"}:
        return None
    return text


def single_id(text: str | None) -> str | None:
    """The model ID when a replacement cell names exactly one (`gpt-5.6-sol` (`reasoning.mode: pro`))."""
    if not text:
        return None
    ids = re.findall(r"`([^`]+)`", text)
    if len(ids) >= 1 and " or " not in text and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._@:/-]*", ids[0]):
        # `gpt-5.6-sol` (`reasoning.mode: pro`): the second span is a parameter, not a model.
        if len(ids) == 1 or ":" in ids[1]:
            return ids[0]
    return None


def category(model: str, kind: str | None) -> str:
    """Arbor's label for what a model does; only used to explain why a swap needs review."""
    lower = model.lower()
    if kind == "managed_agent" or lower.startswith(("deep-research", "antigravity")):
        return "agent"
    if "embedding" in lower:
        return "embedding"
    if any(s in lower for s in ("imagen", "image", "dall-e", "imagetext", "nano-banana")):
        return "image"
    if any(s in lower for s in ("veo-", "sora", "omni", "video")):
        return "video"
    if any(s in lower for s in ("tts", "audio", "realtime", "transcribe", "whisper", "live", "speech")):
        return "audio"
    if "robotics" in lower:
        return "robotics"
    if "lyria" in lower:
        return "music"
    if "moderation" in lower:
        return "moderation"
    if "computer-use" in lower:
        return "computer_use"
    return "text"


def human_date(iso_date: str) -> str:
    d = dt.date.fromisoformat(iso_date)
    return f"{d.day} {d.strftime('%b')} {d.year}"


def verification(item: dict) -> tuple[str, str | None]:
    """`unverified` whenever the recipe says so (a live result may cover only part of
    the claim), else `live` with the date of a recorded live result, else `docs`."""
    if item.get("status") == "unverified":
        return "unverified", None
    live = item.get("live_result")
    if isinstance(live, dict) and live.get("date"):
        return "live", iso(live["date"])
    return "docs", None


def live_models(item: dict) -> list:
    """The models a live result called: it vouches for those only."""
    live = item.get("live_result")
    if not isinstance(live, dict):
        return []
    models = live.get("models")
    if models is None:
        raise GenerationError(f"{item.get('id')}: live_result without models")
    return [str(m) for m in as_list(models)]


def otherwise_verified(rule: dict, item: dict) -> str:
    """The label a live rule carries where its live check did not reach: `docs` when the
    recipe cites a page for it, unless RULES says the cited page does not cover the
    rule there (it then needs a `why`)."""
    override = item.get("otherwise_verified")
    if override is None:
        return "docs" if docs_backed(rule) else "unverified"
    if override != "unverified":
        raise GenerationError(f"{rule['id']}: otherwise_verified can only be lowered to unverified")
    return override


def live_dates_cited(rule: dict, message: str) -> None:
    """A message that names its live check ("live check, 18 Oct 2026") must name the
    recipe's live_result date, so a newer check cannot leave an old date in the text."""
    for cited in re.findall(r"live check, (\d{1,2} [A-Z][a-z]{2} \d{4})", message):
        live = rule.get("live_result") or {}
        date = iso(live.get("date")) if isinstance(live, dict) else None
        if date is None or human_date(date) != cited:
            raise GenerationError(
                f"{rule['id']}: the message cites a live check of {cited}, but the recipe's live_result date is {date!r}"
            )


def docs_backed(item: dict) -> bool:
    """Whether the recipe cites a page for the rule, so it stands on the docs where the
    live check did not reach."""
    return bool(item.get("source_url") or item.get("evidence") or item.get("evidence_quotes"))


def new_entry(model, table, recipe, status, date, replacement, replacement_text, action,
              rule_set, source_family, kind, verified, verified_on, notes):
    if replacement is not None and replacement == model:
        raise GenerationError(f"{model}: replacement equals the model")
    return {
        "model": model,
        "table": table,
        "recipe": recipe,
        "status": status,
        "date": date,
        "replacement": replacement,
        "replacement_text": replacement_text,
        "action": action,
        "rule_set": rule_set,
        "source_family": source_family,
        "category": category(model, kind),
        "kind": kind,
        "verified": verified,
        "verified_on": verified_on,
        "notes": notes,
    }


def action_of(value: dict) -> str:
    action = value.get("action")
    if action in ("rewrite", "rewrite_with_params"):
        return "rewrite"
    if action == "report_only":
        return "report_only"
    raise GenerationError(f"unknown action {action!r}")


# ---------------------------------------------------------------------------
# Entries per recipe
# ---------------------------------------------------------------------------


def openai_dated(recipe: dict) -> tuple[list, list]:
    """openai-2026-10-23 and openai-2027-01-14: every entry shares the recipe date."""
    rid = recipe["id"]
    date = iso(recipe["shutdown_date"])
    entries, fine_tunes = [], []
    for model, value in recipe["value_map"].items():
        verified, verified_on = verification(value)
        notes = []
        status = "retiring"
        live = value.get("live_result") or {}
        outcome = str(live.get("outcome", ""))
        checked = iso(live.get("date"))
        if (value.get("basis") == "models_api" or "404" in outcome) and not checked:
            raise GenerationError(f"{model}: a live finding without a live_result date")
        if value.get("basis") == "models_api":
            # The deprecations page has no row for the bare aliases (they appear there only as
            # recommended replacements in older rows); the Chat Completions API reference
            # dates each one, and the recipe quotes it (28 Oct audit, RECIPE-DRIFT items 10-13).
            reference = f'"{model}" Deprecated. Announced shutdown date: {date}'
            quotes = [str(q.get("quote", "")) for q in value.get("evidence_quotes") or [] if isinstance(q, dict)]
            if not any(reference in quote for quote in quotes):
                raise GenerationError(
                    f"{model}: an alias dated by the models API needs the API reference quote {reference!r}"
                )
            notes.append(
                f"{model} is an alias: OpenAI's deprecations page gives it no shutdown date (its "
                f"{human_date(date)} rows name only dated snapshots), but OpenAI's API reference lists it "
                f"as deprecated with shutdown date {human_date(date)}, and its models API gave that date "
                f"on {human_date(checked)}."
            )
        elif value.get("kind") == "listed_variant" and "404" in outcome:
            status = "shut_down"
            notes.append(f"Not a callable model ID: the API answers 404 (checked {human_date(checked)}).")
        elif "404" in outcome:
            status = "shut_down"
            notes.append(
                f"OpenAI lists {human_date(date)}, but the API already answers 404 (checked {human_date(checked)})."
            )
        text = value.get("listed_replacement_text")
        replacement = value.get("to")
        entries.append(new_entry(
            model, "openai", rid, status, date, replacement, clean_text(text) if text else None,
            action_of(value), rid, value.get("source_family"), value.get("kind"),
            verified, verified_on, notes,
        ))
    fine_tunes = fine_tuned(recipe, date)
    return entries, fine_tunes


def fine_tuned(recipe: dict, date: str) -> list:
    """Fine-tunes the recipe reports: the bases its `rows` list (with the base OpenAI
    lists for them), or, for a recipe without rows, its snapshots. Whether the listed
    base can be fine-tuned comes from FINE_TUNED, checked against the recipe's reason."""
    rid = recipe["id"]
    fine = recipe.get("fine_tuned")
    if not fine:
        return []
    spec = FINE_TUNED.get(rid)
    if spec is None:
        raise GenerationError(f"{rid}: fine_tuned has no entry in FINE_TUNED")
    reason = " ".join(str(fine.get("reason", "")).split())
    if spec["reason_has"] not in reason:
        raise GenerationError(
            f"{rid}: fine_tuned.reason no longer says {spec['reason_has']!r}; update FINE_TUNED"
        )
    if fine.get("action") != "report_only":
        raise GenerationError(f"{rid}: fine_tuned.action {fine.get('action')!r} is not report_only")
    rows = fine.get("rows") or {}
    if rows and spec.get("bases"):
        raise GenerationError(f"{rid}: fine_tuned has rows now; drop `bases` from FINE_TUNED")
    if not rows and spec.get("bases") != "snapshots":
        raise GenerationError(f"{rid}: fine_tuned lists no rows")
    pairs = []
    for label, base_replacement in rows.items():
        if not label.startswith("ft-"):
            raise GenerationError(f"{rid}: fine-tune label {label!r} does not start with ft-")
        pairs.append((label[len("ft-"):], base_replacement))
    if not rows:
        pairs = [
            (model, None)
            for model, value in recipe["value_map"].items()
            if value.get("kind") == "snapshot"
        ]
        if not pairs:
            raise GenerationError(f"{rid}: fine_tuned without rows and without snapshots")
    if INTERNAL_WORDS.search(spec["note"]):
        raise GenerationError(f"{rid}: FINE_TUNED note has internal wording")
    return [
        {
            "base": base,
            "table": "openai",
            "recipe": rid,
            "status": "retiring",
            "date": date,
            "replacement_base": replacement,
            "fine_tunable": spec["fine_tunable"],
            "note": spec["note"],
        }
        for base, replacement in pairs
    ]


def openai_upcoming(recipe: dict) -> list:
    """The page's other upcoming rows that no dated recipe maps yet (report only, no rules)."""
    entries = []
    for model, row in (recipe.get("not_in_this_recipe_ids") or {}).items():
        text = row.get("listed_replacement_text")
        replacement = single_id(text)
        shown = clean_text(text)
        entries.append(new_entry(
            model, "openai", recipe["id"], "retiring", iso(row["shutdown"]), replacement,
            None if shown == replacement else shown, "report_only", None, None, None, "docs", None, [],
        ))
    return entries


def openai_past(recipe: dict) -> list:
    entries = []
    for model, value in recipe["value_map"].items():
        if value.get("state") != "shut_down":
            raise GenerationError(f"{model}: past shutdown with state {value.get('state')!r}")
        text = value.get("listed_replacement_text")
        verified, verified_on = verification(value)
        entries.append(new_entry(
            model, "openai", recipe["id"], "shut_down", iso(value["shutdown"]), value.get("to"),
            clean_text(text) if text else None, "report_only", None, None, None,
            verified, verified_on, [],
        ))
    return entries


def anthropic(recipe: dict) -> list:
    rid = recipe["id"]
    vm = recipe["value_map"]
    entries = []
    for model, value in vm.items():
        verified, verified_on = verification(value)
        state = value.get("state")
        date = iso(value.get("retires"))
        notes = []
        replacement = value.get("to")
        text = clean_text(value.get("listed_replacement_text"))
        if state == "retired":
            status = "shut_down"
        elif state == "deprecated" and date:
            status = "retiring"
        elif state == "deprecated":
            status = "deprecated"
            announced = iso(value.get("announced"))
            notes.append(
                f"Deprecated on {human_date(announced)}; Anthropic has not announced a retirement date."
            )
        elif state in ("inferred_deprecated", "inferred_retired"):
            status = "unconfirmed"
            target = value["resolves_to"]
            if target not in vm:
                raise GenerationError(f"{model}: resolves_to {target} is not an entry")
            date = iso(vm[target].get("retires"))
            verb = "retired" if vm[target].get("state") == "retired" else "retires"
            notes.append(
                f"Alias of {target}, which Anthropic {verb} on {human_date(date)}; "
                "Anthropic does not say whether the alias stops working."
            )
            candidates = value.get("candidate_replacements")
            if candidates and replacement is None:
                text = " or ".join(dict.fromkeys(candidates.values()))
        else:
            raise GenerationError(f"{model}: unknown Anthropic state {state!r}")
        if value.get("conflict") in CONFLICT_NOTES:
            notes.append(CONFLICT_NOTES[value["conflict"]])
        entries.append(new_entry(
            model, "anthropic", rid, status, date, replacement, text, action_of(value), rid,
            None, value.get("kind"), verified, verified_on, notes,
        ))
    return entries


GEMINI_STATES = {
    "shut_down": "shut_down",
    "scheduled": "retiring",
    "auto_routed": "auto_routed",
    "replacement_listed_no_date": "deprecated",
    "past_earliest_date": "unconfirmed",
}


def gemini_api(recipe: dict) -> list:
    rid = recipe["id"]
    entries = []
    for model, value in recipe["value_map"].items():
        verified, verified_on = verification(value)
        status = GEMINI_STATES[value["state"]]
        if status == "shut_down" and value.get("status") == "unverified":
            status = "unconfirmed"
        notes = []
        if value.get("variant_of"):
            notes.append(
                f"Spelling of {value['variant_of']} used in Google's launch notes; Google does not confirm they are the same model."
            )
        if status == "auto_routed":
            notes.append(f"Google routes requests for this ID to {value['to']}: calls still work.")
        if value.get("conflict") in CONFLICT_NOTES:
            notes.append(CONFLICT_NOTES[value["conflict"]])
        text = clean_text(value.get("listed_replacement_text"))
        entries.append(new_entry(
            model, "gemini_api", rid, status, iso(value.get("shutdown")), value.get("to"), text,
            action_of(value), rid, None, value.get("kind"), verified, verified_on, notes,
        ))
    for group in recipe.get("changelog_only_shutdowns") or []:
        confirmed = group.get("status") == "confirmed" and group.get("shut_down")
        date = iso(group.get("shut_down") or group.get("announced_shutdown") or group.get("announced_deprecation"))
        text = group.get("replacement_text")
        if text:
            text = re.sub(r"\s*\(changelog\)\s*$", "", text)
        notes = ["Listed in Google's changelog, not in the deprecations table."]
        if group.get("note") and "inference" in group["note"]:
            notes.append("Google's changelog retires the model by name; matching it to this ID is Arbor's reading.")
        for model in group["ids"]:
            entries.append(new_entry(
                model, "gemini_api", rid, "shut_down" if confirmed else "unconfirmed", date, None,
                text, "report_only", None, None, None,
                "docs" if group.get("status") == "confirmed" else "unverified", None, list(notes),
            ))
    return entries


def vertex(recipe: dict) -> list:
    rid = recipe["id"]
    entries = []
    for model, value in recipe["value_map"].items():
        verified, verified_on = verification(value)
        if "retired" in value:
            status, date = "shut_down", iso(value["retired"])
        else:
            status, date = "retiring", iso(value["retires"])
            if value.get("status") == "unverified":
                status = "unconfirmed"
        text = clean_text(value.get("listed_replacement_text"))
        entries.append(new_entry(
            model, "vertex", rid, status, date, value.get("to"), text, action_of(value), rid,
            None, value.get("kind"), verified, verified_on, [],
        ))
    return entries


# ---------------------------------------------------------------------------
# Rules
# ---------------------------------------------------------------------------


def as_list(value) -> list:
    if value is None:
        return []
    return list(value) if isinstance(value, (list, tuple)) else [value]


# Endpoint names a recipe uses as keys of a per-endpoint parameter.
KNOWN_ENDPOINTS = {"chat.completions", "responses", "completions_legacy", "assistants"}

# Rule keys that only document the rule (evidence, history); the data does not need them.
RULE_INFO_KEYS = {
    "id", "action", "status", "when", "evidence", "summary", "live_check", "source_url",
    "live_result", "note", "founder_decision", "sdk_compat", "evidence_quotes",
    "consequence_if_not_chosen", "proposed_action", "reason", "disclose",
}
# Rule keys this generator translates.
RULE_HANDLED_KEYS = {
    "param", "keep_when", "otherwise", "if_param_present_and_not_none", "choice", "options",
    "resolved", "default", "see_choice", "choice_note", "requires",
}
# Rule keys that change what the rule does in a way keyword names cannot show: the
# RULES entry must say how it is translated (`why`, or an `else_message` for the
# endpoint the change cannot be made on).
RULE_OVERRIDE_KEYS = {"only_if", "caveat", "merge", "non_responses_endpoint"}


def is_kwarg_name(name) -> bool:
    """A name a Python call can pass as a keyword argument: an identifier that is not
    a reserved word (`from=` cannot be passed, so a param `{from: ..., to: ...}` under
    an add action is a rename written with the wrong action, not two parameters)."""
    return (
        isinstance(name, str)
        and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name) is not None
        and not keyword.iskeyword(name)
    )


def add_specs(param, endpoints) -> list:
    """Normalises a recipe `param` into [{endpoint, name, value}]: `{name, value}`,
    `{param_name: value}`, or `{endpoint: {param_name: value}}`."""
    if not isinstance(param, dict) or not param:
        raise GenerationError(f"unexpected param {param!r}")
    if "name" in param:
        if set(param) != {"name", "value"}:
            raise GenerationError(f"unexpected param {param!r}")
        targets = endpoints or ["*"]
        specs = [{"endpoint": e, "name": param["name"], "value": param["value"]} for e in targets]
    elif set(param) <= KNOWN_ENDPOINTS:
        specs = []
        for endpoint, inner in param.items():
            if not isinstance(inner, dict) or len(inner) != 1:
                raise GenerationError(f"unexpected param for {endpoint}: {inner!r}")
            (name, value), = inner.items()
            specs.append({"endpoint": endpoint, "name": name, "value": value})
    elif set(param) & KNOWN_ENDPOINTS:
        raise GenerationError(f"param {param!r} mixes endpoints and parameter names")
    else:
        specs = [
            {"endpoint": e, "name": name, "value": value}
            for name, value in param.items()
            for e in (endpoints or ["*"])
        ]
    bad = sorted({str(s["name"]) for s in specs if not is_kwarg_name(s["name"])})
    if bad:
        raise GenerationError(f"param {param!r}: {bad} cannot be passed as keyword arguments")
    return specs


def keyword_names(rid: str, what: str, names) -> list:
    names = [str(n) for n in as_list(names)]
    bad = [n for n in names if not is_kwarg_name(n)]
    if bad or not names:
        raise GenerationError(f"{rid}: {what} {names} are not keyword names")
    return names


def public_text(rid: str, text) -> str:
    """Recipe text shown to people as it is: one line, no internal references."""
    if not isinstance(text, str) or not text.strip():
        raise GenerationError(f"{rid}: expected text, got {text!r}")
    text = " ".join(LIVE_CHECK_REF.sub("", text).split())
    if INTERNAL_WORDS.search(text):
        raise GenerationError(f"{rid}: text shown to people has an internal reference: {text!r}")
    return text


def translate_options(rule: dict, endpoints: list) -> list:
    """The options of a report_choice rule, as edits the engine can show."""
    rid = rule["id"]
    options = rule.get("options")
    if not isinstance(options, list) or len(options) < 2:
        raise GenerationError(f"{rid}: report_choice needs at least two options")
    out = []
    for option in options:
        unknown = set(option) - OPTION_KEYS
        missing = OPTION_KEYS - set(option)
        if unknown or missing:
            raise GenerationError(
                f"{rid}: option {option.get('id')!r} has {sorted(unknown)} / lacks {sorted(missing)}"
            )
        oid = option["id"]
        label = OPTION_LABELS.get(f"{rule.get('choice')}/{oid}") or OPTION_LABELS.get(oid)
        if label is None:
            raise GenerationError(f"{rid}: option {oid!r} has no label in OPTION_LABELS")
        if option["status"] not in OPTION_STATUSES:
            raise GenerationError(f"{rid}: option {oid} has status {option['status']!r}")
        add, remove, keep = [], [], []
        for edit in option["edits"] or []:
            if not isinstance(edit, dict) or len(edit) != 1:
                raise GenerationError(f"{rid}: option {oid}: an edit is one kind, not {edit!r}")
            (kind, value), = edit.items()
            if kind not in EDIT_KINDS:
                raise GenerationError(f"{rid}: option {oid}: unknown edit {kind!r}")
            if kind == "add_param":
                add += add_specs(value, endpoints)
            elif kind == "remove_param":
                remove += keyword_names(rid, f"option {oid} remove_param", value)
            else:
                keep += keyword_names(rid, f"option {oid} keep", value)
        out.append({
            "id": oid,
            "label": label,
            "add": add,
            "remove": remove,
            "keep": keep,
            "effect": public_text(rid, option["effect"]),
            "status": OPTION_STATUSES[option["status"]],
        })
    return out


def check_rule_keys(rule: dict, specs: list) -> None:
    rid = rule["id"]
    unknown = set(rule) - RULE_INFO_KEYS - RULE_HANDLED_KEYS - RULE_OVERRIDE_KEYS
    if unknown:
        raise GenerationError(f"{rid}: rule keys {sorted(unknown)} are not translated; map them in the generator")
    override = set(rule) & RULE_OVERRIDE_KEYS
    if override and not any(item.get("why") or item.get("else_message") for item in specs):
        raise GenerationError(f"{rid}: {sorted(override)} change the rule; say how in RULES (why or else_message)")
    action = rule.get("action")
    if action not in KNOWN_ACTIONS:
        raise GenerationError(f"{rid}: unknown action {action!r}")
    for key in ("if_param_present_and_not_none", "otherwise"):
        if key in rule and rule[key] != "report_only":
            raise GenerationError(f"{rid}: {key}: {rule[key]!r} is not translated")
    keep_when = rule.get("keep_when") or {}
    if set(keep_when) - {"target", "effective_reasoning_effort"}:
        raise GenerationError(f"{rid}: keep_when {sorted(keep_when)} is not translated")
    # The reasoning effort is a per-repository choice: no automatic action may set it
    # (the recipe validator's rule C1; repeated here so the data cannot drift).
    if action in AUTOMATIC_ACTIONS:
        named = json.dumps([rule.get("param"), rule.get("requires")], default=str)
        if any(f'"{name}"' in named for name in PER_REPO_PARAMS):
            raise GenerationError(
                f"{rid}: automatic action {action} names a per-repo parameter "
                f"({', '.join(PER_REPO_PARAMS)}); it must be a report_choice"
            )
    if action == "report_choice":
        if not rule.get("choice"):
            raise GenerationError(f"{rid}: report_choice without a choice")
        if rule["choice"] == rid:
            resolved = rule.get("resolved") or {}
            if resolved.get("decision") != "per_repo_choice" or rule.get("default") != "report_only":
                raise GenerationError(f"{rid}: a choice anchor must be resolved per repo with default report_only")
    elif any(k in rule for k in ("choice", "options", "resolved", "default")):
        raise GenerationError(f"{rid}: choice fields on a {action} rule")
    if "see_choice" in rule:
        if not any(item.get("choice_note") for item in specs):
            raise GenerationError(f"{rid}: see_choice needs a choice_note in RULES")
    elif any(item.get("choice_note") for item in specs):
        raise GenerationError(f"{rid}: RULES has a choice_note but the recipe names no choice")


def translate_rule(rule: dict, rule_set: str) -> tuple[list, list]:
    rid = rule["id"]
    spec = RULES.get(rid)
    if spec is None:
        raise GenerationError(
            f"recipe rule {rid} ({rule_set}) has no entry in RULES: translate it or skip it with a reason"
        )
    if isinstance(spec, dict) and "skip" in spec:
        return [], [{"id": rid, "rule_set": rule_set, "reason": spec["skip"]}]

    when = rule.get("when") or {}
    unknown = set(when) - WHEN_KEYS
    specs = spec if isinstance(spec, list) else [spec]
    check_rule_keys(rule, specs)
    action = rule.get("action")
    verified, verified_on = verification(rule)
    out = []
    for item in specs:
        overrides = {k for k in item if k not in ("message", "else_message", "suffix", "why", "choice_note")}
        if unknown and not item.get("why"):
            raise GenerationError(f"{rid}: condition {sorted(unknown)} is not visible from keyword names; give a translation with a reason")
        if overrides and not item.get("why"):
            raise GenerationError(f"{rid}: overrides {sorted(overrides)} need a reason")
        live_dates_cited(rule, item["message"])
        effect = item.get("effect") or ACTION_EFFECTS.get(action)
        if effect is None:
            raise GenerationError(f"{rid}: action {action!r} needs an explicit effect")
        if action == "report_choice" and effect != "choice":
            raise GenerationError(
                f"{rid}: a report_choice rule is a per-repo choice; RULES cannot make it {effect!r}"
            )
        requires = rule.get("requires")
        if requires is not None and (
            effect != "rename" or not isinstance(requires, dict) or set(requires) - {"add_param"}
        ):
            raise GenerationError(f"{rid}: requires {requires!r} is translated only as add_param on a rename")
        endpoints = [e for e in as_list(when.get("endpoint"))]
        item_verified = item.get("verified", verified)
        live = item_verified == "live"
        engine = {
            "id": rid + ("/" + item["suffix"] if item.get("suffix") else ""),
            "rule_set": rule_set,
            "effect": effect,
            "endpoints": endpoints,
            "routings": ["google_generativeai_legacy"] if when.get("client") == "legacy_sdk" else [],
            "source_families": as_list(when.get("source_family")),
            "targets": as_list(when.get("target")),
            "source_models": as_list(when.get("source_ids")),
            "any_kwargs": as_list(when.get("request_has_any")),
            "all_kwargs": [],
            "lacks_kwargs": as_list(when.get("request_lacks")),
            "add": [],
            "rename": None,
            "keep_targets": [],
            "keep_needs_effort_none": False,
            "choice": None,
            "options": [],
            "see_choice": None,
            "choice_note": None,
            "message": item["message"],
            "else_message": item.get("else_message"),
            "verified": item_verified,
            "verified_on": verified_on if live else None,
            # A live result vouches for the models it called, on the API it called.
            "live_models": live_models(rule) if live else [],
            "live_endpoints": list(LIVE_ENDPOINTS) if live else [],
            "otherwise_verified": otherwise_verified(rule, item) if live else None,
            "source_url": rule.get("source_url"),
        }
        if effect == "add" and "add" not in item:
            engine["add"] = add_specs(rule["param"], endpoints)
        if effect == "rename":
            param = rule.get("param")
            if (
                not isinstance(param, dict)
                or set(param) != {"from", "to"}
                or not all(is_kwarg_name(param[k]) for k in ("from", "to"))
            ):
                raise GenerationError(f"{rid}: a rename needs param {{from, to}} with keyword names, not {param!r}")
            engine["rename"] = {"from": param["from"], "to": param["to"]}
            required = (rule.get("requires") or {}).get("add_param") or {}
            engine["add"] = add_specs(required, endpoints) if required else []
        if effect == "keep":
            keep_when = rule.get("keep_when") or {}
            engine["keep_targets"] = as_list(keep_when.get("target"))
            engine["keep_needs_effort_none"] = keep_when.get("effective_reasoning_effort") == "none"
        if effect == "choice":
            engine["choice"] = rule["choice"]
            engine["options"] = translate_options(rule, endpoints)
        if rule.get("see_choice"):
            engine["see_choice"] = rule["see_choice"]
            engine["choice_note"] = item.get("choice_note")
        if "otherwise_verified" in item and not live:
            raise GenerationError(f"{rid}: otherwise_verified in RULES on a rule without a live result")
        for key in ("endpoints", "targets", "any_kwargs", "all_kwargs", "lacks_kwargs", "add"):
            if key in item:
                engine[key] = copy.deepcopy(item[key])
        # Keyword names only: drop recipe entries that are not argument names
        # (`anthropic-beta header`, `text_editor_*`).
        for key in ("any_kwargs", "all_kwargs", "lacks_kwargs"):
            bad = [k for k in engine[key] if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", k)]
            if bad:
                raise GenerationError(f"{rid}: {key} {bad} are not keyword names; override them")
        if effect in ("add", "rename") and not engine["add"] and not engine["rename"]:
            raise GenerationError(f"{rid}: {effect} rule without a parameter")
        out.append(engine)
    return out, []


def check_choices(rules: list) -> None:
    """Every report_choice rule names an anchor in its own recipe and offers the
    anchor's options in the same order; every see_choice names an anchor."""
    anchors = {
        (r["rule_set"], r["id"]): r
        for r in rules
        if r.get("choice") and r["choice"] == r["id"]
    }
    for rule in rules:
        for key in ("choice", "see_choice"):
            name = rule.get(key)
            if not name:
                continue
            anchor = anchors.get((rule["rule_set"], name))
            if anchor is None:
                raise GenerationError(f"{rule['id']}: {key} {name} is not a choice in {rule['rule_set']}")
            if key == "choice":
                ids = [o["id"] for o in rule["options"]]
                if ids != [o["id"] for o in anchor["options"]]:
                    raise GenerationError(f"{rule['id']}: options {ids} differ from {name}'s")


# ---------------------------------------------------------------------------
# Assembly
# ---------------------------------------------------------------------------


def recipe_info(recipe: dict, file_name: str, sha: str) -> dict:
    semantics = recipe.get("shutdown_semantics")
    if semantics not in DATE_KINDS:
        raise GenerationError(f"{recipe['id']}: unknown shutdown_semantics {semantics!r}")
    urls = []
    for source in recipe.get("source_urls") or []:
        url = source.get("url")
        if url and url not in urls:
            urls.append(url)
    retrieved = iso(recipe.get("retrieved_on"))
    if not urls or not retrieved:
        raise GenerationError(f"{recipe['id']}: no source_urls or retrieved_on")
    return {
        "id": recipe["id"],
        "version": str(recipe["version"]),
        "file": file_name,
        "sha256": sha,
        "table": TABLES[recipe["id"]],
        "provider": PROVIDER_NAMES[TABLES[recipe["id"]]],
        "date_kind": DATE_KINDS[semantics],
        "retrieved_on": retrieved,
        "source_urls": urls,
    }


def build(recipes_dir: Path, source: str, generated_at: str) -> dict:
    loaded = load_recipes(recipes_dir)
    recipes, entries, fine_tunes, rules, skipped = [], [], [], [], []
    handled_rules = set()
    for recipe, file_name, sha in loaded:
        rid = recipe["id"]
        recipes.append(recipe_info(recipe, file_name, sha))
        if rid in ("openai-2026-10-23", "openai-2027-01-14"):
            e, f = openai_dated(recipe)
            entries += e
            fine_tunes += f
            entries += openai_upcoming(recipe)
        elif rid == "openai-past-shutdowns":
            entries += openai_past(recipe)
        elif rid == "anthropic-retirements":
            entries += anthropic(recipe)
        elif rid == "gemini-api-model-retirements":
            entries += gemini_api(recipe)
        elif rid == "vertex-gemini-model-retirements":
            entries += vertex(recipe)
        # Past shutdowns carry only "report everything" rules: nothing to check per call.
        if rid == "openai-past-shutdowns":
            for rule in recipe.get("parameter_rules") or []:
                skipped.append({"id": rule["id"], "rule_set": rid, "reason": "the recipe reports these IDs only; no per-call rule"})
                handled_rules.add(rule["id"])
            continue
        for rule in recipe.get("parameter_rules") or []:
            if rule["id"] in handled_rules:
                raise GenerationError(f"rule id {rule['id']} appears twice")
            handled_rules.add(rule["id"])
            r, s = translate_rule(rule, rid)
            rules += r
            skipped += s

    stale = sorted(set(RULES) - handled_rules)
    if stale:
        raise GenerationError(f"RULES has translations for rules no recipe defines: {stale}")

    # One entry per model per table (recipe rule R3 says so too).
    seen = {}
    for entry in entries:
        key = (entry["table"], entry["model"])
        if key in seen:
            raise GenerationError(f"{entry['model']} is listed twice for {entry['table']} ({seen[key]}, {entry['recipe']})")
        seen[key] = entry["recipe"]

    # Google limits gemini-2.5 models to keys that used them before (deprecations page).
    for entry in entries:
        if entry["table"] == "gemini_api" and (entry["replacement"] or "").startswith("gemini-2.5"):
            entry["notes"].append("Google limits gemini-2.5 models to API keys that used them before.")

    check_choices(rules)
    validate(entries, fine_tunes, rules)
    data = {
        "schema": SCHEMA,
        # The oldest date any provider page was read: the data is at least this
        # fresh. Each recipe keeps its own `retrieved_on`.
        "data_as_of": min(r["retrieved_on"] for r in recipes),
        "generated_at": generated_at,
        "generator": GENERATOR,
        "source": source,
        "digest": "",
        "recipes": recipes,
        "entries": [compact(e) for e in entries],
        "fine_tunes": [compact(f) for f in fine_tunes],
        "rules": [compact_rule(r) for r in rules],
        "skipped_rules": skipped,
    }
    data["digest"] = digest(data)
    return data


def compact(item: dict) -> dict:
    """Drops null, empty and false fields: the engine reads them as their defaults."""
    return {k: v for k, v in item.items() if v is not None and v != [] and v is not False}


def compact_rule(rule: dict) -> dict:
    rule = compact(rule)
    if "options" in rule:
        rule["options"] = [compact(o) for o in rule["options"]]
    return rule


def validate(entries, fine_tunes, rules) -> None:
    statuses = {"shut_down", "retiring", "deprecated", "unconfirmed", "auto_routed"}
    for entry in entries:
        if entry["status"] not in statuses:
            raise GenerationError(f"{entry['model']}: bad status {entry['status']}")
        if entry["status"] in ("retiring",) and not entry["date"]:
            raise GenerationError(f"{entry['model']}: retiring without a date")
        if entry["replacement"] and entry["replacement_text"] and entry["replacement"] not in entry["replacement_text"]:
            raise GenerationError(f"{entry['model']}: replacement text does not name the replacement")
    rule_sets = {e["rule_set"] for e in entries if e["rule_set"]}
    for rule in rules:
        if rule["rule_set"] not in rule_sets:
            raise GenerationError(f"{rule['id']}: rule set {rule['rule_set']} has no entries")
        if rule["effect"] in AUTOMATIC_EFFECTS:
            names = [a["name"] for a in rule["add"]] + ([rule["rename"]["to"]] if rule["rename"] else [])
            effort = any(
                a["name"] == "reasoning" and isinstance(a["value"], dict) and "effort" in a["value"]
                for a in rule["add"]
            )
            if effort or set(names) & set(PER_REPO_PARAMS):
                raise GenerationError(
                    f"{rule['id']}: an automatic {rule['effect']} sets a per-repo parameter; only a choice may"
                )
    for fine in fine_tunes:
        if not fine["date"]:
            raise GenerationError(f"fine-tune {fine['base']} without a date")
    check_plain(entries)
    check_plain(fine_tunes)
    check_plain(rules)


def check_plain(value) -> None:
    """Only strings, ints, bools, null, lists and objects; no control characters."""
    if isinstance(value, bool) or value is None or isinstance(value, int):
        return
    if isinstance(value, float):
        raise GenerationError("floats are not allowed in the data file")
    if isinstance(value, str):
        if any(ord(c) < 0x20 or ord(c) == 0x7F for c in value):
            raise GenerationError(f"control character in {value!r}")
        return
    if isinstance(value, list):
        for item in value:
            check_plain(item)
        return
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str):
                raise GenerationError(f"non-string key {key!r}")
            check_plain(item)
        return
    raise GenerationError(f"unsupported value {value!r}")


# Keys the digest leaves out: the digest itself, and the day the file was written
# (`--check` keeps it from the existing file).
UNDIGESTED = ("digest", "generated_at")


def digest(data: dict) -> str:
    """FNV-1a 64 over the canonical JSON (sorted keys, no spaces) of every key except
    UNDIGESTED: entries, rules, provenance, recipe hashes, dates and skipped rules.

    The Rust test computes the same value, so editing the JSON by hand fails CI.
    """
    body = {key: value for key, value in data.items() if key not in UNDIGESTED}
    canonical = json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    h = 0xCBF29CE484222325
    for byte in canonical.encode("utf-8"):
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"fnv1a64:{h:016x}"


def render(data: dict) -> str:
    """One top-level key per line and one line per list item: a diff shows one model per line."""
    lines = ["{"]
    keys = list(data)
    for index, key in enumerate(keys):
        comma = "," if index < len(keys) - 1 else ""
        value = data[key]
        if isinstance(value, list) and value:
            lines.append(f"  {json.dumps(key)}: [")
            for item_index, item in enumerate(value):
                item_comma = "," if item_index < len(value) - 1 else ""
                lines.append("    " + json.dumps(item, ensure_ascii=False, separators=(", ", ": ")) + item_comma)
            lines.append(f"  ]{comma}")
        else:
            lines.append(f"  {json.dumps(key)}: {json.dumps(value, ensure_ascii=False)}{comma}")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--recipes", required=True, type=Path, help="directory holding the recipe YAML files")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT, help="data file to write or check")
    parser.add_argument("--source", help="where the recipes came from (repository@commit:path); kept from the existing file when omitted")
    parser.add_argument("--generated-at", help="YYYY-MM-DD; defaults to today (UTC)")
    parser.add_argument("--check", action="store_true", help="exit 1 if the data file differs from a fresh build")
    args = parser.parse_args(argv)

    existing = None
    if args.out.is_file():
        existing = json.loads(args.out.read_text(encoding="utf-8"))
    source = args.source or (existing or {}).get("source") or "unknown"
    generated_at = args.generated_at or dt.datetime.now(dt.timezone.utc).date().isoformat()

    try:
        if args.check:
            if existing is None:
                print(f"{args.out} does not exist", file=sys.stderr)
                return 1
            # generated_at and source describe the last run, not the content.
            fresh = build(args.recipes, existing.get("source", source), existing.get("generated_at", generated_at))
            if render(fresh) == args.out.read_text(encoding="utf-8"):
                print(f"OK: {args.out.name} matches the recipes ({len(fresh['entries'])} models, {len(fresh['rules'])} rules)")
                return 0
            old = {r["id"]: r for r in existing.get("recipes", [])}
            for recipe in fresh["recipes"]:
                before = old.get(recipe["id"])
                if before is None:
                    print(f"new recipe: {recipe['id']} {recipe['version']}", file=sys.stderr)
                elif before.get("sha256") != recipe["sha256"] or before.get("version") != recipe["version"]:
                    print(f"changed recipe: {recipe['id']} {before.get('version')} -> {recipe['version']}", file=sys.stderr)
            print(f"DRIFT: {args.out} is out of date; regenerate it with {GENERATOR}", file=sys.stderr)
            return 1
        data = build(args.recipes, source, generated_at)
    except GenerationError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(render(data), encoding="utf-8", newline="\n")
    print(f"wrote {args.out} ({len(data['entries'])} models, {len(data['fine_tunes'])} fine-tune bases, {len(data['rules'])} rules, {len(data['skipped_rules'])} rules skipped)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
