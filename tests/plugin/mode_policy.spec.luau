--!strict
-- Unit spec for rodeo-plugin/src/library/mode_policy.luau, run under lune by
-- tests/plugin/modePolicy.test.ts. Each edit-DOM case replays what the edit
-- DOM observes once per poll (time, StudioTestService.EditModeActive, whether
-- a start of ours is pending) and checks what it decides.
local policy = require("../../rodeo-plugin/src/library/mode_policy")

local failures = 0
local function check(ok: boolean, what: string)
	if not ok then
		failures += 1
		print(`FAIL {what}`)
	end
end

local IN_PROGRESS = "Failed to start the test because a previous one is still in progress."

-- Server DOM: which sessions each target ends. nil = a play session whose
-- solo/multiplayer kind isn't known yet.
do
	local kinds: { any } = { "run", "test", "play", "unknown" }
	local expected: { [string]: { [string]: boolean } } = {
		[""] = { run = false, test = false, play = false, unknown = false },
		edit = { run = true, test = true, play = true, unknown = true },
		run = { run = false, test = true, play = true, unknown = true },
		-- #39: switching session kinds ends the live one first. test and play
		-- used to satisfy each other, so a solo test was never ended for play.
		test = { run = true, test = false, play = true, unknown = false },
		play = { run = true, test = true, play = false, unknown = false },
	}
	for target, row in expected do
		for _, kind in kinds do
			local session = if kind == "unknown" then nil else kind
			local got = policy.serverShouldEnd(target, session)
			check(got == row[kind], `serverShouldEnd({target}, {kind}) = {got}, want {row[kind]}`)
		end
	end
end

-- Drive one edit state from t0 to t1 (inclusive) in 0.25 s polls with a fixed
-- observation; returns the first non-wait action and the time it happened.
local function runUntil(
	s: policy.EditState,
	t0: number,
	t1: number,
	editModeActive: boolean?,
	startPending: boolean
): (policy.Action, string?, number)
	local t = t0
	while t <= t1 + 1e-9 do
		local action, reason =
			policy.stepEdit(s, { now = t, editModeActive = editModeActive, startPending = startPending })
		if action ~= "wait" then
			return action, reason, t
		end
		t += 0.25
	end
	return "wait", nil, t1
end

-- Fresh studio: start once the engine has been idle for IDLE_SETTLE.
do
	local s = policy.newEdit("test", 0)
	local action, _, at = runUntil(s, 0, 5, true, false)
	check(
		action == "start" and at == policy.IDLE_SETTLE,
		`idle engine: {action} at {at}, want start at {policy.IDLE_SETTLE}`
	)
end

-- #27/#39: never start while the engine reports a session (running or
-- tearing down); start a settled second after it goes idle.
do
	local s = policy.newEdit("test", 0)
	local action = runUntil(s, 0, 10, false, false)
	check(action == "wait", `busy engine: {action}, want wait`)
	local after, _, at = runUntil(s, 10.25, 15, true, false)
	check(after == "start" and at == 10.25 + policy.IDLE_SETTLE, `after teardown: {after} at {at}`)
end

-- No stacking: while our own start is pending (its session is up or coming
-- up), never start again, even if the engine looks idle.
do
	local s = policy.newEdit("run", 0)
	local action = runUntil(s, 0, policy.START_TIMEOUT - 1, true, true)
	check(action == "wait", `start pending: {action}, want wait`)
end

-- EditModeActive unreadable: only our own pending start counts as busy.
do
	local s = policy.newEdit("test", 0)
	local action, _, at = runUntil(s, 0, 5, nil, false)
	check(action == "start" and at == policy.IDLE_SETTLE, `unreadable, idle: {action} at {at}`)
	local pending = runUntil(policy.newEdit("test", 0), 0, 30, nil, true)
	check(pending == "wait", `unreadable, pending: {pending}`)
end

