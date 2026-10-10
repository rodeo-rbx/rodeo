import { describe, it, expect } from "bun:test";
import { existsSync, utimesSync } from "node:fs";
import {
  keptMarkerFor,
  killLaunchedStudios,
  launchedSessions,
  pluginFileFor,
  processMatches,
  removeKeptPluginFile,
  runRodeo,
  spawnBackground,
  waitForHealthy,
  waitUntil,
} from "../helpers.js";

const PORT = 46202;
// Studios this file launches are found and reaped by session, never by pattern.
const STARTED = Date.now();

describe("--detach flag (CLI)", () => {
  it("run --place --detach keeps Studio alive", async () => {
    const result = runRodeo([
      "run", "--place", "--detach",
      "--port", String(PORT),
      "--source", "return nil",
    ]);
    expect(result.ok).toBe(true);

    await Bun.sleep(1000);

    // The launched Studio should still be running.
    expect(launchedSessions(PORT, STARTED).some((session) => processMatches(`rodeo-bootstrap-${session}`))).toBe(true);
  });

  // Each studio backend installs its own plugin file and leaves it installed
  // on exit; other backends' sweeps remove it once its keep window has passed,
  // but never while a detached Studio still runs the plugin, since deleting
  // the file unloads the plugin from that Studio at once. The one-shot run
  // above started a serve on PORT, launched detached, and exited.
  it("a detached Studio keeps its plugin file until the Studio is gone", async () => {
    const file = pluginFileFor(PORT);
    await Bun.sleep(2000); // let the exited backend's cleanup finish (it must not delete)
    expect(existsSync(file)).toBe(true);

    // A fresh backend's start-time sweep must leave it even once the keep
    // window has passed: nothing answers on PORT any more, but the detached
    // Studio still references PORT + 1.
    const marker = keptMarkerFor(PORT);
    expect(existsSync(marker)).toBe(true);
    const twoHoursAgo = new Date(Date.now() - 2 * 60 * 60 * 1000);
    utimesSync(marker, twoHoursAgo, twoHoursAgo);
    const other = spawnBackground(["serve", "--port", String(PORT + 10)]);
    try {
      await waitForHealthy(PORT + 10);
      await Bun.sleep(1000);
      expect(existsSync(file)).toBe(true);
    } finally {
      other.kill();
      await other.exited;
      removeKeptPluginFile(PORT + 10);
    }

    // Kill the detached Studio; the next sweep has nothing left to keep the
    // file for (its keep window is past).
    killLaunchedStudios(PORT, STARTED);
    await Bun.sleep(2000);
    const another = spawnBackground(["serve", "--port", String(PORT + 20)]);
    try {
      await waitForHealthy(PORT + 20);
      await waitUntil(() => !existsSync(file), 15_000, "the detached Studio's plugin file to be swept");
    } finally {
      another.kill();
      await another.exited;
      removeKeptPluginFile(PORT + 20);
    }
  });
});
