// Studio mode transitions driven by `--mode`: issues #27 and #39.
//
// #27: after a run-mode session, `--mode test` runs sat queued forever while
// the edit DOM retried ExecutePlayModeAsync once a second ("a previous one is
// still in progress"), and nothing reached the CLI. Reported on a published
// place; both a local .rbxl and a published place are exercised here.
//
// #39: `--mode play` against a solo `--mode test` session never ended the
// test, retried ExecutePlayModeAsync forever, and the client run hung
// silently; `--mode edit` could not end a session.
//
// The last suite hides a session from rodeo (its DataModels' plugins stay
// dormant) to reproduce #27's wedged state on purpose: Studio reports edit to
// rodeo while the engine has a test in progress. The queued run must fail
// with the engine's reason instead of waiting forever.
//
// Every suite launches its own Studio on its own port and reaps only the
// Studios it launched (launchedSessions / killLaunchedStudios).
import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { execSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  killLaunchedStudios,
  runRodeo,
  spawnBackground,
  waitForOwnedStudio,
  type BackgroundProcess,
} from "../helpers.js";

// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
// rodeo-cli's published test place (also used by isolatedPlay.test.ts).
const PUBLISHED_PLACE = process.env.RODEO_MODE_TRANSITION_PLACE ?? "72824109308551";
// Bound on one transition + run. A hang (the bug) hits it; a working
// transition takes seconds.
const RUN_BOUND_MS = 120_000;

type RunOut = { code: number | null; out: string; timedOut: boolean; ms: number };

// `rodeo <args>` without blocking the event loop, so several runs can be
// queued at once (as in #27) and a hang is cut off at `timeoutMs`.
async function rodeo(args: string[], timeoutMs = RUN_BOUND_MS): Promise<RunOut> {
  const started = Date.now();
  const proc = Bun.spawn(["rodeo", ...args], { stdout: "pipe", stderr: "pipe", stdin: "ignore" });
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    proc.kill(9);
  }, timeoutMs);
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout as ReadableStream).text(),
    new Response(proc.stderr as ReadableStream).text(),
  ]);
  const code = await proc.exited;
  clearTimeout(timer);
  const out = stdout + stderr + (timedOut ? `\n[killed after ${timeoutMs}ms]` : "");
  return { code, out, timedOut, ms: Date.now() - started };
}

type StudioSnap = { studioId: string; studioMode: string; doms: Array<{ domId: string; domKind: string }> };

function studioState(port: number, studioId: string): StudioSnap {
  const r = runRodeo(["state", "--json", "--port", String(port)]);
  expect(r.ok, r.stdout + r.stderr).toBe(true);
  const snap = JSON.parse(r.stdout) as { studios?: StudioSnap[] };
  const studio = (snap.studios ?? []).find((s) => s.studioId === studioId);
  if (!studio) throw new Error(`studio ${studioId} is not in rodeo state:\n${r.stdout}`);
  return studio;
}

// A minimal local place (a floor to stand on), built with rojo.
function buildLocalPlace(dir: string): string {
  const project = join(dir, "default.project.json");
  writeFileSync(
    project,
    JSON.stringify({
      name: "ModeTransitions",
      tree: {
        $className: "DataModel",
        Workspace: {
          $className: "Workspace",
          Floor: {
            $className: "Part",
            $properties: { Anchored: true, Size: [64, 1, 64], Position: [0, -0.5, 0] },
          },
        },
      },
    }),
  );
  const place = join(dir, "place.rbxl");
  execSync(`rojo build ${project} -o ${place}`, { stdio: "inherit" });
  return place;
}

const IS_RUN = `
local RunService = game:GetService("RunService")
return if RunService:IsRunning() and RunService:IsRunMode() then "PASS-run" else "FAIL-run"`;

// A solo test's server reports "test" once its player has joined.
const IS_TEST_SERVER = `
local RunService = game:GetService("RunService")
local players = #game:GetService("Players"):GetPlayers()
return if RunService:IsRunning() and not RunService:IsRunMode() and players > 0
  then "PASS-test-server" else \`FAIL-test-server players={players}\``;

