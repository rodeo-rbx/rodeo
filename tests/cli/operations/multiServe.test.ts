import { describe, it, expect, afterAll } from "bun:test";
import { existsSync } from "node:fs";
import { cliStudioHandle, keptMarkerFor, pluginFileFor, removeKeptPluginFile, runRodeo } from "../helpers.js";

// Two serves of the same build on different ports, running side by side. Each
// studio backend installs its own `rodeo-<build>-<port>.rbxm`, owns only the
// Studios it launched, and leaves its file installed when it exits, for the
// next serve on its port. The same mechanics let two different builds coexist
// (crossVersion.test.ts).
//
// Note on ordering: this suite brings both serves fully up first, then runs.
// An early version raced a run against B's startup and flaked once, which was
// read at the time as Studio reloading A's plugin on B's file install. That
// was later measured not to happen (pluginFolderChurn.test.ts: a run survives
// another serve starting/stopping with no reconnect); the ordering here simply
// keeps this suite about ownership, not timing.
const PORT_A = 46296;
const PORT_B = 46298;

type StateJson = { studios?: Array<{ studioId: string; sessionId?: string | null }> };

function stateOf(port: number): StateJson {
  const r = runRodeo(["state", "--port", String(port), "--json"]);
  expect(r.ok).toBe(true);
  return JSON.parse(r.stdout) as StateJson;
}

function ownedStudios(state: StateJson): string[] {
  return (state.studios ?? []).filter((s) => s.sessionId).map((s) => s.studioId);
}

describe("two serves side by side (CLI)", () => {
  const a = cliStudioHandle(PORT_A, { keepPluginFile: true });
  const b = cliStudioHandle(PORT_B, { keepPluginFile: true });
  afterAll(async () => {
    await b.close();
    await a.close();
    removeKeptPluginFile(PORT_A);
    removeKeptPluginFile(PORT_B);
  });

  it("each backend installs its own plugin file and neither sees the other's Studio", async () => {
    await a.spawn();
    expect(existsSync(pluginFileFor(PORT_A))).toBe(true);

    await b.spawn();
    expect(existsSync(pluginFileFor(PORT_B))).toBe(true);
    // B's start-time sweep probed A's master, found the same build alive, and
    // left A's file alone.
    expect(existsSync(pluginFileFor(PORT_A))).toBe(true);

    // Both serves up: a run on each lands in that serve's own Studio. (A brief
    // reload from B's startup, if any, has settled by now — the plugin
    // reconnects on its own.)
    const bRun = await b.runFn({ source: "return 'b-done'" });
    expect(bRun.ok).toBe(true);
    expect(bRun.return).toBe("b-done");

    const aRun = await a.runFn({ source: "return 'a-done'" });
    expect(aRun.ok).toBe(true);
    expect(aRun.return).toBe("a-done");

    // Owned Studios are exclusive to the serve that launched them: the other
    // build's plugin sees a `rodeoPort` that isn't its own and stays dormant.
    const aState = stateOf(PORT_A);
    const bState = stateOf(PORT_B);
    const [aStudio] = ownedStudios(aState);
    const [bStudio] = ownedStudios(bState);
    expect(aStudio).toBeDefined();
    expect(bStudio).toBeDefined();
    expect((bState.studios ?? []).map((s) => s.studioId)).not.toContain(aStudio);
    expect((aState.studios ?? []).map((s) => s.studioId)).not.toContain(bStudio);
  });

  it("a backend leaves its plugin file installed on exit and records it", async () => {
    await b.close();
    expect(existsSync(pluginFileFor(PORT_B))).toBe(true);
    expect(existsSync(keptMarkerFor(PORT_B))).toBe(true);
    // A, still running, owns its file: no record.
    expect(existsSync(pluginFileFor(PORT_A))).toBe(true);
    expect(existsSync(keptMarkerFor(PORT_A))).toBe(false);

    await a.close();
    expect(existsSync(pluginFileFor(PORT_A))).toBe(true);
    expect(existsSync(keptMarkerFor(PORT_A))).toBe(true);
  });
});
