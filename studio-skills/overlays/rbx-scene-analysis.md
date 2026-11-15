---
description: Analyze a Roblox scene with SceneAnalysisService: rendering cost, memory, instance composition and unparented instances. Use when investigating performance, memory or leaks.
---

`SceneAnalysisService` only works at **runtime**, so run every query against a play DM: `rodeo run --mode test --context client` for rendering/triangle queries (view-dependent), `--mode test --context server` for memory/unparented/leak queries (server-side leaks are common). `--mode test` starts the playtest itself; over the Studio MCP, call `start_stop_play` first. `commands/` holds the four analysis routines; `QuerySpecs.md` documents the return shapes.
