--!strict
--[[
	mode_policy.luau — the decisions behind mode_driver.luau, as functions of
	what one DOM can observe. No Roblox globals, so it runs under lune
	(tests/plugin/mode_policy.spec.luau) as well as in Studio.

	Server DOM: ends the live session when it doesn't satisfy the target.
	Edit DOM: starts the target's session once the engine is idle, and gives
	up with a reason instead of retrying forever.
]]

export type SessionKind = "run" | "test" | "play"

export type Observation = {
	now: number,
	-- StudioTestService.EditModeActive, read in the edit DOM: true while
	-- Studio can start a session, false while one is running or tearing
	-- down. nil when it can't be read (the property is undocumented).
	editModeActive: boolean?,
	-- A session start of ours has not returned yet. A successful
	-- Execute*Async yields for the whole session it started.
	startPending: boolean,
}

export type EditState = {
	target: string,
	since: number,
	idleSince: number?,
	busySince: number?,
	startErrors: number,
	lastError: string?,
	-- One start was made although the engine reported busy (see stepEdit).
	forced: boolean,
}

export type Action = "wait" | "start" | "fail"

local M = {}

-- How long the engine must report idle before the edit DOM starts a session.
-- Works around a Studio bug: a start fired while the previous session is
-- still tearing down never returns and never starts, and the studio can't
-- enter play again until restarted (measured on Studio 0.734: teardown + 0 s
-- hung, teardown + 1 s started play). Waiting for a settled idle engine keeps
-- every start out of that window. To check whether Roblox fixed it: enter run
-- mode, EndTest from the server DataModel and immediately ExecutePlayModeAsync
-- from the edit DataModel; if play starts, this can drop to 0.
M.IDLE_SETTLE = 1
-- Execute*Async errors on an idle engine, for one target, before giving up
-- (attempts are a settled second or more apart, so ~12 s). Enough for a Studio
-- still finishing its startup; an engine that keeps refusing an idle start is
-- not transient (#27: "a previous one is still in progress", retried once a
-- second for good).
M.MAX_START_ERRORS = 10
-- How long the engine may report a session in progress, with no start of
-- ours pending, before the edit DOM concludes nothing rodeo runs is ending
-- it. Ending a session (the server DOM's EndTest) takes a few seconds.
M.BUSY_TIMEOUT = 30
-- Overall bound on one target, from when it arrived. Starting a session and
-- connecting its DataModels takes seconds; this only has to outlast a slow
-- place.
M.START_TIMEOUT = 120

-- The StudioTestService call that starts each target's session.
M.START_API = {
	run = "ExecuteRunModeAsync",
	test = "ExecutePlayModeAsync",
	play = "ExecuteMultiplayerTestAsync",
} :: { [string]: string }

-- Server DOM: whether the live session must end for `target`. `session` is
-- what this server knows about its own session: "run" (run mode), "test"
-- (solo play test), "play" (multiplayer test), or nil while a play session's
-- kind is still unknown — then it is left alone rather than ended on a guess.
function M.serverShouldEnd(target: string, session: SessionKind?): boolean
	if target == "edit" then
		return true
	elseif target == "run" then
		return session ~= "run"
	elseif target == "test" then
		return session == "run" or session == "play"
	elseif target == "play" then
		return session == "run" or session == "test"
	end
	return false
end

function M.newEdit(target: string, now: number): EditState
	return {
		target = target,
		since = now,
		idleSince = nil,
		busySince = nil,
		startErrors = 0,
		lastError = nil,
		forced = false,
	}
end

-- Record an Execute*Async error for this target's attempt.
function M.noteStartError(s: EditState, err: string)
	s.startErrors += 1
	s.lastError = err
end

local function api(s: EditState): string
	return `StudioTestService:{M.START_API[s.target] or "?"}`
end

local function startFailure(s: EditState): string
	local call = api(s)
	if s.forced then
		local busy = `StudioTestService.EditModeActive stayed false for {M.BUSY_TIMEOUT}s`
		return `{call} failed: {s.lastError} (Studio still reports a session in progress: {busy}; `
			.. "stop it in Studio, or restart Studio)"
	end
	return `{call} failed {s.startErrors} times: {s.lastError}`
end

local function timeoutFailure(s: EditState, o: Observation): string
	local parts = {}
	if o.startPending then
		table.insert(
			parts,
			`{api(s)} was called but no {s.target} DataModel connected to rodeo within {M.START_TIMEOUT}s (restart Studio if this persists)`
		)
	else
		table.insert(parts, `timed out after {M.START_TIMEOUT}s`)
	end
	if s.lastError then
		table.insert(parts, `last error from {api(s)}: {s.lastError}`)
	end
	if o.editModeActive == false then
		table.insert(parts, "StudioTestService.EditModeActive is false (Studio considers a session in progress)")
	end
	return table.concat(parts, "; ")
end

-- Edit DOM, once per poll: start the target's session, keep waiting, or give
-- up (the second return is then the reason).
function M.stepEdit(s: EditState, o: Observation): (Action, string?)
	if s.target == "edit" then
		-- Nothing to start: the server DOM ends the session. Only a session
		-- that never ends is worth reporting.
		if o.editModeActive == false then
			s.busySince = s.busySince or o.now
			if o.now - (s.busySince :: number) >= M.BUSY_TIMEOUT then
				return "fail",
					`the running session did not end within {M.BUSY_TIMEOUT}s (StudioTestService.EditModeActive is still false)`
			end
		else
			s.busySince = nil
		end
		if o.now - s.since >= M.START_TIMEOUT then
			return "fail", `Studio did not return to edit within {M.START_TIMEOUT}s`
		end
		return "wait"
	end

	if s.startErrors >= M.MAX_START_ERRORS or (s.forced and s.lastError ~= nil) then
		return "fail", startFailure(s)
	end
	if o.now - s.since >= M.START_TIMEOUT then
		return "fail", timeoutFailure(s, o)
	end

	-- Our own start is still running: either its session is up (the master
	-- clears the target once the queued runs route to it) or it is starting.
	-- Never stack a second start on it.
	if o.startPending then
		s.idleSince = nil
		s.busySince = nil
		return "wait"
	end

	if o.editModeActive == false then
		-- Another session is running or tearing down; the server DOM ends it
		-- when it doesn't satisfy the target.
		s.idleSince = nil
		s.busySince = s.busySince or o.now
		if not s.forced and o.now - (s.busySince :: number) >= M.BUSY_TIMEOUT then
			-- Nothing ended it. Try once anyway: if the session is real, the
			-- engine's refusal is the most useful thing to report (#27); if
			-- EditModeActive was simply wrong, the start goes through. An
			-- older error is dropped so only this attempt's outcome decides.
			s.forced = true
			s.lastError = nil
			return "start"
		end
		return "wait"
	end

	-- Idle (or EditModeActive unreadable: then only our own start counts as
	-- busy). Start once the idle streak has settled.
	s.busySince = nil
	s.idleSince = s.idleSince or o.now
	if o.now - (s.idleSince :: number) >= M.IDLE_SETTLE then
		s.idleSince = nil
		return "start"
	end
	return "wait"
end

return M
