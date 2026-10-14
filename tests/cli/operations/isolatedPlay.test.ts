import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { makeCliRunFn, spawnBackground, waitForDom, type BackgroundProcess } from "../helpers.js";

describe("isolated play mode (CLI)", () => {
  describe("empty place", () => {
    const PORT = 46260;
    let bg: BackgroundProcess;
    const run = makeCliRunFn(PORT);

    beforeAll(async () => {
      bg = spawnBackground([
        "run", "--port", String(PORT), "--place", "--mode", "play", "--context", "server",
      ]);
      await waitForDom(PORT);
    });
    afterAll(async () => { bg.kill(); await bg.exited; });

    it("play:server — IsRunning is true", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game:GetService('RunService'):IsRunning()",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — no LocalPlayer on server", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game:GetService('Players').LocalPlayer == nil",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:client:1 — spawns client with LocalPlayer", async () => {
      const result = await run({
        mode: "play", domKind: "client",
        source: "return game:GetService('Players').LocalPlayer ~= nil",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — server sees connected player", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return #game:GetService('Players'):GetPlayers() > 0",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:client — append spawns second client", async () => {
      const result = await run({
        mode: "play", domKind: "client",
        source: "task.wait(3); return game:GetService('Players').LocalPlayer ~= nil",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — server sees two players", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return #game:GetService('Players'):GetPlayers() >= 2",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });
  });

  // Multiplayer-test launch against a published place via the CLI. Mirrors
  // the empty-place suite (server + two clients) and adds identity +
  // universe-service checks: real PlaceId/GameId match, PlaceVersion
  // populated. Exercises the full download → stage → patch GUID → StartServer
  // → plugin gate → script run chain via the CLI subprocess path.
  describe("published place", () => {
    // Clear of the empty-place serve's pair (46260 and its backend's 46261):
    // on Windows that backend can still be shutting down when this starts.
    const PORT = 46264;
    const PLACE_ID = 72824109308551;
    const UNIVERSE_ID = 8612861022;
    let bg: BackgroundProcess;
    const run = makeCliRunFn(PORT);

    beforeAll(async () => {
      bg = spawnBackground([
        "run", "--port", String(PORT),
        "--place", String(PLACE_ID),
        "--mode", "play", "--context", "server",
      ]);
      await waitForDom(PORT);
    });
    afterAll(async () => { bg.kill(); await bg.exited; });

    it("play:server — IsRunning is true", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game:GetService('RunService'):IsRunning()",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — no LocalPlayer on server", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game:GetService('Players').LocalPlayer == nil",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — game.PlaceId matches the requested placeId", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game.PlaceId",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(PLACE_ID);
    });

    it("play:server — game.GameId is the resolved universeId", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game.GameId",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(UNIVERSE_ID);
    });

    // Studio-first multiplayer can't set placeVersion: the server is spawned by
    // ExecuteMultiplayerTestAsync from the edit DataModel (PlaceVersion 0), and
    // -task EditPlace ignores -placeVersion (verified). The old path forced it via
    // StartServer's -placeVersion, which rodeo no longer launches. Known limitation
    // — PlaceId/GameId/DataStore still resolve correctly.
    it.skip("play:server — game.PlaceVersion is non-zero (N/A in studio-first multiplayer)", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return game.PlaceVersion ~= 0",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — universe-scoped DataStoreService round-trip succeeds", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: `
          local DataStoreService = game:GetService("DataStoreService")
          local ds = DataStoreService:GetDataStore("rodeo_mptest_placeid_probe_cli")
          local stamp = os.time()
          ds:SetAsync("ping", stamp)
          return ds:GetAsync("ping") == stamp
        `,
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:client:1 — spawns client with LocalPlayer", async () => {
      const result = await run({
        mode: "play", domKind: "client",
        source: "return game:GetService('Players').LocalPlayer ~= nil",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:client:1 — game.PlaceId matches the published placeId", async () => {
      const result = await run({
        mode: "play", domKind: "client",
        source: "return game.PlaceId",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(PLACE_ID);
    });

    it("play:server — server sees connected player", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return #game:GetService('Players'):GetPlayers() > 0",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:client — append spawns second client", async () => {
      const result = await run({
        mode: "play", domKind: "client",
        source: "task.wait(3); return game:GetService('Players').LocalPlayer ~= nil",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });

    it("play:server — server sees two players", async () => {
      const result = await run({
        mode: "play", context: "server",
        source: "return #game:GetService('Players'):GetPlayers() >= 2",
      });
      expect(result.ok).toBe(true);
      expect(result.return).toBe(true);
    });
  });
});
