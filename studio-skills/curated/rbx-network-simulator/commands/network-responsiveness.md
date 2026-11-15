Measure how responsive the game feels under a network preset: action → result latency, how long the server takes to catch up to client input, and client-server consistency.

## What to do

1. Connect to Studio (multi-instance: `list_roblox_studios` + `set_active_studio`). **Read and record the current `settings().Network` values first** (the starting values) — don't assume zero; settings persist and may already be shaped.

2. **Pick the conditions.** Use the preset or values the user named (default: **Bad Connection (3G)**). Always also measure **Ideal Fiber** as the baseline so every number has a comparison.

3. **Install the probe** once, before the first playtest, with `datamodel_type = "Edit"` (SKILL.md "Install the probe"). It adds a temporary `__NetProbe` RemoteEvent and server Script, marked with the `__NetSimProbe` attribute. Tell the user, and ask them not to save until it's removed. If it returns `STOPPED: ... already exists`, the place has its own object with that name: don't touch it. Tell the user and skip the probe-based measurements.

4. **For each condition** (baseline first):
   - Apply the preset via `execute_luau` and read it back.
   - `start_stop_play` to enter a playtest; wait a few seconds for the character to spawn and the connection to settle.
   - Read the client `NetworkPing` (`datamodel_type = "Client"`; see SKILL.md "Verifying the real effect") for context.
   - **Measure the action → server → result round trip** with `datamodel_type = "Client"`.
   - **Measure movement** (server catch-up + client-server consistency) with `datamodel_type = "Client"`. If it reports the character didn't move, use the MoveTo fallback.
   - **Time a real game interaction** (optional, recommended): if the user named an interaction, or the game has an obvious core one (button, ability, purchase), read the game's scripts to find its input and the replicated value it changes, fill in the template, and run it with `datamodel_type = "Client"`.
   - Stop the playtest before applying the next condition (the probe stays installed between playtests).

5. **Remove the probe** right after the last playtest stops, with `datamodel_type = "Edit"` (SKILL.md "Remove the probe"). Confirm it reports the objects removed.

6. **Report** using the responsiveness format in SKILL.md (bullets — **no Markdown tables, no fenced code blocks**), each value next to its baseline. Explain what the numbers mean for this game: local response should stay ~1–3 frames (prediction); round trip and server catch-up grow with the shaping; divergence in studs is how far off server-side hit/touch checks are; snaps or a lasting offset at rest mean rubber-banding.

7. **Ask how to finish** (SKILL.md "Ending a test"): restore the starting values (offer this first if they weren't 0), clear to 0, or keep the last preset. Don't clear automatically.

## Notes

- Round trips use `os.clock()` in one datamodel and are exact to ~1 frame. One-way splits and server catch-up use `workspace:GetServerTimeNow()` across client and server — label them **estimates**.
- Don't report sub-frame (<~16ms) differences as meaningful.
- If a measurement says the probe is missing or returns no server trace, stop the playtest, check that both `__NetProbe` objects exist in the Edit datamodel (install them if not), and start again. Don't create the probe from the play server; it may not replicate to the client.
- Shaping applies during a playtest (Play or Team Test), not while editing.