// CanLeaveTest is false in a solo test's client, true in a multiplayer one's.
const IS_TEST_CLIENT = `
local ok, canLeave = pcall(function() return game:GetService("StudioTestService"):CanLeaveTest() end)
return if game:GetService("Players").LocalPlayer and ok and canLeave == false
  then "PASS-test-client" else \`FAIL-test-client {ok} {canLeave}\``;

const IS_PLAY_CLIENT = `
local ok, canLeave = pcall(function() return game:GetService("StudioTestService"):CanLeaveTest() end)
return if game:GetService("Players").LocalPlayer and ok and canLeave == true
  then "PASS-play-client" else \`FAIL-play-client {ok} {canLeave}\``;

const IS_PLAY_SERVER = `
local RunService = game:GetService("RunService")
local players = #game:GetService("Players"):GetPlayers()
return if RunService:IsRunning() and not RunService:IsRunMode() and players >= 1
  then "PASS-play-server" else \`FAIL-play-server players={players}\``;

const IS_EDIT = `return if game:GetService("RunService"):IsEdit() then "PASS-edit" else "FAIL-edit"`;

// StudioTestService.EditModeActive (undocumented) is what the mode driver
// waits on before starting a session: true while Studio can start one.
const EDIT_MODE_ACTIVE = `return "EditModeActive=" .. tostring((game:GetService("StudioTestService") :: any).EditModeActive)`;

// One Studio on `port` for a describe block: a persistent `rodeo run --place`
// (it owns the serve), torn down with its Studio after the suite.
function launchedStudio(port: number, place: () => string, timeoutMs = 120_000) {
  let bg: BackgroundProcess | null = null;
  let studioId = "";
  return {
    spawn: async () => {
      bg = spawnBackground(["run", "--port", String(port), "--place", place()]);
      studioId = (await waitForOwnedStudio(port, timeoutMs)).studioId;
    },
    close: async () => {
      bg?.kill();
      await bg?.exited;
      killLaunchedStudios(port, STARTED);
    },
    id: () => studioId,
    // A run pinned to this Studio, printing its return value.
    run: (routing: string[], source: string, timeoutMs?: number) =>
      rodeo(
        ["run", "--port", String(port), "--studio-id", studioId, ...routing, "--show-return", "--source", source],
        timeoutMs,
      ),
  };
}

function expectPass(r: RunOut, marker: string) {
  expect(r.timedOut, r.out).toBe(false);
  expect(r.code, r.out).toBe(0);
  expect(r.out).toContain(marker);
}

// ── #27: run → test ─────────────────────────────────────────────────────
for (const variant of [
  { name: "local .rbxl", port: 47580, published: false },
  { name: "published place", port: 47582, published: true },
]) {
  describe(`#27 run → test, ${variant.name}`, () => {
    let dir = "";
    const studio = launchedStudio(
      variant.port,
      () => (variant.published ? PUBLISHED_PLACE : buildLocalPlace(dir)),
      variant.published ? 180_000 : 120_000,
    );
    beforeAll(async () => {
      dir = mkdtempSync(join(tmpdir(), "rodeo-mode-transitions-"));
      await studio.spawn();
    }, 240_000);
    afterAll(async () => {
      await studio.close();
      rmSync(dir, { recursive: true, force: true });
    }, 60_000);

    it("enters run mode", async () => {
      expectPass(await studio.run(["--mode", "run", "--context", "server"], IS_RUN), "PASS-run");
    }, RUN_BOUND_MS + 10_000);

    // The reported repro: a server and a client test run queued together
    // right after a run-mode session.
    it("then test mode: queued server and client runs both complete", async () => {
      const [server, client] = await Promise.all([
        studio.run(["--mode", "test", "--context", "server"], IS_TEST_SERVER),
        studio.run(["--mode", "test", "--context", "client"], IS_TEST_CLIENT),
      ]);
      expectPass(server, "PASS-test-server");
      expectPass(client, "PASS-test-client");
      expect(studioState(variant.port, studio.id()).studioMode).toBe("test");
    }, RUN_BOUND_MS + 10_000);

    it("survives repeated run ↔ test switches", async () => {
      for (let i = 0; i < 3; i++) {
        expectPass(await studio.run(["--mode", "run", "--context", "server"], IS_RUN), "PASS-run");
        expectPass(await studio.run(["--mode", "test", "--context", "client"], IS_TEST_CLIENT), "PASS-test-client");
      }
    }, 6 * RUN_BOUND_MS);
  });
}

