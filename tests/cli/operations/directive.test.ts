import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { cliStudioHandle, killLaunchedStudios, launchedSessions, runRodeo } from "../helpers.js";

const PORT = 46220;

function mkTmp(ext: string): string {
  return join(tmpdir(), `rodeo-dir-${randomUUID()}${ext}`);
}

function writeScript(directive: string, body: string): string {
  const path = mkTmp(".luau");
  writeFileSync(path, `${directive}\n${body}`);
  return path;
}

function rmIfExists(path: string): void {
  try { unlinkSync(path); } catch {}
}

describe("directives (CLI)", () => {
  const cli = cliStudioHandle(PORT);
  beforeAll(cli.spawn);
  afterAll(cli.close);

  // Regression for the originally-reported --return bug. The pre-refactor
  // directive applier was a hand-written match that silently dropped any
  // flag without an explicit arm — `--return` was one of those. The argv
  // splice makes directives flow through the same clap parse as CLI args,
  // so this Just Works.
  it("--return directive writes file", () => {
    const outPath = mkTmp(".luau");
    const script = writeScript(
      `-- @rodeo run --return ${outPath}`,
      `return { ok = true, n = 42 }`,
    );
    try {
      const r = runRodeo(["run", "--port", String(PORT), script]);
      expect(r.ok).toBe(true);
      expect(existsSync(outPath)).toBe(true);
      const content = readFileSync(outPath, "utf8");
      expect(content).toContain('["ok"] = true');
      expect(content).toContain('["n"] = 42');
    } finally {
      rmIfExists(script);
      rmIfExists(outPath);
    }
  });

  // Validates the structural-fix premise: a CLI flag that was never
  // enumerated in the old hand-written directive switch should work in a
  // directive *automatically* under the splice. If `--output` is ever
  // removed or repurposed, pick another previously-unmirrored flag
  // (--sourcemap, --no-hud, --place.universe, --verbose).
  it("auto-parity: --output directive routes prints to file", () => {
    const outPath = mkTmp(".txt");
    const script = writeScript(
      `-- @rodeo run --output ${outPath}`,
      `print("output_directive_ok") return nil`,
    );
    try {
      const r = runRodeo(["run", "--port", String(PORT), script]);
      expect(r.ok).toBe(true);
      expect(existsSync(outPath)).toBe(true);
      expect(readFileSync(outPath, "utf8")).toContain("output_directive_ok");
    } finally {
      rmIfExists(script);
      rmIfExists(outPath);
    }
  });

  // CLI overrides directive on conflict. Splice injects directive tokens
  // *before* user CLI args, so clap's last-arg-wins resolves to the CLI
  // value for scalar fields. Observable via RunService:IsRunning() —
  // true in run mode, false in edit mode. (--mode is the transition flag;
  // context no longer implies mode, so this exercises the override on the
  // flag that actually changes studio state.)
  it("CLI --mode overrides directive --mode", () => {
    const script = writeScript(
      `-- @rodeo run --mode edit --show-return`,
      `return game:GetService("RunService"):IsRunning()`,
    );
    try {
      const r = runRodeo([
        "run", "--port", String(PORT), script,
        "--mode", "run", "--context", "server",
      ]);
      expect(r.ok).toBe(true);
      expect(r.stdout + r.stderr).toContain("true");
    } finally {
      rmIfExists(script);
    }
  });

  // Sanity that --show-return directive (one of the few that *was* in the
  // old switch) still works post-refactor. Mirrors the coverage in
  // executionTests.ts:scriptFile but in the new dedicated home.
  it("--show-return directive prints return value", () => {
    const script = writeScript(
      `-- @rodeo run --show-return`,
      `return "show_return_directive_ok"`,
    );
    try {
      const r = runRodeo(["run", "--port", String(PORT), script]);
      expect(r.ok).toBe(true);
      expect(r.stdout + r.stderr).toContain("show_return_directive_ok");
    } finally {
      rmIfExists(script);
    }
  });
});

// Issue #28: a script whose directive launches a place could not be re-run
// against an already-open Studio. `--studio-id` conflicted with the
// directive's `--place`, and `--dom-id` launched a second Studio, ran in the
// pinned DOM instead, then closed the launch.
describe("a CLI pin overrides a directive's --place", () => {
  const PIN_PORT = 47520;
  // Studios this suite launches are reaped by session, never by pattern.
  const STARTED = Date.now();
  const cli = cliStudioHandle(PIN_PORT);
  beforeAll(async () => {
    await cli.spawn();
    // Mark the open Studio so a run can prove it executed there.
    const mark = runRodeo([
      "run", "--port", String(PIN_PORT), "--studio-id", cli.studio().studioId,
      "--source", `workspace:SetAttribute("rodeoIssue28", "open Studio")`,
    ]);
    expect(mark.ok, mark.stderr).toBe(true);
  });
  afterAll(async () => {
    await cli.close();
    killLaunchedStudios(PIN_PORT, STARTED);
  });

  // The directive's --save belongs to its launch: it must not save the
  // Studio the user pins the script to.
  const saveOut = mkTmp(".rbxl");
  const script = () => writeScript(
    `-- @rodeo run --place --save ${saveOut} --show-return`,
    `return workspace:GetAttribute("rodeoIssue28")`,
  );

  it("--studio-id runs the script in the open Studio without launching", () => {
    const path = script();
    const since = Date.now();
    try {
      const r = runRodeo(["run", "--port", String(PIN_PORT), path, "--studio-id", cli.studio().studioId]);
      expect(r.ok, r.stderr).toBe(true);
      expect(r.stdout).toContain("open Studio");
      expect(launchedSessions(PIN_PORT, since)).toEqual([]);
      expect(existsSync(saveOut)).toBe(false);
    } finally {
      rmIfExists(path);
      rmIfExists(saveOut);
    }
  });

  it("--dom-id runs the script in the pinned DOM without launching", () => {
    const path = script();
    const since = Date.now();
    try {
      const r = runRodeo(["run", "--port", String(PIN_PORT), path, "--dom-id", cli.studio().editDomId!]);
      expect(r.ok, r.stderr).toBe(true);
      expect(r.stdout).toContain("open Studio");
      expect(launchedSessions(PIN_PORT, since)).toEqual([]);
      expect(existsSync(saveOut)).toBe(false);
    } finally {
      rmIfExists(path);
      rmIfExists(saveOut);
    }
  });

  it("--dom-id and --place typed together are rejected", () => {
    const since = Date.now();
    const r = runRodeo([
      "run", "--port", String(PIN_PORT), "--dom-id", cli.studio().editDomId!, "--place", "--source", "return 1",
    ]);
    expect(r.ok).toBe(false);
    expect(r.stderr).toContain("cannot be used with");
    expect(launchedSessions(PIN_PORT, since)).toEqual([]);
  });
});
