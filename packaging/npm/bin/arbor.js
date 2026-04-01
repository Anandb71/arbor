#!/usr/bin/env node
"use strict";

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const binDir = __dirname;
const unixBin = path.join(binDir, "arbor");
const winBin = path.join(binDir, "arbor.exe");

const target = process.platform === "win32" ? winBin : unixBin;

if (!fs.existsSync(target)) {
  console.error("Arbor binary is not installed yet.");
  console.error("Run: npm rebuild @arbor-graph/cli or reinstall the package.");
  process.exit(1);
}

const result = spawnSync(target, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 0);
