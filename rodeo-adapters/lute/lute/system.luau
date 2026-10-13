-- @lute/system shim — host facts from rodeo's process runtime. `os` and
-- `arch` use uname's names, as Lute does (sysname: Darwin/Linux/Windows_NT;
-- machine: arm64 on macOS and Windows, aarch64 on Linux, x86_64), since
-- @std/system/platform compares them that way. hostName, the memory figures,
-- uptime and cpus have no host support and error clearly.
local g = _G.__rodeo_procs[_G.__rodeo_eid]
local host = g.process.__host

local SYSNAME = { macos = "Darwin", linux = "Linux", windows = "Windows_NT" }

local system = {}

system.os = SYSNAME[host.platform] or host.platform
system.arch = if host.arch == "aarch64" and host.platform ~= "linux" then "arm64" else host.arch

function system.tmpdir(): string
	-- Lute (libuv) drops the trailing separator the host's temp dir path has.
	local dir = string.gsub(host.tmpdir, "[/\\]+$", "")
	return if dir == "" then host.tmpdir else dir
end

function system.threadCount(): number
	return host.threadCount
end

for _, name in { "hostName", "totalMemory", "freeMemory", "uptime", "cpus" } do
	system[name] = function()
		error(`@lute/system.{name} is not supported by rodeo's Lute adapter`, 2)
	end
end

return system
