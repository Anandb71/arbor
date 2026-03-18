#!/usr/bin/env bash
set -euo pipefail

VERSION="latest"
INSTALL_DIR="${HOME}/.local/bin"
DRY_RUN="false"
FORCE="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="$2"
      shift 2
      ;;
    --install-dir)
      INSTALL_DIR="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN="true"
      shift
      ;;
    --force)
      FORCE="true"
      shift
      ;;
    -h|--help)
      cat <<EOF
Arbor installer

Usage:
  ./install.sh [--version <tag>] [--install-dir <path>] [--dry-run] [--force]

Examples:
  ./install.sh
  ./install.sh --version v1.6.1.1
  ./install.sh --install-dir ~/.arbor/bin
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

step() {
  echo "[arbor-install] $*"
}

os="$(uname -s)"
arch="$(uname -m)"

asset_name=""
case "$os" in
  Linux)
    case "$arch" in
      x86_64|amd64) asset_name="arbor-linux-x64" ;;
      aarch64|arm64) asset_name="arbor-linux-arm64" ;;
      *) echo "Unsupported Linux architecture: $arch" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$arch" in
      x86_64) asset_name="arbor-macos-x64" ;;
      arm64|aarch64) asset_name="arbor-macos-arm64" ;;
      *) echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $os" >&2
    exit 1
    ;;
esac

api_base="https://api.github.com/repos/Anandb71/arbor/releases"
if [[ "$VERSION" == "latest" ]]; then
  release_url="${api_base}/latest"
else
  release_url="${api_base}/tags/${VERSION}"
fi

step "Resolving release metadata (${VERSION})..."
if command -v curl >/dev/null 2>&1; then
  release_json="$(curl -fsSL -H "User-Agent: arbor-install-script" "$release_url")"
elif command -v wget >/dev/null 2>&1; then
  release_json="$(wget -qO- --header="User-Agent: arbor-install-script" "$release_url")"
else
  echo "Need curl or wget to download release metadata." >&2
  exit 1
fi

asset_url="$(printf '%s' "$release_json" | grep -oE '"browser_download_url"\s*:\s*"[^"]+"' | sed -E 's/.*"([^"]+)"/\1/' | grep "/${asset_name}$" | head -n1 || true)"

if [[ -z "$asset_url" ]]; then
  echo "Could not find asset '${asset_name}' in release '${VERSION}'." >&2
  exit 1
fi

target_dir="${INSTALL_DIR}"
target_bin="${target_dir}/arbor"

if [[ "$DRY_RUN" == "true" ]]; then
  step "Dry run enabled."
  echo "Would install: ${asset_url}"
  echo "Target path : ${target_bin}"
  exit 0
fi

if [[ -f "$target_bin" && "$FORCE" != "true" ]]; then
  step "Existing arbor binary found at ${target_bin}. Use --force to overwrite."
  exit 0
fi

mkdir -p "$target_dir"

tmp_file="$(mktemp)"
trap 'rm -f "$tmp_file"' EXIT

step "Downloading ${asset_name} ..."
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$asset_url" -o "$tmp_file"
else
  wget -qO "$tmp_file" "$asset_url"
fi

chmod +x "$tmp_file"
mv "$tmp_file" "$target_bin"

step "Installed to ${target_bin}"

case ":$PATH:" in
  *":${target_dir}:"*)
    step "Install directory already in PATH."
    ;;
  *)
    step "${target_dir} is not in PATH."
    echo "Add this to your shell profile (e.g. ~/.bashrc or ~/.zshrc):"
    echo "  export PATH=\"${target_dir}:\$PATH\""
    ;;
esac

echo "Run: arbor --version"
