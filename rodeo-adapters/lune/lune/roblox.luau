-- @lune/roblox has no rodeo counterpart: its instances and Roblox types are
-- Lune's own reimplementation, which Studio's can't stand in for. Requiring it
-- errors, so a script that only mentions it still bundles.
error("@lune/roblox is not supported by rodeo's Lune adapter; use Studio's own APIs", 0)
return nil -- unreachable; darklua bundles only modules that end with a return
