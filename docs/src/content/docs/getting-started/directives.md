---
title: Directives
---

A **directive** is a single-line comment at the top of a Luau script that pre-fills the `rodeo run` flags. It lets a script declare its own runtime configuration (target, place, save behavior, fflags, and so on) so the caller doesn't have to remember the right `rodeo run` invocation, and so you don't have to orchestrate the script manually from a separate runtime via the rodeo client.

Directives have **full parity** with the [`rodeo run`](/rodeo/cli/#rodeo-run) subcommand. Any flag you can pass to `rodeo run` on the CLI can go inside a directive.

```luau
-- @rodeo run --place 12345 --context elevated --save

local workspace = game:GetService("Workspace")
print("running in", workspace.Name)
```

Then:

```bash
rodeo run my-script.luau
```

That's it. No flags needed at the call site; the script's directive runs it under `--place 12345 --context elevated --save`.

## Syntax

```
-- @rodeo run [flags] [-- script-args]
```

- Everything before `--` is parsed as flags for `rodeo run` (same flags as `rodeo run --help`).
- Everything after `--` becomes `process.args` inside the script.
- The directive must be on a single line; it can sit anywhere in the file but is conventionally the first line.

```luau
-- @rodeo run --place ./game.rbxl --mode test --context client -- --user frank --rooms 4

local process = require("@rodeo/process")
print(process.args)  --> { "--user", "frank", "--rooms", "4" }
```

## Override

User-supplied flags at the CLI call site override directive flags:

```bash
rodeo run my-script.luau --mode run --context server
# script ran with --mode run --context server, NOT the context from its directive.
```

This lets the directive set sensible defaults while leaving room for one-off overrides.

A `--studio-id` or `--dom-id` at the call site also overrides the directive's `--place`, along with the directive's `--save`, which saves the Studio that `--place` launches. A `--place` at the call site overrides the directive's `--studio-id` or `--dom-id`. To run a script whose directive launches a place in a Studio that is already open:

```bash
rodeo run my-script.luau --studio-id 6db9a5e3
# runs in that Studio; the directive's --place and --save are ignored, its other flags still apply.
```

## Short names

Scripts in `.rodeo/` can be invoked by bare name:

```bash
rodeo run demo
# resolves to .rodeo/demo.luau
```

Combined with directives, this turns `.rodeo/` into a project-local command palette: each script is a self-contained command with its own runtime config.
