// Shared execution test cases — port of tests/utils/executionTests.luau.
// Each module is a function taking a `run(opts)` closure that executes against
// some DOM (caller chooses: edit:plugin, test:server, run:server, etc). Mode
// transitions are driven by opts.target on every runCode dispatch, so factories
// never need to set Studio mode explicitly.

import { it, expect } from "bun:test";
import { execSync } from "node:child_process";
import { existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { RodeoClient } from "../../rodeo-client-ts/src/index.js";
import type { RunCodeOpts, RunResult } from "../../rodeo-client-ts/src/run.js";

export type RunFn = (opts: RunCodeOpts) => Promise<RunResult>;

// ── inlineSource (4 tests) ───────────────────────────────────────────────

export function inlineSource(run: RunFn): void {
  it("inline source returns value", async () => {
    const result = await run({ source: "return 42" });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(42);
  });

  it("inline source returns table", async () => {
    const result = await run({ source: 'return {a=1, b="hello"}' });
    expect(result.ok).toBe(true);
    expect(result.return).toEqual({ a: 1, b: "hello" });
  });

  // Issue #16: inline runs used to be named after their full source, and
  // Instance.Name caps at 200,000 chars, so anything larger failed with
  // "Unable to assign property Name" before its first line ran.
  it("inline source larger than the Instance.Name limit runs", async () => {
    const payload = "x".repeat(225_000);
    const result = await run({ source: `local payload = [[${payload}]]\nreturn #payload` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(payload.length);
  });

  // Issue #15 (second half): a self-referential return value must come back
  // with a cycle marker, not blow the stack inside normalize.
  it("cyclic return value is marked, not a stack overflow", async () => {
    const result = await run({ source: "local t = { name = 'root' }\nt.self = t\nreturn t" });
    expect(result.ok).toBe(true);
    expect(result.return).toEqual({ name: "root", self: "*** cycle ***" });
  });

  it("inline source captures print output", async () => {
    const result = await run({ source: "print('hello from inline')\nreturn nil" });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("hello from inline");
  });

  it("inline source error is not ok", async () => {
    const result = await run({ source: "error('boom')" });
    expect(result.ok).toBe(false);
  });
}

// ── largeSource ──────────────────────────────────────────────────────────

export function largeSource(run: RunFn): void {
  it("executes source larger than the Source property assignment limit", async () => {
    const path = mkTmp(".luau");
    const payload = "x".repeat(225_000);
    try {
      writeFileSync(path, `local payload = [[${payload}]]\nreturn #payload`);
      const result = await run({ file: path });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(payload.length);
    } finally {
      rmIfExists(path);
    }
  });
}

// ── ensureReturn (4 tests) ───────────────────────────────────────────────

export function ensureReturn(run: RunFn): void {
  it("script without return succeeds", async () => {
    const result = await run({ source: "print('no return')" });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("no return");
  });

  it("script with return still works", async () => {
    const result = await run({ source: "return 42" });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(42);
  });

  it("script with only assignments succeeds", async () => {
    const result = await run({ source: "local x = 1\nlocal y = x + 1" });
    expect(result.ok).toBe(true);
  });

  it("script with error still reports error", async () => {
    const result = await run({ source: "error('boom')" });
    expect(result.ok).toBe(false);
  });
}

// ── errorHandling (3 tests) ──────────────────────────────────────────────

export function errorHandling(run: RunFn): void {
  it("error is not ok", async () => {
    const result = await run({ source: "error('intentional failure')" });
    expect(result.ok).toBe(false);
    expect(result.exitCode).toBe(1);
  });

  it("missing script file is not ok", async () => {
    // Lute equivalent: `rodeo run rodeo-nonexistent-script-12345.luau`.
    // TS path: processSource throws synchronously. CLI path: `rodeo run` exits
    // non-zero and returns { ok: false }. Accept either.
    let failed = false;
    try {
      const result = await run({ file: "rodeo-nonexistent-script-12345.luau" });
      failed = !result.ok;
    } catch {
      failed = true;
    }
    expect(failed).toBe(true);
  });

  it("unconnected port is not ok", async () => {
    // Lute equivalent: `rodeo run --source "return 1" --port 59999`
    // Connect with a short deadline to an unused port; connect should throw.
    let connected = true;
    try {
      const client = await RodeoClient.connect("http://localhost:59999", { readyTimeoutMs: 1000 });
      await client.close();
    } catch {
      connected = false;
    }
    expect(connected).toBe(false);
  });
}

// ── outputFlags (3 tests) ────────────────────────────────────────────────
// Mapping from CLI flags to LogFilter per rodeo-cli/src/commands/process_source/execution.rs.

export function outputFlags(run: RunFn): void {
  it("no output suppresses print", async () => {
    const result = await run({
      source: "print('should be hidden')\nreturn nil",
      logFilter: {
        enableWarn: false,
        enableError: false,
        enableInfo: false,
        enableOutput: false,
        enableLogs: false,
      },
    });
    expect(result.ok).toBe(true);
    expect(result.output).not.toContain("should be hidden");
  });

  it("no print suppresses print but allows warn", async () => {
    const result = await run({
      source: "print('hidden') warn('visible')\nreturn nil",
      logFilter: { enableOutput: false },
    });
    expect(result.ok).toBe(true);
    expect(result.output).not.toContain("hidden");
    expect(result.output).toContain("visible");
  });

  it("no warn suppresses warn but allows print", async () => {
    const result = await run({
      source: "print('visible') warn('hidden')\nreturn nil",
      logFilter: { enableWarn: false },
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("visible");
    expect(result.output).not.toContain("hidden");
  });
}

// ── returnFile (2 tests) ─────────────────────────────────────────────────
// Tests below exercise the explicit `returnFile` path (disk side-effect via the
// plugin). For the reload-from-file roundtrip case we read the script's return
// value straight from `result.return` (now in-memory on the wire).

// User-facing Luau source returning one value of each of the eight Roblox
// types that rodeo-shared/normalize.luau + serialize.luau know how to
// round-trip. Used by the returnFile suite below to exercise every tagged
// emitter at once. Numeric literals are kept simple and distinct so the
// emitted output is unambiguous to substring-assert against.
export const ALL_ROBLOX_TYPES_SOURCE = `return {
  vec3 = Vector3.new(1, 2, 3),
  vec2 = Vector2.new(4, 5),
  cf   = CFrame.new(0, 0, 0),
  c3   = Color3.new(1, 0, 0),
  ud   = UDim.new(0.5, 10),
  ud2  = UDim2.new(0, 100, 0, 50),
  nr   = NumberRange.new(1, 10),
  rect = Rect.new(0, 0, 100, 100),
}`;

function mkTmp(ext: string): string {
  return join(tmpdir(), `rodeo-rt-${randomUUID()}${ext}`);
}

function rmIfExists(path: string): void {
  try { unlinkSync(path); } catch {}
}

// ── returnValueCap ───────────────────────────────────────────────────────
// The in-wire return value (ExecutionDone.return_value) is capped at 2MiB —
// it rides the done message, a single unchunkable hop with a hard transport
// limit (an oversized one used to kill the backend↔master stream). Return
// files and --show-return stdout are chunked and size-unbounded. Sizes hug
// the cap (±64KiB) so each test moves ~2MB, not transport-killing payloads.
//
// NOTE: requires a run fn whose result.return reflects the WIRE field (the
// API helper). The CLI helper shadows returns via a temp --return file, so
// the CLI variant of these tests lives in cli/operations/returnCap.test.ts
// as direct runRodeo invocations.
export function returnValueCap(run: RunFn): void {
  const CAP = 2 * 1024 * 1024;

  it("under-cap return value rides the wire", async () => {
    const n = CAP - 65536;
    const result = await run({ source: `return string.rep("a", ${n})` });
    expect(result.ok).toBe(true);
    expect(typeof result.return).toBe("string");
    expect((result.return as string).length).toBe(n);
  });

  it("over-cap return value fails with an actionable error", async () => {
    const n = CAP + 65536;
    const result = await run({ source: `return string.rep("a", ${n})` });
    expect(result.ok).toBe(false);
    expect(result.return).toBeUndefined();
    expect(result.output).toContain("return value too large");
    expect(result.output).toContain("--return");
  });

  it("over-cap return value succeeds through a return file", async () => {
    const n = CAP + 65536;
    const path = mkTmp(".json");
    try {
      const result = await run({
        source: `return string.rep("a", ${n})`,
        returnFile: path,
      });
      expect(result.ok).toBe(true);
      // JSON of an all-ASCII string is the string plus two quotes.
      expect(readFileSync(path, "utf-8").length).toBe(n + 2);
    } finally {
      rmIfExists(path);
    }
  });

  it("over-cap with showReturn prints the value and omits result.return", async () => {
    const n = CAP + 65536;
    const result = await run({
      source: `return "S__" .. string.rep("a", ${n}) .. "__E"`,
      showReturn: true,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBeUndefined();
    expect(result.output).toContain("S__");
    expect(result.output).toContain("__E");
    expect(result.output).toContain("omitted from result.return");
  });
}

export function returnFile(run: RunFn): void {
  it("writes plain table to .luau", async () => {
    const path = mkTmp(".luau");
    try {
      const result = await run({
        source: 'return { value = 42, name = "test" }',
        returnFile: path,
      });
      expect(result.ok).toBe(true);
      expect(existsSync(path)).toBe(true);
      const content = readFileSync(path, "utf-8");
      expect(content.startsWith("return {")).toBe(true);
      expect(content).toContain('["name"] = "test"');
      expect(content).toContain('["value"] = 42');
    } finally {
      rmIfExists(path);
    }
  });

  it("writes plain table to .json", async () => {
    const path = mkTmp(".json");
    try {
      const result = await run({
        source: 'return { value = 42, name = "test" }',
        returnFile: path,
      });
      expect(result.ok).toBe(true);
      const parsed = JSON.parse(readFileSync(path, "utf-8"));
      expect(parsed).toEqual({ value: 42, name: "test" });
    } finally {
      rmIfExists(path);
    }
  });

  it("writes Roblox types to .luau as constructor calls", async () => {
    const path = mkTmp(".luau");
    try {
      const result = await run({ source: ALL_ROBLOX_TYPES_SOURCE, returnFile: path });
      expect(result.ok).toBe(true);
      const content = readFileSync(path, "utf-8");
      // Vector3 emits via `vector.create()` (Luau native vector ctor),
      // Vector2 keeps the Roblox `Vector2.new()` form. See
      // rodeo-shared/serialize.luau TYPE_EMITTERS.
      expect(content).toContain("vector.create(1, 2, 3)");
      expect(content).toContain("Vector2.new(4, 5)");
      expect(content).toContain("CFrame.new(");
      expect(content).toContain("Color3.new(1, 0, 0)");
      expect(content).toContain("UDim.new(0.5, 10)");
      expect(content).toContain("UDim2.new(0, 100, 0, 50)");
      expect(content).toContain("NumberRange.new(1, 10)");
      expect(content).toContain("Rect.new(0, 0, 100, 100)");
    } finally {
      rmIfExists(path);
    }
  });

  it("writes Roblox types to .json as tagged structs", async () => {
    const path = mkTmp(".json");
    try {
      const result = await run({ source: ALL_ROBLOX_TYPES_SOURCE, returnFile: path });
      expect(result.ok).toBe(true);
      const parsed = JSON.parse(readFileSync(path, "utf-8"));
      expect(parsed.vec3).toEqual({ type: "Vector3", value: [1, 2, 3] });
      expect(parsed.vec2).toEqual({ type: "Vector2", value: [4, 5] });
      expect(parsed.cf.type).toBe("CFrame");
      expect(parsed.cf.value).toHaveLength(12);
      expect(parsed.c3).toEqual({ type: "Color3", value: [1, 0, 0] });
      expect(parsed.ud).toEqual({ type: "UDim", value: [0.5, 10] });
      expect(parsed.ud2).toEqual({ type: "UDim2", value: [0, 100, 0, 50] });
      expect(parsed.nr).toEqual({ type: "NumberRange", value: [1, 10] });
      expect(parsed.rect).toEqual({ type: "Rect", value: [0, 0, 100, 100] });
    } finally {
      rmIfExists(path);
    }
  });

  it("round-trips Roblox types through .luau via require", async () => {
    // Project-relative path so darklua's bundle pass can resolve the
    // require. The require resolves at bundle time — Studio just sees the
    // inlined content. A mis-emitted constructor call surfaces here as a
    // parse error or wrong tagged-struct shape after normalize-on-the-way-back.
    const relPathNoExt = `.rodeo/.temp/rodeo-rt-${randomUUID()}`;
    const relPath = `${relPathNoExt}.luau`;
    try {
      const writeResult = await run({ source: ALL_ROBLOX_TYPES_SOURCE, returnFile: relPath });
      expect(writeResult.ok).toBe(true);

      const reloaded = await run({
        source: `return require("./${relPathNoExt}")`,
      });
      expect(reloaded.ok).toBe(true);
      const parsed = reloaded.return as Record<string, { type: string; value: unknown }>;
      expect(parsed.vec3.type).toBe("Vector3");
      expect(parsed.vec3.value).toEqual([1, 2, 3]);
      expect(parsed.cf.type).toBe("CFrame");
      expect(parsed.c3).toEqual({ type: "Color3", value: [1, 0, 0] });
    } finally {
      rmIfExists(relPath);
    }
  });

  // Regression: integer-keyed (non-array-like) maps must keep BOTH the numeric
  // key and its value. The serializer stringified keys (`tostring(k)`) then
  // re-indexed the table with the string key, so values stored under an integer
  // key were lost (emitted as `["100"] = nil`); Luau supports `[100] = ...`.
  // See rodeo-shared/serialize.luau (getSortedKeys / walk).
  it("preserves a numeric map key and its Vector3 value (.luau)", async () => {
    const path = mkTmp(".luau");
    try {
      const result = await run({
        source: "return { [100] = Vector3.new(13, 14, 15) }",
        returnFile: path,
      });
      expect(result.ok).toBe(true);
      const content = readFileSync(path, "utf-8");
      expect(content).toContain("[100] = vector.create(13, 14, 15)");
      expect(content).not.toContain('["100"] = nil');
    } finally {
      rmIfExists(path);
    }
  });

  it("preserves a large/sparse numeric map key and its value (.luau)", async () => {
    const path = mkTmp(".luau");
    try {
      const result = await run({
        source: "return { [1234005] = Vector3.new(1, 2, 3) }",
        returnFile: path,
      });
      expect(result.ok).toBe(true);
      const content = readFileSync(path, "utf-8");
      expect(content).toContain("[1234005] = vector.create(1, 2, 3)");
      expect(content).not.toContain('["1234005"]');
    } finally {
      rmIfExists(path);
    }
  });

  it("preserves both integer and string keys in a mixed map (.luau)", async () => {
    const path = mkTmp(".luau");
    try {
      const result = await run({
        source: "return { [999] = Vector3.new(1, 2, 3), bare = Vector3.new(7, 8, 9) }",
        returnFile: path,
      });
      expect(result.ok).toBe(true);
      const content = readFileSync(path, "utf-8");
      expect(content).toContain("[999] = vector.create(1, 2, 3)");
      expect(content).toContain('["bare"] = vector.create(7, 8, 9)');
      expect(content).not.toContain('["999"] = nil');
    } finally {
      rmIfExists(path);
    }
  });
}

// ── scriptFile (2 tests) ─────────────────────────────────────────────────

export function scriptFile(run: RunFn): void {
  it("runs luau file and captures output", async () => {
    const path = "rodeo-test-script-tmp.luau";
    writeFileSync(path, "print('from file')\nreturn 'ok'");
    try {
      const result = await run({ file: path });
      expect(result.ok).toBe(true);
      expect(result.output).toContain("from file");
      expect(result.return).toBe("ok");
    } finally {
      unlinkSync(path);
    }
  });

  it("directive enables show return", async () => {
    // Relies on __process_source honoring `--@rodeo run --show-return`
    // header directive so show-return is auto-applied. The return value
    // also rides on the wire, so we assert against `.return` directly.
    const path = "rodeo-test-directive-tmp.luau";
    writeFileSync(path, "--@rodeo run --show-return\nreturn 'directive works'");
    try {
      const result = await run({ file: path });
      expect(result.ok).toBe(true);
      expect(result.return).toBe("directive works");
    } finally {
      unlinkSync(path);
    }
  });
}

// ── targetIdentity (8 tests) ─────────────────────────────────────────────
// Mirrors executionTests.luau targetIdentity — elevated tests first (require
// pure edit mode so MCP's execute_luau routes to the edit DataModel).

export function targetIdentity(run: RunFn): void {
  it("edit:elevated can access DebuggerManager", async () => {
    const result = await run({
      context: "elevated",
      source: "return tostring(DebuggerManager())",
    });
    expect(result.ok).toBe(true);
    expect(String(result.return)).toContain("DebuggerManager");
  });

  it("edit:elevated can use @rodeo/fs", async () => {
    const result = await run({
      context: "elevated",
      source: 'return require("@rodeo/fs").exists(".")',
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("edit:elevated can use @rodeo/process", async () => {
    // `echo` is a real program on Unix but a cmd.exe builtin on Windows (there
    // is no echo.exe), so route it through `cmd /c` there. @rodeo/process.run
    // execs on the host, so the host platform is what matters.
    const argv =
      process.platform === "win32"
        ? '{"cmd", "/c", "echo", "hi"}'
        : '{"echo", "hi"}';
    const result = await run({
      context: "elevated",
      source: `local r = require("@rodeo/process").run(${argv}) return r.ok`,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("edit:elevated propagates errors", async () => {
    const result = await run({ context: "elevated", source: 'error("boom")' });
    expect(result.ok).toBe(false);
  });

  // cmdbar — command-bar identity through the BindableFunction bridge the
  // launch bootstrap installs (no StudioMCP). The harness launches its Studio
  // through rodeo, so the bridge is present.
  it("edit:cmdbar runs at command-bar identity (4)", async () => {
    const result = await run({
      context: "cmdbar",
      source: 'printidentity("identity:") return true',
    });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("identity: 4");
  });

  it("edit:cmdbar can access DebuggerManager", async () => {
    const result = await run({
      context: "cmdbar",
      source: "return tostring(DebuggerManager())",
    });
    expect(result.ok).toBe(true);
    expect(String(result.return)).toContain("DebuggerManager");
  });

  it("edit:cmdbar can use @rodeo/fs", async () => {
    const result = await run({
      context: "cmdbar",
      source: 'return require("@rodeo/fs").exists(".")',
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("edit:cmdbar returns values like the plugin context", async () => {
    // The bridge hands the raw value back through _G (shared VM), so nested
    // tables and arrays survive without a JSON detour.
    const result = await run({
      context: "cmdbar",
      source: 'return { n = 1, s = "two", list = { 1, 2, 3 }, nested = { ok = true } }',
    });
    expect(result.ok).toBe(true);
    expect(result.return).toEqual({ n: 1, s: "two", list: [1, 2, 3], nested: { ok: true } });
  });

  it("edit:cmdbar propagates errors", async () => {
    const result = await run({ context: "cmdbar", source: 'error("boom")' });
    expect(result.ok).toBe(false);
  });

  it("plugin context stays at identity 5 (cmdbar is opt-in)", async () => {
    const result = await run({ source: 'printidentity("identity:") return true' });
    expect(result.ok).toBe(true);
    expect(result.output).toContain("identity: 5");
  });

  it("plugin identity works in edit mode (no target)", async () => {
    const result = await run({
      source: "return typeof(game) == 'Instance'",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("run:server can access ServerStorage", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('ServerStorage') ~= nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("test:client can access LocalPlayer", async () => {
    const result = await run({
      mode: "test",
      context: "client",
      source: "return game:GetService('Players').LocalPlayer ~= nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("run:server cannot access LocalPlayer", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('Players').LocalPlayer == nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });
}

// ── reloadRequires (4 tests) ──────────────────────────────────────────────

// The fixture place's own scripts set `globalState.value = "mutated"` at
// runtime, so this module distinguishes the two require polarities exactly:
// the cached (default) path resolves to the live module the game mutated,
// while --reload-requires clones the require tree and yields a fresh copy
// still holding its initial "original".
// Both run:server cases run first on purpose: the test:client cases need a
// run→play transition, which hangs for 60s on Studio versions where the
// session never reaches play mode (a requires-free script reproduces it
// identically). Ordering keeps that from poisoning the run-mode assertions.
export function reloadRequires(run: RunFn): void {
  const requireGlobalState = "return require(game.ReplicatedStorage.globalState).value";

  it("run:server requires resolve to the game's live module by default", async () => {
    const result = await run({ mode: "run", context: "server", source: requireGlobalState });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("mutated");
  });

  it("run:server --reload-requires gets a fresh copy, not the live module", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      reloadRequires: true,
      source: requireGlobalState,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("test:client requires resolve to the game's live module by default", async () => {
    const result = await run({ mode: "test", context: "client", source: requireGlobalState });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("mutated");
  });

  it("test:client --reload-requires gets a fresh copy, not the live module", async () => {
    const result = await run({
      mode: "test",
      context: "client",
      reloadRequires: true,
      source: requireGlobalState,
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });
}

// ── execFiltering (11 tests) ─────────────────────────────────────────────

const SOURCE_DOM = `
    local RS = game:GetService('RunService')
    return {
        studio = RS:IsStudio(),
        server = RS:IsServer(),
        client = RS:IsClient(),
        running = RS:IsRunning()
    }
`;

const SOURCE_PLUGIN = `
    local RS = game:GetService('RunService')
    return {
        edit = RS:IsEdit(),
        studio = RS:IsStudio(),
        server = RS:IsServer(),
        client = RS:IsClient(),
        running = RS:IsRunning()
    }
`;

export function execFiltering(run: RunFn): void {
  // Edit mode tests

  it("edit:plugin targets edit DOM", async () => {
    const result = await run({ context: "plugin", source: SOURCE_PLUGIN });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.edit).toBe(true);
    expect(r.running).toBe(false);
  });

  it("edit:plugin can call IsEdit", async () => {
    const result = await run({
      context: "plugin",
      source: "return game:GetService('RunService'):IsEdit()",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("no target matches any DOM", async () => {
    const result = await run({ source: SOURCE_PLUGIN });
    expect(result.ok).toBe(true);
    expect((result.return as Record<string, boolean>).studio).toBe(true);
  });

  // Run mode tests

  it("run:server targets server DOM in run mode", async () => {
    const result = await run({ mode: "run", context: "server", source: SOURCE_DOM });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.running).toBe(true);
    expect(r.server).toBe(true);
  });

  it("run:server runs as Script (can access ServerStorage)", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('ServerStorage') ~= nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("run:server:plugin runs as ModuleScript on server DOM", async () => {
    const result = await run({ mode: "run", context: "plugin", source: SOURCE_PLUGIN });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.running).toBe(true);
    expect(r.server).toBe(true);
    expect(r).toHaveProperty("edit");
  });

  // Play/test mode tests

  it("test:server targets server DOM in play mode", async () => {
    const result = await run({ mode: "test", context: "server", source: SOURCE_DOM });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.running).toBe(true);
    expect(r.server).toBe(true);
  });

  it("test:server:plugin runs as ModuleScript on server DOM", async () => {
    const result = await run({ mode: "test", context: "plugin", source: SOURCE_PLUGIN });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.running).toBe(true);
    expect(r.server).toBe(true);
    expect(r).toHaveProperty("edit");
  });

  it("test:client targets client DOM in play mode", async () => {
    const result = await run({ mode: "test", context: "client", source: SOURCE_DOM });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.client).toBe(true);
    expect(r.running).toBe(true);
  });

  it("test:client runs as LocalScript (can access LocalPlayer)", async () => {
    const result = await run({
      mode: "test",
      context: "client",
      source: "return game:GetService('Players').LocalPlayer ~= nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("test:client:plugin runs as ModuleScript on client DOM", async () => {
    const result = await run({ mode: "test", domKind: "client", context: "plugin", source: SOURCE_PLUGIN });
    expect(result.ok).toBe(true);
    const r = result.return as Record<string, boolean>;
    expect(r.running).toBe(true);
    expect(r.client).toBe(true);
    expect(r).toHaveProperty("edit");
  });
}

// ── uncachedRequireTraversal (6 tests) ───────────────────────────────────

export function uncachedRequireTraversal(run: RunFn): void {
  // Fresh module state is opt-in: --reload-requires runs the cloning
  // traversal. (Before 1.3.0 this was the default and the flag was
  // --cache-requires for the opposite.)
  const exec = (source: string) =>
    run({
      mode: "run",
      context: "server",
      source,
      verbose: true,
      reloadRequires: true,
    });

  it("leaf require gets fresh state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.leaf).value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("quoted-service require still gets fresh state", async () => {
    // Regression: a require written with a quoted form —
    // game:GetService("ReplicatedStorage") — puts a `"` in the inline source.
    // For inline scripts the module's Name *is* that source, and the resolver
    // embeds the Name into generated Luau (script.Parent["<name>"]) to
    // re-anchor `script`. The unescaped quote breaks that temp resolver module,
    // so the require silently fails to resolve and the module is never cloned
    // for fresh state — leaving this require to hit Roblox's cache ("mutated")
    // instead of the uncached "original". The compile error lands in Studio's
    // output before execution, so it never reaches rodeo; the only observable
    // symptom is the wrong (cached) value here.
    const result = await exec('return require(game:GetService("ReplicatedStorage").leaf).value');
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("sibling require gets fresh state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.mid).value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("sibling transitive dep gets fresh state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.mid).leaf.value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("@self require gets fresh state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.deep).value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("ancestor require gets fresh state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.deep).child.value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });

  it("ancestor transitive dep gets fresh state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.deep).child.leaf.value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("original");
  });
}

// ── cachedRequireTraversal (5 tests) ─────────────────────────────────────

export function cachedRequireTraversal(run: RunFn): void {
  // Cached requires are the default now — no flag: the require resolves to
  // the live module the game already mutated.
  const exec = (source: string) =>
    run({
      mode: "run",
      context: "server",
      source,
      verbose: true,
    });

  it("leaf require sees mutated state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.leaf).value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("mutated");
  });

  it("sibling require sees mutated state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.mid).value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("mutated");
  });

  it("@self require sees mutated state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.deep).value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("mutated");
  });

  it("ancestor transitive dep sees mutated state", async () => {
    const result = await exec("return require(game.ReplicatedStorage.deep).child.leaf.value");
    expect(result.ok).toBe(true);
    expect(result.return).toBe("mutated");
  });

  it("module edited after it was cached comes back fresh, with its requirers", async () => {
    // Issue #10: the cache used to hand back the old module after its Source
    // changed. Writing Source needs plugin identity; the requires run as
    // server Scripts so they use the game's require cache.
    const plugin = (source: string) => run({ mode: "run", context: "plugin", source });

    const setup = await plugin(`
      local folder = Instance.new("Folder"); folder.Name = "EditRepro"; folder.Parent = game:GetService("ReplicatedStorage")
      local inner = Instance.new("ModuleScript"); inner.Name = "Inner"; inner.Source = "return 1"; inner.Parent = folder
      local outer = Instance.new("ModuleScript"); outer.Name = "Outer"; outer.Source = "return require(script.Parent.Inner)"; outer.Parent = folder`);
    expect(setup.ok, setup.output).toBe(true);
    try {
      const cached = await exec(
        'local RS = game:GetService("ReplicatedStorage")\nreturn { inner = require(RS.EditRepro.Inner), outer = require(RS.EditRepro.Outer) }',
      );
      expect(cached.return).toEqual({ inner: 1, outer: 1 });

      const edit = await plugin('game:GetService("ReplicatedStorage").EditRepro.Inner.Source = "return 2"');
      expect(edit.ok, edit.output).toBe(true);

      const literal = await exec("return require(game.ReplicatedStorage.EditRepro.Inner)");
      expect(literal.return).toBe(2);
      // Outer is unchanged but requires Inner, so it reloads too; leaf is
      // untouched by the edit and stays the game's live module.
      const viaLocal = await exec(
        'local RS = game:GetService("ReplicatedStorage")\nreturn { outer = require(RS.EditRepro.Outer), leaf = require(RS.leaf).value }',
      );
      expect(viaLocal.return).toEqual({ outer: 2, leaf: "mutated" });

      // Back to the Source the game cached: the live module again.
      const revert = await plugin('game:GetService("ReplicatedStorage").EditRepro.Inner.Source = "return 1"');
      expect(revert.ok, revert.output).toBe(true);
      const live = await exec("return require(game.ReplicatedStorage.EditRepro.Outer)");
      expect(live.return).toBe(1);
    } finally {
      await plugin('game:GetService("ReplicatedStorage").EditRepro:Destroy()');
    }
  });
}

// ── autoTransition (6 tests) ─────────────────────────────────────────────
// Verifies that passing `--mode` drives the expected studio transition (context
// never transitions on its own). Order of cases matters — they traverse
// edit→run→test→run with specific expectations at each hop.

export function autoTransition(run: RunFn): void {
  it("edit → run:server auto-enters run mode", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('RunService'):IsRunning() and game.Players.LocalPlayer == nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("run → test:client auto-transitions to play mode", async () => {
    const result = await run({
      mode: "test",
      context: "client",
      source: "return game:GetService('Players').LocalPlayer ~= nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("test → run:server auto-transitions back to run mode", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('RunService'):IsRunning() and game:GetService('ServerStorage') ~= nil",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("no transition when already in correct mode", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('RunService'):IsRunning()",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("run → test:server auto-transitions to play mode", async () => {
    const result = await run({
      mode: "test", context: "server",
      source: "return game:GetService('RunService'):IsRunning() and #game:GetService('Players'):GetPlayers() > 0",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe(true);
  });

  it("test → run:server auto-transitions to run mode", async () => {
    const result = await run({
      mode: "run",
      context: "server",
      source: "return game:GetService('RunService'):IsRunning() and #game:GetService('Players'):GetPlayers() == 0 and 'PASS' or 'FAIL'",
    });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("PASS");
  });
}

// ── bundle (12 tests) ────────────────────────────────────────────────────
// Drives scripts through the bundle/require-resolver pipeline using fixture
// files under tests/fixtures/resolve/. The "missing script" case accepts
// either behavior: the api path throws from runCode, the CLI path returns a
// non-ok result — both are valid failure modes.

export function bundle(run: RunFn): void {
  const FIXTURES = "tests/fixtures/resolve";

  it("resolves @rodeo/* requires", async () => {
    const result = await run({ file: `${FIXTURES}/rodeo-alias.luau` });
    expect(result.ok).toBe(true);
    // "resolved" is written to stdout via stream.write, "alias_ok" is the
    // script's return value.
    expect(result.output).toContain("resolved");
    expect(result.return).toBe("alias_ok");
  });

  it("resolves external local dep", async () => {
    const result = await run({ file: `${FIXTURES}/ext-main.luau` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("from_helper");
  });

  it("resolves nested external dep", async () => {
    const result = await run({ file: `${FIXTURES}/nested-main.luau` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("nested_ok");
  });

  it("resolves package with internal requires", async () => {
    const result = await run({ file: `${FIXTURES}/pkg-internal.luau` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("5");
  });

  it("resolves cross-directory require", async () => {
    const result = await run({ file: `${FIXTURES}/cross-dir.luau` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("cross_dir_ok");
  });

  it("resolves deep transitive chain", async () => {
    const result = await run({ file: `${FIXTURES}/transitive.luau` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("transitive_ok");
  });

  it("resolves @lune/* shims", async () => {
    const result = await run({ file: `${FIXTURES}/lune-shim.luau` });
    expect(result.ok).toBe(true);
    // "lune_shim_ok" is written to stdout, "cwd:..." is the return value.
    expect(result.output).toContain("lune_shim_ok");
    expect(String(result.return ?? "")).toContain("cwd:");
  });

  it("resolves mixed @rodeo + relative + @lune requires", async () => {
    const result = await run({ file: `${FIXTURES}/mixed-requires.luau` });
    expect(result.ok).toBe(true);
    // stdout has the assembled "mixed:from_helper:..." line; return is "mixed_ok".
    expect(result.output).toContain("mixed:from_helper:");
    expect(result.return).toBe("mixed_ok");
  });

  it("bundles script with no requires", async () => {
    const result = await run({ file: `${FIXTURES}/no-deps.luau` });
    expect(result.ok).toBe(true);
    expect(result.return).toBe("no_deps_ok");
  });

  it("bundles via @rodeo run directive", async () => {
    const result = await run({ file: `${FIXTURES}/directive-bundle.luau` });
    expect(result.ok).toBe(true);
    // The script writes "directive_ok" to stdout and also returns it.
    expect(result.return).toBe("directive_ok");
  });

  it("excludes in-game deps via sourcemap", async () => {
    const smDir = `${FIXTURES}/sourcemap-test`;
    execSync(`rojo sourcemap ${smDir}/default.project.json -o ${smDir}/sourcemap.json`, { stdio: "inherit" });
    const result = await run({
      file: `${smDir}/main.luau`,
      sourcemap: `${smDir}/sourcemap.json`,
    });
    expect(result.ok).toBe(true);
    const r = String(result.return ?? "");
    expect(r).toContain("sm:");
    expect(r).toContain("from_helper");
  });

  it("fails on missing script", async () => {
    let threw = false;
    let result: RunResult | undefined;
    try {
      result = await run({ file: "nonexistent-script-12345.luau" });
    } catch {
      threw = true;
    }
    // api runCode throws; CLI returns non-ok — accept both.
    expect(threw || (result !== undefined && !result.ok)).toBe(true);
  });
}
