"use strict";
// Finds the Arbor binary for this platform, or downloads it.
//
// Used by install.js (npm postinstall) and by bin/arbor.js on first run.
// npm 12 runs install scripts only for packages its `allow-scripts` setting
// names, so the postinstall download is an optimisation: when it was skipped,
// the first `arbor` run downloads the same release archive (#281).
//
// A download is the archive for this platform from the GitHub Release whose
// tag matches this package's version. It is checked against that release's
// SHA256SUMS before anything is extracted, and only the binary is taken out
// of it. Node's own modules only: no dependencies.
//
// Environment:
//   ARBOR_NPM_DOWNLOAD_BASE  folder holding the release archives and
//                            SHA256SUMS (a mirror). https, or http on
//                            localhost only. Default: the GitHub Release.
//   ARBOR_NPM_CACHE_DIR      where to put the binary when this package's own
//                            bin/ folder is not writable.
//   HTTPS_PROXY, NO_PROXY    used on Node.js 22.21+ and 24.5+ (also read
//                            from npm's https-proxy, proxy and noproxy).

const fs = require("fs");
const os = require("os");
const path = require("path");

const VERSION = require("../package.json").version;
const REPO = "Anandb71/arbor";
const INSTALL_DOCS = `https://github.com/${REPO}/blob/main/docs/INSTALL.md`;

const PLATFORM_MAP = {
  "darwin-x64": "arbor-macos-x86_64.tar.gz",
  "darwin-arm64": "arbor-macos-aarch64.tar.gz",
  "linux-x64": "arbor-linux-x86_64.tar.gz",
  "linux-arm64": "arbor-linux-aarch64.tar.gz",
  "win32-x64": "arbor-windows-x86_64.zip",
};

const PACKAGE_BIN_DIR = path.join(__dirname, "..", "bin");
const IDLE_TIMEOUT_MS = 30000;
const MAX_REDIRECTS = 5;
const MAX_DOWNLOAD_BYTES = 256 * 1024 * 1024;

class ArborInstallError extends Error {}

function binaryName(platform = process.platform) {
  return platform === "win32" ? "arbor.exe" : "arbor";
}

function assetFor(platform = process.platform, arch = process.arch) {
  return PLATFORM_MAP[`${platform}-${arch}`] || null;
}

// The binary may live in this package's bin/ folder (put there by
// postinstall or an earlier first run) or, when that folder was not
// writable, in a per-user cache folder for this version.
function userCacheDir(env = process.env, platform = process.platform) {
  if (env.ARBOR_NPM_CACHE_DIR) return path.join(env.ARBOR_NPM_CACHE_DIR, `v${VERSION}`);
  const home = os.homedir();
  let root;
  if (platform === "win32") root = env.LOCALAPPDATA || path.join(home, "AppData", "Local");
  else if (platform === "darwin") root = path.join(home, "Library", "Caches");
  else root = env.XDG_CACHE_HOME || path.join(home, ".cache");
  return path.join(root, "arbor", "npm", `v${VERSION}`);
}

function defaultDirs(env = process.env, platform = process.platform) {
  return [PACKAGE_BIN_DIR, userCacheDir(env, platform)];
}

function findBinary(dirs = defaultDirs(), platform = process.platform) {
  for (const dir of dirs) {
    const candidate = path.join(dir, binaryName(platform));
    try {
      if (fs.statSync(candidate).isFile()) return candidate;
    } catch (_) {
      // Not there; try the next folder.
    }
  }
  return null;
}

function isLoopback(hostname) {
  return hostname === "localhost" || hostname === "[::1]" || /^127\.\d+\.\d+\.\d+$/.test(hostname);
}

// Only https, or plain http to this machine (a test server or local mirror):
// a checksum fetched over plain http from elsewhere would prove nothing.
function checkUrl(url, what) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch (_) {
    throw new ArborInstallError(`${what} is not a valid URL: ${url}`);
  }
  if (parsed.protocol === "https:") return parsed;
  if (parsed.protocol === "http:" && isLoopback(parsed.hostname)) return parsed;
  throw new ArborInstallError(`${what} must use https (plain http is accepted only for localhost): ${url}`);
}

