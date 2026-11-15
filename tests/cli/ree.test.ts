// `rodeo run --ree`: a run executed by ree (roblox-engine-emulator),
// which rodeo starts for the one run and answers itself. No Studio and no
// serve are involved, so this suite runs wherever `rodeo` and a `ree` with the
// host call are (RODEO_REE, else PATH); it is skipped where there is no ree.
//
// What it pins is the claim behind the flag: the run's code in ree is the
// plugin's own (library/run), so a script and its @rodeo/* calls behave as
// they do in Studio — output, return values, exit codes, files, stdin.

import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";

function findRee(): string | undefined {
  if (process.env.RODEO_REE) return process.env.RODEO_REE;
  const name = process.platform === "win32" ? "ree.exe" : "ree";
  for (const dir of (process.env.PATH ?? "").split(delimiter)) {
    if (dir && existsSync(join(dir, name))) return join(dir, name);
  }
  return undefined;
}

// A ree without the host call (0.2.0-rodeo.1 and older) fails every run with
// a message saying so: only a ree that has it can run the suite.
function hasHostCall(ree: string): boolean {
  const dir = mkdtempSync(join(tmpdir(), "rodeo-ree-probe-"));
  try {
    const harness = join(dir, "Probe.luau");
    writeFileSync(harness, "return function(Context) print(Context.HostCall ~= nil) print('@done') end");
    const proc = Bun.spawnSync([ree, "run", "--harness", harness, "--seconds", "1"], {
      env: { ...process.env, REE_HOST_CALL: "127.0.0.1:9" },
      timeout: 30_000,
    });
    return proc.exitCode === 0 && JSON.parse(proc.stdout.toString()).output?.[0]?.message === "true";
  } catch {
    return false;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const REE = findRee();
const suite = REE && hasHostCall(REE) ? describe : describe.skip;

type Result = { exitCode: number; stdout: string; stderr: string };

suite("--ree (CLI)", () => {
  let dir: string;

  // `ree`: ree's own options, the string after `--ree=`.
  function rodeo(args: string[], opts: { stdin?: string; env?: Record<string, string>; ree?: string } = {}): Result {
    const flag = opts.ree === undefined ? "--ree" : `--ree=${opts.ree}`;
    const proc = Bun.spawnSync(["rodeo", "run", flag, ...args], {
      cwd: dir,
      env: { ...process.env, RODEO_REE: REE!, ...opts.env },
      stdin: opts.stdin === undefined ? undefined : Buffer.from(opts.stdin),
      timeout: 60_000,
    });
    return { exitCode: proc.exitCode ?? -1, stdout: proc.stdout.toString(), stderr: proc.stderr.toString() };
  }

  beforeAll(() => {
    dir = mkdtempSync(join(tmpdir(), "rodeo-ree-"));
  });
  afterAll(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  it("prints, warns and returns as a Studio run does", () => {
    const result = rodeo(["--show-return", "--source", `print("a print") warn("a warning") return { answer = 41 + 1 }`]);
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain("a print\n");
    expect(result.stdout).toContain("a warning");
    expect(result.stdout.trimEnd().split("\n").at(-1)).toBe(`{"answer":42}`);
  });

  it("fails with the script's error and exit code 1, after its output", () => {
    const result = rodeo(["--source", `print("before the error") error("boom")`]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toContain("before the error");
    expect(result.stderr).toContain("boom");
  });

  it("makes ree an edit session, a server or a client from --context", () => {
    const probe = `local RunService = game:GetService("RunService") return { running = RunService:IsRunning(), server = RunService:IsServer() }`;
    const kind = (args: string[]) => JSON.parse(rodeo([...args, "--show-return", "--source", probe]).stdout);
    expect(kind([])).toEqual({ running: false, server: false });
    expect(kind(["--context", "server"])).toEqual({ running: true, server: true });
    expect(kind(["--context", "client"])).toEqual({ running: true, server: false });
    // A running mode alone means the server, as it does for a Studio.
    expect(kind(["--mode", "run"])).toEqual({ running: true, server: true });
  });

  it("reads and writes host files through @rodeo/fs and @rodeo/stream, relative to where rodeo ran", () => {
    const result = rodeo(["--show-return", "--source", `
      local fs = require("@rodeo/fs")
      local stream = require("@rodeo/stream")
      local process = require("@rodeo/process")
      fs.mkdir("data")
      local out = fs.open("data/out.txt", "w")
      stream.write(out, "written from ree\\n")
      stream.close(out)
      local back = fs.open("data/out.txt", "r")
      local text = stream.read(back)
      stream.close(back)
      local names = {}
      for _, entry in fs.listdir("data") do table.insert(names, entry.name) end
      return { text = text, exists = fs.exists("data/out.txt"), missing = fs.exists("nope"), names = names, cwd = process.cwd() }
    `]);
    expect(result.exitCode).toBe(0);
    const value = JSON.parse(result.stdout);
    expect(value.text).toBe("written from ree\n");
    expect(value.exists).toBe(true);
    expect(value.missing).toBe(false);
    expect(value.names).toEqual(["out.txt"]);
    expect(readFileSync(join(dir, "data/out.txt"), "utf8")).toBe("written from ree\n");
    // (macOS reports its temp dir through /private)
    expect(value.cwd.endsWith(dir.replace(/^\/private/, ""))).toBe(true);
  });

  it("pipes stdin to the script and its stream writes to stdout", () => {
    const result = rodeo(["--source", `
      local io = require("@rodeo/io")
      local stream = require("@rodeo/stream")
      local name = stream.read(io.stdin)
      stream.write(io.stdout, "hello, " .. name .. "\\n")
    `], { stdin: "frank" });
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toBe("hello, frank\n");
  });

  it("runs a host process for the script", () => {
    const result = rodeo(["--show-return", "--source", `
      local process = require("@rodeo/process")
      local child = process.run({ "${process.execPath.replace(/\\/g, "\\\\")}", "-e", "console.log('from a child')" })
      return { ok = child.ok, out = child.out }
    `]);
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(result.stdout)).toEqual({ ok: true, out: "from a child\n" });
  });

  it("exits with process.exit's code after the output logged before it", () => {
    const result = rodeo(["--source", `local process = require("@rodeo/process") print("printed before exit") process.exit(7) print("never")`]);
    expect(result.exitCode).toBe(7);
    expect(result.stdout).toBe("printed before exit\n");
    expect(rodeo(["--source", `require("@rodeo/process").exit(0)`]).exitCode).toBe(0);
  });

  it("passes script arguments and writes a --return file", () => {
    const result = rodeo(["--return", "ret.json", "--source", `return { args = require("@rodeo/process").args }`, "--", "alpha", "beta"]);
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(readFileSync(join(dir, "ret.json"), "utf8"))).toEqual({ args: ["alpha", "beta"] });
  });

  it("runs a script file and the modules it requires by path", () => {
    mkdirSync(join(dir, "scripts"), { recursive: true });
    writeFileSync(join(dir, "scripts/helper.luau"), `return { greet = function(name) return "hi " .. name end }`);
    writeFileSync(join(dir, "scripts/main.luau"), `local helper = require("./helper") return helper.greet("ree")`);
    const result = rodeo(["--show-return", "scripts/main.luau"]);
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(result.stdout)).toBe("hi ree");
  });

  it("hands ree its own options, and shows the script only the project they mount", () => {
    mkdirSync(join(dir, "game/src/ReplicatedStorage"), { recursive: true });
    writeFileSync(join(dir, "game/src/ReplicatedStorage/Greeter.luau"), `return { greet = function(name) return "hello, " .. name end }`);
    const probe = `
      local ReplicatedStorage = game:GetService("ReplicatedStorage")
      local names = {}
      for _, child in ReplicatedStorage:GetChildren() do table.insert(names, child.Name) end
      return { greeting = require(ReplicatedStorage.Greeter).greet("ree"), children = names }
    `;
    const result = rodeo(["--context", "server", "--show-return", "--source", probe], { ree: "--roots game/src --offline" });
    expect(result.exitCode).toBe(0);
    // rodeo's own modules and harness are not in the place the script sees.
    expect(JSON.parse(result.stdout)).toEqual({ greeting: "hello, ree", children: ["Greeter"] });

    // Quotes keep a word of the options together: a folder with a space.
    mkdirSync(join(dir, "my game/src/ReplicatedStorage"), { recursive: true });
    writeFileSync(join(dir, "my game/src/ReplicatedStorage/Greeter.luau"), `return { greet = function(name) return "hello from a space, " .. name end }`);
    const spaced = rodeo(["--context", "server", "--show-return", "--source", probe], { ree: "--roots 'my game/src'" });
    expect(spaced.exitCode).toBe(0);
    expect(JSON.parse(spaced.stdout).greeting).toBe("hello from a space, ree");
  });

  it("runs a script on ree from its own directive, options and all", () => {
    mkdirSync(join(dir, "scripts"), { recursive: true });
    writeFileSync(
      join(dir, "scripts/directed.luau"),
      `-- @rodeo run --ree="--roots game/src" --context server --show-return\nreturn require(game:GetService("ReplicatedStorage").Greeter).greet("a directive")`,
    );
    // (a port nothing serves on: a run that did not go to ree fails there)
    const proc = Bun.spawnSync(["rodeo", "run", "scripts/directed.luau"], { cwd: dir, env: { ...process.env, RODEO_REE: REE!, RODEO_PORT: "9" }, timeout: 60_000 });
    expect(proc.stderr.toString()).toBe("");
    expect(proc.exitCode).toBe(0);
    expect(JSON.parse(proc.stdout.toString())).toBe("hello, a directive");
  });

  it("takes no host time for the script's waits on its own clock", () => {
    const started = Date.now();
    const result = rodeo(["--show-return", "--source", `local before = os.clock() task.wait(600) return os.clock() - before >= 600`]);
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(result.stdout)).toBe(true);
    expect(Date.now() - started).toBeLessThan(30_000);
  });

  it("ends a script that outlasts the run's virtual time, and says so", () => {
    const result = rodeo(["--source", `print("waiting") task.wait(60) print("never")`], { env: { RODEO_REE_SECONDS: "5" } });
    expect(result.exitCode).toBe(2);
    expect(result.stdout).toBe("waiting\n");
    expect(result.stderr).toContain("still running after 5 virtual seconds");
    expect(result.stderr).toContain("RODEO_REE_SECONDS");
  });

  it("names a member ree does not emulate when a script uses one", () => {
    // EncodingService's hashes are not emulated (ree's src/services/encoding.rs):
    // ComputeBufferHash answers nil.
    const result = rodeo(["--show-return", "--source", `return game:GetService("EncodingService"):ComputeBufferHash(buffer.fromstring("ab"), Enum.HashAlgorithm.Sha256) == nil`]);
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(result.stdout)).toBe(true);
    expect(result.stderr).toContain("EncodingService.ComputeBufferHash is not emulated");
  });

  it("returns and bakes a buffer from every kind of run", () => {
    // A buffer crosses as base64 from EncodingService, which a Studio has in
    // its edit session and on a client as well as on a server.
    const source = `
      require("@rodeo/roblox").bake("baked/" .. require("@rodeo/process").args[1] .. ".luau", { data = buffer.fromstring("ab") })
      return { data = buffer.fromstring("ab") }
    `;
    for (const [kind, flags] of [["edit", []], ["client", ["--context", "client"]], ["server", ["--context", "server"]]] as const) {
      const result = rodeo([...flags, "--show-return", "--source", source, "--", kind]);
      expect(result.stderr).toBe("");
      expect(result.exitCode).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({ data: { type: "buffer", value: "YWI=" } });
      expect(readFileSync(join(dir, `baked/${kind}.luau`), "utf8")).toContain(`buffer.fromstring("ab")`);
    }
  });

  it("refuses what only a Studio has", () => {
    const mcp = rodeo(["--context", "mcp", "--source", "return 1"]);
    expect(mcp.exitCode).not.toBe(0);
    expect(mcp.stderr).toContain("is a Studio identity");
    const pinned = rodeo(["--studio-id", "abc", "--source", "return 1"]);
    expect(pinned.exitCode).not.toBe(0);
    expect(pinned.stderr).toContain("--studio-id is for a Studio");
  });
});

