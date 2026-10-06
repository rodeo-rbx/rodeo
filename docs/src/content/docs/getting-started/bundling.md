---
title: Bundling
---

When you pass a file to `rodeo run`, rodeo resolves and bundles its dependencies into a single script before sending it to Studio. You don't need to flatten your code, vendor your deps, or invoke a separate bundler. It's always-on.

```bash
rodeo run my-script.luau
```

Filesystem requires (`require("./util")`, `require("../helpers")`), Wally / pesde package requires (via sourcemap), and `@rodeo` runtime requires are all handled automatically. The resulting bundle runs inside the Studio DOM.

## Cross-runtime adapters

Adapters let you run scripts that were originally written for **other Luau runtimes** (Lune today; more later) without modifying the source. Each adapter re-implements the foreign runtime's standard library on top of `@rodeo`.

The Lune adapter covers:

| Lune module |
|-------------|
| `@lune/fs` |
| `@lune/process` |
| `@lune/stdio` |
| `@lune/task` |
| `@lune/serde` |

So a Lune-targeted script like:

```luau
local fs = require("@lune/fs")
local process = require("@lune/process")

local config = fs.readFile("config.json")
local result = process.exec("git", { "rev-parse", "--short", "HEAD" })
print(config, result.stdout)
```

runs unmodified under `rodeo run`.

## Caveats

Adapter coverage isn't 1:1. Only the parts of each runtime's stdlib that map cleanly to `@rodeo` are supported. Anything missing throws a clear error at runtime.
