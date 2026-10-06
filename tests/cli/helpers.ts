// End-to-end CLI test helpers. Unlike tests/api/ which drives the TS
// RodeoClient library, tests/cli/ spawns `rodeo` as a subprocess and
// asserts on its stdout/stderr/exit. `makeCliRunFn` bridges those subprocess
// invocations into the RunFn signature used by tests/utils/executionTests.ts
// so the shared factories can run against the CLI unchanged.

import type { Subprocess } from "bun";
import { existsSync, readdirSync, readFileSync, statSync, unlinkSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { RodeoClient } from "../../rodeo-client-ts/src/index.js";
import type { RunCodeOpts, RunResult } from "../../rodeo-client-ts/src/run.js";

// Hard cap for any synchronous `rodeo ...` subprocess. Without a timeout, a
// crashed/hung Studio leaves Bun.spawnSync blocked forever — `bun test
// --timeout` can't cancel synchronous calls, so the whole suite stalls. Set
// it to match the suite's `--timeout` so the spawn returns at the same
// wall-clock the test would have failed at anyway.
const SUBPROCESS_TIMEOUT_MS = 60_000;

export type ProcResult = {
  ok: boolean;
  stdout: string;
  stderr: string;
  exitCode: number;
};

export function runRodeo(args: string[], opts: { timeout?: number } = {}): ProcResult {
  const timeout = opts.timeout ?? SUBPROCESS_TIMEOUT_MS;
  const proc = Bun.spawnSync(["rodeo", ...args], { timeout });
  const stdout = proc.stdout.toString();
  let stderr = proc.stderr.toString();
  if (proc.signalCode) {
    stderr += `\n[runRodeo: killed after ${timeout}ms via ${proc.signalCode}]`;
  }
  return {
    ok: proc.exitCode === 0,
    stdout,
    stderr,
    exitCode: proc.exitCode ?? -1,
  };
}

// Cross-platform process matchers. Unix uses pgrep/pkill with `-f` (match the
// full command line); Windows has neither, so shell to PowerShell's CIM process
// query, whose `CommandLine` field gives the same match surface. `pattern` is a
// regex in both worlds (pgrep -f and PowerShell -match both take regex).
//
// Every Windows query excludes its own powershell (`$_.ProcessId -ne $PID`):
// the query's command line contains the pattern text, which itself matches the
// pattern as a regex. Without the exclusion, processMatches always returns
// true (it counts itself), killMatching can Stop-Process its own host and
// abort the pipeline before later matches die, and pidsMatching captures a
// transient powershell pid that Windows then reuses for an unrelated process
// inside waitForPidsGone's window — a phantom "survivor" (observed as a flaky
// false fail in processCleanup). pgrep/pkill on macOS already exclude
// themselves.
const IS_WINDOWS = process.platform === "win32";

/** True if any running process's command line matches `pattern`. */
export function processMatches(pattern: string): boolean {
  if (IS_WINDOWS) {
    const r = Bun.spawnSync([
      "powershell", "-NoProfile", "-Command",
      `@(Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match '${pattern}' }).Count`,
    ]);
    return parseInt(r.stdout.toString().trim() || "0", 10) > 0;
  }
  return Bun.spawnSync(["pgrep", "-f", pattern]).exitCode === 0;
}

/** Force-kill every process whose command line matches `pattern`. SIGKILL on
 *  Unix: Studio ignores SIGTERM, so a plain `pkill` left the Studios this is
 *  meant to reap alive. */
export function killMatching(pattern: string): void {
  if (IS_WINDOWS) {
    Bun.spawnSync([
      "powershell", "-NoProfile", "-Command",
      `Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match '${pattern}' } | ` +
        `ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }`,
    ]);
    return;
  }
  Bun.spawnSync(["pkill", "-9", "-f", pattern]);
}

/** Session ids of the Studios a serve on `masterPort` launched since
 *  `sinceMs`: each launch's RunScript bootstrap, written to the serve's
 *  .rodeo/.temp (the repo root for tests), stamps its backend port. Lets a
 *  test reap exactly its own Studios; the developer's, other sessions' and
 *  other projects' Studios are never touched. */
export function launchedSessions(masterPort: number, sinceMs: number): string[] {
  const dir = join(".rodeo", ".temp");
  if (!existsSync(dir)) return [];
  return readdirSync(dir).flatMap((name) => {
    const session = /^rodeo-bootstrap-([0-9a-f-]+)\.luau$/.exec(name)?.[1];
    if (!session) return [];
    const path = join(dir, name);
    const ours = statSync(path).mtimeMs >= sinceMs && readFileSync(path, "utf8").includes(`"rodeoPort", ${masterPort + 1})`);
    return ours ? [session] : [];
  });
}

/** Force-kill the Studios a serve on `masterPort` launched since `sinceMs`
 *  (see launchedSessions), e.g. `--detach` Studios that outlive their serve. */
export function killLaunchedStudios(masterPort: number, sinceMs: number): void {
  for (const session of launchedSessions(masterPort, sinceMs)) killMatching(`rodeo-bootstrap-${session}`);
}

/** PIDs of processes whose command line matches `pattern`. */
export function pidsMatching(pattern: string): number[] {
  const r = IS_WINDOWS
    ? Bun.spawnSync([
        "powershell", "-NoProfile", "-Command",
        `Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match '${pattern}' } | ForEach-Object { $_.ProcessId }`,
      ])
    : Bun.spawnSync(["pgrep", "-f", pattern]);
  return r.stdout.toString().split(/\s+/).map((s) => parseInt(s, 10)).filter((n) => n > 0);
}

/**
 * Wait for every pid in `pids` to exit, up to `timeoutMs`; returns true if all
 * are gone. This is EVENT-DRIVEN, not name-polling: Windows blocks on the
 * process handles via `Wait-Process` (woken the instant they exit, one call —
 * not a slow per-tick CIM query), and other platforms early-return off the
 * cheap native `process.kill(pid, 0)` liveness check. Empty `pids` ⇒ true.
 */
export async function waitForPidsGone(pids: number[], timeoutMs: number): Promise<boolean> {
  if (pids.length === 0) return true;
  if (IS_WINDOWS) {
    const sec = Math.max(1, Math.ceil(timeoutMs / 1000));
    const idList = pids.join(",");
    // The post-wait liveness check must ignore ZOMBIES: a freshly-killed
    // process lingers in .NET's process list (which Get-Process uses) for as
    // long as anything holds a handle to it — Studio's crash handler holds
    // one for a few seconds after Studio dies. Wait-Process correctly returns
    // immediately (the zombie's handle is signaled), but a bare Get-Process
    // then reports the corpse as alive and this helper returned a false
    // "survivor" (~50% processCleanup flake). `HasExited` reads the handle's
    // signaled state, so it's true for zombies.
    const r = Bun.spawnSync([
      "powershell", "-NoProfile", "-Command",
      `Wait-Process -Id ${idList} -Timeout ${sec} -ErrorAction SilentlyContinue; ` +
        `if (@(${idList}) | Where-Object { $p = Get-Process -Id $_ -ErrorAction SilentlyContinue; $p -and -not $p.HasExited }) { exit 1 } else { exit 0 }`,
    ]);
    return r.exitCode === 0;
  }
  const alive = () => pids.some((pid) => { try { process.kill(pid, 0); return true; } catch { return false; } });
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!alive()) return true;
    await Bun.sleep(200);
  }
  return !alive();
}

