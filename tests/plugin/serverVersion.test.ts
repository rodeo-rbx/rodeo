// Plugin unit tests that need no Studio: pure Luau modules under
// rodeo-plugin/src run under lune, and bun asserts on the exit.
import { describe, it, expect } from "bun:test";
import { join } from "node:path";

const SPEC = join(import.meta.dir, "server_version.spec.luau");

describe("plugin server_version", () => {
  it("refuses only servers older than the plugin version check", () => {
    const r = Bun.spawnSync(["lune", "run", SPEC], { timeout: 30_000 });
    const out = r.stdout.toString() + r.stderr.toString();
    expect(out, out).not.toContain("FAIL");
    expect(r.exitCode, out).toBe(0);
  });
});
