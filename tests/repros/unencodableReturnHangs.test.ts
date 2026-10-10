// Repro: the runner turns a run's return value into JSON (or a .luau file)
// after the user's module has finished, outside every error handler. When that
// step throws, the runner never fires its results event, so the CLI is never
// told the run ended: it prints nothing and waits forever. The error itself
// only reaches Studio's Output window.
//
// Three ways in, one cause:
//   - a returned string that isn't valid UTF-8 (raw bytes from buffer.tostring,
//     string.char(0xFF), ...): HttpService:JSONEncode throws "Can't convert to
//     JSON" — on the wire copy, --show-return, and .json return files alike.
//   - any other value JSONEncode refuses (a boolean table key: "Invalid table
//     key type used"), whose bare message doesn't say it was the return value.
//   - a return file that can't be written: fs.open's rpc error is raised in
//     the runner.
//
// Expected: the run ends with a non-zero exit and an error that says which
// value or path failed and what to do instead.
import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cliStudioHandle, killLaunchedStudios, runRodeo } from "../cli/helpers.js";

const PORT = 47600;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const cli = cliStudioHandle(PORT);
// A run that completes takes a second or two against a warm Studio; anything
// near this means it hung.
const TIMEOUT_MS = 30_000;

const BAD_STRING_SOURCE = "return { head = string.char(0xFF) }";

function run(args: string[]) {
  return runRodeo(["run", "--port", String(PORT), "--studio-id", cli.studio().studioId, ...args], {
    timeout: TIMEOUT_MS,
  });
}

describe("unencodable return values end the run (no hang)", () => {
  let dir: string;
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), "rodeo-unencodable-"));
    await cli.spawn();
  });
  afterAll(async () => {
    await cli.close();
    Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
    Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
    killLaunchedStudios(PORT, STARTED);
    rmSync(dir, { recursive: true, force: true });
  });

  it("an invalid UTF-8 string with --show-return fails and names the value", () => {
    const r = run(["--show-return", "--source", BAD_STRING_SOURCE]);
    const output = r.stdout + r.stderr;
    // FAILS before the fix: killed by the timeout with no output.
    expect(r.exitCode, output).toBe(1);
    expect(output).toContain("result.head");
    expect(output).toContain("UTF-8");
    expect(output).toContain("buffer");
  });

  it("an invalid UTF-8 string on the wire (no flags) fails and names the value", () => {
    const r = run(["--source", BAD_STRING_SOURCE]);
    const output = r.stdout + r.stderr;
    expect(r.exitCode, output).toBe(1);
    expect(output).toContain("result.head");
    expect(output).toContain("UTF-8");
  });

  it("an invalid UTF-8 string to a .json return file fails and names the value", () => {
    const path = join(dir, "bad.json");
    const r = run(["--return", path, "--source", BAD_STRING_SOURCE]);
    const output = r.stdout + r.stderr;
    expect(r.exitCode, output).toBe(1);
    expect(output).toContain("result.head");
    expect(output).toContain("UTF-8");
  });

  it("a value JSON can't encode for another reason fails and says it was the return value", () => {
    const r = run(["--source", "return { flags = { [true] = 1 } }"]);
    const output = r.stdout + r.stderr;
    expect(r.exitCode, output).toBe(1);
    expect(output).toContain("couldn't encode the return value as JSON");
    expect(output).toContain("Invalid table key type");
  });

  it("a return file that can't be written fails and names the path", () => {
    // A regular file can't be a directory, so nothing can be created under it.
    const blocker = join(dir, "not-a-directory");
    writeFileSync(blocker, "");
    const path = join(blocker, "out.json");
    const r = run(["--return", path, "--source", "return 1"]);
    const output = r.stdout + r.stderr;
    // FAILS before the fix: fs.open's error is raised outside every handler.
    expect(r.exitCode, output).toBe(1);
    expect(output).toContain("not-a-directory");
  });
});
