// A failed run's reason reaches API callers as `result.error` (issue #34):
// the module's own error with its trace, or the runner's. Before, it only
// travelled in ExecutionDone.error, which no client surfaced.
import { describe, it, expect } from "bun:test";
import { setupStudio, makeApiRunFn } from "../helpers.js";

const ctx = setupStudio();
const run = makeApiRunFn(ctx);

function occurrences(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

describe("run errors (API)", () => {
  it("a successful run has no error", async () => {
    const r = await run({ source: "return 1" });
    expect(r.ok).toBe(true);
    expect(r.error).toBeUndefined();
  });

  it("a module error is result.error, and the output shows it once", async () => {
    const r = await run({ source: "local x = 1\nerror('rodeo-34-api-module-error')" });
    expect(r.ok).toBe(false);
    expect(r.error).toContain("rodeo-34-api-module-error");
    // The engine's report, not require's generic "Requested module experienced an error while loading".
    expect(r.error).not.toContain("Requested module experienced an error");
    expect(occurrences(r.output, "rodeo-34-api-module-error"), r.output).toBe(1);
  });

  it("an elevated module error is the engine's report too", async () => {
    // StudioMCP runs the module in this Studio's edit DataModel, so the engine
    // reports it there as well: the real error, shown once.
    const r = await run({ source: "error('rodeo-34-api-elevated-error')", context: "elevated" });
    expect(r.ok).toBe(false);
    expect(r.error).toContain("rodeo-34-api-elevated-error");
    expect(r.error).not.toContain("Requested module experienced an error");
    expect(occurrences(r.output, "rodeo-34-api-elevated-error"), r.output).toBe(1);
  });

  it("the runner's own error is result.error and in the output", async () => {
    // Hide this Studio's command-bar bridge, as a hand-opened Studio has none.
    const hide = await run({ source: 'game:GetService("CoreGui").rodeoCmdbar.Name = "rodeoCmdbarHidden" return true' });
    expect(hide.ok, hide.output).toBe(true);
    const r = await run({ source: "return 1", context: "cmdbar" });
    expect(r.ok).toBe(false);
    expect(r.error).toContain("cmdbar context needs a rodeo-launched Studio");
    expect(occurrences(r.output, "cmdbar context needs a rodeo-launched Studio"), r.output).toBe(1);
  });
});
