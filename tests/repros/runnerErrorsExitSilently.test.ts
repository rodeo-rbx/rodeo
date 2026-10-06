// Repro (issue #34): errors the runner raises itself, rather than the user's
// module, reached only ExecutionDone.error, which no client prints. So
// `--context cmdbar` on a Studio without the launch bootstrap's bridge (one
// opened by hand), or `--context elevated` on a Studio StudioMCP doesn't
// know, exited 1 with no output at all.
//
// Expected: the reason reaches stderr exactly once. A module's own error is
// logged by the engine and already streams through the run's output, so it
// must not be repeated — in every context, elevated (StudioMCP) included.
import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { cliStudioHandle, killLaunchedStudios, runRodeo } from "../cli/helpers.js";

const PORT = 47540;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const cli = cliStudioHandle(PORT);

function occurrences(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

function run(args: string[]) {
  return runRodeo(["run", "--port", String(PORT), "--studio-id", cli.studio().studioId, ...args]);
}

describe("runner errors reach the terminal once (issue #34)", () => {
  beforeAll(cli.spawn);
  afterAll(async () => {
    await cli.close();
    Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
    Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
    killLaunchedStudios(PORT, STARTED);
  });

  it("a module error is printed once", () => {
    const r = run(["--source", "error('rodeo-34-module-error')"]);
    const output = r.stdout + r.stderr;
    expect(r.exitCode, output).toBe(1);
    expect(occurrences(output, "rodeo-34-module-error"), output).toBe(1);
  });

  it("a module error under cmdbar is printed once", () => {
    const r = run(["--context", "cmdbar", "--source", "error('rodeo-34-cmdbar-error')"]);
    const output = r.stdout + r.stderr;
    expect(r.exitCode, output).toBe(1);
    expect(occurrences(output, "rodeo-34-cmdbar-error"), output).toBe(1);
  });

  it("a module error under elevated is printed once", () => {
    const r = run(["--context", "elevated", "--source", "error('rodeo-34-elevated-error')"]);
    const output = r.stdout + r.stderr;
    expect(r.exitCode, output).toBe(1);
    expect(occurrences(output, "rodeo-34-elevated-error"), output).toBe(1);
  });

  it("cmdbar on a Studio without the bridge says why", () => {
    // A hand-opened Studio has no bridge. Renaming this one's gives the same
    // Studio without opening one by hand; the Studio is this file's own.
    const hide = run([
      "--source",
      'game:GetService("CoreGui").rodeoCmdbar.Name = "rodeoCmdbarHidden" return true',
    ]);
    expect(hide.exitCode, hide.stdout + hide.stderr).toBe(0);

    const r = run(["--context", "cmdbar", "--source", "return 1"]);
    expect(r.exitCode, r.stdout + r.stderr).toBe(1);
    // FAILS before the fix: exit 1 with empty stdout and stderr.
    expect(occurrences(r.stderr, "cmdbar context needs a rodeo-launched Studio"), r.stderr).toBe(1);
  });

  it("--no-error hides the run's error too", () => {
    // The bridge is still hidden from the case above.
    const r = run(["--context", "cmdbar", "--no-error", "--source", "return 1"]);
    expect(r.exitCode, r.stdout + r.stderr).toBe(1);
    expect(occurrences(r.stderr, "cmdbar context needs a rodeo-launched Studio"), r.stderr).toBe(0);
  });
});
