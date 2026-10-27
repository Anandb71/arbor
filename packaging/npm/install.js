#!/usr/bin/env node
// Arbor CLI npm postinstall: downloads the platform binary ahead of the first
// run. Optional since #281: npm 12 skips install scripts unless they are
// allowed, and bin/arbor.js then downloads the same, checksum-verified binary
// on first run. So a failure here is a warning, not a failed install.
"use strict";

const { execSync } = require("child_process");
const { VERSION, ensureBinary } = require("./lib/download");

async function main() {
  try {
    const { path: installed, downloaded } = await ensureBinary({ log: (line) => console.log(line) });
    console.log(downloaded ? "Arbor installed successfully!" : `Arbor v${VERSION} is already installed: ${installed}`);
  } catch (err) {
    console.warn(`arbor: ${err.message}`);
    console.warn("");
    console.warn("The npm install itself succeeded; `arbor` will try the download again on its first run.");
    return;
  }
  warnIfShadowed();
}

// A shell runs the first `arbor` on PATH. An older install there (often a
// `cargo install` in ~/.cargo/bin) keeps answering after this one is
// installed, silently. Say so. Advisory only: never fail the install.
function warnIfShadowed() {
  try {
    const finder = process.platform === "win32" ? "where arbor" : "command -v arbor";
    const found = execSync(finder, { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .filter((line) => process.platform !== "win32" || /\.(exe|cmd|bat)$/i.test(line));
    if (found.length === 0) return;
    const first = found[0];
    const banner = execSync(`"${first}" --version`, {
      stdio: ["ignore", "pipe", "ignore"],
      timeout: 5000,
    }).toString();
    const version = (banner.match(/\d+\.\d+\.\d+/) || [])[0];
    if (!version || version === VERSION) return;
    console.warn("");
    console.warn(`Warning: \`arbor\` on your PATH is ${first} (v${version}), not this v${VERSION}.`);
    console.warn("Your shell will keep running the older one. Remove it (for a cargo install:");
    console.warn("cargo uninstall arbor-graph-cli) or put npm's global bin directory earlier on PATH.");
    console.warn("");
  } catch (_) {
    // Advisory only.
  }
}

main();