export type BackgroundProcess = {
  kill: () => void;
  exited: Promise<number>;
  pid: number;
};

// `rodeo run` / `rodeo serve` accept --ppid; we pass our pid so the subprocess
// self-exits when bun dies, and its --ppid chain tears down master + backends
// + Studio. Without this, Bun.spawn children survive bun and leak (macOS has
// no parent-death signal).
export function spawnBackground(args: string[]): BackgroundProcess {
  const proc = Bun.spawn(["rodeo", ...args, "--ppid", String(process.pid)], {
    stderr: "inherit",
    stdout: "inherit",
    stdin: "ignore",
  }) as Subprocess;
  return {
    pid: proc.pid,
    kill: () => proc.kill(),
    exited: proc.exited,
  };
}

// Studio's local plugins folder, where each studio backend installs its own
// `rodeo-<build>-<port>.rbxm` (port = the backend's WebSocket port, master + 1).
export function pluginsDir(): string {
  if (IS_WINDOWS) return join(process.env.LOCALAPPDATA ?? "", "Roblox", "Plugins");
  return join(homedir(), "Documents", "Roblox", "Plugins");
}

// Build id of the `rodeo` on PATH (`rodeo --version` → "rodeo <build>").
export function cliBuildId(): string {
  return runRodeo(["--version"]).stdout.trim().replace(/^rodeo\s+/, "");
}

