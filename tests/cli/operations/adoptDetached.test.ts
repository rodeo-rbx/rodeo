// Issue #35: `rodeo run --place --detach` with no serve on its port starts a
// serve that exits with the run, leaving the Studio up. A later serve on the
// port saw that Studio as hand-opened when its plugin reconnected: `rodeo
// kill` refused it ("not launched by rodeo") and `rodeo state` had no paths
// for it. The later serve now adopts it by its launch bootstrap.
import { describe, afterAll, it, expect } from "bun:test";
import { RodeoClient } from "../../../rodeo-client-ts/src/index.js";
import {
  killLaunchedStudios,
  launchedSessions,
  pidsMatching,
  processMatches,
  removeKeptPluginFile,
  runRodeo,
  spawnBackground,
  waitForHealthy,
  waitForPidsGone,
  waitUntil,
  type BackgroundProcess,
} from "../helpers.js";

const PORT = 47710;
// Studios this file launches are found and reaped by session, never by pattern.
const STARTED = Date.now();

type StudioSnap = { studioId: string; sessionId?: string | null; workingPath?: string | null };

// The Studio launched for `session`, once its plugin has reconnected to the
// serve on PORT and that serve has claimed it.
async function claimedStudio(session: string, timeoutMs: number): Promise<StudioSnap> {
  const client = await RodeoClient.connect(`http://localhost:${PORT}`);
  const start = Date.now();
  let seen: StudioSnap[] = [];
  try {
    while (Date.now() - start < timeoutMs) {
      const state = (await client.getState().catch(() => null)) as { studios?: StudioSnap[] } | null;
      seen = state?.studios ?? [];
      const claimed = seen.find((s) => s.sessionId === session);
      if (claimed) return claimed;
      await Bun.sleep(250);
    }
    throw new Error(`the serve never claimed session ${session}; studios: ${JSON.stringify(seen)}`);
  } finally {
    await client.close();
  }
}

describe("a --detach Studio outliving its serve (CLI)", () => {
  let serve: BackgroundProcess | undefined;

  afterAll(async () => {
    if (serve) {
      serve.kill();
      await serve.exited;
    }
    killLaunchedStudios(PORT, STARTED);
    removeKeptPluginFile(PORT);
  });

  it("is adopted by the next serve on its port, and rodeo kill closes it", async () => {
    expect(processMatches(`__master --port ${PORT}`)).toBe(false);

    // No serve on PORT, so the run starts its own, which exits with the run.
    const run = runRodeo(
      ["run", "--place", "--detach", "--port", String(PORT), "--show-return", "--source", 'return "up"'],
      { timeout: 110_000 },
    );
    expect(run.ok).toBe(true);
    const sessions = launchedSessions(PORT, STARTED);
    expect(sessions.length).toBe(1);
    const session = sessions[0];
    const pids = pidsMatching(`rodeo-bootstrap-${session}`);
    expect(pids.length).toBeGreaterThan(0);

    await waitUntil(
      () => !processMatches(`__master --port ${PORT}`) && !processMatches(`__studio-backend --port ${PORT + 1}`),
      30_000,
      "the run's serve to exit",
    );
    expect(processMatches(`rodeo-bootstrap-${session}`)).toBe(true);

    serve = spawnBackground(["serve", "--port", String(PORT)]);
    await waitForHealthy(PORT);
    const studio = await claimedStudio(session, 60_000);
    expect(studio.workingPath ?? "").toMatch(/\.rbxlx?$/);

    const kill = runRodeo(["kill", studio.studioId, "--port", String(PORT)], { timeout: 60_000 });
    expect(kill.stderr).not.toContain("not launched by rodeo");
    expect(kill.ok).toBe(true);
    expect(await waitForPidsGone(pids, 30_000)).toBe(true);
  }, 240_000);
});
