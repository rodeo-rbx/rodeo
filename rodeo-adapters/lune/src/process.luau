-- @lune/process shim — maps to rodeo's _G.__rodeo_procs APIs
local g = _G.__rodeo_procs[_G.__rodeo_eid]
local platform = g.process.platform()
-- lune's os values are "windows" | "macos" | "linux" — same strings the
-- host reports.
local isWindows = platform == "windows"
-- lune guarantees cwd ends with the platform separator.
local separator = if isWindows then "\\" else "/"
local cwdStr = g.process.cwd()
if cwdStr:sub(-1) ~= separator then
	cwdStr ..= separator
end

local STDIO_KINDS = { default = true, inherit = true, forward = true, none = true }

local function stdioKind(kind: any, option: string): string
	if kind == nil then
		return "default"
	end
	if not STDIO_KINDS[kind] then
		error(`Invalid value for option '{option}' - expected default, inherit, forward or none, got '{kind}'`, 3)
	end
	return kind
end

-- What a child's stream leaves in the result, after sending it to this run's
-- matching output for "inherit" and "forward".
local function route(kind: string, output: string, parent): string
	if output ~= "" and (kind == "inherit" or kind == "forward") then
		g.stream.write(parent, output)
	end
	return if kind == "default" or kind == "inherit" then output else ""
end

-- lune's process.exec over @rodeo/process run: same arguments, options and
-- { ok, code, stdout, stderr } result. rodeo captures the child's output
-- either way, so "inherit" and "forward" write it to this run's output when
-- the child exits rather than as it is produced.
local function exec(program: string, params: { string }?, options: { [string]: any }?)
	options = options or {}
	local args = table.clone(params or {})

	-- lune's shell option: true picks the platform's shell, and the program
	-- and params are joined with spaces into its -c command.
	local shell = options.shell
	if shell == true then
		shell = if isWindows then "powershell" else "/bin/sh"
	elseif shell ~= nil and type(shell) ~= "string" then
		error(`Invalid type for option 'shell' - expected 'true' or 'string', got '{typeof(shell)}'`, 2)
	end
	local argv = if shell
		then { shell, "-c", table.concat({ program, table.unpack(args) }, " ") }
		else { program, table.unpack(args) }

	-- lune replaces a leading ~ path component with the home directory.
	local cwd = options.cwd
	if type(cwd) == "string" and (cwd == "~" or string.match(cwd, "^~[/\\]")) then
		cwd = g.process.homedir() .. string.sub(cwd, 2)
	end

	local stdio = options.stdio
	local stdout, stderr, input = "default", "default", nil
	if type(stdio) == "table" then
		stdout = stdioKind(stdio.stdout, "stdout")
		stderr = stdioKind(stdio.stderr, "stderr")
		input = stdio.stdin
	elseif stdio ~= nil then
		stdout = stdioKind(stdio, "stdio")
		stderr = stdout
	end

	local result = g.process.run(argv, { cwd = cwd, env = options.env, input = input })
	-- Without an exit code (killed by a signal) lune reports 1 if the child
	-- wrote to stderr, else 0; rodeo reports -1.
	local code = result.exitcode
	if code == -1 then
		code = if result.err ~= "" then 1 else 0
	end
	return table.freeze({
		ok = code == 0,
		code = code,
		stdout = route(stdout, result.out, g.io.stdout),
		stderr = route(stderr, result.err, g.io.stderr),
	})
end

return {
	-- lune's args are read-only (writes must error mentioning read-only);
	-- freeze a clone so the shared @rodeo args table stays untouched.
	args = table.freeze(table.clone(g.process.args)),
	env = g.process.env,
	cwd = cwdStr,
	exit = g.process.exit,
	exec = exec,
	os = platform,
}