// The plugin file a serve on `masterPort` installs.
export function pluginFileFor(masterPort: number): string {
  return join(pluginsDir(), `rodeo-${cliBuildId()}-${masterPort + 1}.rbxm`);
}

// Polls `pred` until it holds or `timeoutMs` passes.
export async function waitUntil(pred: () => boolean, timeoutMs: number, what: string): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (pred()) return;
    await Bun.sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

// Waits until a master answers on `port`, then disconnects.
export async function waitForHealthy(port: number): Promise<void> {
  const client = await RodeoClient.connect(`http://localhost:${port}`);
  await client.close();
}

// Polls the master for a process in the requested state (e.g. "running",
// "done"). Returns the first matching run ID, or null on timeout.
// Replaces tests/utils/waitForProcess.luau.
export async function waitForProcess(
  port: number,
  state: string,
  timeoutMs = 30_000,
): Promise<string | null> {
  const client = await RodeoClient.connect(`http://localhost:${port}`);
  const start = Date.now();
  try {
    while (Date.now() - start < timeoutMs) {
      const procs = await client.listProcesses().catch(() => []);
      for (const p of procs as Array<{ executionId: string; state: string }>) {
        if (p.state === state) return p.executionId;
      }
      await Bun.sleep(200);
    }
    return null;
  } finally {
    await client.close();
  }
}

// Waits until at least one connected DOM shows up on the master. Used after
// spawnBackground(["run","--place",...]) to ensure Studio is ready before
// the first `rodeo run --source` call. Without this, parallel test files
// racing the studio-daemon's 4-slot pool can see `rodeo run` time out
// waiting for a DOM.
export async function waitForDom(port: number, timeoutMs = 60_000): Promise<void> {
  const client = await RodeoClient.connect(`http://localhost:${port}`);
  const start = Date.now();
  try {
    while (Date.now() - start < timeoutMs) {
      const vms = await client.getDoms().catch(() => []);
      if (vms.some((v) => v.connected)) return;
      await Bun.sleep(250);
    }
    throw new Error(`timed out waiting for DOM on port ${port}`);
  } finally {
    await client.close();
  }
}

// Waits until the Studio this port's serve launched (the one with a
// sessionId) has its edit DOM connected, and returns its snapshot. Stricter
// than waitForDom: every running backend's plugin also loads into hand-opened
// Studios, which register on this port too — one already sitting in the same
// place would satisfy "some DOM connected" before the launch has worked.
export type OwnedStudioSnap = {
  studioId: string;
  sessionId?: string | null;
  placeId: number | string;
  placeName: string;
  status: string;
  editDomId?: string | null;
  doms: Array<{ domId: string; domKind: string }>;
};

export async function waitForOwnedStudio(port: number, timeoutMs = 60_000): Promise<OwnedStudioSnap> {
  const client = await RodeoClient.connect(`http://localhost:${port}`);
  const start = Date.now();
  try {
    while (Date.now() - start < timeoutMs) {
      const state = await client.getState().catch(() => null) as { studios?: OwnedStudioSnap[] } | null;
      const owned = (state?.studios ?? []).find((s) => s.sessionId && s.editDomId);
      if (owned) return owned;
      await Bun.sleep(250);
    }
    throw new Error(`timed out waiting for the launched Studio's edit DOM on port ${port}`);
  } finally {
    await client.close();
  }
}

