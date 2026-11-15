---
description: Set breakpoints and inspect threads, stacks and variables in a running Roblox game with ScriptDebuggerService. Use to investigate script bugs beyond static analysis.
---

## Driving this externally (rodeo / Studio MCP) — VERIFIED

The base skill below assumes the in-Studio Assistant's privileged context. From outside (rodeo
`--context mcp` or MCP `execute_luau` — the same channel), it works, but follow this recipe and heed
the pitfalls; both were learned the hard way.

### Deadlock-free recipe
1. Play must be RUNNING first (`start_stop_play(true)`, or `rodeo run --mode test`, which starts a
   playtest). Breakpoints, `OnStopped`, and inspection need ELEVATED identity in the play DM —
   plugin identity sees `ScriptDebuggerService` as `nil`. `OnStopped` is per-DM, so do this in each
   DM you debug (Server / Client).
2. **Arm `OnStopped` BEFORE the breakpoint exists:** set `dbg.OnStopped = handler` first, THEN
   `local bp = dbg:AddBreakpoint(playInstance, {Line=N})`. The handler reads via `Evaluate` and
   returns `Enum.DebuggerResumeType.Resume`. Guard it (`if captured then return Resume end`) — it
   fires every frame for a per-frame line.
3. Poll for the handler's captured result (fires the next frame the line runs).
4. Cleanup: `dbg:RemoveBreakpoint(bp); dbg.OnStopped = nil`. **Never `ClearBreakpoints()`** — it
   wipes the user's breakpoints.

### Pitfalls
- **No attaching to an already-parked frame.** A breakpoint that hits with no armed handler parks
  the whole DM; the only channel that could install one runs *in* that frozen DM → deadlock. You
  also can't resume it from here — only Studio **Continue** / the command bar, or `start_stop_play(false)`
  (stop), releases it. So always arm-then-break; never let a breakpoint pre-exist your handler.
- **Break the PLAY-DM instance** (find the script in the running DM, not the edit copy), on a line
  that runs **every frame unconditionally** — e.g. a `for … in q do` statement line, not a loop body
  that may have zero iterations.
- **`Evaluate(expr, frameId)` = the command-bar:** runs code in the live frame — reads locals+upvalues,
  calls methods (`world:query(cts.X)`, `world:get(e, c)`), and mutates, all on live refs (verified on a
  real system: `world.max_component_id=215`, `world:query(cts.Renderable)` = 229 live entities). Only
  the *returned* value is marshalled. `GetRootVariables`/`GetVariables` return only string descriptors
  + handles — prefer `Evaluate`.
- **`world`/`cts`/`resources` are upvalues only where the frame fn references them.** If `cts` isn't
  in scope at your line, require it inline: `Evaluate("world:query(require(game…cts).Foo)", fid)`.
- **Dead-variable elimination:** a local/param past its last use reads `nil` (Luau frees the register)
  — break where it's still live, or use upvalues (stable). `getfenv`/`debug.info` are sandboxed out.
- **Frame selection:** `GetStackTrace().Frames[1]` is the top frame; pass its `.Id` to `Evaluate` /
  `GetRootVariables`. For a specific function's locals, pick the frame at your break line.
- **Authoring:** pass debugger scripts via the `execute_luau` code param or a file (`rodeo run <file>`),
  not inline `rodeo run --source` (shell mangles multi-line Luau).

Everything below is the engine API.

<!-- after: # Roblox Studio Debugging with rbx-debug Skill -->
> **Reconcile with the top recipe.** The reference below is Roblox's, written for the **in-Studio
> Assistant** (already elevated, with a privileged debugger channel). Where it assumes that context,
> the **"Driving this externally" recipe at the top wins** for rodeo/MCP use — verified overrides:
> - **Identity:** "plugin-level security, usable by plugins" holds for the Assistant, but via
>   rodeo/MCP in a *play* DM plugin identity gets `nil` — use rodeo's **`--context mcp`**.
> - **Late-set `OnStopped`:** does **not** work externally (you can't run code in a parked DM) —
>   **arm before the hit**.
> - **`ClearBreakpoints()`:** don't — it wipes the user's breakpoints; track and `RemoveBreakpoint`
>   your own.

<!-- after: If `OnStopped` is set while the DataModel is already stopped at a breakpoint -->
> **External caveat (rodeo / Studio MCP):** late-set requires running code *in* the parked DM — the in-Studio Assistant / command bar can, but `execute_luau` **cannot** (a parked DM is frozen to thread injection → the call times out, deadlock). Externally you must arm `OnStopped` *before* the breakpoint can hit; you also can't resume a frame parked without your handler — only Studio **Continue** or `start_stop_play(false)` releases it.
