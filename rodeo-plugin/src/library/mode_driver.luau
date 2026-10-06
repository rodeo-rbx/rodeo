--!strict
--[[
	mode_driver.luau — plugin-side convergence loop. The master sends every
	DOM of a studio its target mode (SetTargetModeMsg); each DOM does its part
	until the target is superseded:

	- Server DOM (IsServer, not edit) ends a session that doesn't satisfy the
	  target: EndTest. That includes switching session kinds (run <-> test <->
	  play) and target "edit".
	- Edit DOM (IsEdit) starts the target's session once the engine is idle:
	  ExecuteRunModeAsync / ExecutePlayModeAsync / ExecuteMultiplayerTestAsync.
	  When it can't — the engine keeps refusing, or the studio never gets
	  there — it gives up and reports why (ModeTransitionFailed), so the
	  master fails the runs waiting on the transition instead of leaving them
	  queued forever.
	- Client DOMs: no-op.

	The decisions live in mode_policy.luau (pure, unit-tested under lune).
]]

local RunService = game:GetService("RunService")
local StudioTestService = game:GetService("StudioTestService")

local mode_policy = require(script.Parent.mode_policy)
local studio = require(script.Parent.studio)

local isEdit = RunService:IsEdit()
local isServer = RunService:IsServer()

local POLL_INTERVAL = 0.25
-- Pause between EndTest attempts while the session is still up.
local END_RETRY_INTERVAL = 1

local currentTarget = ""
local currentPlayers = 0
local token = 0
-- The edit DOM reported a failure for currentTarget and stopped.
local gaveUp = false

export type Reporter = (targetMode: string, err: string) -> ()
local reporter: Reporter? = nil

-- A session start of ours that hasn't returned. A successful Execute*Async
-- yields for the whole lifetime of the session it started and returns when
-- that session ends, so this stays set while the session is up. It is module
-- state, not per-target: the call belongs to the session, which outlives the
-- target that started it. (It used to be cleared on every new target, so a
-- re-sent target fired a second start onto the live session.)
local startPending = false
local endPending = false

-- StudioTestService.EditModeActive is undocumented: true while Studio can
-- start a session, false while one runs or tears down. nil if unreadable.
local function readEditModeActive(): boolean?
	local ok, value = pcall(function()
		return (StudioTestService :: any).EditModeActive
	end)
	if ok and type(value) == "boolean" then
		return value
	end
	return nil
end

-- Test args tag the sessions rodeo starts with their kind, so a play server
-- knows it is a solo test or a multiplayer test before any client reports
-- (studio.luau reads them back through GetTestArgs).
local function sessionArgs(kind: string): { [string]: any }
	return { rodeo = true, rodeoSession = kind }
end

local function startSession(target: string, numPlayers: number, onError: (string) -> ())
	startPending = true
	task.spawn(function()
		local sts = StudioTestService :: any
		local ok, err = pcall(function()
			if target == "run" then
				sts:ExecuteRunModeAsync(sessionArgs("run"))
			elseif target == "test" then
				sts:ExecutePlayModeAsync(sessionArgs("test"))
			else
				sts:ExecuteMultiplayerTestAsync(numPlayers, sessionArgs("play"))
			end
		end)
		startPending = false
		if not ok then
			local msg = tostring(err)
			warn(`[rodeo] mode_driver StudioTestService:{mode_policy.START_API[target]} failed: {msg}`)
			onError(msg)
		end
	end)
end

local function editLoop(myToken: number, target: string, numPlayers: number)
	local state = mode_policy.newEdit(target, os.clock())
	while token == myToken do
		local action, reason = mode_policy.stepEdit(state, {
			now = os.clock(),
			editModeActive = readEditModeActive(),
			startPending = startPending,
		})
		if action == "start" then
			startSession(target, numPlayers, function(err)
				-- The error belongs to this target's attempt only.
				if token == myToken then
					mode_policy.noteStartError(state, err)
				end
			end)
		elseif action == "fail" then
			gaveUp = true
			local msg = reason or "unknown failure"
			warn(`[rodeo] mode_driver could not enter {target} mode: {msg}`)
			local report = reporter
			if report then
				report(target, msg)
			end
			return
		end
		task.wait(POLL_INTERVAL)
	end
end

local function serverLoop(myToken: number, target: string)
	while token == myToken do
		local kind: mode_policy.SessionKind? = if RunService:IsRunMode() then "run" else studio.getSessionKind()
		if not endPending and mode_policy.serverShouldEnd(target, kind) then
			endPending = true
			task.spawn(function()
				local ok, err = pcall(function()
					StudioTestService:EndTest("end_test")
				end)
				if not ok then
					warn(`[rodeo] mode_driver StudioTestService:EndTest failed: {err}`)
				end
				task.wait(END_RETRY_INTERVAL)
				endPending = false
			end)
		end
		task.wait(POLL_INTERVAL)
	end
end

local function setTarget(target: string?, numPlayers: number?)
	local t = target or ""
	local n = numPlayers or 0
	-- The master re-sends an unchanged target whenever the studio's DOM set
	-- changes (that is how a newly connected DOM learns it). Restarting here
	-- used to reset the in-flight guard and stack a second start onto a live
	-- session. A target the edit DOM gave up on is the exception: the master
	-- answers a failure report by clearing the target, so the same target
	-- arriving again means the report was lost (reconnect) or crossed this
	-- message, and the queued runs need a fresh attempt.
	if t == currentTarget and n == currentPlayers and not gaveUp then
		return
	end
	currentTarget, currentPlayers, gaveUp = t, n, false
	token += 1
	if t == "" then
		return
	end
	if isEdit then
		task.spawn(editLoop, token, t, n)
	elseif isServer then
		task.spawn(serverLoop, token, t)
	end
end

-- Where the edit DOM sends a transition failure: the current connection's
-- send, or nil while disconnected.
local function setReporter(fn: Reporter?)
	reporter = fn
end

return {
	setTarget = setTarget,
	setReporter = setReporter,
}
