-- @lute/task shim — Roblox's task, extended with lute's additions: resume,
-- deferSelf, and Duration-typed waits/delays (@lute/time values carry a
-- toSeconds method; numbers pass through).
local function toSeconds(duration): number?
	if duration == nil then
		return nil
	end
	if type(duration) == "number" then
		return duration
	end
	return duration:toSeconds()
end

return {
	spawn = task.spawn,
	defer = task.defer,
	resume = function(thread: thread): thread
		coroutine.resume(thread)
		return thread
	end,
	deferSelf = function()
		task.defer(coroutine.running())
		coroutine.yield()
	end,
	wait = function(duration): number
		return task.wait(toSeconds(duration))
	end,
	delay = function(duration, routine, ...)
		return task.delay(toSeconds(duration) or 0, routine, ...)
	end,
}
