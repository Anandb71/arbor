"use strict";
// Tests for the first-run download (#281). Run: npm test (from packaging/npm)
// or node --test packaging/npm/test/download.test.js. Everything is served from
// a local HTTP server; nothing here touches the network.

const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const zlib = require("node:zlib");

const PACKAGE_DIR = path.join(__dirname, "..");
const lib = require("../lib/download");

// ---- archive fixtures -------------------------------------------------------

function tarHeader(name, size, type) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  header.write("0000755\0", 100);
  header.write("0000000\0", 108);
  header.write("0000000\0", 116);
  header.write(`${size.toString(8).padStart(11, "0")}\0`, 124);
  header.write("00000000000\0", 136);
  header.write("        ", 148);
  header.write(type, 156);
  header.write("ustar\0", 257);
  header.write("00", 263);
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148);
  return header;
}

function tarEntry(name, data, type = "0") {
  const pad = Buffer.alloc((512 - (data.length % 512)) % 512);
  return Buffer.concat([tarHeader(name, data.length, type), data, pad]);
}

function paxEntry(records) {
  const body = records
    .map(([key, value]) => {
      const rest = ` ${key}=${value}\n`;
      let len = rest.length + 1;
      while (String(len).length + rest.length !== len) len = String(len).length + rest.length;
      return `${len}${rest}`;
    })
    .join("");
  return tarEntry("PaxHeader/x", Buffer.from(body), "x");
}

function tarGz(entries) {
  return zlib.gzipSync(Buffer.concat([...entries, Buffer.alloc(1024)]));
}

// A minimal zip writer. CRCs are left at 0: the extractor relies on the
// archive's SHA-256 from SHA256SUMS for integrity, not on zip CRCs.
function zip(entries, { store = false } = {}) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBuf = Buffer.from(name);
    const body = store ? data : zlib.deflateRawSync(data);
    const method = store ? 0 : 8;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(method, 10);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, nameBuf, body);
    centrals.push(central, nameBuf);
    offset += 30 + nameBuf.length + body.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

const sha256 = (buf) => crypto.createHash("sha256").update(buf).digest("hex");
const sumsFor = (files) =>
  Object.entries(files)
    .map(([name, buf]) => `${sha256(buf)}  ${name}`)
    .join("\n") + "\n";

// ---- a local release server ---------------------------------------------------

async function serve(routes) {
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push(req.url);
    const route = routes[req.url];
    if (route === undefined) {
      res.writeHead(404).end("not found");
    } else if (route && route.redirect) {
      res.writeHead(302, { Location: route.redirect }).end();
    } else {
      res.writeHead(200, { "Content-Type": "application/octet-stream" }).end(route);
    }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  return { base, requests, close: () => new Promise((resolve) => server.close(resolve)) };
}

function tmpDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "arbor-npm-test-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

function cleanEnv(extra) {
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (/^(https?_proxy|no_proxy|npm_config_(https_)?proxy|npm_config_noproxy|ARBOR_NPM_)/i.test(key)) delete env[key];
  }
  return { ...env, ...extra };
}

const quiet = () => {};

// The fixture server runs in this process, so the shim must run without
// blocking the event loop (spawnSync would starve the server).
function runNode(args, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, { env });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk) => (stdout += chunk));
    child.stderr.setEncoding("utf8").on("data", (chunk) => (stderr += chunk));
    child.on("error", reject);
    child.on("close", (status) => resolve({ status, stdout, stderr }));
  });
}
const BINARY = Buffer.from("#!/bin/sh\necho fake arbor\n");

// ---- unit tests ---------------------------------------------------------------------

test("maps every supported platform to a release asset, and nothing else", () => {
  assert.equal(lib.assetFor("linux", "x64"), "arbor-linux-x86_64.tar.gz");
  assert.equal(lib.assetFor("linux", "arm64"), "arbor-linux-aarch64.tar.gz");
  assert.equal(lib.assetFor("darwin", "x64"), "arbor-macos-x86_64.tar.gz");
  assert.equal(lib.assetFor("darwin", "arm64"), "arbor-macos-aarch64.tar.gz");
  assert.equal(lib.assetFor("win32", "x64"), "arbor-windows-x86_64.zip");
  assert.equal(lib.assetFor("win32", "arm64"), null);
  assert.equal(lib.assetFor("freebsd", "x64"), null);
  assert.equal(lib.binaryName("win32"), "arbor.exe");
  assert.equal(lib.binaryName("linux"), "arbor");
});

