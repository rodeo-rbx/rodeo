import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import {
  runRodeo,
  spawnBackground,
  waitForProcess,
  waitForDom,
  killLaunchedStudios,
  type BackgroundProcess,
} from "../helpers.js";
import { RodeoClient } from "../../../rodeo-client-ts/src/index.js";

const PORT = 46210;

describe("state (CLI)", () => {
  let bg: BackgroundProcess;

  beforeAll(async () => {
    bg = spawnBackground(["run", "--port", String(PORT), "--place"]);
    await waitForDom(PORT);
  });
  afterAll(async () => { bg.kill(); await bg.exited; });

  it("lists the studio and its DOMs", () => {
    const result = runRodeo(["state", "--port", String(PORT)]);
    expect(result.ok).toBe(true);
    const out = result.stdout + result.stderr;
    expect(out).toContain("LOCAL");
    expect(out).toContain("DOMS");
    expect(out).toContain("edit");
  });

  it("--json exposes studios[].doms[].domId and editDomId", () => {
    const result = runRodeo(["state", "--json", "--port", String(PORT)]);
    expect(result.ok).toBe(true);
    const snap = JSON.parse(result.stdout);
    expect(snap.studios.length).toBeGreaterThan(0);
    const studio = snap.studios[0];
    expect(studio.studioId).toBeTruthy();
    expect(studio.studioMode).toBe("edit");
    expect(studio.doms.length).toBeGreaterThan(0);
    const edit = studio.doms.find((d: any) => d.domKind === "edit");
    expect(edit).toBeTruthy();
    expect(studio.editDomId).toBe(edit.domId);
  });

  it("joins a running run to its DOM and studio", async () => {
    // The run table is live-only: a normal run leaves it the moment it
    // finishes, so assert against a still-running run.
    const scriptProc = spawnBackground([
      "run", "--port", String(PORT), "--source", "task.wait(30) return nil",
    ]);

    try {
      const id = await waitForProcess(PORT, "running");
      expect(id).not.toBeNull();

      const pretty = runRodeo(["state", "--port", String(PORT)]);
      expect(pretty.ok).toBe(true);
      expect(pretty.stdout + pretty.stderr).toContain(id!);
      expect(pretty.stdout + pretty.stderr).toContain("running");

      const json = runRodeo(["state", "--json", "--port", String(PORT)]);
      const snap = JSON.parse(json.stdout);
      const run = (snap.processes ?? []).find((p: any) => p.executionId === id);
      expect(run).toBeTruthy();
      // Default route resolves to edit/edit/plugin.
      expect(run.mode).toBe("edit");
      expect(run.domKind).toBe("edit");
      expect(run.context).toBe("plugin");
      expect(run.domId).toBeTruthy();
      expect(run.studioId).toBe(snap.studios[0].studioId);
    } finally {
      scriptProc.kill();
      await scriptProc.exited;
    }
  });

  it("pins a run to a DOM via --dom-id (unique prefix ok)", () => {
    const json = runRodeo(["state", "--json", "--port", String(PORT)]);
    const domId: string = JSON.parse(json.stdout).studios[0].editDomId;
    expect(domId).toBeTruthy();

    // Full id.
    const full = runRodeo([
      "run", "--port", String(PORT), "--dom-id", domId,
      "--show-return", "--source", "return 'pinned'",
    ]);
    expect(full.ok).toBe(true);
    expect(full.stdout + full.stderr).toContain("pinned");

    // 8-char prefix (as shown in the state DOMS table) resolves the same DOM.
    const prefix = runRodeo([
      "run", "--port", String(PORT), "--dom-id", domId.slice(0, 8),
      "--show-return", "--source", "return 'prefix'",
    ]);
    expect(prefix.ok).toBe(true);
    expect(prefix.stdout + prefix.stderr).toContain("prefix");
  });

  it("rejects --dom-id combined with routing flags", () => {
    const json = runRodeo(["state", "--json", "--port", String(PORT)]);
    const domId: string = JSON.parse(json.stdout).studios[0].editDomId;
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", domId, "--mode", "run",
      "--source", "return 1",
    ]);
    expect(result.ok).toBe(false);
  });

  it("--context elevated composes with --dom-id", () => {
    const json = runRodeo(["state", "--json", "--port", String(PORT)]);
    const domId: string = JSON.parse(json.stdout).studios[0].editDomId;
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", domId, "--context", "elevated",
      "--show-return", "--source", "return tostring(DebuggerManager())",
    ]);
    // Elevated needs StudioMCP; assert it at least didn't reject at parse time.
    expect(result.stdout + result.stderr).not.toContain("mode/dom don't apply");
  });

  it("--dom edit routes to the edit DOM", () => {
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom", "edit",
      "--show-return", "--source", "return game:GetService('RunService'):IsEdit()",
    ]);
    expect(result.ok).toBe(true);
    expect(result.stdout + result.stderr).toContain("true");
  });

  it("--context server without --mode is rejected (mode never inferred)", () => {
    // mode defaults to edit; (edit, server) has no server DOM, so this fails at
    // validation rather than silently transitioning the studio to run mode.
    const result = runRodeo([
      "run", "--port", String(PORT), "--context", "server",
      "--source", "return 1",
    ]);
    expect(result.ok).toBe(false);
    expect(result.stdout + result.stderr).toContain("edit");
  });
});

