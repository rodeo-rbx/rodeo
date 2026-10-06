//! Thin JSON-RPC wrappers over `rodeo __spawn_canonical_client`.
//!
//! Public API is preserved 1:1 from the pre-daemon client so `tests/`
//! needs no changes. All Studio lifecycle / DOM discovery / runCode streaming
//! logic lives in the `rodeo-client` Rust crate; this file is just handle
//! plumbing + a runCode-to-final-RunResult collector.

import { Daemon } from "./daemon.js";
import type { DomRunCodeOpts, LogFilter, RunCodeOpts, RunResult } from "./run.js";

export type { DomRunCodeOpts, LogFilter, RunCodeOpts, RunResult };

// ---------------------------------------------------------------------------
// Shared runCode plumbing (all three tiers: client / studio / dom)
// ---------------------------------------------------------------------------

/// Process the source, open a stream, and collect the final RunResult. `method`
/// selects the tier (`client.runCode` / `studio.runCode` / `dom.runCode`);
/// `target` carries the tier's identifier (`{}` / `{studioHandle}` / `{domId}`).
async function daemonRunCode(
  daemon: Daemon,
  method: string,
  target: Record<string, unknown>,
  opts: RunCodeOpts,
): Promise<RunResult> {
  // Process source via rodeo __process_source (bundle + shims + ensure_return).
  const processed = processSource({ source: opts.source, file: opts.file, sourcemap: opts.sourcemap });

  // Local tag for the default profile-dir name only — NOT the run id (the
  // master mints that; it comes back on the result as `executionId`).
  const profileTag = crypto.randomUUID();
  const profileDir = opts.profile !== undefined
    ? (opts.profile || `.rodeo/.temp/profiles/${profileTag}`)
    : undefined;

  // Client-allocated streamId: we register the callback BEFORE sending the
  // request, so notifications can arrive at any time (even before the RPC
  // response) without being lost. Matches the LSP progress-token pattern.
  const streamId = crypto.randomUUID();

  let bufferedOutput = "";
  let resolveRun!: (r: RunResult) => void;
  let rejectRun!: (e: Error) => void;
  const runPromise = new Promise<RunResult>((res, rej) => { resolveRun = res; rejectRun = rej; });

  daemon.registerStream(streamId, (m, params) => {
    if (m === "stream.data") {
      const kind = params.kind as string;
      if (kind === "stdout" || kind === "stderr") {
        bufferedOutput += String(params.chunk ?? "");
      }
    } else if (m === "stream.done") {
      daemon.unregisterStream(streamId);
      const r = (params.result ?? {}) as {
        ok?: boolean; output?: string; exitCode?: number;
        executionId?: string | null; returnValue?: string | null; error?: string | null;
      };
      let parsedReturn: unknown = undefined;
      if (typeof r.returnValue === "string" && r.returnValue.length > 0) {
        try { parsedReturn = JSON.parse(r.returnValue); } catch { parsedReturn = undefined; }
      }
      resolveRun({
        ok: r.ok ?? false,
        output: (r.output && r.output.length > 0) ? r.output : bufferedOutput,
        exitCode: r.exitCode ?? 0,
        executionId: r.executionId ?? undefined,
        return: parsedReturn,
        error: r.error ?? undefined,
      });
    } else if (m === "stream.error") {
      daemon.unregisterStream(streamId);
      rejectRun(new Error(String(params.error ?? "run failed")));
    }
  });

  // Routing fields are present only on the routed tiers (RunCodeOpts).
  const route = opts as RunCodeOpts;
  try {
    await daemon.request<{ streamId: string }>(method, {
      ...target,
      streamId,
      source: processed.script,
      mode: route.mode ?? null,
      domKind: route.domKind ?? null,
      context: opts.context ?? null,
      showReturn: opts.showReturn ?? false,
      reloadRequires: opts.reloadRequires ?? false,
      verbose: opts.verbose ?? false,
      scriptArgs: opts.scriptArgs ?? [],
      profileDir: profileDir ?? null,
      returnFile: opts.returnFile ?? null,
      instancePath: processed.instancePath ?? null,
      scriptPath: processed.scriptPath ?? null,
      logFilter: opts.logFilter ?? null,
    });
  } catch (e) {
    daemon.unregisterStream(streamId);
    throw e;
  }

  return await runPromise;
}

