--!strict
-- What the plugin makes of the server build named in the WelcomeMsg. Pure:
-- no Roblox globals, so it runs under lune (tests/plugin/server_version.spec.luau)
-- as well as in Studio.
--
-- Since rodeo 1.4.0 the backend compares the plugin's build with its own and
-- never routes runs to a plugin of another build (1.4 closes its socket,
-- 1.5 parks it), unless RODEO_SKIP_VERSION_CHECK told it to accept the
-- mismatch. Older backends don't check: they register whatever plugin
-- connects and send it runs, although the two builds' run and RPC messages
-- may not match. A newer plugin in a Studio launched by such a backend (both
-- on the default port) took those runs and the run hung with no output
-- (issue #31). Those servers can't change, so the plugin refuses their runs.

-- The first release whose backend checks the plugin's build.
local HANDSHAKE_MAJOR, HANDSHAKE_MINOR = 1, 4

-- Whether the server named by `version` (WelcomeMsg.version) predates the
-- build check. Every backend that checks reports its own build id
-- (`1.4.0-rc.1+sha` or later); older ones report their crate version
-- (`1.3.0`). A version that doesn't parse is not judged.
local function predatesHandshake(version: string): boolean
	if version == "" then
		return true
	end
	local major, minor = string.match(version, "^(%d+)%.(%d+)%.%d+")
	if major == nil or minor == nil then
		return false
	end
	local ma, mi = tonumber(major) :: number, tonumber(minor) :: number
	return ma < HANDSHAKE_MAJOR or (ma == HANDSHAKE_MAJOR and mi < HANDSHAKE_MINOR)
end

export type Plugin = {
	-- Build id, e.g. `1.5.0-rc.11+d24dcfa`.
	build: string,
	-- Release version, e.g. `1.5.0-rc.11`.
	version: string,
	-- This plugin's file in Studio's plugins folder.
	file: string,
}

-- The error a refused run reports to its CLI. Names both builds, why the
-- plugin won't run it, and the two ways out.
local function refusal(plugin: Plugin, serverVersion: string): string
	local server = if serverVersion == "" then "an older rodeo" else `rodeo {serverVersion}`
	return `this Studio's rodeo plugin ({plugin.build}) refused the run: the server is {server}, `
		.. "which predates rodeo's plugin version check, so the two may not speak the same protocol. "
		.. `Use one rodeo version for both: upgrade this project's rodeo to {plugin.version} or newer, `
		.. `or delete {plugin.file} from Studio's plugins folder and launch the Studio again.`
end

return {
	predatesHandshake = predatesHandshake,
	refusal = refusal,
}
