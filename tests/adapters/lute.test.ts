// Lute adapter conformance: rodeo-authored scripts asserting the API
// contracts in submodules/lute/definitions/*.luau, run through `rodeo run`
// against a resident Studio. See tests/adapters/lute/README.md for why
// these aren't verbatim upstream tests (lute's own suite depends on its
// separate @std distribution).
import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { mkdtempSync, rmSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cliStudioHandle } from "../cli/helpers.js";

const PORT = 46290;
const ROOT = join(import.meta.dir, "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");
const SUITE_DIR = join(import.meta.dir, "lute");

// Expected stdout fragments per script (io.write must actually reach stdout).
const EXPECT_STDOUT: Record<string, string> = {
  "io.luau": "lute-io-multi-arg",
  "process.luau": "lute-inherit", // stdio = "inherit" prints
};

const studio = cliStudioHandle(PORT);
const scratchDirs: string[] = [];

beforeAll(async () => {
  await studio.spawn();
}, 180_000);

afterAll(async () => {
  await studio.close();
  for (const d of scratchDirs) {
    try { rmSync(d, { recursive: true, force: true }); } catch {}
  }
});

describe("lute conformance", () => {
  const files = readdirSync(SUITE_DIR).filter((f) => f.endsWith(".luau")).sort();

  it("covers every adapter module", () => {
    expect(files).toEqual(["fs.luau", "io.luau", "process.luau", "task.luau", "time.luau"]);
  });

  for (const file of files) {
    it(
      file,
      () => {
        const cwd = mkdtempSync(join(tmpdir(), "rodeo-lute-conformance-"));
        scratchDirs.push(cwd);
        const proc = Bun.spawnSync(
          [RODEO, "run", join(SUITE_DIR, file), "--port", String(PORT)],
          { cwd, stdout: "pipe", stderr: "pipe", timeout: 90_000 },
        );
        const stdout = proc.stdout?.toString() ?? "";
        const stderr = proc.stderr?.toString() ?? "";
        expect(proc.exitCode, `${file}\n--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
        if (EXPECT_STDOUT[file]) {
          expect(stdout).toContain(EXPECT_STDOUT[file]);
        }
        if (file === "process.luau") {
          expect(stdout).not.toContain("lute-none"); // stdio = "none" discards
        }
      },
      120_000,
    );
  }
});