function releaseBase(env = process.env) {
  const override = (env.ARBOR_NPM_DOWNLOAD_BASE || "").trim();
  if (!override) return `https://github.com/${REPO}/releases/download/v${VERSION}`;
  const base = override.replace(/\/+$/, "");
  checkUrl(base, "ARBOR_NPM_DOWNLOAD_BASE");
  return base;
}

// Proxy settings for Node's built-in proxy support (`proxyEnv` on an Agent,
// Node.js 22.21+ and 24.5+; older versions ignore the option). npm exports
// its own https-proxy/proxy/noproxy settings to the commands it runs as
// npm_config_*, so a proxy configured only in .npmrc is used too.
function proxyEnvFrom(env = process.env) {
  const out = {};
  for (const key of ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "NO_PROXY", "no_proxy"]) {
    if (env[key]) out[key] = env[key];
  }
  if (!out.HTTPS_PROXY && !out.https_proxy) {
    const npmProxy = env.npm_config_https_proxy || env.npm_config_proxy;
    if (npmProxy) out.HTTPS_PROXY = npmProxy;
  }
  if (!out.HTTP_PROXY && !out.http_proxy && env.npm_config_proxy) out.HTTP_PROXY = env.npm_config_proxy;
  if (!out.NO_PROXY && !out.no_proxy && env.npm_config_noproxy) out.NO_PROXY = env.npm_config_noproxy;
  return out;
}

function nodeUsesProxyEnv(version = process.versions.node) {
  const [major, minor] = version.split(".").map(Number);
  return major >= 25 || (major === 24 && minor >= 5) || (major === 22 && minor >= 21);
}

class HttpStatusError extends Error {
  constructor(status, url) {
    super(`HTTP ${status} from ${url}`);
    this.status = status;
    this.url = url;
  }
}

function download(url, env, redirects = 0) {
  const http = require("http");
  const https = require("https");
  return new Promise((resolve, reject) => {
    let parsed;
    try {
      parsed = checkUrl(url, "Download URL");
    } catch (err) {
      reject(err);
      return;
    }
    const proxyEnv = proxyEnvFrom(env);
    const mod = parsed.protocol === "http:" ? http : https;
    const agent = new mod.Agent(Object.keys(proxyEnv).length ? { proxyEnv } : {});
    const req = mod.get(parsed, { agent, headers: { "User-Agent": `arbor-npm/${VERSION}` } }, (res) => {
      const status = res.statusCode;
      if (status >= 300 && status < 400 && res.headers.location) {
        res.resume();
        if (redirects >= MAX_REDIRECTS) {
          reject(new Error(`more than ${MAX_REDIRECTS} redirects from ${url}`));
          return;
        }
        resolve(download(new URL(res.headers.location, parsed).href, env, redirects + 1));
        return;
      }
      if (status !== 200) {
        res.resume();
        reject(new HttpStatusError(status, url));
        return;
      }
      const chunks = [];
      let size = 0;
      res.on("data", (chunk) => {
        size += chunk.length;
        if (size > MAX_DOWNLOAD_BYTES) {
          req.destroy(new Error(`${url} is larger than ${MAX_DOWNLOAD_BYTES} bytes`));
          return;
        }
        chunks.push(chunk);
      });
      res.on("end", () => {
        if (res.complete) resolve(Buffer.concat(chunks));
        else reject(Object.assign(new Error("the connection closed before the download finished"), { code: "ECONNRESET" }));
      });
      res.on("error", reject);
    });
    req.setTimeout(IDLE_TIMEOUT_MS, () => {
      req.destroy(Object.assign(new Error(`no data for ${IDLE_TIMEOUT_MS / 1000} seconds`), { code: "ETIMEDOUT" }));
    });
    req.on("error", reject);
  });
}