// ── #39: test → play → test → edit, pinned with --studio-id ─────────────
describe("#39 switching session kinds", () => {
  const PORT = 47584;
  let dir = "";
  const studio = launchedStudio(PORT, () => buildLocalPlace(dir));
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), "rodeo-mode-transitions-"));
    await studio.spawn();
  }, 240_000);
  afterAll(async () => {
    await studio.close();
    rmSync(dir, { recursive: true, force: true });
  }, 60_000);

  // The mode driver gates every start on this. If it reads anything but true
  // in an idle edit Studio, every transition waits out the busy timeout.
  it("EditModeActive is true in an idle edit Studio", async () => {
    expectPass(await studio.run(["--dom", "edit"], EDIT_MODE_ACTIVE), "EditModeActive=true");
  }, RUN_BOUND_MS);

  it("starts a solo test", async () => {
    expectPass(await studio.run(["--mode", "test", "--context", "client"], IS_TEST_CLIENT), "PASS-test-client");
    expect(studioState(PORT, studio.id()).studioMode).toBe("test");
  }, RUN_BOUND_MS + 10_000);

  // ...and false while a session runs (read from the edit DOM, which
  // `--dom edit` reaches without ending the session).
  it("EditModeActive is false during the session, and --dom edit leaves the session running", async () => {
    expectPass(await studio.run(["--dom", "edit"], EDIT_MODE_ACTIVE), "EditModeActive=false");
    expect(studioState(PORT, studio.id()).studioMode).toBe("test");
  }, RUN_BOUND_MS);

  it("--mode play --dom client ends the solo test and starts a multiplayer one", async () => {
    expectPass(
      await studio.run(["--mode", "play", "--dom", "client", "--context", "client"], IS_PLAY_CLIENT),
      "PASS-play-client",
    );
    expect(studioState(PORT, studio.id()).studioMode).toBe("play");
  }, RUN_BOUND_MS + 10_000);

  it("--mode play --context server runs on that multiplayer server", async () => {
    expectPass(await studio.run(["--mode", "play", "--context", "server"], IS_PLAY_SERVER), "PASS-play-server");
  }, RUN_BOUND_MS + 10_000);

  it("--mode test goes back from play to a solo test", async () => {
    expectPass(await studio.run(["--mode", "test", "--context", "server"], IS_TEST_SERVER), "PASS-test-server");
    expect(studioState(PORT, studio.id()).studioMode).toBe("test");
  }, RUN_BOUND_MS + 10_000);

  it("--mode edit ends the session", async () => {
    expectPass(await studio.run(["--mode", "edit"], IS_EDIT), "PASS-edit");
    const state = studioState(PORT, studio.id());
    expect(state.studioMode).toBe("edit");
    expect(state.doms.map((d) => d.domKind)).toEqual(["edit"]);
  }, RUN_BOUND_MS + 10_000);

  it("--mode edit in edit runs at once", async () => {
    const r = await studio.run(["--mode", "edit"], IS_EDIT);
    expectPass(r, "PASS-edit");
    expect(r.ms).toBeLessThan(15_000);
  }, RUN_BOUND_MS);
});