// ---------------------------------------------------------------------------
// Shape of daemon responses
// ---------------------------------------------------------------------------

type DomSnapshotDTO = {
  domId: string;
  backendId: string;
  mode: string;
  domKind: string;
  sessionGuid?: string | null;
  placeId: number;
  gameName: string;
  connected: boolean;
  activeRuns: number;
  userName?: string | null;
  userId?: number | null;
};

// Minimal per-DOM entry on a studio in the studio-first snapshot.
type StudioDomEntryDTO = {
  domId: string;
  domKind: string;
  userName?: string | null;
  userId?: number | null;
};

type StudioDTO = {
  studioId: string;
  backendId: string;
  /** Launch session identity; absent for manually-connected studios. */
  sessionId?: string | null;
  placeName: string;
  placeId: number;
  status: string;
  studioMode: string;
  /** The root edit DOM's id; absent if no edit DOM is connected. */
  editDomId?: string | null;
  doms: StudioDomEntryDTO[];
};

type StateSnapshotDTO = {
  backends?: BackendInfoDTO[];
  processes?: ProcessInfoDTO[];
  studios: StudioDTO[];
};

type BackendInfoDTO = {
  id: string;
  kind: string;
  name: string;
};

type ProcessInfoDTO = Record<string, unknown>;

// ---------------------------------------------------------------------------
// Dom
// ---------------------------------------------------------------------------

export class Dom {
  readonly domId: string;
  readonly backendId: string;
  readonly mode: string;
  readonly domKind: string;
  readonly sessionGuid: string | undefined;
  readonly placeId: number;
  readonly gameName: string;
  readonly connected: boolean;
  readonly activeRuns: number;
  readonly userName: string | undefined;
  readonly userId: number | undefined;
  protected daemon: Daemon;

  constructor(snap: DomSnapshotDTO, daemon: Daemon) {
    this.domId = snap.domId;
    this.backendId = snap.backendId ?? "";
    this.mode = snap.mode ?? "";
    this.domKind = snap.domKind ?? "";
    this.sessionGuid = snap.sessionGuid ?? undefined;
    this.placeId = Number(snap.placeId ?? 0);
    this.gameName = snap.gameName ?? "";
    this.connected = snap.connected;
    this.activeRuns = snap.activeRuns;
    this.userName = snap.userName ?? undefined;
    this.userId = snap.userId ?? undefined;
    this.daemon = daemon;
  }

  /** Run on THIS DOM (pinned — no routing). Only `context` applies. */
  async runCode(opts: DomRunCodeOpts): Promise<RunResult> {
    return daemonRunCode(this.daemon, "dom.runCode", { domId: this.domId }, opts);
  }
}

// ---------------------------------------------------------------------------
// RodeoClient
// ---------------------------------------------------------------------------

export type ConnectOpts = {
  /** Max time to wait for the server to come up. Default 30000ms. */
  readyTimeoutMs?: number;
  /** Poll interval while waiting for the server. Default 200ms. */
  readyPollMs?: number;
};

/** Result of the daemon's `client.checkVersion` handshake. */
type VersionCheck = { ok: boolean; server: string; client: string; message?: string };

export class RodeoClient {
  readonly daemon: Daemon;
  /** Build id of the master this client connected to (`version[+git sha]`).
   *  Equals the build id of the `rodeo` binary the daemon was spawned from,
   *  since `connect` refuses a mismatch. */
  serverVersion = "";

  private constructor(url?: string) {
    if (url === undefined) {
      // No port: the daemon resolves it itself (RODEO_PORT, then 44872).
      this.daemon = new Daemon("localhost", undefined);
    } else {
      const { host, port } = parseUrl(url);
      this.daemon = new Daemon(host, port);
    }
  }

