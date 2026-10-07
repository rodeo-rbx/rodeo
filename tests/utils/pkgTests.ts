// Shared @rodeo/* API test cases — port of tests/utils/pkgTests.luau.
// Each module is a function taking a `run(opts)` closure that executes against
// some DOM. No case names or script sources are modified from the lute version.

import { it, expect } from "bun:test";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { randomUUID } from "node:crypto";
import type { RunCodeOpts, RunResult } from "../../rodeo-client-ts/src/run.js";
import { writeMinimalGlb } from "./glb.js";

export type RunFn = (opts: RunCodeOpts) => Promise<RunResult>;

// Remove a directory and everything under it, ignoring errors (for idempotent
// fixtures). Uses node's fs so it works without a Unix `rm` on Windows.
function rmrf(path: string): void {
  try {
    rmSync(path, { recursive: true, force: true });
  } catch {}
}

// Cross-platform Luau table-literal sources for spawning test programs. The
// @rodeo/process API execs on the host, so the program must exist there — but
// Unix `echo`/`cat`/`sleep`/`false` have no stock-Windows equivalents. Rather
// than branch per OS, drive every program through the JS runtime already
// running these tests: its absolute path needs no PATH lookup and is
// guaranteed present, and the `-e` snippets are plain node-compatible JS that
// behaves identically on every platform.
//
// `process.execPath` is read off globalThis because the `process()` factory
// exported below is a function declaration: it hoists and shadows the global
// `process` binding for the entire module, so a bare `process.execPath` here
// would resolve to that function (and be undefined).
const RT = globalThis.process.execPath.replace(/\\/g, "\\\\");
const echoArgs = (msg: string) =>
  `{ "${RT}", "-e", "process.stdout.write('${msg}')" }`;
const catArgs = `{ "${RT}", "-e", "process.stdin.pipe(process.stdout)" }`;
const sleepArgs = `{ "${RT}", "-e", "setTimeout(function(){}, 999000)" }`;
const falseArgs = `{ "${RT}", "-e", "process.exit(1)" }`;

// ── smoke (1 test) ────────────────────────────────────────────────────────

export function smoke(run: RunFn): void {
  it("all rodeo modules exist", async () => {
    const result = await run({
      showReturn: true,
      source: `return {
          fs = require("@rodeo/fs") ~= nil,
          io = require("@rodeo/io") ~= nil,
          process = require("@rodeo/process") ~= nil,
          stream = require("@rodeo/stream") ~= nil,
      }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"fs":true');
    expect(result.output).toContain('"io":true');
    expect(result.output).toContain('"process":true');
    expect(result.output).toContain('"stream":true');
  });
}

// ── fs (8 tests) ──────────────────────────────────────────────────────────

export function fs(run: RunFn): void {
  it("fs: open + write + read round-trip", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-fs.txt", "w")
        stream.write(f, "hello fs")
        stream.close(f)
        local f2 = fs.open("rodeo-test-fs.txt", "r")
        local data = stream.read(f2)
        stream.close(f2)
        fs.remove("rodeo-test-fs.txt")
        return data`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("hello fs");
  });

  it("fs: overwrite replaces content", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-overwrite.txt", "w")
        stream.write(f, "first")
        stream.close(f)
        local f2 = fs.open("rodeo-test-overwrite.txt", "w")
        stream.write(f2, "second")
        stream.close(f2)
        local f3 = fs.open("rodeo-test-overwrite.txt", "r")
        local data = stream.read(f3)
        stream.close(f3)
        fs.remove("rodeo-test-overwrite.txt")
        return data`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("second");
  });

  it("fs: exists and type", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-exists.txt", "w")
        stream.write(f, "x")
        stream.close(f)
        local exists = fs.exists("rodeo-test-exists.txt")
        local ftype = fs.type("rodeo-test-exists.txt")
        local missing = fs.exists("rodeo-nonexistent-xyz.txt")
        fs.remove("rodeo-test-exists.txt")
        return { exists = exists, type = ftype, missing = missing }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"exists":true');
    expect(result.output).toContain('"type":"file"');
    expect(result.output).toContain('"missing":false');
  });

  it("fs: mkdir + listdir + rmdir", async () => {
    rmrf("rodeo-test-dir");
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        fs.mkdir("rodeo-test-dir")
        local exists = fs.exists("rodeo-test-dir")
        local dtype = fs.type("rodeo-test-dir")
        local f = fs.open("rodeo-test-dir/a.txt", "w")
        stream.write(f, "a")
        stream.close(f)
        local entries = fs.listdir("rodeo-test-dir")
        fs.remove("rodeo-test-dir/a.txt")
        fs.rmdir("rodeo-test-dir")
        return { exists = exists, type = dtype, entryCount = #entries }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"exists":true');
    expect(result.output).toContain('"type":"dir"');
    expect(result.output).toContain('"entryCount":1');
  });

  it("fs: remove deletes file", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-rm.txt", "w")
        stream.write(f, "x")
        stream.close(f)
        fs.remove("rodeo-test-rm.txt")
        return fs.exists("rodeo-test-rm.txt")`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("false");
  });

  it("fs: copy duplicates file", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-copy-src.txt", "w")
        stream.write(f, "copy me")
        stream.close(f)
        fs.copy("rodeo-test-copy-src.txt", "rodeo-test-copy-dst.txt")
        local f2 = fs.open("rodeo-test-copy-dst.txt", "r")
        local data = stream.read(f2)
        stream.close(f2)
        local srcExists = fs.exists("rodeo-test-copy-src.txt")
        fs.remove("rodeo-test-copy-src.txt")
        fs.remove("rodeo-test-copy-dst.txt")
        return { data = data, srcExists = srcExists }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("copy me");
    expect(result.output).toContain('"srcExists":true');
  });

  it("fs: stat returns metadata", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-stat.txt", "w")
        stream.write(f, "meta")
        stream.close(f)
        local stat = fs.stat("rodeo-test-stat.txt")
        fs.remove("rodeo-test-stat.txt")
        return { hasType = stat.type ~= nil, statType = stat.type }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"hasType":true');
    expect(result.output).toContain('"statType":"file"');
  });

  it("fs: read large file (2MB JSON)", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("tests/fixtures/pkg/giant_file.json", "r")
        local content = stream.read(f)
        stream.close(f)
        local len = #content
        local parsed = game:GetService("HttpService"):JSONDecode(content)
        local hasElevation = parsed.elevation ~= nil
        local width = parsed.elevation and parsed.elevation.width or 0
        return { len = len, hasElevation = hasElevation, width = width }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"hasElevation":true');
    expect(result.output).toContain('"width":454');
  });

  it("stream: readBytes/writeBytes round-trip non-UTF-8 bytes", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local input = buffer.create(6)
        buffer.writeu8(input, 0, 0x00); buffer.writeu8(input, 1, 0xFF)
        buffer.writeu8(input, 2, 0xC0); buffer.writeu8(input, 3, 0xC1)
        buffer.writeu8(input, 4, 0xFE); buffer.writeu8(input, 5, 0xFF)

        local w = fs.open("rodeo-test-bytes.bin", "w")
        stream.writeBytes(w, input)
        stream.close(w)

        local r = fs.open("rodeo-test-bytes.bin", "r")
        local out = stream.readBytes(r)
        stream.close(r)
        fs.remove("rodeo-test-bytes.bin")

        return {
          len = buffer.len(out),
          b0 = buffer.readu8(out, 0), b1 = buffer.readu8(out, 1),
          b2 = buffer.readu8(out, 2), b3 = buffer.readu8(out, 3),
          b4 = buffer.readu8(out, 4), b5 = buffer.readu8(out, 5),
        }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"len":6');
    expect(result.output).toContain('"b0":0');
    expect(result.output).toContain('"b1":255');
    expect(result.output).toContain('"b2":192');
    expect(result.output).toContain('"b3":193');
    expect(result.output).toContain('"b4":254');
    expect(result.output).toContain('"b5":255');
  });
}