// ── #27's wedge, on purpose: a session rodeo can't see or end ────────────
describe("a transition Studio refuses fails the run with the engine's reason", () => {
  const PORT = 47586;
  // How long the hidden session lives: past the driver's 30 s busy timeout
  // and its one forced start, short enough to recover within the suite.
  const HOLD_S = 75;
  let dir = "";
  const studio = launchedStudio(PORT, () => buildLocalPlace(dir));
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), "rodeo-mode-transitions-"));
    await studio.spawn();
  }, 240_000);
  afterAll(async () => {
    await studio.close();
    rmSync(dir, { recursive: true, force: true });
  }, 60_000);

  async function editModeActive(): Promise<string> {
    const r = await studio.run(["--dom", "edit"], EDIT_MODE_ACTIVE, 30_000);
    return /EditModeActive=(\w+)/.exec(r.out)?.[1] ?? `unreadable: ${r.out}`;
  }

  async function waitForEditModeActive(want: string, timeoutMs: number) {
    const deadline = Date.now() + timeoutMs;
    let seen = "";
    while (Date.now() < deadline) {
      seen = await editModeActive();
      if (seen === want) return;
      await Bun.sleep(1000);
    }
    throw new Error(`EditModeActive stayed ${seen}, wanted ${want}`);
  }

  it("hides a solo test from rodeo: Studio reports edit, the engine has a session", async () => {
    // The session's DataModels are clones of the edit DataModel: with another
    // port in `rodeoPort`, every rodeo plugin in them stays dormant, so no
    // server DOM ever connects to end the session. A server Script ends it
    // after HOLD_S.
    const hide = await studio.run(
      ["--dom", "edit"],
      `
      local ws = workspace
      local port = ws:GetAttribute("rodeoPort")
      local ender = Instance.new("Script")
      ender.Name = "RodeoTestHiddenSessionEnder"
      ender.Source = 'task.wait(${HOLD_S}) game:GetService("StudioTestService"):EndTest("hidden session over")'
      ender.Parent = game:GetService("ServerScriptService")
      ws:SetAttribute("rodeoTestPort", port)
      ws:SetAttribute("rodeoPort", 1)
      task.spawn(function()
        game:GetService("StudioTestService"):ExecutePlayModeAsync({})
      end)
      return "PASS-hidden"`,
    );
    expectPass(hide, "PASS-hidden");
    await waitForEditModeActive("false", 30_000);
    // The clones are made: put the edit DataModel back as it was.
    await Bun.sleep(5000);
    expectPass(
      await studio.run(
        ["--dom", "edit"],
        `
        local ws = workspace
        ws:SetAttribute("rodeoPort", ws:GetAttribute("rodeoTestPort"))
        ws:SetAttribute("rodeoTestPort", nil)
        game:GetService("ServerScriptService").RodeoTestHiddenSessionEnder:Destroy()
        return "PASS-restored"`,
      ),
      "PASS-restored",
    );
    const state = studioState(PORT, studio.id());
    expect(state.studioMode).toBe("edit");
    expect(state.doms.map((d) => d.domKind)).toEqual(["edit"]);
  }, 2 * RUN_BOUND_MS);

  it("--mode test fails within the bound, naming the refused start", async () => {
    const r = await studio.run(["--mode", "test", "--context", "server"], `return "PASS-unexpected"`, 100_000);
    console.log(`--mode test against the hidden session: exit ${r.code} after ${Math.round(r.ms / 1000)}s\n${r.out.trim()}`);
    expect(r.timedOut, r.out).toBe(false);
    expect(r.code, r.out).toBe(2);
    expect(r.out).toContain("could not enter test mode");
    expect(r.out).toContain("ExecutePlayModeAsync");
  }, 110_000);

  it("--mode test works again once the session is over", async () => {
    await waitForEditModeActive("true", (HOLD_S + 30) * 1000);
    expectPass(await studio.run(["--mode", "test", "--context", "server"], IS_TEST_SERVER), "PASS-test-server");
  }, (HOLD_S + 30) * 1000 + RUN_BOUND_MS);
});