test("the default download is this package version's GitHub Release", () => {
  assert.equal(
    lib.releaseBase(cleanEnv({})),
    `https://github.com/Anandb71/arbor/releases/download/v${lib.VERSION}`
  );
  assert.equal(lib.releaseBase(cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: "https://mirror.example/arbor/" })), "https://mirror.example/arbor");
  assert.equal(lib.releaseBase(cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: "http://127.0.0.1:9/x" })), "http://127.0.0.1:9/x");
  assert.throws(
    () => lib.releaseBase(cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: "http://mirror.example/arbor" })),
    /must use https/
  );
});

test("checksum: accepts the SHA256SUMS line, rejects a mismatch or a missing line", () => {
  const archive = Buffer.from("archive bytes");
  const sums = `${sha256(Buffer.from("other"))}  arbor-linux-aarch64.tar.gz\n${sha256(archive)} *arbor-linux-x86_64.tar.gz\r\n`;
  assert.equal(lib.verifyChecksum(archive, "arbor-linux-x86_64.tar.gz", sums), sha256(archive));
  assert.throws(() => lib.verifyChecksum(archive, "arbor-linux-aarch64.tar.gz", sums), /does not match SHA256SUMS/);
  assert.throws(() => lib.verifyChecksum(archive, "arbor-windows-x86_64.zip", sums), /no line for arbor-windows-x86_64.zip/);
});

test("tar.gz: takes only the binary, past decoys, ./ prefixes and pax headers", () => {
  const archive = tarGz([
    tarEntry("._arbor", Buffer.from("AppleDouble")),
    tarEntry("docs/", Buffer.alloc(0), "5"),
    paxEntry([["path", "./arbor"], ["mtime", "1"]]),
    tarEntry("ignored-name-from-header", BINARY),
    tarEntry("arbor.sig", Buffer.from("sig")),
  ]);
  assert.deepEqual(lib.extractFromTarGz(archive, "arbor"), BINARY);
  assert.equal(lib.extractFromTarGz(archive, "arbor.exe"), null);
  assert.throws(() => lib.extractFromTarGz(Buffer.from("not gzip"), "arbor", "x.tar.gz"), /x.tar.gz looks damaged/);
});

test("zip: takes only the binary, deflated or stored", () => {
  for (const store of [false, true]) {
    const archive = zip([["README.txt", Buffer.from("readme")], ["arbor.exe", BINARY]], { store });
    assert.deepEqual(lib.extractFromZip(archive, "arbor.exe"), BINARY);
    assert.equal(lib.extractFromZip(archive, "arbor"), null);
  }
  assert.throws(() => lib.extractFromZip(Buffer.alloc(100), "arbor.exe", "x.zip"), /x.zip looks damaged/);
});

test("proxy settings come from the environment or from npm's config", () => {
  assert.deepEqual(lib.proxyEnvFrom({}), {});
  assert.deepEqual(lib.proxyEnvFrom({ npm_config_https_proxy: "http://p:8080", npm_config_noproxy: "localhost" }), {
    HTTPS_PROXY: "http://p:8080",
    NO_PROXY: "localhost",
  });
  assert.equal(lib.proxyEnvFrom({ HTTPS_PROXY: "http://env:1", npm_config_https_proxy: "http://npm:2" }).HTTPS_PROXY, "http://env:1");
  assert.equal(lib.nodeUsesProxyEnv("20.18.0"), false);
  assert.equal(lib.nodeUsesProxyEnv("22.21.0"), true);
  assert.equal(lib.nodeUsesProxyEnv("24.4.1"), false);
  assert.equal(lib.nodeUsesProxyEnv("24.5.0"), true);
});

// ---- ensureBinary against a local release ------------------------------------------