// Plain-English reasons for a failed download. Never prints a proxy URL,
// which can carry a password.
function explainDownloadError(err, env = process.env) {
  const host = err.url ? new URL(err.url).host : "the download host";
  const code = err.code || "";
  const lines = [];
  if (err instanceof HttpStatusError) {
    if (err.status === 404) lines.push(`${err.url} was not found (HTTP 404). Is v${VERSION} released yet?`);
    else if (err.status === 407) lines.push("The proxy asked for credentials (HTTP 407): put them in HTTPS_PROXY (http://user:password@host:port).");
    else lines.push(`${err.url} answered HTTP ${err.status}.`);
  } else if (err instanceof ArborInstallError) {
    lines.push(err.message);
  } else if (code === "ENOTFOUND" || code === "EAI_AGAIN") {
    lines.push(`Could not look up ${err.hostname || host}: this machine looks offline, or DNS is blocked.`);
  } else if (/CERT|SELF_SIGNED|UNABLE_TO_VERIFY|UNABLE_TO_GET_ISSUER/.test(code)) {
    lines.push(`The TLS certificate ${host} presented is not trusted (${code}). A proxy that inspects HTTPS usually`);
    lines.push("causes this: set NODE_EXTRA_CA_CERTS to the path of its CA certificate (PEM) and run again.");
  } else {
    lines.push(`Could not download from ${host} (${code ? `${code}: ` : ""}${err.message}). Check the network connection.`);
  }
  const proxy = proxyEnvFrom(env);
  if (proxy.HTTPS_PROXY || proxy.https_proxy) {
    lines.push(
      nodeUsesProxyEnv()
        ? `A proxy is set in HTTPS_PROXY (or npm's https-proxy): check that it is reachable and lets ${host} through.`
        : `A proxy is set in HTTPS_PROXY (or npm's https-proxy), but Node.js ${process.version} does not use it here; Node.js 22.21+ and 24.5+ do.`
    );
  }
  return lines;
}

function verifyChecksum(archive, asset, sumsText) {
  const crypto = require("crypto");
  let expected = null;
  for (const line of sumsText.split(/\r?\n/)) {
    const match = /^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$/.exec(line);
    if (match && match[2] === asset) expected = match[1].toLowerCase();
  }
  if (!expected) {
    throw new ArborInstallError(`SHA256SUMS has no line for ${asset}, so the download cannot be checked. Nothing was installed.`);
  }
  const actual = crypto.createHash("sha256").update(archive).digest("hex");
  if (actual !== expected) {
    throw new ArborInstallError(
      `${asset} does not match SHA256SUMS (expected ${expected}, got ${actual}). The download was discarded; nothing was installed.`
    );
  }
  return actual;
}

function normalizeEntryName(name) {
  return name.replace(/\\/g, "/").replace(/^(\.\/)+/, "");
}

function corrupt(asset, what) {
  return new ArborInstallError(`${asset} looks damaged (${what}). Nothing was installed.`);
}

