---
title: CLI reference
description: Every rodeo subcommand and flag (auto-generated).
---

## `rodeo`

Command-line interface for Roblox Studio

**Usage:** `rodeo [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `serve` — Start persistent server (no Studio launch — use `run --place` for that)
* `run` — Run a script in Studio
* `state` — Show the canonical rodeo state: studios, their DOMs, and runs
* `kill` — Kill a run or close a Studio by id
* `save` — Save the Studio place
* `setup` — Generate type definitions and configure .luaurc

###### **Options:**

* `-v`, `--verbose` — Enable debug output



## `rodeo serve`

Start persistent server (no Studio launch — use `run --place` for that)

**Usage:** `rodeo serve [OPTIONS]`

###### **Options:**

* `--port <PORT>` — Master port. Resolution: this flag, then RODEO_PORT, then 44872. The studio backend listens on port + 1. With --studio, the studio backend's own port instead (default: --master-port + 1)
* `--master` — Run as master only (central orchestrator)
* `--studio` — Run as studio backend only (connects to master)
* `--master-host <MASTER_HOST>` — Master host to connect to (for --studio)

  Default value: `localhost`
* `--master-port <MASTER_PORT>` — Master port to connect to (for --studio). Resolution: this flag, then RODEO_PORT, then 44872
* `--ppid <PPID>` — Parent PID — exit when this process dies



## `rodeo run`

Run a script in Studio

**Usage:** `rodeo run [OPTIONS] [SCRIPT] [-- <SCRIPT_ARGS>...]`

###### **Arguments:**

* `<SCRIPT>` — Path to the script to execute, or '-' for stdin
* `<SCRIPT_ARGS>` — Script arguments (passed after --)

###### **Options:**

* `-s`, `--source <SOURCE>` — Execute source code passed as string. Hyphen-leading values are legal — Luau hotcomments (`--!native`, `--!optimize 2`) start scripts, and clap would otherwise reject them as flags
* `--sourcemap <SOURCEMAP>` — Path to sourcemap.json for instance resolution
* `--output <OUTPUT>` — Path to file for execution output (prints/logs)
* `--return <RETURN_FILE>` — Path to file for return value JSON
* `--show-return` — Print return value to stdout
* `--mode <MODE>` — Studio mode to run in (auto-transitions Studio, ending a running session of another kind first). Defaults to edit; never inferred from --context/--dom, so a server/client run must pass --mode explicitly (e.g. --mode run --context server). An explicit --mode edit ends a running session first; omit --mode (e.g. --dom edit) to leave it running

  Possible values: `edit`, `run`, `test`, `play`

* `--dom <DOM>` — Which DOM receives the script: edit, server, or client (usually inferred). `edit` targets the edit DOM even while a session runs

  Possible values: `edit`, `server`, `client`

* `--context <CONTEXT>` — Identity level the code executes at: plugin, server (server-runtime identity), client (client-runtime identity), elevated (command bar via StudioMCP), or cmdbar (command bar via the launch bootstrap's bridge — edit DOM of a rodeo-launched Studio, no StudioMCP needed)

  Possible values: `plugin`, `server`, `client`, `elevated`, `cmdbar`

* `--studio-id <STUDIO_ID>` — Scope routing to one studio by id (from `rodeo state`; unique prefix ok)
* `--no-warn` — Disable warning output
* `--no-error` — Disable error output
* `--no-info` — Disable info output
* `--no-print` — Disable print statements
* `--no-output` — Disable all output
* `--reload-requires` — Re-evaluate instance requires instead of reusing the VM's require cache. By default a require resolves to the live module the game is already using; this gives the run its own fresh copies
* `--ppid <PPID>` — Parent PID — exit when this process dies
* `--host <HOST>` — Host of running server

  Default value: `localhost`
* `--port <PORT>` — Master port of the running server. Resolution: this flag, then RODEO_PORT, then 44872

  Default value: `44872`
* `--place <PLACE>` — Launch Studio: empty (no value), place ID (number), or file path (.rbxl/.rbxlx)
* `--dom-id <DOM_ID>` — Pin the run to a specific DOM by id (from `rodeo state`; unique prefix ok). Only --context may accompany it — no mode/dom routing
* `--place.universe <UNIVERSE_ID>` — Universe ID (resolved from place ID if omitted)
* `--focus` — Bring Studio to the front on launch (default: background). Studio only takes keyboard focus when it opens on the display you are working on; on another display it is raised there and your focus stays put
* `--detach` — Keep Studio/Player running after rodeo exits
* `--show-widgets <WIDGETS>` — Allow-list of Studio dock widgets to keep visible; everything else (panels, ribbon, command bar) is hidden. `none` hides all; a comma list keeps those (aliases: output, explorer, properties, editor, toolbox, assistant, ribbon, commandbar, rodeo (this serve's own panel); or a raw panel ID). Restored on exit
* `--profile <PROFILE>` — Enable microprofiler auto-capture and collect dumps (optional: output directory)
* `--save <SAVE>` — Save Studio place on exit, optionally to a specific path. With --studio-id/--dom-id instead of --place, saves that Studio after a successful run, like `rodeo save <id> [--out <path>]`
* `--fflag.override <KEY=VALUE>` — Set FFlag override (Key=Value, repeatable)
* `--fflag.file <PATH>` — Load FFlag overrides from a JSON file



## `rodeo state`

Show the canonical rodeo state: studios, their DOMs, and runs

**Usage:** `rodeo state [OPTIONS]`

###### **Options:**

* `--json` — Print the raw state snapshot as JSON
* `--host <HOST>` — Host of running server

  Default value: `localhost`
* `--port <PORT>` — Master port of the running server. Resolution: this flag, then RODEO_PORT, then 44872

  Default value: `44872`



## `rodeo kill`

Kill a run or close a Studio by id

**Usage:** `rodeo kill [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>` — Run ID or Studio ID (from `rodeo state`; prefixes accepted). Killing a Studio fails its active runs as disconnected

###### **Options:**

* `--host <HOST>` — Host of running server

  Default value: `localhost`
* `--port <PORT>` — Master port of the running server. Resolution: this flag, then RODEO_PORT, then 44872

  Default value: `44872`



## `rodeo save`

Save the Studio place

**Usage:** `rodeo save [OPTIONS] [ID]`

###### **Arguments:**

* `<ID>` — Studio ID to save (from `rodeo state`; prefixes accepted). Defaults to the only connected Studio

###### **Options:**

* `--out <OUT>` — Copy saved file to this output path (overrides the default of copying back to the launch's SOURCE_PATH)
* `--host <HOST>` — Host of running server

  Default value: `localhost`
* `--port <PORT>` — Master port of the running server. Resolution: this flag, then RODEO_PORT, then 44872

  Default value: `44872`



## `rodeo setup`

Generate type definitions and configure .luaurc

**Usage:** `rodeo setup`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>