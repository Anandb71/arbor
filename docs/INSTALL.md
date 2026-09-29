# Arbor Installation Guide

Install Arbor without building from source.

## Fastest Install (Recommended)

For local evaluation, one-line install is fine. For production/CI, use version-pinned install and review scripts before execution.

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 | iex
```

## Install Specific Version

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash -s -- --version <tag>
```

### Windows (PowerShell)

```powershell
iwr https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 -OutFile install.ps1
.\install.ps1 -Version <tag>
```

> For advanced options (`--install-dir`, `--force`, `--dry-run`), download and run the script locally.

### Safer Script Execution Pattern

Instead of piping directly to shell, download and inspect first:

```bash
curl -fsSLo install.sh https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh
less install.sh
bash install.sh --version <tag>
```

```powershell
iwr https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 -OutFile install.ps1
Get-Content .\install.ps1
.\install.ps1 -Version <tag>
```

## Verify

```bash
arbor --version
arbor doctor
```

## Cargo Install (Alternative)

If you already use Rust tooling:

```bash
cargo install arbor-graph-cli
```

crates.io can lag the GitHub releases. Check with `cargo search arbor-graph-cli`; if it is behind the [latest release](https://github.com/Anandb71/arbor/releases/latest), build the release tag from source instead:

```bash
cargo install --git https://github.com/Anandb71/arbor --tag v3.0.3 arbor-graph-cli
```

Run `arbor --version` afterwards. An older `arbor` earlier on your `PATH` (for example in `~/.cargo/bin`) wins over a newer one elsewhere. The crates.io crate named plain `arbor` is an unrelated project.

## Scoop (Windows)

The manifest lives in this repository rather than in a bucket, so install it by URL:

```powershell
scoop install https://raw.githubusercontent.com/Anandb71/arbor/main/packaging/scoop/arbor.json
```

## npm wrapper

Downloads the matching release binary on install:

```bash
npx @anandb71/arbor-cli
```

## Homebrew

A formula is kept at [`packaging/homebrew/arbor.rb`](../packaging/homebrew/arbor.rb), but no tap is published yet, so `brew install` cannot find it. On macOS and Linux use the install script above.

## GitHub Packages (GHCR Container)

Arbor container images are published to GitHub Container Registry (GHCR) after the release workflow finishes.

Pull image:

```bash
docker pull ghcr.io/anandb71/arbor:latest
```

Or pull a specific release tag:

```bash
docker pull ghcr.io/anandb71/arbor:<tag>
```

Run MCP bridge over stdio:

```bash
docker run --rm -i ghcr.io/anandb71/arbor:latest
```

## Manual Release Assets

Download prebuilt binaries directly from GitHub Releases:

- `arbor-windows-x86_64.zip`
- `arbor-linux-x86_64.tar.gz`
- `arbor-linux-aarch64.tar.gz`
- `arbor-macos-x86_64.tar.gz`
- `arbor-macos-aarch64.tar.gz`

Release page:

`https://github.com/Anandb71/arbor/releases`

---

For maintainers shipping new versions across registries (GitHub Releases, crates.io, GHCR, npm, VS Code Marketplace, Open VSX, Scoop), follow:

- [Release Runbook](./RELEASING.md)
