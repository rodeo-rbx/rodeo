---
title: Profiling
---

rodeo can capture Roblox's microprofiler data while your script runs, then hand
you the dumps to inspect.

## 1. Launch a recording Studio

In one terminal, start a Studio with profiling enabled. This command keeps
running, holding the instance open:

```bash
rodeo run --place ./game.rbxl --profile ./profiles --port 44875
```

## 2. Run code and collect dumps

In another terminal, send a script to that Studio. Point `--profile` at the same
directory and `--port` at the same server:

```bash
rodeo run profile-me.luau --profile ./profiles --port 44875
```

rodeo captures the microprofiler frames recorded while your script was running
and writes the dumps into `./profiles`. This command doesn't launch or close
Studio, so you can run it as many times as you like against the same instance.

While a serve runs, rodeo deletes Studio's auto-capture dumps from Roblox's
`ProfilerCaptures` folder five minutes after they're written. The copies in
`./profiles` stay.

## 3. Analyze the dumps

Once the dumps are collected, you can either:

- Analyze the aggregated microprofiler dumps with
  [rbx-microprofiler](https://github.com/revvy02/rbx-microprofiler), or
- Combine them into a single HTML dump you can open in a browser.
