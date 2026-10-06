--!strict
-- Unit spec for rodeo-plugin/src/library/server_version.luau, run under lune
-- by tests/plugin/serverVersion.test.ts. Issue #31: a server older than the
-- plugin version check routed runs to a newer plugin and they hung; the
-- plugin must refuse exactly those servers, and only those.
local server_version = require("../../rodeo-plugin/src/library/server_version")

local failures = 0
local function check(name: string, have: any, want: any)
	if have ~= want then
		failures += 1
		print(`FAIL {name}: got {tostring(have)}, want {tostring(want)}`)
	end
end

local cases: { { any } } = {
	-- Pre-handshake servers report their bare crate version.
	{ "1.3.0", true },
	{ "1.2.0-rc.2", true },
	{ "1.0.0", true },
	{ "0.2.0", true },
	-- A welcome without a version comes from no server that checks.
	{ "", true },
	-- Every server since 1.4.0-rc.1 checks (closes, parks, or accepts on
	-- purpose under RODEO_SKIP_VERSION_CHECK) and reports its build id.
	{ "1.4.0-rc.1+e776f77", false },
	{ "1.4.0-rc.4+b493edb", false },
	{ "1.5.0-rc.11+d24dcfa", false },
	{ "1.5.0", false },
	{ "1.10.0+abcdef0", false },
	{ "2.0.0", false },
	-- Not a version: not judged.
	{ "dev", false },
}
for _, case in cases do
	check(`predatesHandshake({case[1]})`, server_version.predatesHandshake(case[1]), case[2])
end

local plugin = { build = "1.5.0+abcdef0", version = "1.5.0", file = "rodeo-1.5.0+abcdef0-44873.rbxm" }
local message = server_version.refusal(plugin, "1.3.0")
for _, part in
	{
		"1.5.0+abcdef0",
		"rodeo 1.3.0",
		"version check",
		"upgrade this project's rodeo to 1.5.0",
		"delete rodeo-1.5.0+abcdef0-44873.rbxm",
	}
do
	check(`refusal mentions {part}`, string.find(message, part, 1, true) ~= nil, true)
end
check(
	"refusal for an unversioned server",
	string.find(server_version.refusal(plugin, ""), "an older rodeo", 1, true) ~= nil,
	true
)

if failures > 0 then
	error(`{failures} assertion(s) failed`)
end
print(`ok: {#cases} versions`)
