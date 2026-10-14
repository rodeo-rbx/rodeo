import { describe, it, expect } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { runRodeo } from "../cli/helpers.js";

// Port of tests/cli/setup.test.luau. Pure CLI tests — no background Studio.

describe("rodeo setup", () => {
  it("writes typedef files to ~/.rodeo/typedefs/{version}", () => {
    const r = runRodeo(["setup"]);
    expect(r.ok).toBe(true);
    expect(r.stderr).toContain("Wrote type definitions");

    const match = r.stderr.match(/typedefs\/([^/\n]+)/);
    expect(match).not.toBeNull();
    const version = match![1];

    const typedefs = join(homedir(), ".rodeo", "typedefs", version);
    expect(existsSync(join(typedefs, "init.luau"))).toBe(true);
    expect(existsSync(join(typedefs, "fs.luau"))).toBe(true);
    expect(existsSync(join(typedefs, "io.luau"))).toBe(true);
    expect(existsSync(join(typedefs, "process.luau"))).toBe(true);
    expect(existsSync(join(typedefs, "stream.luau"))).toBe(true);
  });

  it("creates .rodeo/.luaurc with rodeo alias", () => {
    if (existsSync(".rodeo/.luaurc")) unlinkSync(".rodeo/.luaurc");

    const r = runRodeo(["setup"]);
    expect(r.ok).toBe(true);
    expect(r.stderr).toContain(".rodeo/.luaurc");

    expect(existsSync(".rodeo/.luaurc")).toBe(true);
    const rc = JSON.parse(readFileSync(".rodeo/.luaurc", "utf8"));
    expect(rc.aliases).not.toBeUndefined();
    expect(rc.aliases.rodeo).not.toBeUndefined();
    expect(rc.aliases.rodeo).toContain("~/.rodeo/typedefs/");
  });

  it("preserves existing .luaurc aliases", () => {
    if (!existsSync(".rodeo")) mkdirSync(".rodeo", { recursive: true });
    writeFileSync(
      ".rodeo/.luaurc",
      JSON.stringify({ aliases: { custom: "./some/path" } }, null, 2),
    );

    const r = runRodeo(["setup"]);
    expect(r.ok).toBe(true);

    const rc = JSON.parse(readFileSync(".rodeo/.luaurc", "utf8"));
    expect(rc.aliases.custom).toBe("./some/path");
    expect(rc.aliases.rodeo).not.toBeUndefined();

    unlinkSync(".rodeo/.luaurc");
  });

  it("can run script with require(@rodeo) after setup", () => {
    runRodeo(["setup"]);

    // Bundle only works with file paths — write a temp script.
    const scriptPath = ".rodeo-test-require-rodeo.luau";
    writeFileSync(scriptPath, 'local rodeo = require("@rodeo")\nreturn type(rodeo)');

    const r = runRodeo([
      "run", "--place", "--port", "46000",
      scriptPath, "--show-return",
    ]);

    try { unlinkSync(scriptPath); } catch {}

    expect(r.ok).toBe(true);
    expect(r.stdout + r.stderr).toContain("table");
  });
});

// The skill `setup` installs: this repo's, which the binary under test embeds.
const SKILL = readFileSync(join(import.meta.dir, "..", "..", ".claude", "skills", "rodeo", "SKILL.md"), "utf8");
const VERSION = /\n  version: (\S+)\n/.exec(SKILL)![1];

function skillFor(version: string): string {
  return `---\nname: rodeo\nmetadata:\n  version: ${version}\n---\n\n# an edited copy\n`;
}

// `rodeo setup` in a scratch project laid out by `prepare`.
function setupIn(prepare: (dir: string) => void): { dir: string; stderr: string; exitCode: number | null } {
  const dir = mkdtempSync(join(tmpdir(), "rodeo-setup-skill-"));
  prepare(dir);
  const proc = Bun.spawnSync(["rodeo", "setup"], { cwd: dir });
  return { dir, stderr: proc.stderr.toString(), exitCode: proc.exitCode };
}

describe("rodeo setup: the agent skill", () => {
  it("installs into the skill folders the project has, and creates none", () => {
    const r = setupIn((dir) => mkdirSync(join(dir, ".claude", "skills"), { recursive: true }));
    try {
      expect(r.exitCode, r.stderr).toBe(0);
      expect(readFileSync(join(r.dir, ".claude", "skills", "rodeo", "SKILL.md"), "utf8")).toBe(SKILL);
      expect(existsSync(join(r.dir, ".agents"))).toBe(false);
      expect(r.stderr).toContain("Installed the rodeo skill to .claude/skills/rodeo/SKILL.md");
    } finally {
      rmSync(r.dir, { recursive: true, force: true });
    }
  });

  it("replaces a copy for an older rodeo and keeps one for this or a newer rodeo", () => {
    const r = setupIn((dir) => {
      for (const [folder, copy] of [
        [".agents", skillFor("1.3.0")],
        [".claude", skillFor(VERSION)],
      ]) {
        mkdirSync(join(dir, folder, "skills", "rodeo"), { recursive: true });
        writeFileSync(join(dir, folder, "skills", "rodeo", "SKILL.md"), copy);
      }
    });
    try {
      expect(r.exitCode, r.stderr).toBe(0);
      expect(readFileSync(join(r.dir, ".agents", "skills", "rodeo", "SKILL.md"), "utf8")).toBe(SKILL);
      expect(r.stderr).toContain(`Updated the rodeo skill in .agents/skills/rodeo/SKILL.md (1.3.0 → ${VERSION})`);
      expect(readFileSync(join(r.dir, ".claude", "skills", "rodeo", "SKILL.md"), "utf8")).toBe(skillFor(VERSION));
      expect(r.stderr).toContain("The rodeo skill in .claude/skills/rodeo/SKILL.md is up to date");
    } finally {
      rmSync(r.dir, { recursive: true, force: true });
    }
  });

  it("keeps a copy for a newer rodeo", () => {
    const r = setupIn((dir) => {
      mkdirSync(join(dir, ".claude", "skills", "rodeo"), { recursive: true });
      writeFileSync(join(dir, ".claude", "skills", "rodeo", "SKILL.md"), skillFor("99.0.0"));
    });
    try {
      expect(r.exitCode, r.stderr).toBe(0);
      expect(readFileSync(join(r.dir, ".claude", "skills", "rodeo", "SKILL.md"), "utf8")).toBe(skillFor("99.0.0"));
      expect(r.stderr).toContain("it's for rodeo 99.0.0, newer than this one");
    } finally {
      rmSync(r.dir, { recursive: true, force: true });
    }
  });
});