-- The engine refusing an idle start is reported with its own message after
-- MAX_START_ERRORS attempts, about a second apart — not retried forever (#27).
do
	local s = policy.newEdit("test", 0)
	local t = 0
	local starts = 0
	local action: policy.Action, reason: string? = "wait", nil
	while t < 60 do
		action, reason = policy.stepEdit(s, { now = t, editModeActive = true, startPending = false })
		if action == "start" then
			starts += 1
			policy.noteStartError(s, IN_PROGRESS)
		elseif action == "fail" then
			break
		end
		t += 0.25
	end
	check(action == "fail", `refused starts: {action}, want fail`)
	check(starts == policy.MAX_START_ERRORS, `refused starts: {starts} attempts, want {policy.MAX_START_ERRORS}`)
	check(t >= policy.MAX_START_ERRORS * policy.IDLE_SETTLE, `refused starts: gave up at {t}s, too fast`)
	local failedTimes = `ExecutePlayModeAsync failed {policy.MAX_START_ERRORS} times`
	check(
		reason ~= nil and reason:find(failedTimes, 1, true) ~= nil and reason:find(IN_PROGRESS, 1, true) ~= nil,
		`refused starts reason: {reason}`
	)
end

-- #27 wedge: the engine reports a session nothing rodeo runs will end. After
-- BUSY_TIMEOUT one start is tried anyway; its refusal is the reported reason.
do
	local s = policy.newEdit("test", 0)
	local action, _, at = runUntil(s, 0, 60, false, false)
	check(
		action == "start" and at == policy.BUSY_TIMEOUT,
		`wedge: {action} at {at}, want start at {policy.BUSY_TIMEOUT}`
	)
	policy.noteStartError(s, IN_PROGRESS)
	local final, reason = policy.stepEdit(s, { now = at + 0.25, editModeActive = false, startPending = false })
	check(final == "fail", `wedge after refused start: {final}`)
	check(
		reason ~= nil
			and reason:find("EditModeActive stayed false", 1, true) ~= nil
			and reason:find(IN_PROGRESS, 1, true) ~= nil,
		`wedge reason: {reason}`
	)
end

-- The forced start goes through (EditModeActive was wrong): keep waiting on
-- it, and it is tried only once.
do
	local s = policy.newEdit("test", 0)
	local action, _, at = runUntil(s, 0, 60, false, false)
	check(action == "start", `forced start: {action}`)
	local pending = runUntil(s, at + 0.25, policy.START_TIMEOUT - 1, false, true)
	check(pending == "wait", `forced start pending: {pending}`)
	local again = runUntil(s, policy.START_TIMEOUT - 0.75, policy.START_TIMEOUT - 0.25, false, false)
	check(again == "wait", `forced start is not repeated: {again}`)
end

-- An error from before the forced start doesn't decide it: only the forced
-- attempt's own outcome does.
do
	local s = policy.newEdit("test", 0)
	local first = runUntil(s, 0, 5, true, false)
	check(first == "start", `first start: {first}`)
	policy.noteStartError(s, "transient")
	local forced, _, at = runUntil(s, 1.25, 60, false, false)
	check(forced == "start", `forced after an earlier error: {forced}`)
	local next = policy.stepEdit(s, { now = at + 0.25, editModeActive = false, startPending = true })
	check(next == "wait", `old error must not fail the forced attempt: {next}`)
end

-- A start that never brings the session up is bounded by START_TIMEOUT.
do
	local s = policy.newEdit("play", 0)
	local action, reason, at = runUntil(s, 0, policy.START_TIMEOUT + 5, false, true)
	check(action == "fail" and at == policy.START_TIMEOUT, `stuck start: {action} at {at}`)
	check(
		reason ~= nil and reason:find("ExecuteMultiplayerTestAsync was called but no play DataModel", 1, true) ~= nil,
		`stuck start reason: {reason}`
	)
end

-- Target edit: nothing to start; a session that never ends is reported.
do
	local s = policy.newEdit("edit", 0)
	local action, reason, at = runUntil(s, 0, 60, false, false)
	check(action == "fail" and at == policy.BUSY_TIMEOUT, `edit, busy: {action} at {at}`)
	check(reason ~= nil and reason:find("did not end", 1, true) ~= nil, `edit, busy reason: {reason}`)

	local idle = policy.newEdit("edit", 0)
	local idleAction, idleReason, idleAt = runUntil(idle, 0, policy.START_TIMEOUT + 5, true, false)
	check(idleAction == "fail" and idleAt == policy.START_TIMEOUT, `edit, idle: {idleAction} at {idleAt}`)
	check(
		idleReason ~= nil and idleReason:find("did not return to edit", 1, true) ~= nil,
		`edit, idle reason: {idleReason}`
	)
end

if failures > 0 then
	error(`{failures} assertion(s) failed`)
end
print("ok: mode_policy")