  /** Connect to a running `rodeo serve` and block until it's healthy.
   *  Omit `url` to use the CLI's own port resolution: the `RODEO_PORT`
   *  environment variable, then 44872 — the same serve `rodeo run` would use
   *  from this directory.
   *  Throws after `readyTimeoutMs` (default 30s) if the server never responds,
   *  and immediately if the master is a different rodeo build than the
   *  `rodeo` binary on PATH (set RODEO_SKIP_VERSION_CHECK=1 to override). */
  static async connect(url?: string, opts: ConnectOpts = {}): Promise<RodeoClient> {
    const timeoutMs = opts.readyTimeoutMs ?? 30_000;
    const pollMs = opts.readyPollMs ?? 200;
    const where = url ?? "the default port (RODEO_PORT, else 44872)";
    const client = new RodeoClient(url);
    const deadline = Date.now() + timeoutMs;
    let healthy = false;
    while (Date.now() < deadline) {
      try {
        healthy = await client.daemon.request<boolean>("client.isHealthy");
        if (healthy) break;
      } catch {
        // server not up yet — retry until deadline
      }
      await new Promise((r) => setTimeout(r, pollMs));
    }
    if (!healthy) {
      await client.daemon.shutdown().catch(() => {});
      throw new Error(`RodeoClient.connect: timed out after ${timeoutMs}ms waiting for rodeo at ${where}`);
    }
    const check = await client.daemon.request<VersionCheck>("client.checkVersion");
    if (!check.ok) {
      await client.daemon.shutdown().catch(() => {});
      throw new Error(`RodeoClient.connect: ${check.message ?? "version mismatch"}`);
    }
    client.serverVersion = check.server;
    return client;
  }

  /** Call this in afterAll / teardown to shut down the daemon subprocess. */
  async close(): Promise<void> {
    await this.daemon.shutdown();
  }

  // State & discovery

  async getState(): Promise<StateSnapshotDTO> {
    return await this.daemon.request<StateSnapshotDTO>("client.getState");
  }

  // Process management

  async listProcesses(): Promise<ProcessInfoDTO[]> {
    return await this.daemon.request<ProcessInfoDTO[]>("client.listProcesses");
  }

  async kill(executionId: string): Promise<void> {
    await this.daemon.request<null>("client.kill", { executionId });
  }

  // Backend discovery

  async listBackends(kind?: string): Promise<BackendInfoDTO[]> {
    return await this.daemon.request<BackendInfoDTO[]>("client.listBackends", kind ? { kind } : {});
  }

  async getLocalStudio(): Promise<StudioBackend> {
    const resp = await this.daemon.request<{ backendHandle: string; info: { id: string; name: string } }>("client.getLocalStudio");
    return new StudioBackend(resp.backendHandle, resp.info, this.daemon);
  }

  /** Select a studio BACKEND (a machine running a studio backend) by id
   *  prefix or exact name — not a studio instance. */
  async getBackend(idOrName: string): Promise<StudioBackend> {
    const resp = await this.daemon.request<{ backendHandle: string; info: { id: string; name: string } }>("client.getBackend", { idOrName });
    return new StudioBackend(resp.backendHandle, resp.info, this.daemon);
  }

  /** Serve-wide run tier: the master routes by mode/domKind/context across all
   *  connected DOMs. Use `Studio.runCode` to scope to one studio, or
   *  `Dom.runCode` to pin. */
  async runCode(opts: RunCodeOpts): Promise<RunResult> {
    return daemonRunCode(this.daemon, "client.runCode", {}, opts);
  }

  // DOM discovery

  async getDoms(): Promise<Dom[]> {
    const state = await this.getState();
    return domsFromStudios(state, this.daemon);
  }

  async getDom(domId: string): Promise<Dom> {
    const doms = await this.getDoms();
    const found = doms.find((v) => v.domId === domId);
    if (!found) throw new Error(`dom '${domId}' not found`);
    return found;
  }
}

// ---------------------------------------------------------------------------
// StudioBackend / options
// ---------------------------------------------------------------------------

/** Bound on a call that waits for Studio state to change (a Studio to
 *  connect, a mode transition to produce its DOMs, a client to join).
 *  Omit to wait indefinitely. */
export type WaitOpts = {
  timeoutMs?: number;
};

export type OpenPlaceOpts = WaitOpts & {
  placeId: number;
  fflags?: string[];
  background?: boolean;
  profile?: boolean;
  /** Allow-list of Studio dock widgets to keep visible ("none" = hide all;
   *  comma list keeps those). Everything unlisted (panels, ribbon, command
   *  bar) is hidden. Omit for a normal Studio. */
  showWidgets?: string;
  /** Studio process survives `studio.close()` and rodeo serve exit. Caller
   *  owns the OS lifecycle from there — the Studio is no longer in rodeo's
   *  managed set after close. */
  detached?: boolean;
};

