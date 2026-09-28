# Receipts

After every turn of your coding agent, Arbor tells you in plain English what the agent changed, what it touched that you didn't ask for, and what to try by hand before you ship.

```text
Arbor receipt · "make the pricing button green"
Changed 2 files · 2 functions

⚠ Not in your request (1):
  lib/auth/session.ts · Sign in · refreshSession

Could affect: Sign in · Payments · /checkout page · /pricing page · /api/checkout API · 4 other functions use this code

Test before you ship:
  1. Sign out, then sign back in
  2. Open /checkout and check it works
  3. Open /pricing and check it works
  4. Run a test checkout
Saved as 20260929T101203-f0c322c1 · `arbor receipt show`
```

Everything is computed locally from git and the Arbor graph. No model is called and nothing leaves your machine.

## Set it up

```bash
arbor setup            # index the project once
arbor hook claude      # wire Arbor into Claude Code
```

`arbor hook claude` adds two hooks to `.claude/settings.json`:

| Hook | Runs | Does |
|------|------|------|
| `UserPromptSubmit` | `arbor receipt begin` | Snapshots the working tree when you send a request |
| `Stop` | `arbor receipt end --hook` | Explains the difference when Claude finishes, shown to you in Claude Code |

Running it again is safe; existing hooks are left alone.

## What a receipt contains

- **Changed:** the files the turn changed and the functions whose lines changed. Changes you made before the request are not included, even if your tree was already dirty.
- **Not in your request:** changed files that share no words with your request and aren't one call away from a file that does. When the change touches sign in, payments, your database or app settings, the heading is a warning. Package manifests, lockfiles and tests are never flagged.
- **Could affect:** what the change reaches, named the way you'd test it: pages and API routes (Next.js app and pages routers, SvelteKit, Remix, Nuxt), areas such as sign in, payments, the database, emails, file uploads and admin, and how many other functions use the changed code.
- **Test before you ship:** up to four things to try, starting with anything touched outside your request.
- **Notes:** what the receipt couldn't check, stated plainly. For example, a request too vague to judge ("fix the bug") flags nothing rather than guessing.

## Commands

| Command | Description |
|---------|-------------|
| `arbor receipt list` | Recent receipts, newest first (`--limit N`, `--json`) |
| `arbor receipt show [id]` | One receipt in full, the latest by default (`--json`) |
| `arbor receipt begin` / `end` | Used by the hooks; `end` without `--hook` prints to the terminal |

Receipts are saved as JSON in `.arbor/receipts/`.

## Limits

- Relationships are static. "Could affect" means the code can reach it, not that it will break.
- Scope matching is by words. A request that names a page or feature works well; "make it better" can't be judged, and the receipt says so.
- Routes come from file-based routers. Frameworks that declare routes in code (Express, FastAPI, Rails) show their entry points by function name instead.
- The hooks need `git` and an indexed project. Outside a git repository they do nothing.
