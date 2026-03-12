#!/usr/bin/env bash
set -euo pipefail

DEEP="${1:-}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

targets=(
  "crates/target"
  "visualizer/build"
  "visualizer/.dart_tool"
)

if [[ "$DEEP" == "--deep" ]]; then
  targets+=(
    ".arbor"
    "visualizer/.flutter-plugins"
    "visualizer/.flutter-plugins-dependencies"
  )
fi

dir_size_bytes() {
  local path="$1"
  if [[ ! -e "$path" ]]; then
    echo 0
    return
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY' "$path"
import os, sys
p = sys.argv[1]
total = 0
for root, _, files in os.walk(p):
    for f in files:
        fp = os.path.join(root, f)
        try:
            total += os.path.getsize(fp)
        except OSError:
            pass
print(total)
PY
  else
    # Fallback if python3 is unavailable
    du -sb "$path" 2>/dev/null | awk '{print $1}'
  fi
}

echo "Arbor workspace cleanup started in: $REPO_ROOT"
freed_bytes=0

for target in "${targets[@]}"; do
  if [[ -e "$target" ]]; then
    size_before="$(dir_size_bytes "$target")"
    rm -rf "$target"
    freed_bytes=$((freed_bytes + size_before))
    size_mb=$(awk "BEGIN { printf \"%.2f\", $size_before / 1024 / 1024 }")
    echo "Removed $target (${size_mb} MB)"
  else
    echo "Skipped $target (not found)"
  fi
done

size_gb=$(awk "BEGIN { printf \"%.2f\", $freed_bytes / 1024 / 1024 / 1024 }")
echo "Cleanup complete. Freed approximately ${size_gb} GB."
echo "Tip: run 'cargo test --workspace' afterwards to rebuild only what you need."
