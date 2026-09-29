# Arbor Installation Guide

Install Arbor without building from source.

> Updated for March 2026 standards (reproducibility + safer install flows).

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

crates.io can lag the GitHub releases: it currently has `arbor-graph-cli` 2.6.0, while the latest release is 3.0.0. To build a release from source instead:

```bash
cargo install --git https://github.com/Anandb71/arbor --tag v3.0.0 arbor-graph-cli
```

Run `arbor --version` afterwards. An older `arbor` earlier on your `PATH` (for example in `~/.cargo/bin`) wins over a newer one elsewhere. The crates.io crate named plain `arbor` is an unrelated project.

## GitHub Packages (GHCR Container)

Arbor container images are published to GitHub Container Registry (GHCR) when a release is published.

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

For maintainers shipping new versions across registries (GitHub Releases, crates.io, GHCR, VS Code Marketplace, Open VSX), follow:

- [Release Runbook](./RELEASING.md)
