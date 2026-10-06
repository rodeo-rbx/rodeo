// Plugin unit test that needs no Studio: the --reload-requires prelude is
// pure Luau under rodeo-plugin/src, run under lune; bun asserts on the exit.
import { describe, it, expect } from "bun:test";
import { join } from "node:path";

const SPEC = join(import.meta.dir, "reload_prelude.spec.luau");

describe("plugin reload_prelude", () => {
  it("injects the require wrapper after the comment header", () => {
    const r = Bun.spawnSync(["lune", "run", SPEC], { timeout: 30_000 });
    const out = r.stdout.toString() + r.stderr.toString();
    expect(out, out).not.toContain("FAIL");
    expect(r.exitCode, out).toBe(0);
  });
});
