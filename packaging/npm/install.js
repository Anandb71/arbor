#!/usr/bin/env node
// Arbor CLI npm postinstall — downloads the correct platform binary
"use strict";

const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const https = require("https");

const VERSION = require("./package.json").version;
const REPO = "Anandb71/arbor";

const PLATFORM_MAP = {
  "darwin-x64": "arbor-macos-x86_64.tar.gz",
  "darwin-arm64": "arbor-macos-aarch64.tar.gz",
  "linux-x64": "arbor-linux-x86_64.tar.gz",
  "linux-arm64": "arbor-linux-aarch64.tar.gz",
  "win32-x64": "arbor-windows-x86_64.zip",
};

function getPlatformKey() {
  return `${process.platform}-${process.arch}`;
}

function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    const follow = (url) => {
      https.get(url, { headers: { "User-Agent": "arbor-npm" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return follow(res.headers.location);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`Download failed: HTTP ${res.statusCode}`));
        }
        const file = fs.createWriteStream(dest);
        res.pipe(file);
        file.on("finish", () => { file.close(); resolve(); });
      }).on("error", reject);
    };
    follow(url);
  });
}

async function main() {
  const key = getPlatformKey();
  const asset = PLATFORM_MAP[key];

  if (!asset) {
    console.error(`Unsupported platform: ${key}`);
    console.error("Install from source: cargo install arbor-graph-cli");
    process.exit(1);
  }

  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${asset}`;
  const binDir = path.join(__dirname, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const tmpFile = path.join(binDir, asset);

  console.log(`Downloading Arbor v${VERSION} for ${key}...`);

  try {
    await downloadFile(url, tmpFile);

    if (asset.endsWith(".tar.gz")) {
      execSync(`tar -xzf "${tmpFile}" -C "${binDir}"`, { stdio: "inherit" });
      fs.unlinkSync(tmpFile);
      fs.chmodSync(path.join(binDir, "arbor"), 0o755);
    } else if (asset.endsWith(".zip")) {
      // On Windows, use PowerShell to extract
      execSync(
        `powershell -Command "Expand-Archive -Path '${tmpFile}' -DestinationPath '${binDir}' -Force"`,
        { stdio: "inherit" }
      );
      fs.unlinkSync(tmpFile);
    }

    console.log("Arbor installed successfully!");
  } catch (err) {
    console.error("Failed to install Arbor binary:", err.message);
    console.error("Fallback: cargo install arbor-graph-cli");
    process.exit(1);
  }
}

main();