// Returns the bytes of the regular file `wanted` in a .tar.gz, or null.
function extractFromTarGz(archive, wanted, asset = "the archive") {
  const zlib = require("zlib");
  let tar;
  try {
    tar = zlib.gunzipSync(archive);
  } catch (err) {
    throw corrupt(asset, `gunzip: ${err.message}`);
  }
  const text = (buf, start, len) => {
    const field = buf.subarray(start, start + len);
    const nul = field.indexOf(0);
    return field.subarray(0, nul === -1 ? len : nul).toString("utf8");
  };
  let pending = {};
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    let name = text(header, 0, 100);
    const prefix = text(header, 345, 155);
    if (text(header, 257, 5) === "ustar" && prefix) name = `${prefix}/${name}`;
    let size = parseInt(text(header, 124, 12).trim() || "0", 8);
    if (!Number.isFinite(size) || size < 0) throw corrupt(asset, "bad tar header");
    const type = header[156] === 0 ? "0" : String.fromCharCode(header[156]);
    const start = offset + 512;
    if (start + size > tar.length) throw corrupt(asset, "truncated tar entry");
    const data = tar.subarray(start, start + size);
    offset = start + Math.ceil(size / 512) * 512;

    if (type === "x") {
      // pax extended header: "<len> key=value\n" records for the next entry.
      let i = 0;
      while (i < data.length) {
        const space = data.indexOf(0x20, i);
        const len = space === -1 ? 0 : parseInt(data.subarray(i, space).toString("utf8"), 10);
        if (!len) break;
        const record = data.subarray(space + 1, i + len - 1).toString("utf8");
        const eq = record.indexOf("=");
        if (eq > 0) pending[record.slice(0, eq)] = record.slice(eq + 1);
        i += len;
      }
      continue;
    }
    if (type === "L") {
      pending.path = text(data, 0, data.length);
      continue;
    }
    if (type === "g") continue;

    if (pending.path) name = pending.path;
    if (pending.size) {
      size = Number(pending.size);
      if (!Number.isFinite(size) || start + size > tar.length) throw corrupt(asset, "bad pax size");
    }
    pending = {};
    if ((type === "0" || type === "7") && normalizeEntryName(name) === wanted) {
      return Buffer.from(tar.subarray(start, start + size));
    }
  }
  return null;
}