for (const [platform, arch, asset, build] of [
  ["linux", "x64", "arbor-linux-x86_64.tar.gz", () => tarGz([tarEntry("arbor", BINARY)])],
  ["win32", "x64", "arbor-windows-x86_64.zip", () => zip([["arbor.exe", BINARY]])],
]) {
  test(`downloads, verifies and installs ${asset}, then reuses it`, async (t) => {
    const archive = build();
    const server = await serve({ [`/r/${asset}`]: archive, "/r/SHA256SUMS": sumsFor({ [asset]: archive }) });
    t.after(server.close);
    const dir = path.join(tmpDir(t), "bin");
    const options = { env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r` }), platform, arch, dirs: [dir], log: quiet };

    const first = await lib.ensureBinary(options);
    assert.equal(first.downloaded, true);
    assert.equal(first.path, path.join(dir, lib.binaryName(platform)));
    assert.deepEqual(fs.readFileSync(first.path), BINARY);
    assert.deepEqual(fs.readdirSync(dir), [lib.binaryName(platform)], "no temporary files left behind");
    if (process.platform !== "win32") assert.equal(fs.statSync(first.path).mode & 0o111, 0o111);

    const before = server.requests.length;
    const second = await lib.ensureBinary(options);
    assert.deepEqual(second, { path: first.path, downloaded: false });
    assert.equal(server.requests.length, before, "no second download");
  });
}

test("follows redirects, as GitHub Releases does", async (t) => {
  const archive = tarGz([tarEntry("arbor", BINARY)]);
  const server = await serve({
    "/r/arbor-linux-x86_64.tar.gz": { redirect: "/objects/a" },
    "/r/SHA256SUMS": { redirect: "/objects/s" },
    "/objects/a": archive,
    "/objects/s": sumsFor({ "arbor-linux-x86_64.tar.gz": archive }),
  });
  t.after(server.close);
  const dir = tmpDir(t);
  const result = await lib.ensureBinary({
    env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r` }),
    platform: "linux",
    arch: "x64",
    dirs: [dir],
    log: quiet,
  });
  assert.equal(result.downloaded, true);
});

test("installs into the next folder when the package folder is not writable", async (t) => {
  const archive = tarGz([tarEntry("arbor", BINARY)]);
  const server = await serve({ "/r/arbor-linux-x86_64.tar.gz": archive, "/r/SHA256SUMS": sumsFor({ "arbor-linux-x86_64.tar.gz": archive }) });
  t.after(server.close);
  const root = tmpDir(t);
  const notADir = path.join(root, "bin");
  fs.writeFileSync(notADir, "a file where a folder should be");
  const cache = path.join(root, "cache", "v1");
  const result = await lib.ensureBinary({
    env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r` }),
    platform: "linux",
    arch: "x64",
    dirs: [notADir, cache],
    log: quiet,
  });
  assert.equal(result.path, path.join(cache, "arbor"));
});

test("refuses an archive that does not match SHA256SUMS and installs nothing", async (t) => {
  const archive = tarGz([tarEntry("arbor", BINARY)]);
  const server = await serve({
    "/r/arbor-linux-x86_64.tar.gz": archive,
    "/r/SHA256SUMS": sumsFor({ "arbor-linux-x86_64.tar.gz": Buffer.from("something else") }),
  });
  t.after(server.close);
  const dir = tmpDir(t);
  await assert.rejects(
    lib.ensureBinary({ env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r` }), platform: "linux", arch: "x64", dirs: [dir], log: quiet }),
    (err) => {
      assert.ok(err instanceof lib.ArborInstallError);
      assert.match(err.message, /does not match SHA256SUMS/);
      assert.match(err.message, /To install it by hand/);
      return true;
    }
  );
  assert.deepEqual(fs.readdirSync(dir), []);
});

test("a missing release asset says HTTP 404 and how to install by hand", async (t) => {
  const server = await serve({ "/r/SHA256SUMS": "" });
  t.after(server.close);
  await assert.rejects(
    lib.ensureBinary({ env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r` }), platform: "linux", arch: "x64", dirs: [tmpDir(t)], log: quiet }),
    (err) => {
      assert.match(err.message, /HTTP 404/);
      assert.match(err.message, new RegExp(`${server.base}/r/arbor-linux-x86_64\\.tar\\.gz`));
      assert.match(err.message, /docs\/INSTALL\.md/);
      return true;
    }
  );
});

test("an unreachable server is reported as a network error, not a crash", async (t) => {
  const server = await serve({});
  const base = server.base;
  await server.close();
  await assert.rejects(
    lib.ensureBinary({ env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${base}/r` }), platform: "linux", arch: "x64", dirs: [tmpDir(t)], log: quiet }),
    (err) => {
      assert.ok(err instanceof lib.ArborInstallError);
      assert.match(err.message, /Could not download \(ECONNREFUSED/);
      return true;
    }
  );
});

test("a proxy that Node.js will not use is named in the error, without its URL", () => {
  const lines = lib.explainDownloadError(Object.assign(new Error("x"), { code: "ECONNREFUSED" }), {
    HTTPS_PROXY: "http://user:secret@proxy:3128",
  });
  const text = lines.join("\n");
  assert.doesNotMatch(text, /secret/);
  if (!lib.nodeUsesProxyEnv()) assert.match(text, /A proxy is set/);
});