// ── io (3 tests) ──────────────────────────────────────────────────────────

export function io(run: RunFn): void {
  it("io: stdout write is captured", async () => {
    const result = await run({
      source: `local io = require("@rodeo/io")
        local stream = require("@rodeo/stream")
        stream.write(io.stdout, "hello from io\\n")`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("hello from io");
  });

  it("io: stderr write is captured", async () => {
    // Lute asserts `result.err`. TS client merges stdout+stderr into `output`.
    const result = await run({
      source: `local io = require("@rodeo/io")
        local stream = require("@rodeo/stream")
        stream.write(io.stderr, "stderr msg\\n")`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("stderr msg");
  });

  it("io: file round-trip via fs + stream", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local f = fs.open("rodeo-test-io.txt", "w")
        stream.write(f, "line one\\n")
        stream.write(f, "line two\\n")
        stream.close(f)
        local f2 = fs.open("rodeo-test-io.txt", "r")
        local data = stream.read(f2)
        stream.close(f2)
        fs.remove("rodeo-test-io.txt")
        return data`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("line one");
    expect(result.output).toContain("line two");
  });
}

// ── process (14 tests) ────────────────────────────────────────────────────

export function process(run: RunFn): void {
  it("process: cwd, homedir, execpath are functions", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        return {
            cwdIsFunc = type(p.cwd) == "function",
            cwdNonEmpty = #p.cwd() > 0,
            homedirIsFunc = type(p.homedir) == "function",
            homedirNonEmpty = #p.homedir() > 0,
            execpathIsFunc = type(p.execpath) == "function",
        }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"cwdIsFunc":true');
    expect(result.output).toContain('"cwdNonEmpty":true');
    expect(result.output).toContain('"homedirIsFunc":true');
    expect(result.output).toContain('"homedirNonEmpty":true');
    expect(result.output).toContain('"execpathIsFunc":true');
  });

  it("process: args is a table", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        return type(p.args) == "table"`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("true");
  });

  it("process: env is readable and read-only", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local homeExists = p.env.HOME ~= nil or p.env.USERPROFILE ~= nil
        local writeBlocked = not pcall(function() p.env.TEST = "x" end)
        return { homeExists = homeExists, writeBlocked = writeBlocked }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"homeExists":true');
    expect(result.output).toContain('"writeBlocked":true');
  });

  it("process: run executes command", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local r = p.run(${echoArgs("hello")})
        return { ok = r.ok, exitcode = r.exitcode, hasHello = string.find(r.out, "hello") ~= nil }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"ok":true');
    expect(result.output).toContain('"exitcode":0');
    expect(result.output).toContain('"hasHello":true');
  });

  it("process: run returns failure for bad command", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local r = p.run(${falseArgs})
        return { ok = r.ok }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"ok":false');
  });

  it("process: system runs shell command", async () => {
    // `echo` is a builtin in both cmd.exe and POSIX sh, so this exercises
    // system()'s shell without platform-specific syntax. (On Windows it also
    // proves the shell path specifically: `echo` is not a standalone program
    // there, so p.run({"echo",...}) would fail where p.system succeeds.)
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local r = p.system("echo hello")
        return { ok = r.ok, hasOut = string.find(r.out, "hello") ~= nil }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"ok":true');
    expect(result.output).toContain('"hasOut":true');
  });

  it("process: run adds env to the child's environment", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local r = p.run({ "${RT}", "-e", "process.stdout.write(process.env.RODEO_TEST_VAR)" }, {
          env = { RODEO_TEST_VAR = "from env" },
        })
        return r.out`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("from env");
  });

  it("process: run writes input to the child's stdin", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local fromString = p.run(${catArgs}, { input = "piped in" }).out
        local fromBuffer = p.run(${catArgs}, { input = buffer.fromstring("buffered") }).out
        return fromString .. "|" .. fromBuffer`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("piped in|buffered");
  });

  it("process: run reports the signal that ended the child", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local r = p.run({ "${RT}", "-e", "process.kill(process.pid, 'SIGKILL')" })
        return { ok = r.ok, exitcode = r.exitcode, signal = r.signal }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"ok":false');
    // Windows has no signals: the child just exits.
    if (globalThis.process.platform !== "win32") {
      expect(result.output).toContain('"exitcode":-1');
      expect(result.output).toContain('"signal":9');
    }
  });

  it("process: system runs the command in the given shell", async () => {
    if (globalThis.process.platform === "win32") return; // no /bin/bash
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        return p.system("echo $0", { shell = "/bin/bash" }).out`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("/bin/bash");
  });

  it("process: create + stream read", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local stream = require("@rodeo/stream")
        local child = p.create(${echoArgs("piped output")}, { stdio = "piped" })
        local output = stream.read(child.stdout)
        return string.find(output, "piped output") ~= nil`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("true");
  });

  it("process: create + multiple stream reads via stdin/stdout", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local stream = require("@rodeo/stream")
        local child = p.create(${catArgs}, { stdio = "piped" })
        stream.write(child.stdin, "first\\n")
        local r1 = stream.read(child.stdout)
        stream.write(child.stdin, "second\\n")
        local r2 = stream.read(child.stdout)
        stream.close(child.stdin)
        return { r1 = r1, r2 = r2, bothRead = r1 ~= nil and r2 ~= nil }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"bothRead":true');
    expect(result.output).toContain("first");
    expect(result.output).toContain("second");
  });

  it("process: create + stream write to stdin", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local stream = require("@rodeo/stream")
        local child = p.create(${catArgs}, { stdio = "piped" })
        stream.write(child.stdin, "hello from stdin\\n")
        stream.close(child.stdin)
        local output = stream.read(child.stdout)
        return string.find(output, "hello from stdin") ~= nil`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("true");
  });

  it("process: create + kill", async () => {
    const result = await run({
      showReturn: true,
      source: `local p = require("@rodeo/process")
        local sleeper = p.create(${sleepArgs}, { stdio = "piped" })
        p.kill(sleeper)
        local status = p.run(sleeper)
        return status.ok == false`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("true");
  });
}

// ── capture (15 tests, plugin-only) ────────────────────────────────────────
//
// roblox.captureViewport drives Studio's device simulator for `device` / `viewportSize`
// (plugin-handled RPCs) and finalizes the engine's frame on the run client,
// resampling it to exactly the capture's Camera.ViewportSize. These read the
// written PNG's IHDR to check the pixel size independently of what the API
// reports. Every test captures into its own file and removes it.

function pngSize(path: string): { width: number; height: number } {
  const bytes = readFileSync(path);
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
}

function captureOut(name: string): string {
  return `.rodeo/.temp/captures/test-${name}-${randomUUID()}.png`;
}

// Luau prelude: capture into `out` with `opts` and return everything the
// assertions need. The viewport is read before the capture, so for the
// window case it is the plain window size.
function captureSource(out: string, opts: string): string {
  return `local roblox = require("@rodeo/roblox")
    local cam = workspace.CurrentCamera
    local vp = cam.ViewportSize
    local path, info = roblox.captureViewport("${out}", ${opts})
    return { path = path, width = info.width, height = info.height, vpX = vp.X, vpY = vp.Y }`;
}

export function capture(run: RunFn): void {
  it("capture: default output is exactly the window viewport size", async () => {
    const out = captureOut("window");
    try {
      const result = await run({ showReturn: true, source: captureSource(out, "{}") });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string; width: number; height: number; vpX: number; vpY: number };
      expect(existsSync(r.path)).toBe(true);
      const size = pngSize(r.path);
      expect(size).toEqual({ width: Math.round(r.vpX), height: Math.round(r.vpY) });
      expect({ width: r.width, height: r.height }).toEqual(size);
    } finally {
      rmrf(out);
    }
  });

  it("capture: viewportSize yields an image of exactly that size", async () => {
    const out = captureOut("viewport");
    try {
      const result = await run({
        showReturn: true,
        source: captureSource(out, "{ viewportSize = Vector2.new(640, 360), settle = 1 }"),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string; width: number; height: number };
      expect(pngSize(r.path)).toEqual({ width: 640, height: 360 });
      expect({ width: r.width, height: r.height }).toEqual({ width: 640, height: 360 });
    } finally {
      rmrf(out);
    }
  });

  it("capture: device preset selects its viewport", async () => {
    const out = captureOut("device");
    try {
      // hd_720 is a built-in desktop preset: 1280x720 with no insets.
      const result = await run({
        showReturn: true,
        source: captureSource(out, '{ device = "hd_720", settle = 1 }'),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string };
      expect(pngSize(r.path)).toEqual({ width: 1280, height: 720 });
    } finally {
      rmrf(out);
    }
  });

  it("capture: viewportSize overrides a device preset's resolution", async () => {
    const out = captureOut("device-override");
    try {
      const result = await run({
        showReturn: true,
        source: captureSource(out, '{ device = "hd_720", viewportSize = Vector2.new(800, 600), settle = 1 }'),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string };
      expect(pngSize(r.path)).toEqual({ width: 800, height: 600 });
    } finally {
      rmrf(out);
    }
  });

  it("capture: the simulator's maximum, 7680x4320, writes an image of exactly that size", async () => {
    // The frame comes from the engine's capture file, so a 2x display's
    // 15360x8640 frame is fine; the EditableImage route this replaced was
    // capped at 8192 pixels a side and failed here.
    const out = captureOut("8k");
    try {
      const result = await run({
        showReturn: true,
        source: captureSource(out, "{ viewportSize = Vector2.new(7680, 4320), settle = 1 }"),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string; width: number; height: number };
      expect(pngSize(r.path)).toEqual({ width: 7680, height: 4320 });
      expect({ width: r.width, height: r.height }).toEqual({ width: 7680, height: 4320 });
    } finally {
      rmrf(out);
    }
  });

  it("capture: resample = false writes the engine's frame at its rendered size", async () => {
    // The frame is the viewport times the display scale (2x on Retina, 1x on a
    // 1x monitor), so assert the scale rather than a number: both axes share
    // one whole-ish multiple of the requested 640x360, at least 1.
    const out = captureOut("keep-frame");
    try {
      const result = await run({
        showReturn: true,
        source: captureSource(out, "{ viewportSize = Vector2.new(640, 360), resample = false, settle = 1 }"),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string; width: number; height: number };
      const size = pngSize(r.path);
      expect({ width: r.width, height: r.height }).toEqual(size);
      const sx = size.width / 640;
      const sy = size.height / 360;
      expect(sx).toBeGreaterThanOrEqual(1);
      expect(Math.abs(sx - sy)).toBeLessThan(0.02);
    } finally {
      rmrf(out);
    }
  });

  it("capture: a phone preset renders its full resolution, not the inset viewport", async () => {
    // iPhone 13 is 844x390; its Camera.ViewportSize in landscape is the safe
    // area (750x369), but the engine renders and captures the whole screen.
    const out = captureOut("phone-landscape");
    try {
      const result = await run({
        showReturn: true,
        source: captureSource(out, '{ device = "iphone_13", orientation = "LandscapeLeft", settle = 1 }'),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string; width: number; height: number };
      expect(pngSize(r.path)).toEqual({ width: 844, height: 390 });
      expect({ width: r.width, height: r.height }).toEqual({ width: 844, height: 390 });
    } finally {
      rmrf(out);
    }
  });

  it("capture: orientation Portrait on a phone preset yields a portrait image", async () => {
    const out = captureOut("portrait");
    try {
      const result = await run({
        showReturn: true,
        source: captureSource(out, '{ device = "iphone_13", orientation = "Portrait", settle = 1 }'),
      });
      expect(result.ok).toBe(true);
      const r = result.return as { path: string; width: number; height: number };
      const size = pngSize(r.path);
      expect(size.height).toBeGreaterThan(size.width);
      expect({ width: r.width, height: r.height }).toEqual(size);
    } finally {
      rmrf(out);
    }
  });

  it("capture: doubling pixelDensity in ScaleToPhysicalSize halves the kept frame", async () => {
    // The absolute scale is host DPI / density and varies by display; the
    // ratio between two densities does not.
    const a = captureOut("density-a");
    const b = captureOut("density-b");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          local _, lo = roblox.captureViewport("${a}", { viewportSize = Vector2.new(640, 360), scalingMode = "ScaleToPhysicalSize", pixelDensity = 110, resample = false, settle = 1 })
          local _, hi = roblox.captureViewport("${b}", { viewportSize = Vector2.new(640, 360), scalingMode = "ScaleToPhysicalSize", pixelDensity = 220, resample = false, settle = 1 })
          return { lo = lo.width, hi = hi.width }`,
      });
      expect(result.ok).toBe(true);
      const r = result.return as { lo: number; hi: number };
      expect(Math.abs(r.lo / r.hi - 2)).toBeLessThan(0.02);
      expect(pngSize(a).width).toBe(r.lo);
      expect(pngSize(b).width).toBe(r.hi);
    } finally {
      rmrf(a);
      rmrf(b);
    }
  });

  it("capture: an unknown scalingMode errors naming the option, before capturing", async () => {
    const out = captureOut("bad-mode");
    try {
      const result = await run({ source: captureSource(out, '{ viewportSize = Vector2.new(320, 180), scalingMode = "Nope" }') });
      expect(result.ok).toBe(false);
      expect(result.output).toContain("scalingMode");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });

  it("capture: simulator options without device or viewportSize error", async () => {
    const out = captureOut("no-device");
    try {
      const result = await run({ source: captureSource(out, "{ pixelDensity = 72 }") });
      expect(result.ok).toBe(false);
      expect(result.output).toContain("viewportSize");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });

  it("capture: viewportSize over 4320 tall errors before capturing", async () => {
    const out = captureOut("too-tall");
    try {
      const result = await run({ source: captureSource(out, "{ viewportSize = Vector2.new(100, 5000) }") });
      expect(result.ok).toBe(false);
      expect(result.output).toContain("4320");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });

  it("capture: viewportSize over 7680 wide errors before capturing", async () => {
    const out = captureOut("too-wide");
    try {
      const result = await run({ source: captureSource(out, "{ viewportSize = Vector2.new(8000, 100) }") });
      expect(result.ok).toBe(false);
      expect(result.output).toContain("7680");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });

  it("capture: unknown device errors", async () => {
    const out = captureOut("unknown-device");
    try {
      const result = await run({ source: captureSource(out, '{ device = "rodeo-no-such-device" }') });
      expect(result.ok).toBe(false);
      expect(result.output.toLowerCase()).toContain("device");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });

  it("capture: simulator state is restored after the capture", async () => {
    // Studio persists emulation across launches, so the prior state may be
    // "off" or some user device: assert after == before, not after == off.
    const out = captureOut("restore");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          local sim = game:GetService("StudioDeviceSimulatorService")
          local function snapshot()
            local active, res = pcall(function() return sim:GetResolutionAsync() end)
            local okMode, mode = pcall(function() return sim:GetScalingModeAsync().Name end)
            local okDensity, density = pcall(function() return sim:GetPixelDensityAsync() end)
            return {
              device = sim:GetDeviceAsync(),
              active = active,
              resolution = active and (res.X .. "x" .. res.Y) or "off",
              mode = okMode and mode or "off",
              density = okDensity and density or "off",
              viewport = workspace.CurrentCamera.ViewportSize.X .. "x" .. workspace.CurrentCamera.ViewportSize.Y,
            }
          end
          local before = snapshot()
          roblox.captureViewport("${out}", { viewportSize = Vector2.new(320, 180), scalingMode = "ScaleToPhysicalSize", pixelDensity = 150, settle = 1 })
          task.wait(0.5)
          local after = snapshot()
          local leftovers = 0
          for _, id in ipairs(sim:GetDeviceListAsync()) do
            if string.sub(id, 1, 14) == "rodeo-capture-" then leftovers += 1 end
          end
          return { before = before, after = after, leftovers = leftovers }`,
      });
      expect(result.ok).toBe(true);
      const r = result.return as { before: Record<string, unknown>; after: Record<string, unknown>; leftovers: number };
      expect(r.after).toEqual(r.before);
      expect(r.leftovers).toBe(0);
      expect(r.before.viewport).not.toBe("320x180");
    } finally {
      rmrf(out);
    }
  });
}

// ── images (4 tests, plugin-only) ─────────────────────────────────────────
//
// roblox.exportEditableImage / importEditableImage move RGBA8 pixels between
// PNG files on the host and EditableImage objects in Studio.

export function images(run: RunFn): void {
  it("images: export then import round-trips pixels exactly", async () => {
    const out = captureOut("img-roundtrip");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          local AssetService = game:GetService("AssetService")
          local w, h = 8, 4
          local src = AssetService:CreateEditableImage({ Size = Vector2.new(w, h) })
          local pixels = buffer.create(w * h * 4)
          for i = 0, w * h * 4 - 1 do buffer.writeu8(pixels, i, (i * 7) % 256) end
          src:WritePixelsBuffer(Vector2.zero, src.Size, pixels)
          roblox.exportEditableImage("${out}", src)
          local back = roblox.importEditableImage("${out}")
          local got = back:ReadPixelsBuffer(Vector2.zero, back.Size)
          local equal = buffer.tostring(got) == buffer.tostring(pixels)
          local size = back.Size
          src:Destroy(); back:Destroy()
          return { equal = equal, width = size.X, height = size.Y }`,
      });
      expect(result.ok).toBe(true);
      expect(result.return).toEqual({ equal: true, width: 8, height: 4 });
      expect(pngSize(out)).toEqual({ width: 8, height: 4 });
    } finally {
      rmrf(out);
    }
  });

  it("images: import of a capture matches its viewport size", async () => {
    const out = captureOut("img-capture");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          roblox.captureViewport("${out}", { viewportSize = Vector2.new(100, 50), settle = 1 })
          local img = roblox.importEditableImage("${out}")
          local size = img.Size
          img:Destroy()
          return { width = size.X, height = size.Y }`,
      });
      expect(result.ok).toBe(true);
      expect(result.return).toEqual({ width: 100, height: 50 });
    } finally {
      rmrf(out);
    }
  });

  it("images: import of a missing file errors with the path", async () => {
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.importEditableImage("./rodeo-no-such-image-12345.png")`,
    });
    expect(result.ok).toBe(false);
    expect(result.output).toContain("rodeo-no-such-image-12345.png");
  });

  it("images: export to a non-png path errors", async () => {
    const out = captureOut("img-jpg").replace(/\.png$/, ".jpg");
    try {
      const result = await run({
        source: `local roblox = require("@rodeo/roblox")
          local img = game:GetService("AssetService"):CreateEditableImage({ Size = Vector2.new(2, 2) })
          roblox.exportEditableImage("${out}", img)`,
      });
      expect(result.ok).toBe(false);
      expect(result.output).toContain(".png");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });
}

