// Repro (issue #7, item 2): `rodeo run --studio-id <id> --save <path>` ran
// the script, wrote nothing, and exited 0. The save only ran for a Studio
// the run launched itself (--place); a pinned run dropped --save silently.
//
// Expected: a pinned run's --save saves that Studio after a successful run
// (the same verified save as `rodeo save <id> --out <path>`), skips it after
// a failed run, and --save with nothing to save is an error.
import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { cliStudioHandle, killLaunchedStudios, runRodeo } from "../cli/helpers.js";

const PORT = 47530;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const files: string[] = [];

function tmpPlace(): string {
  const path = join(tmpdir(), `rodeo-repro-save-${randomUUID()}.rbxl`);
  files.push(path);
  return path;
}

describe("run --save on a pinned Studio", () => {
  const cli = cliStudioHandle(PORT);
  beforeAll(cli.spawn, 120_000);
  afterAll(async () => {
    await cli.close();
    killLaunchedStudios(PORT, STARTED);
    for (const f of files) rmSync(f, { force: true });
  });

  it("--save with no Studio to save is an error", () => {
    const out = tmpPlace();
    const r = runRodeo(["run", "--port", String(PORT), "--save", out, "--source", "return 1"]);
    expect(r.ok).toBe(false);
    expect(r.stderr).toContain("--save needs a Studio to save");
    expect(existsSync(out)).toBe(false);
  });

  it("--studio-id --save <path> saves the pinned Studio's live place", () => {
    const out = tmpPlace();
    const r = runRodeo([
      "run", "--port", String(PORT), "--studio-id", cli.studio().studioId, "--save", out,
      "--source", `workspace:SetAttribute("rodeoIssue7", "saved after the run")`,
    ], { timeout: 120_000 });
    expect(r.ok, r.stderr).toBe(true);
    expect(existsSync(out)).toBe(true);

    // The file holds the run's edit: open it in a fresh Studio and read it.
    const check = runRodeo([
      "run", "--port", String(PORT), "--place", out, "--show-return",
      "--source", `return workspace:GetAttribute("rodeoIssue7")`,
    ], { timeout: 120_000 });
    expect(check.ok, check.stderr).toBe(true);
    expect(check.stdout).toContain("saved after the run");
  }, 240_000);

  it("--dom-id --save <path> saves the Studio holding that DOM", () => {
    const out = tmpPlace();
    const r = runRodeo([
      "run", "--port", String(PORT), "--dom-id", cli.studio().editDomId!, "--save", out,
      "--source", "return 1",
    ], { timeout: 120_000 });
    expect(r.ok, r.stderr).toBe(true);
    expect(existsSync(out)).toBe(true);
  }, 120_000);

  it("a failed run skips the save and still fails", () => {
    const out = tmpPlace();
    const r = runRodeo([
      "run", "--port", String(PORT), "--studio-id", cli.studio().studioId, "--save", out,
      "--source", `error("rodeoIssue7 boom")`,
    ]);
    expect(r.ok).toBe(false);
    expect(r.stderr).toContain("--save skipped");
    expect(existsSync(out)).toBe(false);
  });
});
