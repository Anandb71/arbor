#!/usr/bin/env bash
# Run a built release binary through a new user's first minutes: version,
# setup, a caller lookup and a change preview on a two-file TypeScript repo.
#
# usage: smoke-release-binary.sh <path to arbor or arbor.exe> <expected version>
#
# Needs bash and git, nothing else, so it runs on every release runner and in
# minimal distribution containers.
set -euo pipefail

arbor="$1"
version="$2"

fail() {
  echo "::error title=release smoke test::$*"
  exit 1
}

actual=$("$arbor" --version)
[ "$actual" = "arbor ${version}" ] || fail "--version printed '${actual}', expected 'arbor ${version}'"

project=$(mktemp -d)
trap 'rm -rf "$project"' EXIT
cd "$project"

mkdir src
printf 'export function decide(x: number): boolean {\n  return x > 1;\n}\n' > src/policy.ts
printf "import { decide } from './policy';\n\nexport function run(): boolean {\n  return decide(3);\n}\n" > src/main.ts
git init -q
git add -A
git -c user.name=smoke -c user.email=smoke@example.invalid commit -qm init

"$arbor" setup . > setup.log 2>&1 || { cat setup.log; fail "arbor setup failed"; }

callers=$("$arbor" callers decide .)
echo "$callers"
grep -q "function run (" <<< "$callers" || fail "callers did not find run() calling decide()"
grep -q "main.ts:3" <<< "$callers" || fail "callers did not point at main.ts:3"

printf 'export function decide(x: number): boolean {\n  return x > 2;\n}\n' > src/policy.ts
preview=$("$arbor" diff . --json 2>/dev/null)
echo "$preview"
grep -q '"src/policy.ts"' <<< "$preview" || fail "diff did not report the changed file"
grep -q '"direct_callers": 1' <<< "$preview" || fail "diff did not count run() as a direct caller"

echo "arbor ${version} passed the release smoke test at ${arbor}."
