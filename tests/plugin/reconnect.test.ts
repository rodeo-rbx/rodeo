// Plugin unit tests that need no Studio: pure Luau modules under
// rodeo-plugin/src run under lune, and bun asserts on the exit.
import { describe, it, expect } from "bun:test";
import { join } from "node:path";

const SPEC = join(import.meta.dir, "reconnect.spec.luau");

describe("plugin reconnect", () => {
  it("backs off a dead backend and trusts the /health probe only once it has answered", () => {
    const r = Bun.spawnSync(["lune", "run", SPEC], { timeout: 30_000 });
    const out = r.stdout.toString() + r.stderr.toString();
    expect(out, out).not.toContain("FAIL");
    expect(r.exitCode, out).toBe(0);
  });
});