export type OpenFileOpts = WaitOpts & {
  fflags?: string[];
  background?: boolean;
  profile?: boolean;
  /** Allow-list of Studio dock widgets to keep visible ("none" = hide all;
   *  comma list keeps those). Everything unlisted (panels, ribbon, command
   *  bar) is hidden. Omit for a normal Studio. */
  showWidgets?: string;
  detached?: boolean;
};

export type OpenOpts = WaitOpts & {
  fflags?: string[];
  background?: boolean;
  profile?: boolean;
  /** Allow-list of Studio dock widgets to keep visible ("none" = hide all;
   *  comma list keeps those). Everything unlisted (panels, ribbon, command
   *  bar) is hidden. Omit for a normal Studio. */
  showWidgets?: string;
  detached?: boolean;
};

export class StudioBackend {
  readonly id: string;
  readonly name: string;
  private handle: string;
  private daemon: Daemon;

  constructor(handle: string, info: { id: string; name: string }, daemon: Daemon) {
    this.handle = handle;
    this.id = info.id;
    this.name = info.name;
    this.daemon = daemon;
  }

  async open(opts: OpenOpts = {}): Promise<Studio> {
    return this.launch("studio.open", {
      backendHandle: this.handle,
      fflags: opts.fflags ?? [],
      background: opts.background ?? false,
      profile: opts.profile ?? false,
      showWidgets: opts.showWidgets,
      detached: opts.detached ?? false,
      timeoutMs: opts.timeoutMs,
    });
  }

  async openPlace(opts: OpenPlaceOpts): Promise<Studio> {
    return this.launch("studio.openPlace", {
      backendHandle: this.handle,
      placeId: opts.placeId,
      fflags: opts.fflags ?? [],
      background: opts.background ?? false,
      profile: opts.profile ?? false,
      showWidgets: opts.showWidgets,
      detached: opts.detached ?? false,
      timeoutMs: opts.timeoutMs,
    });
  }

  async openFile(path: string, opts: OpenFileOpts = {}): Promise<Studio> {
    return this.launch("studio.openFile", {
      backendHandle: this.handle,
      path,
      fflags: opts.fflags ?? [],
      background: opts.background ?? false,
      profile: opts.profile ?? false,
      showWidgets: opts.showWidgets,
      detached: opts.detached ?? false,
      timeoutMs: opts.timeoutMs,
    });
  }

  private async launch(method: string, params: Record<string, unknown>): Promise<Studio> {
    const resp = await this.daemon.request<{
      studioHandle: string;
      sessionGuid: string;
      editDomId: string;
    }>(method, params);
    const studio = new Studio(resp.studioHandle, resp.sessionGuid, this.id, this.daemon);
    // Populate editDom by querying DOMs — the daemon guaranteed it's connected.
    const doms = await studio.getDoms();
    const editDom = doms.find((v) => v.domId === resp.editDomId);
    if (!editDom) {
      throw new Error(`edit DOM ${resp.editDomId} not found in studio ${resp.studioHandle}`);
    }
    studio.editDom = editDom;
    return studio;
  }
}

// ---------------------------------------------------------------------------
// Studio
// ---------------------------------------------------------------------------

export class Studio {
  readonly sessionGuid: string;
  readonly backendId: string;
  /** Opaque daemon handle for this Studio. Used by RPCs keyed on the Studio
   *  (setMode / getDoms / startMultiplayerTest / save / close). */
  readonly studioHandle: string;
  private daemon: Daemon;

  editDom!: Dom;
  serverDom: Dom | null = null;
  clientDom: Dom | null = null;

  constructor(handle: string, sessionGuid: string, backendId: string, daemon: Daemon) {
    this.studioHandle = handle;
    this.sessionGuid = sessionGuid;
    this.backendId = backendId;
    this.daemon = daemon;
  }

  /** Session-scoped run tier: the master routes by mode/domKind/context among
   *  THIS studio's DOMs (auto-transitioning its mode). */
  async runCode(opts: RunCodeOpts): Promise<RunResult> {
    return daemonRunCode(this.daemon, "studio.runCode", { studioHandle: this.studioHandle }, opts);
  }

