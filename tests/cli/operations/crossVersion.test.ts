import { describe, it, expect, beforeAll } from "bun:test";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { killLaunchedStudios, pluginFileFor, pluginsDir, runRodeo, waitUntil } from "../helpers.js";

// Two rodeo versions side by side, run the way projects run them: each
// fixture directory pins its rodeo (and port) in its own `.mise.toml`.
//
//   fixtures/versions/prev  — the previous published release via mise/ubi
//   fixtures/versions/tree  — this working tree's build (the repo's mise.toml
//                              puts bin/ on PATH for every directory under it)
//
// Under the repo, the root mise.toml's bin/ entry precedes a fixture's pinned
// tool on PATH, so `mise -C prev exec -- rodeo` would run the tree build. The
// previous binary is therefore resolved with `mise -C prev which rodeo`, which
// honors the pin, and both sides assert `--version` before launching anything.
//
// Pre-1.5 builds ignore RODEO_PORT and still write the shared `rodeo.rbxm`;
// this build never touches that file, so the previous release's Studio keeps
// its plugin while the tree build runs alongside on its own port.
const FIXTURES = join(import.meta.dir, "..", "..", "fixtures", "versions");
const PREV_DIR = join(FIXTURES, "prev");
const TREE_DIR = join(FIXTURES, "tree");
const PREV_PORT = 46790; // matches prev/.mise.toml; passed explicitly (pre-1.5 ignores the env var)
const PREV_VERSION_PREFIX = "rodeo 1.4.0-rc.4"; // keep in step with prev/.mise.toml

const mise = Bun.which("mise");

function sh(cmd: string[], cwd?: string, timeout = 180_000): { ok: boolean; stdout: string; stderr: string; timedOut: boolean } {
  const r = Bun.spawnSync(cmd, { cwd, timeout });
  return { ok: r.exitCode === 0, stdout: r.stdout.toString(), stderr: r.stderr.toString(), timedOut: r.exitedDueToTimeout === true };
}

type StateJson = { studios?: Array<{ studioId: string; sessionId?: string | null; status?: string }> };

type Spawned = ReturnType<typeof Bun.spawn>;

async function outputOf(proc: Spawned): Promise<string> {
  const out = proc.stdout ? await new Response(proc.stdout as ReadableStream).text() : "";
  const err = proc.stderr ? await new Response(proc.stderr as ReadableStream).text() : "";
  return out + err;
}

// Polls until `stateCmd` reports a connected launched Studio. Fails fast, with
// the process's own output, if the launching process exits first (a Studio
// that dies during launch takes its one-shot serve down with it).
async function waitForLaunchedStudio(proc: Spawned, stateCmd: string[], cwd: string | undefined, what: string): Promise<void> {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (proc.exitCode !== null) {
      throw new Error(`${what}: launcher exited ${proc.exitCode} before its Studio connected:\n${await outputOf(proc)}`);
    }
    const r = sh(stateCmd, cwd);
    if (r.ok) {
      const state = JSON.parse(r.stdout) as StateJson;
      if ((state.studios ?? []).some((s) => s.sessionId && s.status === "connected")) return;
    }
    await Bun.sleep(500);
  }
  throw new Error(`timed out waiting for ${what}`);
}