// ── meshes (4 tests, plugin-only) ─────────────────────────────────────────
//
// roblox.exportEditableMesh / importEditableMesh move geometry, per-corner
// attributes and skinning between glTF files and EditableMesh objects.

// Luau: a closed tetrahedron with normals, UVs and colors on every corner.
const TETRA_LUAU = `
  local AssetService = game:GetService("AssetService")
  local mesh = AssetService:CreateEditableMesh()
  local P = { Vector3.new(0, 0, 0), Vector3.new(4, 0, 0), Vector3.new(0, 4, 0), Vector3.new(0, 0, 4) }
  local vIds = mesh:BatchAdd(Enum.MeshAttribute.Vertex, P)
  local faces = { {1, 3, 2}, {1, 2, 4}, {1, 4, 3}, {2, 3, 4} }
  local faceVerts, faceNormals, faceUVs, faceColors = {}, {}, {}, {}
  for f, tri in faces do
    local a, b, c = P[tri[1]], P[tri[2]], P[tri[3]]
    local n = (b - a):Cross(c - a).Unit
    local nIds = mesh:BatchAdd(Enum.MeshAttribute.Normal, { n, n, n })
    local uIds = mesh:BatchAdd(Enum.MeshAttribute.UV, { Vector2.new(0, 0), Vector2.new(1, 0), Vector2.new(0, 1) })
    local col = Color3.new(f / 4, 0.5, 1 - f / 4)
    local cIds = mesh:BatchAdd(Enum.MeshAttribute.Color, { col, col, col }, { 1, 1, 0.5 })
    faceVerts[f] = { vIds[tri[1]], vIds[tri[2]], vIds[tri[3]] }
    faceNormals[f], faceUVs[f], faceColors[f] = nIds, uIds, cIds
  end
  local fIds = mesh:BatchAdd(Enum.MeshAttribute.Face, faceVerts)
  mesh:BatchSetFaceAttributes(fIds, faceNormals)
  mesh:BatchSetFaceAttributes(fIds, faceUVs)
  mesh:BatchSetFaceAttributes(fIds, faceColors)
`;