// The Studio this port's serve launched, when there is exactly one. Every
// running backend's plugin connects to every hand-opened Studio, so a harness
// backend can see Studios it did not launch — and an unpinned run picks any
// eligible DOM, including one in the developer's own open Studio. Callers
// that launch several Studios on one port manage targeting themselves.
async function ownedStudioId(port: number): Promise<string | undefined> {
  let client: RodeoClient | undefined;
  try {
    client = await RodeoClient.connect(`http://localhost:${port}`);
    const state = await client.getState();
    const owned = (state.studios ?? []).filter((s) => s.sessionId);
    return owned.length === 1 ? owned[0].studioId : undefined;
  } catch {
    return undefined;
  } finally {
    await client?.close();
  }
}

// Builds a RunFn backed by `rodeo run` subprocess. Lets the shared factories
// in tests/utils/executionTests.ts run end-to-end against the CLI binary.
// Runs are pinned with `--studio-id` to the Studio this port launched (see
// ownedStudioId), resolved once on first use.
export function makeCliRunFn(
  port: number,
): (opts: RunCodeOpts) => Promise<RunResult> {
  let pinned: string | undefined | null = null; // null = not yet resolved
  return async (opts: RunCodeOpts): Promise<RunResult> => {
    if (pinned === null) pinned = await ownedStudioId(port);
    const args: string[] = ["run", "--port", String(port)];
    if (pinned) args.push("--studio-id", pinned);

    if (opts.source !== undefined) args.push("--source", opts.source);
    if (opts.sourcemap !== undefined) args.push("--sourcemap", opts.sourcemap);
    if (opts.showReturn) args.push("--show-return");
    if (opts.reloadRequires) args.push("--reload-requires");
    if (opts.mode !== undefined) args.push("--mode", opts.mode);
    if (opts.domKind !== undefined) args.push("--dom", opts.domKind);
    if (opts.context !== undefined) args.push("--context", opts.context);

    // CLI subprocesses don't see the wire-level ExecutionDone, so to give
    // tests a `result.return` to assert against we shadow it via the same
    // `--return <path>` mechanism the CLI already supports: write to a temp
    // JSON file unless the caller already passed their own `returnFile`,
    // then parse it back into the JS `RunResult.return`. The temp file is
    // cleaned up regardless of the run outcome.
    let autoReturnFile: string | undefined;
    if (opts.returnFile === undefined) {
      autoReturnFile = join(tmpdir(), `rodeo-cli-return-${randomUUID()}.json`);
    }

    if (opts.logFilter) {
      if (opts.logFilter.enableWarn === false) args.push("--no-warn");
      if (opts.logFilter.enableError === false) args.push("--no-error");
      if (opts.logFilter.enableInfo === false) args.push("--no-info");
      if (opts.logFilter.enableOutput === false) args.push("--no-print");
      // enableLogs currently has no CLI toggle; factories that depend on it
      // should gate on --no-output as a combined disable. If a factory fails,
      // port the specific case inline rather than inventing a new flag.
      if (opts.logFilter.enableWarn === false &&
          opts.logFilter.enableError === false &&
          opts.logFilter.enableInfo === false &&
          opts.logFilter.enableOutput === false &&
          opts.logFilter.enableLogs === false) {
        args.push("--no-output");
      }
    }

    // --profile accepts an optional output dir; the CLI writes artifacts
    // directly to that path. Tests that need to inspect file bytes read
    // them from disk.
    if (opts.profile !== undefined) {
      args.push("--profile");
      if (opts.profile.length > 0) args.push(opts.profile);
    }

    if (opts.returnFile !== undefined) {
      args.push("--return", opts.returnFile);
    } else if (autoReturnFile !== undefined) {
      args.push("--return", autoReturnFile);
    }

    // File script goes positionally (matches `rodeo run script.luau`).
    if (opts.file !== undefined) args.push(opts.file);

    // scriptArgs is `last = true` in clap — passed after `--`.
    if (opts.scriptArgs && opts.scriptArgs.length > 0) {
      args.push("--", ...opts.scriptArgs);
    }

    const globalArgs: string[] = [];
    if (opts.verbose) globalArgs.push("--verbose");

    const proc = Bun.spawnSync(["rodeo", ...globalArgs, ...args], { timeout: SUBPROCESS_TIMEOUT_MS });
    const stdout = proc.stdout.toString();
    let stderr = proc.stderr.toString();
    if (proc.signalCode) {
      stderr += `\n[makeCliRunFn: killed after ${SUBPROCESS_TIMEOUT_MS}ms via ${proc.signalCode}]`;
    }

    // Merge stdout+stderr — matches Luau's `stdio = "tee"` which the factories
    // assert against. Ordering is approximate (each stream captured separately)
    // but case asserts contain substring matches, not line ordering.
    const output = stdout + stderr;

    // Read back the auto-allocated `--return <path>` JSON, parse it into
    // `result.return`, then delete the temp file. If the caller passed their
    // own returnFile we leave it alone — they manage that file themselves.
    let parsedReturn: unknown = undefined;
    if (autoReturnFile !== undefined && existsSync(autoReturnFile)) {
      try {
        const raw = readFileSync(autoReturnFile, "utf-8");
        if (raw.length > 0) parsedReturn = JSON.parse(raw);
      } catch {
        parsedReturn = undefined;
      }
      try { unlinkSync(autoReturnFile); } catch {}
    }

    // RODEO_TEST_DEBUG=1 dumps a failing run's command and merged output, so
    // a red `expect(result.ok)` in a shared factory can be diagnosed from the
    // test log alone.
    if (proc.exitCode !== 0 && process.env.RODEO_TEST_DEBUG) {
      console.error(`[makeCliRunFn] rodeo ${[...globalArgs, ...args].join(" ")} -> exit ${proc.exitCode}\n${output}`);
    }

    return {
      ok: proc.exitCode === 0,
      output,
      exitCode: proc.exitCode ?? -1,
      return: parsedReturn,
    };
  };
}

