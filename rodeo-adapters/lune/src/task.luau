-- @lune/task shim — Roblox's task, with wait/delay durations clamped to
-- lune's graceful out-of-range handling (lune issue #346): NaN, negative,
-- infinite, or absurd durations become 0. Roblox task.wait only defaults
-- NaN/inf to 0 — a finite-but-huge duration (1e30) would wait literally.
local function clampDuration(t: number?): number
	if type(t) ~= "number" or t ~= t or t < 0 or t > 86400 * 365 then
		return 0
	end
	return t
end

-- Members the shim does not provide raise an error naming them, instead of
-- indexing to nil and failing later as "attempt to call a nil value".
return setmetatable({
	cancel = task.cancel,
	defer = task.defer,
	spawn = task.spawn,
	delay = function(duration: number?, fn, ...)
		return task.delay(clampDuration(duration), fn, ...)
	end,
	wait = function(duration: number?): number
		return task.wait(clampDuration(duration))
	end,
}, {
	__index = function(_, key)
		error(`@lune/task.{key} is not supported by rodeo's Lune adapter`, 2)
	end,
})
