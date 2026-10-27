#!/usr/bin/env node
"use strict";

// Runs the Arbor binary. npm 12 skips install scripts unless they are
// allowed, so postinstall may never have downloaded it: in that case it is
// downloaded, checked against the release's SHA256SUMS and installed here
// on the first run (#281). Progress and errors go to stderr only, so
// `--json` output on stdout stays clean.

const { spawnSync } = require("child_process");
const os = require("os");
const { ArborInstallError, ensureBinary, findBinary } = require("../lib/download");

async function main() {
  let target = findBinary();
  if (!target) {
    try {
      target = (await ensureBinary()).path;
    } catch (err) {
      const message = err instanceof ArborInstallError ? err.message : err.stack || String(err);
      process.stderr.write(`arbor: ${message}\n`);
      process.exit(2);
    }
  }

  const result = spawnSync(target, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) {
    process.stderr.write(`arbor: could not run ${target}: ${result.error.message}\n`);
    process.exit(2);
  }
  if (result.signal) {
    process.exit(128 + (os.constants.signals[result.signal] || 0));
  }
  process.exit(result.status ?? 1);
}

main();
