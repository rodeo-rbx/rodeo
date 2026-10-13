---
title: Bundling
---

When you pass a file to `rodeo run`, rodeo resolves and bundles its dependencies into a single script before sending it to Studio. You don't need to flatten your code, vendor your deps, or invoke a separate bundler. It's always-on.

```bash
rodeo run my-script.luau
```

Filesystem requires (`require("./util")`, `require("../helpers")`), Wally / pesde package requires (via sourcemap), and `@rodeo` runtime requires are all handled automatically. The resulting bundle runs inside the Studio DOM.

## Runtime adapters

An adapter runs code written for another Luau runtime: that runtime's standard library requires resolve to rodeo's implementations. Enable one per run with `--adapter`, or in the script's [directive](/rodeo/getting-started/directives/):

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

| Adapter | Aliases |
|---------|---------|
| `lune` | `@lune/fs`, `@lune/process`, `@lune/serde` (json), `@lune/stdio`, `@lune/task` |
| `lute` | `@lute/fs`, `@lute/io`, `@lute/process`, `@lute/system`, `@lute/task`, `@lute/time`; Lute's standard library (`@std/*`) and batteries (`@batteries/*`) |

Without an adapter, these aliases resolve through your `.luaurc` like any other alias. A require that nothing resolves stays in the script and fails only if it runs.

## Caveats

Adapter coverage isn't 1:1. Only the parts of each runtime that map cleanly to rodeo are supported; anything else throws a clear error when it's used. In the `lute` adapter, `@lute/crypto`, `@lute/debugger`, `@lute/luau`, `@lute/net`, `@lute/syntax` and `@lute/vm` throw when required, as do `@std/luau`, `@std/net` and `@std/syntax`.