  /** Transition the Studio and wait for the DOMs that mode implies to
   *  connect. Waits indefinitely unless `opts.timeoutMs` is given. */
  async setMode(mode: string, opts: WaitOpts = {}): Promise<void> {
    const resp = await this.daemon.request<{
      serverDomId?: string | null;
      clientDomId?: string | null;
    }>("studio.setMode", { studioHandle: this.studioHandle, mode, timeoutMs: opts.timeoutMs });

    if (mode === "edit") {
      this.serverDom = null;
      this.clientDom = null;
      return;
    }

    // Look up DOM objects by ID from the studio-first snapshot.
    const state = await this.daemon.request<StateSnapshotDTO>("client.getState");
    const byId = (id: string | null | undefined) => {
      if (!id) return null;
      const snap = findDomSnapshot(state, id);
      return snap ? new Dom(snap, this.daemon) : null;
    };

    this.serverDom = byId(resp.serverDomId);
    this.clientDom = (mode === "test" || mode === "play") ? byId(resp.clientDomId) : null;
  }

  async getMode(): Promise<string> {
    return await this.daemon.request<string>("studio.getMode", { studioHandle: this.studioHandle });
  }

  async save(): Promise<{ saved: boolean; path?: string; error?: string }> {
    return await this.daemon.request<{ saved: boolean; path?: string; error?: string }>(
      "studio.save", { studioHandle: this.studioHandle },
    );
  }

  async close(): Promise<void> {
    await this.daemon.request<null>("studio.close", { studioHandle: this.studioHandle });
  }

  async getDoms(): Promise<Dom[]> {
    // Studio-first snapshot: this Studio's DOMs live under its entry in
    // state.studios[].doms (keyed by studioId).
    const state = await this.daemon.request<StateSnapshotDTO>("client.getState");
    const studio = (state.studios ?? []).find((s) => s.sessionId === this.sessionGuid);
    if (!studio) return [];
    return (studio.doms ?? []).map((sv) => new Dom(buildDomSnapshot(studio, sv), this.daemon));
  }

  /** Start an in-Studio multiplayer test with `numPlayers` client DataModels
   *  spawned UP FRONT (a single `ExecuteMultiplayerTestAsync(numPlayers)`).
   *  Access the spawned players with `server.getPlayer(i)`. Requires this Studio
   *  to be open: the server DataModel is spawned from its edit DataModel.
   *  `close()` tears down the whole test at once.
   *
   *  Prefer requesting the client count here over growing a running session
   *  later with `connectClient()`: `StudioTestService:AddPlayers` crashes the
   *  Studio server on some engine versions (observed on 0.726: SIGSEGV the
   *  moment a client is added to a running test). Defaults to 0 clients. */
  async startMultiplayerTest(numPlayers: number = 0, opts: WaitOpts = {}): Promise<MultiplayerTestServer> {
    const resp = await this.daemon.request<{
      mpHandle: string;
      serverDomId: string;
      clientDomIds?: string[];
    }>("studio.startMultiplayerTest", { studioHandle: this.studioHandle, numPlayers, timeoutMs: opts.timeoutMs });

    const state = await this.daemon.request<StateSnapshotDTO>("client.getState");
    const snap = findDomSnapshot(state, resp.serverDomId);
    if (!snap) throw new Error(`server DOM ${resp.serverDomId} not found`);
    const players = (resp.clientDomIds ?? []).map((clientDomId) => {
      const csnap = findDomSnapshot(state, clientDomId);
      if (!csnap) throw new Error(`client DOM ${clientDomId} not found`);
      return new MultiplayerTestClient(resp.mpHandle, csnap, this.daemon);
    });
    return new MultiplayerTestServer(resp.mpHandle, snap, this.daemon, players);
  }
}

// ---------------------------------------------------------------------------
// MultiplayerTestServer / MultiplayerTestClient — Dom-with-extras. Same data
// plane (.runCode, .domId, ...) as any Dom, plus the test lifecycle keyed by the
// daemon-side `mpHandle`.
// ---------------------------------------------------------------------------

export class MultiplayerTestServer extends Dom {
  private mpHandle: string;
  // Players (client DataModels) spawned up front by startMultiplayerTest(numPlayers).
  private readonly players: MultiplayerTestClient[];

  constructor(mpHandle: string, snap: DomSnapshotDTO, daemon: Daemon, players: MultiplayerTestClient[] = []) {
    super(snap, daemon);
    this.mpHandle = mpHandle;
    this.players = players;
  }

  /** The players (client DataModels) spawned up front by
   *  `startMultiplayerTest(numPlayers)`, in spawn order. Clients added later via
   *  `connectClient()` are not included. Returns a copy. */
  getPlayers(): MultiplayerTestClient[] {
    return [...this.players];
  }

