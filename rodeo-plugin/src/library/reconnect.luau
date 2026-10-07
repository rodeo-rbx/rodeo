--!strict
-- When and how the plugin dials its backend again. Pure: no Roblox globals,
-- so it runs under lune (tests/plugin/reconnect.spec.luau) as well as in
-- Studio.
--
-- A Studio can outlive its backend (a `--detach` launch whose serve exited, a
-- crashed serve), and the plugin then keeps dialing a port nobody listens on
-- until a serve comes back there. The plugin runs in every DOM of every
-- Studio that loaded it, so the gap between attempts doubles while they keep
-- failing, up to a ceiling that still picks a returning serve up within
-- seconds.

-- First retry after a failure, and the step the back-off doubles from.
local BASE_DELAY = 1
-- Ceiling for the doubling.
local MAX_DELAY = 5
-- A backend of another build closes (pre-1.5) or parks (1.5+) this plugin's
-- socket. It will never take this plugin, so it is redialed rarely.
local MISMATCH_DELAY = 30
-- How long a WebSocket gets to open. On localhost a refused or dropped
-- connection reports Error or Closed within milliseconds; this bounds an
-- attempt that reports neither, which would otherwise leave the plugin
-- waiting on that socket forever.
local OPEN_TIMEOUT = 10
-- How long the /health probe gets. It answers in milliseconds when the
-- backend is up and fails as fast when nothing listens; a probe still
-- pending (plugins share a small pool of concurrent HTTP requests) says
-- nothing about the backend, so the attempt dials the socket instead.
local PROBE_TIMEOUT = 3

-- Seconds to wait before the next dial after `failures` consecutive failed
-- attempts (1 for the first). `mismatch`: the last backend reached was of
-- another build.
local function retryDelay(failures: number, mismatch: boolean): number
	if mismatch then
		return MISMATCH_DELAY
	end
	return math.min(BASE_DELAY * 2 ^ math.max(failures - 1, 0), MAX_DELAY)
end

-- Before opening a WebSocket, edit and play-server DOMs send GET /health to
-- the backend: when nothing listens, a failed HTTP request is cheaper than a
-- WebSocket client (Roblox allows four between all plugins in a Studio).
-- Play clients cannot make HTTP requests at all and always dial the socket.
-- Whether the engine allows the request in every other DOM (Team Create
-- edit, run-mode server) has not been checked, so a DOM trusts its probe only
-- after it has answered once, and stops probing if a socket opens right
-- after a refused probe.
export type ProbeTrust = "unknown" | "trusted" | "unusable"
export type ProbeResult = "answered" | "refused" | "timeout"

-- What a probe's result means for this attempt: the trust to keep, and
-- whether to open the WebSocket.
local function afterProbe(trust: ProbeTrust, result: ProbeResult): (ProbeTrust, boolean)
	if result == "answered" then
		return "trusted", true
	end
	if result == "refused" and trust == "trusted" then
		-- The probe has answered in this DOM before, so it is allowed here:
		-- the error means nothing listens on the port.
		return trust, false
	end
	-- Timed out, or refused before the probe ever answered here (it may not
	-- be allowed in this DOM): dial and let the socket decide.
	return trust, true
end

-- The trust to keep once this attempt's socket opened. A backend that takes
-- a socket right after the probe was refused is one the probe cannot reach.
local function afterOpen(trust: ProbeTrust, result: ProbeResult?): ProbeTrust
	if result == "refused" then
		return "unusable"
	end
	return trust
end

return {
	OPEN_TIMEOUT = OPEN_TIMEOUT,
	PROBE_TIMEOUT = PROBE_TIMEOUT,
	retryDelay = retryDelay,
	afterProbe = afterProbe,
	afterOpen = afterOpen,
}