describe("--ree flag (CLI)", () => {
  // ree's options start with `--` as rodeo's do, so they belong to --ree only
  // after an `=`. clap's own message for the string given apart suggests
  // passing it after `--`, which would hand ree's options to the script.
  it("says how to give ree its options when they are given apart from the flag", () => {
    const proc = Bun.spawnSync(["rodeo", "run", "--ree", "--roots game/src", "--source", "return 1"], { timeout: 60_000 });
    expect(proc.exitCode).toBe(2);
    expect(proc.stderr.toString()).toContain(`--ree takes ree's options after an '=', as one string: --ree="--roots game/src"`);
  });

  it("refuses options with a quote left open", () => {
    const proc = Bun.spawnSync(["rodeo", "run", "--ree=--roots 'game/src", "--source", "return 1"], { timeout: 60_000 });
    expect(proc.exitCode).not.toBe(0);
    expect(proc.stderr.toString()).toContain("--ree: a ' is never closed");
  });

  it("says how to get ree when there is none", () => {
    const proc = Bun.spawnSync(["rodeo", "run", "--ree", "--source", "return 1"], {
      env: { ...process.env, RODEO_REE: join(tmpdir(), "no-such-ree") },
      timeout: 60_000,
    });
    expect(proc.exitCode).not.toBe(0);
    expect(proc.stderr.toString()).toContain("couldn't start ree");
  });
});