test("an unsupported platform is refused before any download", async (t) => {
  const server = await serve({});
  t.after(server.close);
  await assert.rejects(
    lib.ensureBinary({ env: cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r` }), platform: "freebsd", arch: "x64", dirs: [tmpDir(t)], log: quiet }),
    /no prebuilt binary for freebsd-x64[\s\S]*cargo install arbor-graph-cli/
  );
  assert.deepEqual(server.requests, []);
});

// ---- the bin shim, end to end ---------------------------------------------------------

test("bin/arbor.js downloads on first run, runs the binary, and passes its exit code", { skip: !lib.assetFor() && "no prebuilt binary for this platform" }, async (t) => {
  const root = tmpDir(t);
  const pkg = path.join(root, "pkg");
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.mkdirSync(path.join(pkg, "lib"));
  fs.copyFileSync(path.join(PACKAGE_DIR, "package.json"), path.join(pkg, "package.json"));
  fs.copyFileSync(path.join(PACKAGE_DIR, "bin", "arbor.js"), path.join(pkg, "bin", "arbor.js"));
  fs.copyFileSync(path.join(PACKAGE_DIR, "lib", "download.js"), path.join(pkg, "lib", "download.js"));

  // The "binary": a shell script on macOS/Linux; on Windows a copy of
  // node.exe (stored, not deflated, to keep the test fast).
  const asset = lib.assetFor();
  let archive;
  let versionArgs;
  let expectedVersion;
  let exitArgs;
  if (process.platform === "win32") {
    archive = zip([["arbor.exe", fs.readFileSync(process.execPath)]], { store: true });
    versionArgs = ["--version"];
    expectedVersion = process.version;
    exitArgs = ["-e", "process.exit(3)"];
  } else {
    const script = '#!/bin/sh\nif [ "$1" = "--exit" ]; then exit "$2"; fi\necho "arbor-stub $*"\n';
    archive = tarGz([tarEntry("arbor", Buffer.from(script))]);
    versionArgs = ["--version"];
    expectedVersion = "arbor-stub --version";
    exitArgs = ["--exit", "3"];
  }
  const server = await serve({ [`/r/${asset}`]: archive, "/r/SHA256SUMS": sumsFor({ [asset]: archive }) });
  t.after(server.close);
  const env = cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r`, ARBOR_NPM_CACHE_DIR: path.join(root, "cache") });
  const run = (args) => runNode([path.join(pkg, "bin", "arbor.js"), ...args], env);

  const first = await run(versionArgs);
  assert.equal(first.status, 0, first.stderr);
  assert.equal(first.stdout.trim(), expectedVersion, "stdout carries only the binary's output");
  assert.match(first.stderr, /downloading Arbor v.* once/);
  assert.match(first.stderr, new RegExp(`checked ${asset.replace(/\./g, "\\.")} against SHA256SUMS`));
  assert.ok(fs.existsSync(path.join(pkg, "bin", lib.binaryName())));

  const requests = server.requests.length;
  const second = await run(versionArgs);
  assert.equal(second.status, 0, second.stderr);
  assert.equal(second.stdout.trim(), expectedVersion);
  assert.equal(second.stderr, "", "no download message once installed");
  assert.equal(server.requests.length, requests);

  assert.equal((await run(exitArgs)).status, 3);
});

test("bin/arbor.js exits 2 with a clear message when the download fails", { skip: !lib.assetFor() && "no prebuilt binary for this platform" }, async (t) => {
  const root = tmpDir(t);
  const pkg = path.join(root, "pkg");
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.mkdirSync(path.join(pkg, "lib"));
  fs.copyFileSync(path.join(PACKAGE_DIR, "package.json"), path.join(pkg, "package.json"));
  fs.copyFileSync(path.join(PACKAGE_DIR, "bin", "arbor.js"), path.join(pkg, "bin", "arbor.js"));
  fs.copyFileSync(path.join(PACKAGE_DIR, "lib", "download.js"), path.join(pkg, "lib", "download.js"));
  const server = await serve({});
  t.after(server.close);
  const result = await runNode(
    [path.join(pkg, "bin", "arbor.js"), "--version"],
    cleanEnv({ ARBOR_NPM_DOWNLOAD_BASE: `${server.base}/r`, ARBOR_NPM_CACHE_DIR: path.join(root, "cache") })
  );
  assert.equal(result.status, 2);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /^arbor: could not install the Arbor v/m);
  assert.match(result.stderr, /HTTP 404/);
  assert.doesNotMatch(result.stderr, /\n\s+at /, "no stack trace");
});