function meshOut(name: string, ext: string): string {
  return `.rodeo/.temp/captures/test-${name}-${randomUUID()}.${ext}`;
}

export function meshes(run: RunFn): void {
  it("meshes: export then import round-trips geometry and attributes", async () => {
    const glb = meshOut("mesh", "glb");
    const gltf = meshOut("mesh", "gltf");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          ${TETRA_LUAU}
          roblox.exportEditableMesh("${glb}", mesh)
          roblox.exportEditableMesh("${gltf}", mesh)
          local back = roblox.importEditableMesh("${glb}")
          local faces = back:GetFaces()
          local corners, normalsOk, uvsOk, colorsOk = 0, true, true, true
          for _, f in faces do
            local vs = back:GetFaceVertices(f)
            local ns, us, cs = back:GetFaceNormals(f), back:GetFaceUVs(f), back:GetFaceColors(f)
            for k = 1, 3 do
              corners += 1
              local a, b, c = back:GetPosition(vs[1]), back:GetPosition(vs[2]), back:GetPosition(vs[3])
              local geo = (b - a):Cross(c - a).Unit
              local n = back:GetNormal(ns[k])
              if not n or (n - geo).Magnitude > 1e-3 then normalsOk = false end
              if not back:GetUV(us[k]) then uvsOk = false end
              if not back:GetColor(cs[k]) then colorsOk = false end
            end
          end
          local part = AssetService:CreateMeshPartAsync(Content.fromObject(back), { CollisionFidelity = Enum.CollisionFidelity.Box })
          local sizeOk = (part.Size - Vector3.new(4, 4, 4)).Magnitude < 1e-3
          part:Destroy(); mesh:Destroy(); back:Destroy()
          return { faces = #faces, corners = corners, normalsOk = normalsOk, uvsOk = uvsOk, colorsOk = colorsOk, sizeOk = sizeOk }`,
      });
      expect(result.ok).toBe(true);
      expect(result.return).toEqual({ faces: 4, corners: 12, normalsOk: true, uvsOk: true, colorsOk: true, sizeOk: true });
      expect(readFileSync(glb).subarray(0, 4).toString("latin1")).toBe("glTF");
      expect(readFileSync(gltf, "utf8").trimStart().startsWith("{")).toBe(true);
    } finally {
      rmrf(glb);
      rmrf(gltf);
    }
  });

  it("meshes: a 48000-vertex textured, colored mesh imports (issue #20)", async () => {
    // The importer used to BatchAdd every attribute, and the engine validates
    // a batch of N items as if it added N vertices, so UV and color batches
    // failed once the mesh passed about 30000 vertices. The engine's real
    // caps are 60000 vertices and 20000 triangles per mesh; this sits under
    // both, as a triangle soup with normals, UVs and colors on every corner.
    const glb = meshOut("large", "glb");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          local AssetService = game:GetService("AssetService")
          local mesh = AssetService:CreateEditableMesh()
          local TRIANGLES = 16000
          for f = 0, TRIANGLES - 1 do
            local x, z = (f % 160) * 2, (f // 160) * 2
            local vs, ns, us, cs = {}, {}, {}, {}
            for k, offset in { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 0, 1) } do
              vs[k] = mesh:AddVertex(Vector3.new(x, 0, z) + offset)
              ns[k] = mesh:AddNormal(Vector3.yAxis)
              us[k] = mesh:AddUV(Vector2.new((x + offset.X) / 320, (z + offset.Z) / 200))
              cs[k] = mesh:AddColor(Color3.fromHSV(f / TRIANGLES, 1, 1), 1)
            end
            local face = mesh:AddTriangle(vs[1], vs[2], vs[3])
            mesh:SetFaceNormals(face, ns)
            mesh:SetFaceUVs(face, us)
            mesh:SetFaceColors(face, cs)
          end
          roblox.exportEditableMesh("${glb}", mesh)
          local t0 = os.clock()
          local back = roblox.importEditableMesh("${glb}")
          local seconds = os.clock() - t0
          local faces = back:GetFaces()
          local sample = faces[#faces]
          local ok = back:GetUV(back:GetFaceUVs(sample)[3]) ~= nil
            and back:GetColor(back:GetFaceColors(sample)[3]) ~= nil
            and back:GetNormal(back:GetFaceNormals(sample)[3]) ~= nil
          local vertices = #back:GetVertices()
          mesh:Destroy(); back:Destroy()
          return { vertices = vertices, faces = #faces, attributesOk = ok, seconds = seconds }`,
      });
      expect(result.ok).toBe(true);
      const r = result.return as { vertices: number; faces: number; attributesOk: boolean; seconds: number };
      expect(r.vertices).toBe(48000);
      expect(r.faces).toBe(16000);
      expect(r.attributesOk).toBe(true);
      expect(r.seconds).toBeLessThan(30);
    } finally {
      rmrf(glb);
    }
  });

  it("meshes: past the engine's vertex cap, its error surfaces with the count reached (issue #20)", async () => {
    // No cap is hardcoded in rodeo: the engine refuses the add (60000 vertices
    // per mesh on Studio 0.739) and rodeo keeps its words, annotated with how
    // far the build got. 200000 stays over any plausible raise of the cap, and
    // the assertions avoid the exact refusal index for the same reason.
    const glb = meshOut("over-cap", "glb");
    try {
      writeMinimalGlb(glb, 200000, 1);
      const result = await run({
        source: `local roblox = require("@rodeo/roblox")
          roblox.importEditableMesh("${glb}")`,
      });
      expect(result.ok).toBe(false);
      expect(result.output).toContain("above limit");
      expect(result.output).toContain("adding vertex");
      expect(result.output).toContain("of 200000");
    } finally {
      rmrf(glb);
    }
  });

  it("meshes: skinning round-trips bones, parents, bind poses and weights", async () => {
    const glb = meshOut("skin", "glb");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          ${TETRA_LUAU}
          local rootCF = CFrame.new(0, 0, 0)
          local childCF = CFrame.new(0, 4, 0) * CFrame.Angles(0, math.rad(90), 0)
          local root = mesh:AddBone({ Name = "Root", CFrame = rootCF, Virtual = false })
          local child = mesh:AddBone({ Name = "Child", ParentId = root, CFrame = childCF, Virtual = false })
          for i, v in vIds do
            if i == 3 then
              mesh:SetVertexBones(v, { root, child }); mesh:SetVertexBoneWeights(v, { 0.25, 0.75 })
            else
              mesh:SetVertexBones(v, { root }); mesh:SetVertexBoneWeights(v, { 1 })
            end
          end
          roblox.exportEditableMesh("${glb}", mesh)
          local back = roblox.importEditableMesh("${glb}")
          local bones = back:GetBones()
          local names = {}
          for _, b in bones do names[back:GetBoneName(b)] = b end
          local childBack = names["Child"]
          local parentOk = back:GetBoneParent(childBack) == names["Root"]
          local cf = back:GetBoneCFrame(childBack)
          local cfOk = (cf.Position - childCF.Position).Magnitude < 1e-3 and (cf.LookVector - childCF.LookVector).Magnitude < 1e-3
          -- the vertex at (0, 4, 0) carried the split weights. Roblox stores
          -- bone weights at 8-bit precision (0.25 reads back as 64/255), so
          -- compare with that tolerance.
          local weightsOk = false
          for _, v in back:GetVertices() do
            if (back:GetPosition(v) - Vector3.new(0, 4, 0)).Magnitude < 1e-3 then
              local bs, ws = back:GetVertexBones(v), back:GetVertexBoneWeights(v)
              local byName = {}
              for k, b in bs do byName[back:GetBoneName(b)] = ws[k] end
              weightsOk = byName["Root"] ~= nil and math.abs(byName["Root"] - 0.25) < 1 / 128 and byName["Child"] ~= nil and math.abs(byName["Child"] - 0.75) < 1 / 128
            end
          end
          mesh:Destroy(); back:Destroy()
          return { bones = #bones, parentOk = parentOk, cfOk = cfOk, weightsOk = weightsOk }`,
      });
      expect(result.ok).toBe(true);
      expect(result.return).toEqual({ bones: 2, parentOk: true, cfOk: true, weightsOk: true });
    } finally {
      rmrf(glb);
    }
  });

  it("meshes: import of a missing file errors with the path", async () => {
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.importEditableMesh("./rodeo-no-such-mesh-12345.glb")`,
    });
    expect(result.ok).toBe(false);
    expect(result.output).toContain("rodeo-no-such-mesh-12345.glb");
  });

  it("meshes: export to an unsupported extension errors naming the supported ones", async () => {
    const out = meshOut("stl", "stl");
    try {
      const result = await run({
        source: `local roblox = require("@rodeo/roblox")
          ${TETRA_LUAU}
          roblox.exportEditableMesh("${out}", mesh)`,
      });
      expect(result.ok).toBe(false);
      expect(result.output).toContain(".glb");
      expect(result.output).toContain(".obj");
      expect(existsSync(out)).toBe(false);
    } finally {
      rmrf(out);
    }
  });

  it("meshes: OBJ export and import keep geometry, UV seams and hard normals, and report dropped colors", async () => {
    const obj = meshOut("mesh", "obj");
    try {
      const result = await run({
        showReturn: true,
        source: `local roblox = require("@rodeo/roblox")
          ${TETRA_LUAU}
          local dropped = roblox.exportEditableMesh("${obj}", mesh)
          local back = roblox.importEditableMesh("${obj}")
          -- The engine hands every face default color ids even when none were
          -- set, so "colors dropped" means the source's four distinct face
          -- colors collapse to one default, not that the ids are gone.
          local function distinctColors(m)
            local seen = {}
            for _, f in m:GetFaces() do
              for _, id in m:GetFaceColors(f) do
                seen[tostring(m:GetColor(id)) .. "@" .. tostring(m:GetColorAlpha(id))] = true
              end
            end
            local n = 0
            for _ in seen do n += 1 end
            return n
          end
          local srcColors, backColors = distinctColors(mesh), distinctColors(back)
          -- Match corners by their face's position sum and their own position,
          -- independent of face/vertex ids and cyclic corner order.
          local expectedUVs = {}
          for _, f in mesh:GetFaces() do
            local vs, us = mesh:GetFaceVertices(f), mesh:GetFaceUVs(f)
            local sum = mesh:GetPosition(vs[1]) + mesh:GetPosition(vs[2]) + mesh:GetPosition(vs[3])
            for k, v in vs do
              expectedUVs[tostring(sum) .. ":" .. tostring(mesh:GetPosition(v))] = mesh:GetUV(us[k])
            end
          end
          local faces = back:GetFaces()
          local corners, normalsOk, uvsOk = 0, true, true
          for _, f in faces do
            local vs = back:GetFaceVertices(f)
            local ns, us = back:GetFaceNormals(f), back:GetFaceUVs(f)
            for k = 1, 3 do
              corners += 1
              local a, b, c = back:GetPosition(vs[1]), back:GetPosition(vs[2]), back:GetPosition(vs[3])
              local geo = (b - a):Cross(c - a).Unit
              local n = back:GetNormal(ns[k])
              if not n or (n - geo).Magnitude > 1e-3 then normalsOk = false end
              local expected = expectedUVs[tostring(a + b + c) .. ":" .. tostring(back:GetPosition(vs[k]))]
              if (back:GetUV(us[k]) - expected).Magnitude > 1e-5 then uvsOk = false end
            end
          end
          local part = AssetService:CreateMeshPartAsync(Content.fromObject(back), { CollisionFidelity = Enum.CollisionFidelity.Box })
          local sizeOk = (part.Size - Vector3.new(4, 4, 4)).Magnitude < 1e-3
          part:Destroy(); mesh:Destroy(); back:Destroy()
          return { dropped = dropped, faces = #faces, corners = corners, normalsOk = normalsOk, uvsOk = uvsOk, srcColors = srcColors, backColors = backColors, sizeOk = sizeOk }`,
      });
      expect(result.ok).toBe(true);
      const r = result.return as Record<string, unknown>;
      expect(r.dropped).toEqual(["vertex colors"]);
      expect(r.faces).toBe(4);
      expect(r.corners).toBe(12);
      expect(r.normalsOk).toBe(true);
      expect(r.uvsOk).toBe(true);
      expect(r.srcColors).toBe(8);
      expect(r.backColors).toBeLessThanOrEqual(1);
      expect(r.sizeOk).toBe(true);
      const text = readFileSync(obj, "utf8");
      expect(text.startsWith("# rodeo")).toBe(true);
      expect(text).toContain("\nf ");
    } finally {
      rmrf(obj);
    }
  });

  it("meshes: OBJ imports independent UV/normal indices at a shared position", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local mesh = roblox.importEditableMesh("./tests/fixtures/pkg/obj/seams.obj")
        local found = {}
        for _, f in mesh:GetFaces() do
          local vs, us, ns = mesh:GetFaceVertices(f), mesh:GetFaceUVs(f), mesh:GetFaceNormals(f)
          for k, v in vs do
            if mesh:GetPosition(v) == Vector3.zero then
              local uv, n = mesh:GetUV(us[k]), mesh:GetNormal(ns[k])
              table.insert(found, { u = uv.X, v = uv.Y, nx = n.X, nz = n.Z })
            end
          end
        end
        mesh:Destroy()
        return found`,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toEqual(expect.arrayContaining([
      { u: 0, v: 1, nx: 0, nz: 1 },
      { u: 0.5, v: 0.5, nx: 1, nz: 0 },
    ]));
    expect((result.return as unknown[]).length).toBe(2);
  });

  it("meshes: OBJ concave polygons preserve their area and winding", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local mesh = roblox.importEditableMesh("./tests/fixtures/pkg/obj/concave.obj")
        local area, reversed, attributesOk = 0, 0, true
        for _, f in mesh:GetFaces() do
          local vs, us, ns = mesh:GetFaceVertices(f), mesh:GetFaceUVs(f), mesh:GetFaceNormals(f)
          local a, b, c = mesh:GetPosition(vs[1]), mesh:GetPosition(vs[2]), mesh:GetPosition(vs[3])
          local cross = (b - a):Cross(c - a)
          area += cross.Magnitude / 2
          if cross.Z <= 0 then reversed += 1 end
          for k, v in vs do
            local p = mesh:GetPosition(v)
            attributesOk = attributesOk
              and (mesh:GetUV(us[k]) - Vector2.new(p.X, 1 - p.Y)).Magnitude < 1e-5
              and (mesh:GetNormal(ns[k]) - Vector3.zAxis).Magnitude < 1e-5
          end
        end
        local faces = #mesh:GetFaces()
        mesh:Destroy()
        return { area = area, reversed = reversed, faces = faces, attributesOk = attributesOk }`,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toEqual({ area: 7, reversed: 0, faces: 6, attributesOk: true });
  });

  it("meshes: OBJ rejects non-finite coordinates before building the mesh", async () => {
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.importEditableMesh("./tests/fixtures/pkg/obj/nonfinite.obj")`,
    });
    expect(result.ok).toBe(false);
    expect(result.output).toContain("nonfinite.obj");
    expect(result.output).toContain("line 2");
    expect(result.output).toContain("must be finite");
  });
}

// ── roblox (9 tests, plugin-only) ─────────────────────────────────────────

export function roblox(run: RunFn): void {
  it("roblox: import can parent instances to workspace", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local instances = roblox.importInstances("./tests/fixtures/pkg/test-folder.rbxm")
        instances[1].Parent = workspace
        local found = workspace:FindFirstChild(instances[1].Name) ~= nil
        instances[1]:Destroy()
        return found`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("true");
  });

  it("roblox: import returns instances from rbxm", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local instances = roblox.importInstances("./tests/fixtures/pkg/test-folder.rbxm")
        return { count = #instances, class = instances[1].ClassName }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"count":1');
    expect(result.output).toContain('"class":"Folder"');
  });

  it("roblox: export + import round-trips instance class+name", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local fs = require("@rodeo/fs")
        local folder = Instance.new("Folder")
        folder.Name = "RodeoTestExport"
        local part = Instance.new("Part")
        part.Name = "ChildPart"
        part.Parent = folder

        local outPath = "rodeo-test-export.rbxm"
        roblox.exportInstances(outPath, { folder })

        local imported = roblox.importInstances(outPath)
        fs.remove(outPath)

        return {
          count = #imported,
          name = imported[1].Name,
          class = imported[1].ClassName,
          childName = imported[1]:FindFirstChild("ChildPart") and imported[1].ChildPart.Name or "missing",
          childClass = imported[1]:FindFirstChild("ChildPart") and imported[1].ChildPart.ClassName or "missing",
        }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"count":1');
    expect(result.output).toContain('"name":"RodeoTestExport"');
    expect(result.output).toContain('"class":"Folder"');
    expect(result.output).toContain('"childName":"ChildPart"');
    expect(result.output).toContain('"childClass":"Part"');
  });

  it("roblox: deprecated import/export/capture aliases still work and warn once", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local a = roblox.import("./tests/fixtures/pkg/test-folder.rbxm")
        local b = roblox.import("./tests/fixtures/pkg/test-folder.rbxm")
        return { first = a[1].ClassName, second = b[1].ClassName }`,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toEqual({ first: "Folder", second: "Folder" });
    const warnings = result.output.split("\n").filter((l) => l.includes("roblox.import is deprecated"));
    expect(warnings.length).toBe(1);
    expect(result.output).toContain("roblox.importInstances");
  });

  it("roblox: import nonexistent file errors", async () => {
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.importInstances("./nonexistent-file-12345.rbxm")`,
    });
    expect(result.ok).toBe(false);
  });

  // bake writes host-side, so the assertions read the emitted file directly
  // rather than round-tripping it through the return value. Absolute paths
  // keep the script's fs cwd and the test process in agreement.
  it("roblox: bake emits Roblox types as constructors", async () => {
    const out = "/tmp/rodeo-test-bake-types.luau";
    rmrf(out);
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.bake("${out}", {
          v = Vector3.new(1, 2, 3),
          c = Color3.new(1, 0, 0),
          u = UDim2.new(0, 4, 1, 8),
          n = 5,
          s = "hi",
          b = true,
        })
        return true`,
    });
    expect(result.ok).toBe(true);
    const src = readFileSync(out, "utf8");
    expect(src).toContain("vector.create(1, 2, 3)");
    expect(src).toContain("Color3.new(1, 0, 0)");
    expect(src).toContain("UDim2.new(0, 4, 1, 8)");
    expect(src).toContain('["n"] = 5,');
    expect(src).toContain('["s"] = "hi",');
    expect(src).toContain('["b"] = true,');
    rmrf(out);
  });

  // The enum must be emitted UNQUOTED — a quoted "Enum.Material.Plastic" is a
  // lookalike string that silently fails to round-trip as an EnumItem.
  it("roblox: bake emits enums unquoted", async () => {
    const out = "/tmp/rodeo-test-bake-enum.luau";
    rmrf(out);
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.bake("${out}", { mat = Enum.Material.Plastic })
        return true`,
    });
    expect(result.ok).toBe(true);
    const src = readFileSync(out, "utf8");
    expect(src).toContain('["mat"] = Enum.Material.Plastic,');
    expect(src).not.toContain('"Enum.Material.Plastic"');
    rmrf(out);
  });

  it("roblox: bake writes non-table values", async () => {
    const cases: Array<[string, string, string]> = [
      ["number", "42", "return 42\n"],
      ["string", '"hello"', 'return "hello"\n'],
      ["boolean", "true", "return true\n"],
      ["vector", "Vector3.new(1, 2, 3)", "return vector.create(1, 2, 3)\n"],
    ];
    for (const [label, literal, expected] of cases) {
      const out = `/tmp/rodeo-test-bake-scalar-${label}.luau`;
      rmrf(out);
      const result = await run({
        source: `local roblox = require("@rodeo/roblox")
          roblox.bake("${out}", ${literal})
          return true`,
      });
      expect(result.ok, `bake(${literal}) failed`).toBe(true);
      expect(readFileSync(out, "utf8"), `bake(${literal})`).toBe(expected);
      rmrf(out);
    }
  });

  it("roblox: bake creates parent directories", async () => {
    const dir = "/tmp/rodeo-test-bake-nested";
    rmrf(dir);
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        roblox.bake("${dir}/deeper/data.luau", { ok = true })
        return true`,
    });
    expect(result.ok).toBe(true);
    expect(readFileSync(`${dir}/deeper/data.luau`, "utf8")).toContain('["ok"] = true,');
    rmrf(dir);
  });

  it("roblox: bake stringifies instances", async () => {
    const out = "/tmp/rodeo-test-bake-instance.luau";
    rmrf(out);
    const result = await run({
      source: `local roblox = require("@rodeo/roblox")
        local folder = Instance.new("Folder")
        folder.Name = "BakeMe"
        roblox.bake("${out}", { inst = folder })
        return true`,
    });
    expect(result.ok).toBe(true);
    expect(readFileSync(out, "utf8")).toContain('["inst"] = "BakeMe",');
    rmrf(out);
  });

  it("roblox: import returns instances from rbxmx", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local instances = roblox.importInstances("./tests/fixtures/pkg/test-folder.rbxmx")
        return { count = #instances, class = instances[1].ClassName }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"count":1');
    expect(result.output).toContain('"class":"Folder"');
  });

  it("roblox: export with .rbxmx extension writes XML format", async () => {
    const result = await run({
      showReturn: true,
      source: `local fs = require("@rodeo/fs")
        local stream = require("@rodeo/stream")
        local roblox = require("@rodeo/roblox")

        local folder = Instance.new("Folder")
        folder.Name = "XmlExportTest"

        local path = "rodeo-test-xml-export.rbxmx"
        roblox.exportInstances(path, { folder })

        local r = fs.open(path, "r")
        local content = stream.read(r)
        stream.close(r)
        fs.remove(path)

        local head = content:sub(1, 7)
        -- rbx-xml emits "<roblox version=..." (text); binary .rbxm starts with
        -- "<roblox!\\x89\\xff..." (non-text after the literal "<roblox").
        return { head = head, isXml = head == "<roblox" and content:sub(8, 8) ~= "!" }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"isXml":true');
  });

  it("roblox: export to .rbxmx + import round-trips structure", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local fs = require("@rodeo/fs")

        local folder = Instance.new("Folder")
        folder.Name = "XmlRoundtrip"
        local part = Instance.new("Part")
        part.Name = "XmlChild"
        part.Parent = folder

        local path = "rodeo-test-xml-roundtrip.rbxmx"
        roblox.exportInstances(path, { folder })

        local imported = roblox.importInstances(path)
        fs.remove(path)

        return {
          count = #imported,
          name = imported[1].Name,
          class = imported[1].ClassName,
          childName = imported[1]:FindFirstChild("XmlChild") and imported[1].XmlChild.Name or "missing",
          childClass = imported[1]:FindFirstChild("XmlChild") and imported[1].XmlChild.ClassName or "missing",
        }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"count":1');
    expect(result.output).toContain('"name":"XmlRoundtrip"');
    expect(result.output).toContain('"class":"Folder"');
    expect(result.output).toContain('"childName":"XmlChild"');
    expect(result.output).toContain('"childClass":"Part"');
  });

  it("roblox: export creates nested parent directories", async () => {
    const result = await run({
      showReturn: true,
      source: `local roblox = require("@rodeo/roblox")
        local fs = require("@rodeo/fs")

        local dir = "rodeo-test-nested-" .. tostring(math.random(1, 1e9))
        local path = dir .. "/sub/leaf/snapshot.rbxm"

        local function cleanup()
            if fs.exists(path) then fs.remove(path) end
            if fs.exists(dir .. "/sub/leaf") then fs.rmdir(dir .. "/sub/leaf") end
            if fs.exists(dir .. "/sub") then fs.rmdir(dir .. "/sub") end
            if fs.exists(dir) then fs.rmdir(dir) end
        end

        -- Defensive: clear leftover state from a prior crashed run.
        cleanup()

        local folder = Instance.new("Folder")
        folder.Name = "NestedDirsTest"
        roblox.exportInstances(path, { folder })

        local existed = fs.exists(path)

        cleanup()

        return { existed = existed }`,
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain('"existed":true');
  });
}