// Issue #14: --dom-id could never pin a server or client DOM — the route
// check ran before the pin rule and rejected (edit mode, client DOM) — so the
// only pins that worked were plugin/elevated runs on the edit DOM.
describe("--dom-id pins play DOMs (CLI)", () => {
  const PORT = 47510;
  // Studios this suite launches are reaped by session, never by pattern.
  const STARTED = Date.now();
  let bg: BackgroundProcess;
  let serverDom = "";
  let clientDom = "";

  beforeAll(async () => {
    bg = spawnBackground([
      "run", "--port", String(PORT), "--place", "--mode", "play", "--context", "client",
    ]);
    const client = await RodeoClient.connect(`http://localhost:${PORT}`, { readyTimeoutMs: 60_000 });
    try {
      const deadline = Date.now() + 90_000;
      while (Date.now() < deadline) {
        const state = await client.getState().catch(() => null) as
          { studios?: Array<{ sessionId?: string | null; doms: Array<{ domId: string; domKind: string }> }> } | null;
        const studio = (state?.studios ?? []).find((s) => s.sessionId);
        serverDom = studio?.doms.find((d) => d.domKind === "server")?.domId ?? "";
        clientDom = studio?.doms.find((d) => d.domKind === "client")?.domId ?? "";
        if (serverDom && clientDom) return;
        await Bun.sleep(250);
      }
      throw new Error(`timed out waiting for the play session's server and client DOMs on port ${PORT}`);
    } finally {
      await client.close();
    }
  }, 180_000);
  afterAll(async () => {
    bg?.kill();
    await bg?.exited;
    killLaunchedStudios(PORT, STARTED);
  });

  it("pins a run to the client DOM at --context client", () => {
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", clientDom, "--context", "client",
      "--show-return", "--source", "return game:GetService('Players').LocalPlayer ~= nil",
    ]);
    expect(result.ok, result.stderr).toBe(true);
    expect(result.stdout).toContain("true");
  });

  it("pins a run to the server DOM at --context server (unique prefix ok)", () => {
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", serverDom.slice(0, 8), "--context", "server",
      "--show-return", "--source", "return game:GetService('RunService'):IsServer()",
    ]);
    expect(result.ok, result.stderr).toBe(true);
    expect(result.stdout).toContain("true");
  });

  it("pins a plugin-context run to the client DOM", () => {
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", clientDom,
      "--show-return", "--source", "return game:GetService('RunService'):IsClient()",
    ]);
    expect(result.ok, result.stderr).toBe(true);
    expect(result.stdout).toContain("true");
  });

  it("rejects a context the pinned DOM cannot host, naming its kind", () => {
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", clientDom, "--context", "server",
      "--source", "return 1",
    ]);
    expect(result.ok).toBe(false);
    expect(result.stdout + result.stderr).toContain("context server cannot run on the pinned DOM, which is a client DOM");
  });

  it("still rejects --mode with --dom-id", () => {
    const result = runRodeo([
      "run", "--port", String(PORT), "--dom-id", clientDom, "--mode", "play", "--context", "client",
      "--source", "return 1",
    ]);
    expect(result.ok).toBe(false);
    expect(result.stdout + result.stderr).toContain("mode/dom don't apply");
  });
});
