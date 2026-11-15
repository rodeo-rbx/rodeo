Clear all network shaping back to 0 (no simulation).

## What to do

1. Connect to Studio (multi-instance: `list_roblox_studios` + `set_active_studio`).

2. **Reset** via `execute_luau` — set all six `settings().Network` knobs to 0 (SKILL.md "Resetting"), wrapped in `pcall`.

3. **Verify** by reading the six values back — all should be 0.

4. Tell the user: "Network simulation cleared — all latency, jitter, and loss set to 0."

## Notes

- Setting everything to 0 is the true "off" state (engine default). This matches the toolbar's Reset button intent, and is exactly the **"Current Device"** preset (`NetworkType::None`, all zeros). "Reset the network", "turn off simulation", and "apply Current Device" are the same operation.
- Settings **persist across Studio restarts** (saved to local Studio settings, global across places), so a restart will NOT clear them — this reset is the only way to clear shaping.
- An explicit reset **always clears to 0 immediately**, without asking, even if the user had their own preset before. That's what they asked for. If they want their previous values back instead, that's a *restore* (SKILL.md "Ending a test").
