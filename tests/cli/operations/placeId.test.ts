import { describe, it, expect } from "bun:test";
import { runRodeo } from "../helpers.js";

// Each run pins a spare port: a serve of another build on the default port
// refuses the run.
describe("place id (CLI)", () => {
  it("--place with ID auto-resolves universe and launches", () => {
    const result = runRodeo([
      "run", "--port", "46002", "--place", "72824109308551",
      "--source", "return game.PlaceId",
      "--show-return",
    ]);
    expect(result.ok).toBe(true);
    expect(result.stdout + result.stderr).toContain("72824109308551");
  });

  it("--place with ID and explicit --place.universe", () => {
    const result = runRodeo([
      "run", "--port", "46004", "--place", "72824109308551",
      "--place.universe", "8612861022",
      "--source", "return game.PlaceId",
      "--show-return",
    ]);
    expect(result.ok).toBe(true);
    expect(result.stdout + result.stderr).toContain("72824109308551");
  });
});