  /** Connect one more client player to a *running* test; returns its handle.
   *  WARNING: on some Studio engine versions (0.726) adding a player to a
   *  running multiplayer test crashes the server (SIGSEGV). Prefer passing the
   *  client count to `startMultiplayerTest(numPlayers)` up front. */
  async connectClient(opts: WaitOpts = {}): Promise<MultiplayerTestClient> {
    const resp = await this.daemon.request<{ clientDomId: string }>(
      "mp.connectClient", { mpHandle: this.mpHandle, timeoutMs: opts.timeoutMs },
    );
    const state = await this.daemon.request<StateSnapshotDTO>("client.getState");
    const snap = findDomSnapshot(state, resp.clientDomId);
    if (!snap) throw new Error(`client DOM ${resp.clientDomId} not found`);
    return new MultiplayerTestClient(this.mpHandle, snap, this.daemon);
  }

  /** End the multiplayer test (tears down the server + all clients). */
  async close(): Promise<void> {
    await this.daemon.request<null>("mp.close", { mpHandle: this.mpHandle });
  }
}

export class MultiplayerTestClient extends Dom {
  private mpHandle: string;

  constructor(mpHandle: string, snap: DomSnapshotDTO, daemon: Daemon) {
    super(snap, daemon);
    this.mpHandle = mpHandle;
  }

  /** Disconnect this client from the test. */
  async disconnect(): Promise<void> {
    await this.daemon.request<null>(
      "mp.disconnectClient", { mpHandle: this.mpHandle, domId: this.domId },
    );
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// Build a full DomSnapshotDTO from a studio-first snapshot's parent studio +
// its minimal dom entry. The studio entries carry only domId/domKind/userName/userId, so
// the remaining fields are sourced from the owning studio.
function buildDomSnapshot(studio: StudioDTO, sv: StudioDomEntryDTO): DomSnapshotDTO {
  return {
    domId: sv.domId,
    domKind: sv.domKind,
    mode: sv.domKind === "edit" ? "edit" : studio.studioMode,
    backendId: studio.backendId,
    sessionGuid: studio.sessionId ?? undefined,
    placeId: studio.placeId,
    gameName: studio.placeName,
    connected: true,
    activeRuns: 0,
    userName: sv.userName ?? undefined,
    userId: sv.userId ?? undefined,
  };
}

// Build every Dom across all studios in a studio-first snapshot.
function domsFromStudios(state: StateSnapshotDTO, daemon: Daemon): Dom[] {
  const out: Dom[] = [];
  for (const studio of state.studios ?? []) {
    for (const sv of studio.doms ?? []) {
      out.push(new Dom(buildDomSnapshot(studio, sv), daemon));
    }
  }
  return out;
}

// Find a domId across state.studios[].doms and return the built DomSnapshotDTO,
// or undefined if no studio owns it.
function findDomSnapshot(state: StateSnapshotDTO, domId: string): DomSnapshotDTO | undefined {
  for (const studio of state.studios ?? []) {
    for (const sv of studio.doms ?? []) {
      if (sv.domId === domId) return buildDomSnapshot(studio, sv);
    }
  }
  return undefined;
}

function parseUrl(url: string): { host: string; port: number } {
  // Accept "http://host:port" or "host:port".
  const clean = url.replace(/^https?:\/\//, "");
  const [host, portStr] = clean.split(":");
  const port = parseInt(portStr ?? "", 10);
  if (!host || !portStr || Number.isNaN(port)) {
    throw new Error(`invalid url for RodeoClient: ${url}`);
  }
  return { host, port };
}

type ProcessedSource = {
  script: string;
  scriptPath?: string;
  instancePath?: string;
};

function processSource(opts: { source?: string; file?: string; sourcemap?: string }): ProcessedSource {
  const args = ["__process_source"];
  if (opts.file) {
    args.push(opts.file);
    if (opts.sourcemap) args.push("--sourcemap", opts.sourcemap);
  } else if (opts.source) {
    args.push("--source", opts.source);
  } else {
    throw new Error("runCode requires either `source` or `file`");
  }
  const proc = Bun.spawnSync(["rodeo", ...args]);
  if (proc.exitCode !== 0) {
    const stderr = proc.stderr.toString().trim();
    throw new Error(`source processing failed: ${stderr}`);
  }
  return JSON.parse(proc.stdout.toString()) as ProcessedSource;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