describe.skipIf(!mise)("two rodeo versions side by side (CLI)", () => {
  let prevBin = "";

  beforeAll(() => {
    for (const dir of [PREV_DIR, TREE_DIR]) {
      const t = sh([mise!, "trust", "-q", join(dir, ".mise.toml")]);
      if (!t.ok) throw new Error(`mise trust failed: ${t.stderr}`);
    }
    // Network on first run: ubi downloads the release asset.
    const inst = sh([mise!, "-C", PREV_DIR, "install", "-q"]);
    if (!inst.ok) throw new Error(`mise install failed: ${inst.stderr}`);

    prevBin = sh([mise!, "-C", PREV_DIR, "which", "rodeo"]).stdout.trim();
    expect(prevBin.length).toBeGreaterThan(0);

    // Guard against a PATH surprise testing the tree against itself.
    const prevVersion = sh([prevBin, "--version"]).stdout.trim();
    if (!prevVersion.startsWith(PREV_VERSION_PREFIX)) throw new Error(`previous binary reports ${JSON.stringify(prevVersion)}, expected ${PREV_VERSION_PREFIX}…`);
    const treeVersion = sh([mise!, "-C", TREE_DIR, "exec", "--", "rodeo", "--version"]).stdout.trim();
    expect(treeVersion).toBe(runRodeo(["--version"]).stdout.trim());
    expect(treeVersion).not.toBe(prevVersion);
  });

  it("a previous release's run stays up while the tree build launches and runs beside it", async () => {
    // Previous release: one-shot run with a long script on its own port. It
    // starts its own serve, launches a Studio, and writes/uses `rodeo.rbxm`.
    const prev = Bun.spawn(
      [prevBin, "run", "--port", String(PREV_PORT), "--place", "--show-return", "--source", "task.wait(25) return 'prev-done'"],
      { cwd: PREV_DIR, stdout: "pipe", stderr: "pipe" },
    );
    try {
      // Wait until the previous release's Studio is connected.
      await waitForLaunchedStudio(prev, [prevBin, "state", "--port", String(PREV_PORT), "--json"], undefined, "the previous release's Studio");

      // Tree build, from its fixture: no --port anywhere — RODEO_PORT comes
      // from tree/.mise.toml through `mise exec`. Persistent so its state can
      // be inspected while both are up.
      const tree = Bun.spawn(
        [mise!, "-C", TREE_DIR, "exec", "--", "rodeo", "run", "--place"],
        { cwd: TREE_DIR, stdout: "pipe", stderr: "pipe" },
      );
      try {
        await waitForLaunchedStudio(tree, [mise!, "-C", TREE_DIR, "exec", "--", "rodeo", "state", "--json"], TREE_DIR, "the tree build's Studio");

        const treeRun = sh(
          [mise!, "-C", TREE_DIR, "exec", "--", "rodeo", "run", "--show-return", "--source", "return 'tree-done'"],
          TREE_DIR,
        );
        if (!treeRun.ok) throw new Error(`tree run failed: ${treeRun.stderr}`);
        expect(treeRun.stdout + treeRun.stderr).toContain("tree-done");

        // Neither serve lists the other's Studio: owned Studios are exclusive
        // to the serve that launched them.
        const prevState = JSON.parse(sh([prevBin, "state", "--port", String(PREV_PORT), "--json"]).stdout) as StateJson;
        const treeState = JSON.parse(sh([mise!, "-C", TREE_DIR, "exec", "--", "rodeo", "state", "--json"], TREE_DIR).stdout) as StateJson;
        const prevOwned = (prevState.studios ?? []).filter((s) => s.sessionId).map((s) => s.studioId);
        const treeOwned = (treeState.studios ?? []).filter((s) => s.sessionId).map((s) => s.studioId);
        expect(prevOwned.length).toBe(1);
        expect(treeOwned.length).toBe(1);
        expect((treeState.studios ?? []).map((s) => s.studioId)).not.toContain(prevOwned[0]);
        expect((prevState.studios ?? []).map((s) => s.studioId)).not.toContain(treeOwned[0]);
      } finally {
        tree.kill();
        await tree.exited;
      }

      // The previous release's run must have kept its plugin and finished.
      const exit = await prev.exited;
      const out = await outputOf(prev);
      if (exit !== 0) throw new Error(`previous release run exited ${exit}: ${out}`);
      expect(out).toContain("prev-done");
    } finally {
      if (prev.exitCode === null) prev.kill();
    }

    // The previous release wrote the shared legacy file; this build never
    // removes it.
    expect(existsSync(join(pluginsDir(), "rodeo.rbxm"))).toBe(true);
  });

  it("run --place refuses another build's serve before launching a Studio onto it", async () => {
    const serve = Bun.spawn([prevBin, "serve", "--port", String(PREV_PORT)], { cwd: PREV_DIR, stdout: "pipe", stderr: "pipe" });
    try {
      await waitUntil(() => sh([prevBin, "state", "--port", String(PREV_PORT), "--json"]).ok, 30_000, "the previous release's serve");

      const run = runRodeo(["run", "--port", String(PREV_PORT), "--place", "--source", "return 1"], { timeout: 120_000 });
      expect(run.ok).toBe(false);
      expect(run.stdout + run.stderr).toContain("RODEO_SKIP_VERSION_CHECK");

      // The run must not have left a Studio on the other build's serve.
      const state = JSON.parse(sh([prevBin, "state", "--port", String(PREV_PORT), "--json"]).stdout) as StateJson;
      expect((state.studios ?? []).filter((s) => s.sessionId)).toEqual([]);
    } finally {
      serve.kill();
      await serve.exited;
    }
  }, 180_000);
});

