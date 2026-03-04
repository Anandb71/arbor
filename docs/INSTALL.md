# Arbor Installation Guide

Install Arbor without building from source.

## Fastest Install (Recommended)

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
curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash -s -- --version v1.5.0
```

### Windows (PowerShell)

```powershell
iwr https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 -OutFile install.ps1
.\install.ps1 -Version v1.5.0
```

> For advanced options (`--install-dir`, `--force`, `--dry-run`), download and run the script locally.

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

## Manual Release Assets

Download prebuilt binaries directly from GitHub Releases:

- `arbor-windows-x64.exe`
- `arbor-linux-x64`
- `arbor-linux-arm64`
- `arbor-macos-x64`
- `arbor-macos-arm64`

Release page:

`https://github.com/Anandb71/arbor/releases`
