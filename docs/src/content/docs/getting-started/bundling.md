---
title: Bundling
---

When you pass a file to `rodeo run`, rodeo resolves and bundles its dependencies into a single script before sending it to Studio. You don't need to flatten your code, vendor your deps, or invoke a separate bundler. It's always-on.

```bash
rodeo run my-script.luau
```

Filesystem requires (`require("./util")`, `require("../helpers")`), Wally / pesde package requires (via sourcemap), and `@rodeo` runtime requires are all handled automatically. The resulting bundle runs inside the Studio DOM.

## Runtime adapters

An adapter runs code written for another Luau runtime: that runtime's standard library requires reach the runtime itself, which rodeo runs alongside the script. Enable one per run with `--adapter`, or in the script's [directive](/rodeo/getting-started/directives/):

```bash
rodeo run tool.luau --adapter lune
```

```luau
-- @rodeo run --adapter lute

local fs = require("@std/fs")
local path = require("@std/path")
print(fs.readFileToString(path.join("config", "settings.json")))
```

`--adapter` can be repeated, and a directive's adapters combine with the command line's. The TypeScript and Lune clients take an `adapters` option.

| Adapter | Runtime | Aliases |
|---------|---------|---------|
| `lune` | Lune 0.10.5 | `@lune/*` |
| `lute` | Lute 1.0.1-nightly.20260730 | `@lute/*`; Lute's standard library (`@std/*`) and batteries (`@batteries/*`) |

The first run that uses a runtime downloads it to `~/.rodeo/runtimes`. To use another build, set `RODEO_LUNE` or `RODEO_LUTE` to its executable.

The runtime runs on the machine running `rodeo run`, in its working directory and environment. Writes to stdout and stderr (`@lune/stdio`, `@lute/io`), `process.args` and `process.exit` are the run's own, and `task` and `@lute/time` run in Studio.

Without an adapter, these aliases resolve through your `.luaurc` like any other alias. A require that nothing resolves stays in the script and fails only if it runs.

## Caveats

- `@lune/roblox` isn't available; requiring it errors.
- A table or buffer from the runtime is a copy, updated each time it crosses again. Passed back, it arrives as the runtime's own, which takes the copy's contents. The runtime's other values, like a `DateTime` or a file handle, work through stand-ins whose `typeof` is `userdata`.
- Functions passed to the runtime, like a `net.serve` handler, run in Studio when the runtime calls them. One the runtime calls where it can't wait, like Lute's websocket upgrade handler, errors.
- A call's arguments or results can't exceed 16 MiB.
