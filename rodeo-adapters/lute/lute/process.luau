-- @lute/process shim — maps to rodeo's process API. Lute's run/system take
-- the same arguments as rodeo's; the options and result are translated.
-- pid/onSignal have no host support and error clearly.
local g = _G.__rodeo_procs[_G.__rodeo_eid]
local isWindows = g.process.platform() == "windows"

local STDIO_KINDS = { default = true, inherit = true, none = true }

local function stdioKind(options): string
	local kind = options and options.stdio
	if kind == nil then
		return "default"
	end
	if not STDIO_KINDS[kind] then
		error(`Invalid stdio kind: {kind}`, 3)
	end
	return kind
end

-- Lute's result for rodeo's. rodeo captures the child's output either way:
-- "inherit" sends it to this run's output once the child exits, and both
-- "inherit" and "none" leave the result's stdout/stderr empty, as in Lute.
-- A child ended by a signal has exitcode 0 and the signal number as a
-- string, as in Lute (rodeo reports exitcode -1).
local function mapResult(result, stdio: string)
	if stdio == "inherit" then
		if result.out ~= "" then
			g.stream.write(g.io.stdout, result.out)
		end
		if result.err ~= "" then
			g.stream.write(g.io.stderr, result.err)
		end
	end
	local captured = stdio == "default"
	return {
		stdout = if captured then result.out else "",
		stderr = if captured then result.err else "",
		ok = result.ok,
		exitcode = if result.signal then 0 else result.exitcode,
		signal = if result.signal then tostring(result.signal) else nil,
	}
end

local process = {}

process.args = g.process.args
process.env = g.process.env

function process.homedir(): string
	return g.process.homedir()
end

function process.cwd(): string
	return g.process.cwd()
end

function process.run(args: { string }, options)
	local stdio = stdioKind(options)
	return mapResult(g.process.run(args, options and { cwd = options.cwd, env = options.env }), stdio)
end

-- Lute's default shell is $SHELL (%COMSPEC% on Windows), else /bin/sh (cmd.exe).
function process.system(command: string, options)
	local stdio = stdioKind(options)
	local shell = options and options.system
		or g.process.env[if isWindows then "COMSPEC" else "SHELL"]
		or (if isWindows then "cmd.exe" else "/bin/sh")
	local runOptions = { cwd = options and options.cwd, env = options and options.env, shell = shell }
	return mapResult(g.process.system(command, runOptions), stdio)
end

function process.exit(exitcode: number)
	g.process.exit(exitcode)
end

function process.execPath(): string
	return g.process.execpath()
end

function process.pid(): number
	error("process.pid is not supported under rodeo (the script runs remotely; there is no meaningful local pid)", 2)
end

function process.onSignal(signal, callback)
	error("process.onSignal is not supported under rodeo (no host RPC for signal handlers)", 2)
end

return process
