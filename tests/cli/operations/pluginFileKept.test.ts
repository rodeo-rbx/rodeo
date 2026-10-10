import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { existsSync, utimesSync } from "node:fs";
import {
  keptMarkerFor,
  killLaunchedStudios,
  pluginFileFor,
  removeKeptPluginFile,
  runRodeo,
  spawnBackground,
  waitForHealthy,
  waitUntil,
} from "../helpers.js";

// A backend leaves its plugin file installed when it exits. Launching a Studio
// right after a plugin file is written risks a crash (Studio hot-reloads the
// plugin while its place is still loading), so a launch waits until the file
// is 2.5 s old. Kept, the file is unchanged for the next serve of this build
// on the same port, whose launch then doesn't wait. Other backends' sweeps
// remove a kept file an hour after its serve stopped.
const PORT = 47640;
const OTHER_PORT = 47650;
const FRESH_WAIT = "plugin file is fresh";
// Studios this file launches are found and reaped by session, never by pattern.
const STARTED = Date.now();

function oneShotRun() {
  // --verbose: the serve's info lines (the fresh-file wait among them) reach
  // this run's output.
  const r = runRodeo(["run", "--verbose", "--port", String(PORT), "--place", "--source", "return 'ran'", "--show-return"], {
    timeout: 180_000,
  });
  return { ok: r.ok, out: r.stdout + r.stderr };
}

describe("plugin file kept between serves (CLI)", () => {
  beforeAll(() => removeKeptPluginFile(PORT));
  afterAll(() => {
    killLaunchedStudios(PORT, STARTED);
    removeKeptPluginFile(PORT);
    removeKeptPluginFile(OTHER_PORT);
  });

  it("a serve's first launch waits for its new plugin file, and the file stays after it exits", () => {
    const r = oneShotRun();
    expect(r.ok, r.out).toBe(true);
    expect(r.out).toContain("ran");
    expect(r.out).toContain(FRESH_WAIT);
    expect(existsSync(pluginFileFor(PORT))).toBe(true);
    expect(existsSync(keptMarkerFor(PORT))).toBe(true);
  }, 200_000);

  it("the next serve on the port reuses the file and launches without waiting", () => {
    const r = oneShotRun();
    expect(r.ok, r.out).toBe(true);
    expect(r.out).toContain("ran");
    expect(r.out).not.toContain(FRESH_WAIT);
    expect(existsSync(pluginFileFor(PORT))).toBe(true);
    expect(existsSync(keptMarkerFor(PORT))).toBe(true);
  }, 200_000);

  it("another serve's sweep removes the file once its keep window has passed", async () => {
    const twoHoursAgo = new Date(Date.now() - 2 * 60 * 60 * 1000);
    utimesSync(keptMarkerFor(PORT), twoHoursAgo, twoHoursAgo);
    const other = spawnBackground(["serve", "--port", String(OTHER_PORT)]);
    try {
      await waitForHealthy(OTHER_PORT);
      await waitUntil(() => !existsSync(pluginFileFor(PORT)), 15_000, "the kept plugin file to be swept");
      expect(existsSync(keptMarkerFor(PORT))).toBe(false);
    } finally {
      other.kill();
      await other.exited;
    }
  }, 60_000);
});
