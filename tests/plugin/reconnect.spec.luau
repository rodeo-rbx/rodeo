--!strict
-- Unit spec for rodeo-plugin/src/library/reconnect.luau, run under lune by
-- tests/plugin/reconnect.test.ts. Issue #29: a Studio that outlived its
-- serve must not redial the dead port in a tight loop.
local reconnect = require("../../rodeo-plugin/src/library/reconnect")

local failures = 0
local function check(name: string, have: any, want: any)
	if have ~= want then
		failures += 1
		print(`FAIL {name}: got {tostring(have)}, want {tostring(want)}`)
	end
end

-- Back-off: doubles from 1 s per consecutive failure, capped at 5 s.
local schedule = { 1, 2, 4, 5, 5, 5 }
for n, want in schedule do
	check(`retryDelay({n})`, reconnect.retryDelay(n, false), want)
end
check("retryDelay(1000)", reconnect.retryDelay(1000, false), 5)
-- The attempt count never reads below one failure.
check("retryDelay(0)", reconnect.retryDelay(0, false), 1)
-- No sequence of failures retries faster than once per second.
for n = 0, 50 do
	if reconnect.retryDelay(n, false) < 1 then
		check(`retryDelay({n}) >= 1`, reconnect.retryDelay(n, false), 1)
	end
end
-- A backend of another build is redialed rarely, whatever the count.
check("retryDelay(1, mismatch)", reconnect.retryDelay(1, true), 30)
check("retryDelay(9, mismatch)", reconnect.retryDelay(9, true), 30)

-- Probe outcomes: { trust, result, want trust, want dial }.
local probeCases: { { any } } = {
	-- An answer always dials and makes the probe trusted.
	{ "unknown", "answered", "trusted", true },
	{ "trusted", "answered", "trusted", true },
	-- A refusal from a probe known to work here: nothing listens, skip the socket.
	{ "trusted", "refused", "trusted", false },
	-- A refusal before the probe ever answered may mean HTTP isn't allowed in
	-- this DOM: dial anyway.
	{ "unknown", "refused", "unknown", true },
	-- A probe stuck behind other requests says nothing about the backend.
	{ "unknown", "timeout", "unknown", true },
	{ "trusted", "timeout", "trusted", true },
}
for _, case in probeCases do
	local trust, dial = reconnect.afterProbe(case[1], case[2])
	check(`afterProbe({case[1]}, {case[2]}) trust`, trust, case[3])
	check(`afterProbe({case[1]}, {case[2]}) dial`, dial, case[4])
end

-- A socket that opens right after a refused probe: the probe can't see this
-- backend, so this DOM stops probing.
check("afterOpen(unknown, refused)", reconnect.afterOpen("unknown", "refused"), "unusable")
check("afterOpen(trusted, answered)", reconnect.afterOpen("trusted", "answered"), "trusted")
check("afterOpen(unknown, timeout)", reconnect.afterOpen("unknown", "timeout"), "unknown")
-- No probe ran (a play client, or probing already given up).
check("afterOpen(unusable, nil)", reconnect.afterOpen("unusable", nil), "unusable")

-- Bounds an attempt that never reports; must outlast a localhost handshake.
check("OPEN_TIMEOUT > 0", reconnect.OPEN_TIMEOUT > 0, true)
check("PROBE_TIMEOUT < OPEN_TIMEOUT", reconnect.PROBE_TIMEOUT < reconnect.OPEN_TIMEOUT, true)

if failures > 0 then
	error(`{failures} assertion(s) failed`)
end
print("ok: reconnect policy")
