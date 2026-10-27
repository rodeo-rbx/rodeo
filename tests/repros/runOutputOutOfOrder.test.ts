// Repro: a run's output reached the terminal out of order. Every line Studio
// logs is its own stream.write, sent from its own LogService.MessageOut handler
// thread, so several are in flight at once. The run client answered each call
// in its own task, and whichever task finished first printed first: a warn
// before the print above it, a trace's "Stack End" before its "Script" line.
//
// Expected: lines print in the order Studio logged them.
import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { cliStudioHandle, killLaunchedStudios, runRodeo, stopServe } from "../cli/helpers.js";

const PORT = 47560;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const cli = cliStudioHandle(PORT);
const ITERATIONS = 99;

describe("run output keeps the order Studio logged it in", () => {
  beforeAll(cli.spawn);
  afterAll(async () => {
    await cli.close();
    stopServe(PORT);
    killLaunchedStudios(PORT, STARTED);
  });

  it("prints, warnings and spawned threads' errors", () => {
    const r = runRodeo([
      "run", "--port", String(PORT), "--studio-id", cli.studio().studioId,
      "--source", `for i = 1, ${ITERATIONS} do print("ok") warn(i) task.spawn(function() error(i) end) end`,
    ]);
    expect(r.exitCode, r.stderr).toBe(0);

    const lines = r.stdout.replace(/\x1b\[[0-9;]*m/g, "").split("\n").filter((line) => line !== "");
    const expected: (string | RegExp)[] = [];
    for (let i = 1; i <= ITERATIONS; i++) {
      expected.push("ok", String(i), new RegExp(`:1: ${i}$`), "Stack Begin", /^Script '.*', Line 1$/, "Stack End");
    }
    expect(lines.length, r.stdout).toBe(expected.length);
    const at = expected.findIndex((want, k) => (typeof want === "string" ? lines[k] !== want : !want.test(lines[k])));
    const around = at === -1 ? "" : lines.slice(Math.max(0, at - 3), at + 3).join(" | ");
    expect(at === -1 ? "in order" : `line ${at + 1} out of order: ${around}`).toBe("in order");
  });
});
