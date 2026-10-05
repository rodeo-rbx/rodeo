import { describe, it, expect, beforeAll } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { pluginsDir, runRodeo, waitUntil } from "../helpers.js";

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

function sh(cmd: string[], cwd?: string): { ok: boolean; stdout: string; stderr: string } {
  const r = Bun.spawnSync(cmd, { cwd, timeout: 180_000 });
  return { ok: r.exitCode === 0, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
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

