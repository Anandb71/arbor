#!/usr/bin/env bash
set -euo pipefail

CLIENT="all"
TARGET_DIR="$PWD"
FORCE="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --client)
      CLIENT="$2"
      shift 2
      ;;
    --target-dir)
      TARGET_DIR="$2"
      shift 2
      ;;
    --force)
      FORCE="true"
      shift
      ;;
    -h|--help)
      cat <<EOF
Arbor MCP config bootstrap

Usage:
  ./scripts/setup-mcp.sh [--client <all|claude-code|cursor|vscode>] [--target-dir <path>] [--force]

Examples:
  ./scripts/setup-mcp.sh --client all
  ./scripts/setup-mcp.sh --client cursor --target-dir ~/work/my-repo
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TEMPLATE_DIR="${ROOT_DIR}/templates/mcp"

step() {
  echo "[arbor-mcp-setup] $*"
}

copy_template() {
  local src="$1"
  local dest="$2"

  mkdir -p "$(dirname "$dest")"

  if [[ -f "$dest" && "$FORCE" != "true" ]]; then
    step "Skipping existing file: $dest (use --force to overwrite)"
    return
  fi

  cp "$src" "$dest"
  step "Wrote: $dest"
}

apply_client() {
  local client="$1"
  case "$client" in
    claude-code)
      copy_template "${TEMPLATE_DIR}/claude-code.project.mcp.json" "${TARGET_DIR}/.mcp.json"
      ;;
    cursor)
      copy_template "${TEMPLATE_DIR}/cursor.project.mcp.json" "${TARGET_DIR}/.cursor/mcp.json"
      ;;
    vscode)
      copy_template "${TEMPLATE_DIR}/vscode.project.mcp.json" "${TARGET_DIR}/.vscode/mcp.json"
      ;;
    *)
      echo "Unsupported client: $client" >&2
      exit 1
      ;;
  esac
}

case "$CLIENT" in
  all)
    apply_client "claude-code"
    apply_client "cursor"
    apply_client "vscode"
    ;;
  claude-code|cursor|vscode)
    apply_client "$CLIENT"
    ;;
  *)
    echo "Invalid --client value: $CLIENT" >&2
    exit 1
    ;;
esac

step "Done."
step "Next: restart your client or reload MCP servers."