// A newer plugin in a Studio launched by a server older than the plugin
// version check (rodeo < 1.4; here 1.3.0, fixtures/versions/pre-handshake).
// Such a server registers whatever plugin dials its port and routes runs to
// it. In issue #31 a plugin a newer serve had left installed for the default
// port took a 1.3.0 run, which then hung with no output. This build's plugin
// must refuse such runs with an error the old CLI prints.
//
// Setup, as in the issue: this build's plugin file for the old server's
// backend port is installed with no serve of this build running, then 1.3.0
// serves on that port and launches a Studio. 1.3.0 also installs its legacy
// rodeo.rbxm, which that Studio loads too, so two rodeo plugins connect to
// its backend from each DOM, and which one a DOM's runs go to depends on
// their connection order.
// Rewriting this build's plugin file reloads it, which makes it the one that
// connected last; the test then runs against every edit DOM of the launched
// Studio and requires each run to finish, any failure to be the refusal, and
// at least one refusal.
const PRE_DIR = join(FIXTURES, "pre-handshake");
const PRE_PORT = 47620; // master; the backend (and this build's plugin file) use 47621
const PRE_VERSION_PREFIX = "rodeo 1.3.0"; // keep in step with pre-handshake/.mise.toml
const REFUSAL = "predates rodeo's plugin version check";
const PRE_LOG = join(tmpdir(), `rodeo-crossversion-pre-handshake-${process.pid}.log`);

type PreStateJson = {
  studios?: Array<{ sessionId?: string | null; editDomId?: string | null; doms?: Array<{ domId: string; domKind: string }> }>;
};

