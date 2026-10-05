---
title: CLI usage
---

The simplest way to drive rodeo is from the shell.

## Start the server

In one terminal:

```bash
rodeo serve
```

This starts the rodeo server and waits for Studio to connect. Open Studio (any place) and the installed plugin connects automatically.

## Port

Every command that talks to the server resolves its port the same way: `--port`, then the `RODEO_PORT` environment variable, then `44872`. The plugin's WebSocket is on the next port up.

A project pins its port next to its rodeo version, so projects on different ports run independently, including different rodeo versions:

```toml
# .mise.toml
[tools]
"ubi:revvy02/rodeo" = "1.5.0"

[env]
RODEO_PORT = "46800"
```

A `.env` file in the project works too. Projects that resolve to the same port share one serve, as before.

## Run a one-shot script

In another terminal:

```bash
rodeo run --source 'print("hi from studio")'
```

The script executes inside the connected Studio instance. Output streams back to your terminal.

You can also run a file:

```bash
rodeo run script.luau
```

## Launch a place

`rodeo run` can launch Studio for you:

```bash
# Launch by published place ID
rodeo run --place 12345 --source 'return game.PlaceId'

# Launch a local .rbxl file
rodeo run --place ./my-place.rbxl script.luau
```

Studio launches in the background and the script runs against it. By default rodeo closes the Studio it launched once the run finishes; pass `--detach` to keep it running.

## Launch a detached Studio

Pass `--detach` to spawn a Studio that isn't tied to the `rodeo run` process. Instead of closing the Studio when the script finishes, rodeo leaves it running:

```bash
# Launch a Studio and leave it up after the command exits
rodeo run --place 12345 --detach --source 'print("studio is up")'
```

The command returns, but the Studio stays open and connected to the server, so later `rodeo run` commands can target it without relaunching:

```bash
rodeo run --source 'return game.PlaceId'
```

When you're done, quit the Studio yourself; because it's detached, rodeo won't close it for you.

## Environment variables

| Variable | Effect |
|---|---|
| `RODEO_PORT` | Master port when `--port` isn't given. Default `44872`. See [Port](#port). |
| `RODEO_LAUNCH_TIMEOUT` | Seconds a Studio that rodeo launched has to connect before the launch fails and rodeo closes that Studio. Default `120`; `0` waits indefinitely. |
| `RODEO_SKIP_VERSION_CHECK` | `1` turns a build mismatch between the CLI, the serve and the plugin from an error into a warning. |
| `RODEO_PROFILE_FRAME_INTERVAL` | With `--profile`, frames between microprofiler captures. Default `60`. |
| `RODEO_PROFILE_NUM_FRAMES` | With `--profile`, frames in each microprofiler capture. Default `60`. |
| `RODEO_CAPTURE_DIR` | Directory `roblox.captureViewport` reads Studio's captures from. Default `~/Library/Roblox/tmp-capture-storage` on macOS, `%LOCALAPPDATA%\Roblox\tmp-capture-storage` on Windows. |
| `ROBLOX_STUDIO_PATH` | Studio install to launch instead of the installed one: the `.app` bundle on macOS; on Windows a version folder, or `%LOCALAPPDATA%\Roblox` for its newest version. |
| `RODEO_OPEN_CLOUD_API_KEY` | Open Cloud API key with the `asset:read` permission, used to look up a published place's latest version. |
| `RODEO_VERBOSE` | Any value: debug logging, as `--verbose`. |
| `RUST_LOG` | Log filter in `tracing` `EnvFilter` syntax, for example `rodeo=trace`. Overrides the default levels and `RODEO_VERBOSE`. |
| `RODEO_NO_TIMESTAMPS` | Any value: log lines on stderr have no timestamps. |
| `NO_COLOR` / `FORCE_COLOR` | Non-empty: colored log output off / on, whatever the terminal. |
| `RODEO_LOG_DIR` | Where the serve writes its log files. Default `.rodeo/.temp/logs`. |
| `RODEO_STUDIOMCP_VERBOSE` | Any value: StudioMCP (used by `--context elevated`) logs verbosely, alongside the serve's logs. |

The serve reads `RODEO_LAUNCH_TIMEOUT`, `RODEO_PROFILE_*`, `ROBLOX_STUDIO_PATH`, `RODEO_LOG_DIR` and `RODEO_STUDIOMCP_VERBOSE` when it starts. When `rodeo run` starts its own serve, set them for that command.

## Where to go next

See the full [CLI reference](/rodeo/cli/) for every subcommand and flag, or move on to [Runtime usage](/rodeo/getting-started/runtime-usage/) for what scripts can do inside Studio.
