import { describe, it, expect } from "bun:test";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import {
  killMatching,
  pluginFileFor,
  processMatches,
  runRodeo,
  spawnBackground,
  waitForHealthy,
  waitUntil,
} from "../helpers.js";

const PORT = 46202;
const STARTED = Date.now();

// Sessions this file launched: each launch's RunScript bootstrap, written to
// the serve's .rodeo/.temp, stamps the backend port. Other Studios on the
// machine (the developer's, other sessions' and projects') are never touched.
function launchedSessions(): string[] {
  const dir = join(".rodeo", ".temp");
  return readdirSync(dir).flatMap((name) => {
    const session = /^rodeo-bootstrap-([0-9a-f-]+)\.luau$/.exec(name)?.[1];
    const path = join(dir, name);
    const ours = session && statSync(path).mtimeMs >= STARTED && readFileSync(path, "utf8").includes(`"rodeoPort", ${PORT + 1})`);
    return ours ? [session] : [];
  });
}

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
    expect(launchedSessions().some((session) => processMatches(`rodeo-bootstrap-${session}`))).toBe(true);
  });

  // Each studio backend installs its own plugin file and removes it on exit —
  // unless a detached Studio still runs the plugin, since deleting the file
  // unloads the plugin from that Studio at once. The one-shot run above
  // started a serve on PORT, launched detached, and exited, so its backend
  // must have left the file behind.
  it("a detached Studio keeps its plugin file until the Studio is gone", async () => {
    const file = pluginFileFor(PORT);
    await Bun.sleep(2000); // let the exited backend's cleanup finish (it must not delete)
    expect(existsSync(file)).toBe(true);

    // A fresh backend's start-time sweep must leave it too: nothing answers on
    // PORT any more, but the detached Studio still references PORT + 1.
    const other = spawnBackground(["serve", "--port", String(PORT + 10)]);
    try {
      await waitForHealthy(PORT + 10);
      await Bun.sleep(1000);
      expect(existsSync(file)).toBe(true);
    } finally {
      other.kill();
      await other.exited;
    }

    // Kill the detached Studio; the next sweep has nothing to keep the file for.
    for (const session of launchedSessions()) killMatching(`rodeo-bootstrap-${session}`);
    await Bun.sleep(2000);
    const another = spawnBackground(["serve", "--port", String(PORT + 20)]);
    try {
      await waitForHealthy(PORT + 20);
      await waitUntil(() => !existsSync(file), 15_000, "the detached Studio's plugin file to be swept");
    } finally {
      another.kill();
      await another.exited;
    }
  });
});