describe.skipIf(!mise)("a newer plugin against a server older than the version check (CLI)", () => {
  let preBin = "";

  beforeAll(() => {
    const t = sh([mise!, "trust", "-q", join(PRE_DIR, ".mise.toml")]);
    if (!t.ok) throw new Error(`mise trust failed: ${t.stderr}`);
    const inst = sh([mise!, "-C", PRE_DIR, "install", "-q"]);
    if (!inst.ok) throw new Error(`mise install failed: ${inst.stderr}`);
    preBin = sh([mise!, "-C", PRE_DIR, "which", "rodeo"]).stdout.trim();
    const preVersion = sh([preBin, "--version"]).stdout.trim();
    if (!preVersion.startsWith(PRE_VERSION_PREFIX)) throw new Error(`pre-handshake binary reports ${JSON.stringify(preVersion)}, expected ${PRE_VERSION_PREFIX}…`);
  });

  // Edit DOMs of the Studios the old serve launched (hand-opened Studios have
  // no session). One per plugin when each has its own DOM id, one in all if
  // the plugins share it.
  function launchedEditDoms(): string[] {
    const r = sh([preBin, "state", "--port", String(PRE_PORT), "--json"]);
    if (!r.ok) return [];
    const state = JSON.parse(r.stdout) as PreStateJson;
    const ids = new Set<string>();
    for (const studio of state.studios ?? []) {
      if (!studio.sessionId) continue;
      if (studio.editDomId) ids.add(studio.editDomId);
      for (const dom of studio.doms ?? []) if (dom.domKind === "edit") ids.add(dom.domId);
    }
    return [...ids];
  }

  // DOM connections the old serve has logged so far.
  function domConnects(): number {
    try {
      return (readFileSync(PRE_LOG, "utf8").match(/dom connected/g) ?? []).length;
    } catch {
      return 0;
    }
  }

  it("refuses the old server's runs with a version error instead of hanging", async () => {
    const since = Date.now();
    const pluginFile = pluginFileFor(PRE_PORT);
    let serve: Spawned | undefined;
    try {
      // This build's plugin for the old backend's port, with no serve of this
      // build running: a serve installs it, its bytes are kept, the serve
      // stops (removing its file), and the bytes go back.
      const tree = Bun.spawn(["rodeo", "serve", "--port", String(PRE_PORT), "--ppid", String(process.pid)], { stdout: "ignore", stderr: "ignore" });
      const plugin = await (async () => {
        try {
          await waitUntil(() => existsSync(pluginFile), 60_000, "this build's plugin file");
          await Bun.sleep(500); // the install is a single write; let it land
          return readFileSync(pluginFile);
        } finally {
          tree.kill();
          await tree.exited;
        }
      })();
      await waitUntil(() => !existsSync(pluginFile), 20_000, "this build's serve to remove its plugin file");
      writeFileSync(pluginFile, plugin);

      // The old server on the same port. 1.3.0 writes its rodeo.rbxm at
      // startup; give both plugin files time to settle before a Studio starts
      // (a plugin file landing as Studio starts can crash it).
      rmSync(PRE_LOG, { force: true });
      serve = Bun.spawn([preBin, "serve", "--port", String(PRE_PORT), "--ppid", String(process.pid)], { stdout: Bun.file(PRE_LOG), stderr: Bun.file(PRE_LOG) });
      await waitUntil(() => sh([preBin, "state", "--port", String(PRE_PORT), "--json"]).ok, 30_000, "the old serve");
      await Bun.sleep(3_000);

      // Launch a Studio that outlives this run. Either plugin may take the
      // run; only the launch matters here.
      const launch = sh([preBin, "run", "--port", String(PRE_PORT), "--place", "--detach", "--source", "return 1"], undefined, 120_000);
      console.info(`  launching run: ${launch.ok ? "ok" : "failed"}${launch.timedOut ? " (timed out)" : ""}`);
      await waitUntil(() => launchedEditDoms().length > 0, 60_000, "the launched Studio's edit DOM");

      let refusals = 0;
      for (let attempt = 1; attempt <= 3 && refusals === 0; attempt++) {
        // Reload this build's plugin so it is the last to connect.
        const before = domConnects();
        writeFileSync(pluginFile, plugin);
        await waitUntil(() => domConnects() > before, 30_000, "this build's plugin to reconnect after its reload");
        await Bun.sleep(3_000);
        await waitUntil(() => launchedEditDoms().length > 0, 30_000, "the launched Studio's edit DOM after the reload");

        for (const dom of launchedEditDoms()) {
          const r = sh([preBin, "run", "--port", String(PRE_PORT), "--dom-id", dom, "--show-return", "--source", "return 'old-plugin-ran'"], undefined, 60_000);
          const out = r.stdout + r.stderr;
          console.info(`  attempt ${attempt}, dom ${dom.slice(0, 8)}: ${r.ok ? "ok" : "failed"}${r.timedOut ? " (timed out)" : ""}`);
          // The bug: a run this build's plugin took never finished.
          expect(r.timedOut, out).toBe(false);
          if (out.includes(REFUSAL)) {
            refusals++;
            expect(r.ok, out).toBe(false);
          } else {
            // Taken by 1.3.0's own plugin, which runs it.
            expect(r.ok, out).toBe(true);
            expect(out).toContain("old-plugin-ran");
          }
        }
      }
      expect(refusals).toBeGreaterThan(0);
    } finally {
      if (serve) {
        serve.kill();
        await serve.exited;
      }
      killLaunchedStudios(PRE_PORT, since);
      rmSync(pluginFile, { force: true });
      rmSync(PRE_LOG, { force: true });
    }
  }, 600_000);
});