// Explicit-lifecycle CLI Studio handle. Caller registers the hooks themselves:
//
//   describe("my suite", () => {
//     const cli = cliStudioHandle(46100);
//     beforeAll(cli.spawn);
//     afterAll(cli.close);
//     describe("...", () => factory(cli.runFn));
//   });
export type CliStudioHandle = {
  runFn: (opts: RunCodeOpts) => Promise<RunResult>;
  spawn: () => Promise<void>;
  close: () => Promise<void>;
  /** The launched Studio's snapshot; set once spawn resolves. */
  studio: () => OwnedStudioSnap;
};

export type CliStudioOpts = {
  /** `--place` value: a place id or file. Omitted = empty place. */
  place?: string;
  /** How long to wait for the launched Studio to register (cloud and Team
   *  Create places take longer than an empty one). */
  timeoutMs?: number;
};

export function cliStudioHandle(port: number, opts: CliStudioOpts = {}): CliStudioHandle {
  let bg: BackgroundProcess | null = null;
  let studio: OwnedStudioSnap | null = null;
  return {
    runFn: makeCliRunFn(port),
    spawn: async () => {
      const args = ["run", "--port", String(port), "--place"];
      if (opts.place !== undefined) args.push(opts.place);
      bg = spawnBackground(args);
      // A launch that dies (bad place, Studio's automatic-login account
      // can't open it, ...) exits the run client: fail then, not at the
      // deadline.
      const died = bg.exited.then((code) => {
        throw new Error(
          `rodeo ${args.join(" ")} exited (${code}) before its Studio registered` +
            (opts.place ? "; does Studio's automatic-login account have edit access to the place?" : ""),
        );
      });
      studio = await Promise.race([waitForOwnedStudio(port, opts.timeoutMs), died]);
    },
    close: async () => {
      bg?.kill();
      await bg?.exited;
    },
    studio: () => {
      if (!studio) throw new Error("cliStudioHandle: spawn has not resolved");
      return studio;
    },
  };
}
