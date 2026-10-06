// Plugin unit tests that need no Studio: the mode driver's decisions
// (rodeo-plugin/src/library/mode_policy.luau) run under lune, and bun asserts
// on the exit. Covers issues #27 and #39 at the decision level.
import { describe, it, expect } from "bun:test";
import { join } from "node:path";

const SPEC = join(import.meta.dir, "mode_policy.spec.luau");

describe("plugin mode_policy", () => {
  it("ends mismatched sessions, starts only on an idle engine, and gives up with a reason", () => {
    const r = Bun.spawnSync(["lune", "run", SPEC], { timeout: 30_000 });
    const out = r.stdout.toString() + r.stderr.toString();
    expect(out, out).not.toContain("FAIL");
    expect(r.exitCode, out).toBe(0);
  });
});
