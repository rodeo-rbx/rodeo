import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cliStudioHandle, runRodeo } from "../helpers.js";

// `--adapter` (CLI or directive) enables another runtime's standard library
// aliases for a run. Without one, an alias nothing resolves stays in the
// script and fails only if it runs.
const PORT = 47730;

describe("--adapter (CLI)", () => {
  const cli = cliStudioHandle(PORT);
  const dirs: string[] = [];
  beforeAll(cli.spawn, 180_000);
  afterAll(async () => {
    await cli.close();
    for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
  });

  function script(source: string): string {
    const dir = mkdtempSync(join(tmpdir(), "rodeo-adapter-"));
    dirs.push(dir);
    const path = join(dir, "main.luau");
    writeFileSync(path, source);
    return path;
  }

  function run(args: string[]) {
    const r = runRodeo(["run", "--port", String(PORT), "--show-return", ...args]);
    return { ok: r.ok, out: r.stdout + r.stderr };
  }

  it("a directive enables an adapter for its script", () => {
    const r = run([
      script(
        [
          "-- @rodeo run --adapter lute",
          'local path = require("@std/path")',
          'local json = require("@std/json")',
          "-- Lute's path joins with the host's separator.",
          'local joined = path.format(path.join("a", "b"))',
          'return json.serialize({ joined = joined == "a/b" or joined == "a\\\\b" })',
        ].join("\n"),
      ),
    ]);
    expect(r.ok, r.out).toBe(true);
    expect(r.out).toContain('joined\\":true');
  });

  it("CLI adapters add to the directive's", () => {
    const r = run([
      script(
        [
          "-- @rodeo run --adapter lute",
          'local stringext = require("@std/stringext")',
          'local process = require("@lune/process")',
          'return stringext.hasPrefix(process.cwd, "") and "both"',
        ].join("\n"),
      ),
      "--adapter",
      "lune",
    ]);
    expect(r.ok, r.out).toBe(true);
    expect(r.out).toContain("both");
  });

  it("without an adapter, a require nothing resolves runs only if reached", () => {
    // LibMP's shape: the lute-only require sits behind a runtime check that
    // is false in Studio.
    const r = run([
      script(
        [
          'local isCliMode = typeof(game) == "nil"',
          'local fs = isCliMode and require("@std/fs") or nil',
          'return if fs == nil then "unreached" else "reached"',
        ].join("\n"),
      ),
    ]);
    expect(r.ok, r.out).toBe(true);
    expect(r.out).toContain("unreached");
  });

  it("without the adapter, a reached adapter require fails the run", () => {
    const r = run(["--source", 'return require("@lune/fs") ~= nil']);
    expect(r.ok, r.out).toBe(false);
  });
});
