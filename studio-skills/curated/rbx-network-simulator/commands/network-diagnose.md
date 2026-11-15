Debug a gameplay problem that shows up on bad connections: reproduce it under a network preset, measure it, find the responsible code, and suggest a fix.

## What to do

1. Connect to Studio (multi-instance: `list_roblox_studios` + `set_active_studio`). **Read and record the current `settings().Network` values first** (the starting values) — don't assume zero.

2. **Classify the symptom** using SKILL.md "Common bad-network issues and how to debug them". Ask one short question if it's ambiguous (e.g. which button or weapon):
   - Delayed feedback after input → the action → server → result round trip, plus timing that specific action.
   - Hits not registering → the movement measurement (divergence, server lag), plus timing the attack with target health as the result (hit rate).
   - Longer join times → join probe.
   - Rubber-banding → the movement measurement (snaps, divergence at rest).
   - Errors only on bad connections → playtest under high latency/jitter + `get_console_output`.

3. **Measure baseline then shaped.** If you'll use the round-trip or movement measurements, first install the probe with `datamodel_type = "Edit"` (SKILL.md "Install the probe"); it's needed before the playtest starts. Apply **Ideal Fiber**, playtest, run the chosen probes. Stop. Apply the preset matching the report (default **Bad Connection (3G)**; use the user's numbers if they gave them), playtest, run the same probes. Stop.

4. **Find the cause in code.** Read the scripts the symptom points to (the remote's client and server handlers, hit validation, startup/loading scripts, server scripts that move characters). Tie what you find to the measured numbers, e.g. "server distance check allows 4 studs; measured divergence under 3G is 5.1 studs".

5. **Report** (bullets — no Markdown tables, no fenced code blocks for results): symptom → measured baseline vs shaped → cause in code (file and line) → suggested fix. Use "configured" vs "measured" correctly.

6. **Optionally fix and verify.** If the user agrees, apply the fix, then re-run the same shaped measurement and report before/after.

7. **Clean up and ask how to finish.** Stop the playtest and run the "Remove the probe" snippet with `datamodel_type = "Edit"`. It removes only the `__NetProbe` and `__NetJoinProbe` objects marked with `__NetSimProbe`; confirm none remain, and mention any same-named objects it left alone. For the network knobs, ask (SKILL.md "Ending a test"): restore the starting values (offer this first if they weren't 0), clear to 0, or keep the last preset. Don't clear automatically.

## Notes

- The simulator shapes only the client↔server connection (no bandwidth cap, no asset/HTTP slowdown). If the symptom is really download size or HTTP, say so instead of over-claiming.
- Measure the specific interaction the user reported. Generic probes give context but aren't proof on their own.
- Shaping applies during a playtest (Play or Team Test), not while editing.