// Returns the bytes of the file `wanted` in a .zip, or null.
function extractFromZip(zip, wanted, asset = "the archive") {
  const zlib = require("zlib");
  let eocd = -1;
  for (let i = zip.length - 22; i >= Math.max(0, zip.length - 22 - 0xffff); i--) {
    if (zip.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw corrupt(asset, "no zip directory");
  const count = zip.readUInt16LE(eocd + 10);
  let p = zip.readUInt32LE(eocd + 16);
  for (let n = 0; n < count; n++) {
    if (p + 46 > zip.length || zip.readUInt32LE(p) !== 0x02014b50) throw corrupt(asset, "bad zip directory");
    const flags = zip.readUInt16LE(p + 8);
    const method = zip.readUInt16LE(p + 10);
    const compressedSize = zip.readUInt32LE(p + 20);
    const size = zip.readUInt32LE(p + 24);
    const nameLen = zip.readUInt16LE(p + 28);
    const extraLen = zip.readUInt16LE(p + 30);
    const commentLen = zip.readUInt16LE(p + 32);
    const local = zip.readUInt32LE(p + 42);
    const name = normalizeEntryName(zip.subarray(p + 46, p + 46 + nameLen).toString("utf8"));
    p += 46 + nameLen + extraLen + commentLen;
    if (name !== wanted) continue;

    if (flags & 1) throw corrupt(asset, "encrypted zip entry");
    if (compressedSize === 0xffffffff || size === 0xffffffff || local === 0xffffffff) {
      throw corrupt(asset, "zip64 entries are not supported");
    }
    if (local + 30 > zip.length || zip.readUInt32LE(local) !== 0x04034b50) throw corrupt(asset, "bad zip entry");
    const dataStart = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const data = zip.subarray(dataStart, dataStart + compressedSize);
    if (data.length !== compressedSize) throw corrupt(asset, "truncated zip entry");
    let out;
    if (method === 0) out = Buffer.from(data);
    else if (method === 8) {
      try {
        out = zlib.inflateRawSync(data);
      } catch (err) {
        throw corrupt(asset, `inflate: ${err.message}`);
      }
    } else throw corrupt(asset, `zip compression method ${method}`);
    if (out.length !== size) throw corrupt(asset, "zip entry size");
    return out;
  }
  return null;
}

// Writes the binary under a temporary name, then renames it into place, so
// a concurrent run never sees half a file. If another run got there first
// (Windows will not replace an .exe that is running), its copy is used.
function installInto(dir, binary, platform = process.platform) {
  const crypto = require("crypto");
  fs.mkdirSync(dir, { recursive: true });
  const target = path.join(dir, binaryName(platform));
  const tmp = path.join(dir, `.${binaryName(platform)}.${process.pid}.${crypto.randomBytes(4).toString("hex")}.tmp`);
  try {
    fs.writeFileSync(tmp, binary, { mode: 0o755 });
    fs.chmodSync(tmp, 0o755);
    try {
      fs.renameSync(tmp, target);
    } catch (err) {
      if (findBinary([dir], platform)) return target;
      throw err;
    }
    return target;
  } finally {
    try {
      fs.rmSync(tmp, { force: true });
    } catch (_) {
      // Best effort.
    }
  }
}

function manualSteps(assetUrl, sumsUrl, asset, platform, dir) {
  return [
    "",
    "To install it by hand, download",
    `  ${assetUrl}`,
    `check its SHA-256 against the ${asset} line in`,
    `  ${sumsUrl}`,
    `and extract ${binaryName(platform)} into`,
    `  ${dir}`,
    `Other ways to install Arbor: ${INSTALL_DOCS}`,
  ];
}

// Returns { path, downloaded }. Throws ArborInstallError with a message
// that says what failed and what to do next.
async function ensureBinary(options = {}) {
  const env = options.env || process.env;
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  const dirs = options.dirs || defaultDirs(env, platform);
  const log = options.log || ((line) => process.stderr.write(`${line}\n`));

  const existing = findBinary(dirs, platform);
  if (existing) return { path: existing, downloaded: false };

  const key = `${platform}-${arch}`;
  const asset = assetFor(platform, arch);
  if (!asset) {
    const supported = Object.keys(PLATFORM_MAP).join(", ");
    throw new ArborInstallError(
      [
        `Arbor v${VERSION} has no prebuilt binary for ${key} (prebuilt: ${supported}).`,
        "Build it from source with Rust: cargo install arbor-graph-cli",
        `Other ways to install Arbor: ${INSTALL_DOCS}`,
      ].join("\n")
    );
  }

  const base = releaseBase(env);
  const assetUrl = `${base}/${asset}`;
  const sumsUrl = `${base}/SHA256SUMS`;
  const manual = () => manualSteps(assetUrl, sumsUrl, asset, platform, dirs[0]);
  const fail = (lines) =>
    new ArborInstallError([`could not install the Arbor v${VERSION} binary for ${key}.`, ...lines, ...manual()].join("\n"));

  const fetchOrFail = async (url) => {
    try {
      return await download(url, env);
    } catch (err) {
      // Name the URL that was asked for, not a signed redirect target.
      err.url = url;
      throw fail(explainDownloadError(err, env));
    }
  };

  log(`arbor: downloading Arbor v${VERSION} (${asset}) from ${new URL(base).host}, once...`);
  const sums = (await fetchOrFail(sumsUrl)).toString("utf8");
  const archive = await fetchOrFail(assetUrl);

  let binary;
  try {
    verifyChecksum(archive, asset, sums);
    binary = asset.endsWith(".zip")
      ? extractFromZip(archive, binaryName(platform), asset)
      : extractFromTarGz(archive, binaryName(platform), asset);
  } catch (err) {
    throw fail([err.message]);
  }
  if (!binary) throw fail([`${asset} has no ${binaryName(platform)} in it.`]);

  const problems = [];
  for (const dir of dirs) {
    try {
      const installed = installInto(dir, binary, platform);
      log(`arbor: checked ${asset} against SHA256SUMS and installed ${installed}`);
      return { path: installed, downloaded: true };
    } catch (err) {
      problems.push(`  ${dir}: ${err.code || err.message}`);
    }
  }
  throw fail(["Could not write the binary to any of these folders:", ...problems]);
}

module.exports = {
  ArborInstallError,
  HttpStatusError,
  PLATFORM_MAP,
  VERSION,
  assetFor,
  binaryName,
  defaultDirs,
  ensureBinary,
  explainDownloadError,
  extractFromTarGz,
  extractFromZip,
  findBinary,
  nodeUsesProxyEnv,
  proxyEnvFrom,
  releaseBase,
  userCacheDir,
  verifyChecksum,
};
