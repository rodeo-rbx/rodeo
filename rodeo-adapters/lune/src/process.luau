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

-- The command line for lune's program, params and shell option: true picks
-- the platform's shell, and the program and params are joined with spaces
-- into its -c command.
local function commandLine(program: string, params: { string }?, shell: any): { string }
	local args = table.clone(params or {})
	if shell == true then
		shell = if isWindows then "powershell" else "/bin/sh"
	elseif shell ~= nil and type(shell) ~= "string" then
		error(`Invalid type for option 'shell' - expected 'true' or 'string', got '{typeof(shell)}'`, 3)
	end
	return if shell
		then { shell, "-c", table.concat({ program, table.unpack(args) }, " ") }
		else { program, table.unpack(args) }
end

-- lune replaces a leading ~ path component of cwd with the home directory.
local function expandHome(cwd: any): any
	if type(cwd) == "string" and (cwd == "~" or string.match(cwd, "^~[/\\]")) then
		return g.process.homedir() .. string.sub(cwd, 2)
	end
	return cwd
end

-- lune's process.exec over @rodeo/process run: same arguments, options and
-- { ok, code, stdout, stderr } result. rodeo captures the child's output
-- either way, so "inherit" and "forward" write it to this run's output when
-- the child exits rather than as it is produced.
local function exec(program: string, params: { string }?, options: { [string]: any }?)
	options = options or {}
	local argv = commandLine(program, params, options.shell)
	local cwd = expandHome(options.cwd)

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

-- lune's process.create over @rodeo/process create, with every stream piped:
-- a ChildProcess whose stdin writer, stdout and stderr readers, kill and
-- status follow lune. lune spawns without yielding, but rodeo's spawn is a
-- call to the host, so it runs on its own thread and each method waits for
-- it. A spawn that fails (no such program) is reported on stderr and raised
-- by the first method called, rather than by create.
local function create(program: string, params: { string }?, options: { [string]: any }?)
	options = options or {}
	local argv = commandLine(program, params, options.shell)
	local spawnOptions = { cwd = expandHome(options.cwd), env = options.env, stdio = "piped" }

	local handle, failure
	local waiting: { thread } = {}
	task.spawn(function()
		local ok, result = pcall(g.process.create, argv, spawnOptions)
		if ok then
			handle = result
		else
			failure = result
			g.stream.write(g.io.stderr, `@lune/process.create failed: {result}\n`)
		end
		for _, thread in waiting do
			task.spawn(thread)
		end
	end)
	local function spawned()
		if handle == nil and failure == nil then
			table.insert(waiting, coroutine.running())
			coroutine.yield()
		end
		if failure ~= nil then
			error(failure, 3)
		end
		return handle
	end

	local function reader(stream: string)
		return {
			-- Whatever output is available, up to chunkSize bytes (lune's
			-- default is 1KB); nil once the stream has ended.
			read = function(_, chunkSize: number?): string?
				return g.stream.read(spawned()[stream], chunkSize or 1024)
			end,
			readToEnd = function(_): string
				local pipe = spawned()[stream]
				local chunks = {}
				while true do
					local chunk = g.stream.read(pipe, 65536)
					if chunk == nil then
						return table.concat(chunks)
					end
					table.insert(chunks, chunk)
				end
			end,
		}
	end

	return {
		stdin = {
			write = function(_, data: buffer | string)
				local pipe = spawned().stdin
				if type(data) == "buffer" then
					g.stream.writeBytes(pipe, data)
				else
					g.stream.write(pipe, data)
				end
			end,
			close = function(_)
				g.stream.close(spawned().stdin)
			end,
		},
		stdout = reader("stdout"),
		stderr = reader("stderr"),
		kill = function(_)
			g.process.kill(spawned())
		end,
		status = function(_)
			local result = g.process.run(spawned())
			-- lune reports 9 for a child with no exit code (killed by a
			-- signal); rodeo reports -1.
			local code = if result.exitcode == -1 then 9 else result.exitcode
			return table.freeze({ ok = code == 0, code = code })
		end,
	}
end

-- Members the shim does not provide raise an error naming them, instead of
-- indexing to nil and failing later as "attempt to call a nil value".
return setmetatable({
	-- lune's args are read-only (writes must error mentioning read-only);
	-- freeze a clone so the shared @rodeo args table stays untouched.
	args = table.freeze(table.clone(g.process.args)),
	env = g.process.env,
	cwd = cwdStr,
	exit = g.process.exit,
	exec = exec,
	create = create,
	os = platform,
}, {
	__index = function(_, key)
		error(`@lune/process.{key} is not supported by rodeo's Lune adapter`, 2)
	end,
})
