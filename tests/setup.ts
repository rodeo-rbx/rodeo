// Run-wide setup for `bun test`, preloaded through bunfig.toml.
//
// A serve leaves its plugin file installed when it stops, for the next serve
// on its port, and records when in rodeo's cache directory. Tests start serves
// on dozens of ports. Studios rodeo launches ignore plugins for other ports,
// but a Studio opened by hand loads every one, each redialing its stopped
// serve. So once every test has run, this removes the plugin files this run's
// serves left, except where a serve is running on the port again.
import { afterAll } from "bun:test";
import { readdirSync, rmSync, statSync } from "node:fs";
import { createConnection } from "node:net";
import { join } from "node:path";
import { keptMarkersDir, pluginsDir } from "./cli/helpers.js";

const runStart = Date.now();

// Whether anything accepts connections on `port`. No answer either way counts
// as yes, so an unsure check leaves the file.
function listening(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    const done = (up: boolean) => {
      socket.destroy();
      resolve(up);
    };
    socket.once("connect", () => done(true));
    socket.once("error", () => done(false));
    socket.setTimeout(1000, () => done(true));
  });
}

afterAll(async () => {
  const dir = keptMarkersDir();
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return;
  }
  for (const name of names) {
    const port = Number(name.match(/-(\d+)\.rbxm$/)?.[1]);
    if (!port) continue;
    let recorded: number;
    try {
      recorded = statSync(join(dir, name)).mtimeMs;
    } catch {
      continue;
    }
    if (recorded < runStart) continue; // left before this run
    if (await listening(port - 1)) continue; // its master answers again
    rmSync(join(pluginsDir(), name), { force: true });
    rmSync(join(dir, name), { force: true });
  }
});
