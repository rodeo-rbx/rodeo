List the built-in network presets and apply one by name.

## What to do

1. Connect to Studio (multi-instance: `list_roblox_studios` + `set_active_studio`). Read and record the current `settings().Network` values as the starting values (always available).

2. **If the user just wants to see the options**, list the built-in presets from SKILL.md "Presets" as a **bullet list** (one bullet per preset with its in/out latency/jitter/loss) — **not a Markdown table** (can't render) and **not a fenced code block** (renders gray/de-emphasized). Include any custom values they've defined this session, and the units note (loss is a percent; 0.5 = 0.5%).

3. **If the user names a preset**, resolve it (built-in name, fuzzy term, or custom) and apply it via `execute_luau` (SKILL.md "Applying conditions"), clamped and in a `pcall`.

4. **Verify** by reading the six values back (SKILL.md "Reading back"). Report what was applied.

5. Remind the user that shaping is live only during a playtest (Play or Team Test), and that settings **persist across Studio restarts**. The preset stays applied; if it replaced non-zero values (e.g. the user's own preset), mention you can restore those (SKILL.md "Ending a test"), and that `/network-reset` clears to 0.

## Notes

- User-saved presets: the skill **cannot** read them (`plugin` is nil in `execute_luau`; no preset API). Don't attempt `plugin:GetSetting`. If the user names a saved preset, ask them to select it in the toolbar and click **Apply**, then read `settings().Network` and remember those values for the session — or have them paste the values. See SKILL.md "User-saved presets".
- Don't confuse the loss units — `0.5` is 0.5%, not 50%.
