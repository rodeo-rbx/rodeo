-- Process / command execution available inside Studio scripts.
-- Bundled require: `local process = require("@rodeo/process")`.
--
-- NOTE: typedef-only. Runtime lives in rodeo-pkg/rodeo/shims/process.luau.

local singleton = require("./_internal/singleton")
local stream = require("./stream")

--- How a stdio stream should be wired for a spawned process. `"default"`
--- uses the parent's stream, `"piped"` captures into a `StreamHandle`,
--- `"inherit"` passes through, `"none"` discards, `"tee"` mirrors to both
--- a capture and the parent's stream.
export type StdioKind = "default" | "piped" | "inherit" | "none" | "tee"

--- Options accepted by `run`, `system`, and `create`. All fields optional.
--- `stdio` applies to all three streams; per-stream overrides win.
export type ProcessRunOptions = {
	cwd: string?,
	stdio: StdioKind?,
	stdin: StdioKind?,
	stdout: StdioKind?,
	stderr: StdioKind?,
	env: { [string]: string }?,
}

--- Result returned by `run` and `system`.
--- `out` and `err` are the captured stdout and stderr, empty when `run` waits on a `create` handle.
export type ProcessResult = {
	ok: boolean, -- true when the process exited with status 0
	exitcode: number, -- -1 when the process was terminated by a signal
	out: string,
	err: string,
}

--- Handle to a process started with `create`. Fields are populated when
--- the corresponding stdio is `"piped"`; otherwise `nil`.
export type ProcessHandle = {
	__rodeo_pid: string,
	stdin: stream.StreamHandle?,
	stdout: stream.StreamHandle?,
	stderr: stream.StreamHandle?,
}

local process = {}

--- The command-line arguments passed to this rodeo execution after `--`.
process.args = (nil :: any) :: { string }

--- A read-only view of environment variables visible to this execution.
process.env = (nil :: any) :: { [string]: string? }

--- Returns the current working directory.
function process.cwd(): string
	return (singleton :: any).process.cwd()
end

--- Returns the user's home directory.
function process.homedir(): string
	return (singleton :: any).process.homedir()
end

--- Returns the path of the running rodeo binary.
function process.execpath(): string
	return (singleton :: any).process.execpath()
end

--- Returns the run client's OS: `"macos"`, `"windows"`, or `"linux"`.
function process.platform(): string
	return (singleton :: any).process.platform()
end

--- Exits the current execution with the given code.
function process.exit(code: number)
	(singleton :: any).process.exit(code)
end

--- Runs `args` (or an existing handle), blocking until completion. Returns
--- captured stdout/stderr and exit code.
function process.run(argsOrHandle: { string } | ProcessHandle, options: ProcessRunOptions?): ProcessResult
	return (singleton :: any).process.run(argsOrHandle, options)
end

--- Like `run`, but takes a single shell-style command string.
function process.system(command: string, options: ProcessRunOptions?): ProcessResult
	return (singleton :: any).process.system(command, options)
end

--- Spawns `args` as a child process without waiting. Returns a handle with
--- stdin/stdout/stderr stream handles for live interaction.
function process.create(args: { string }, options: ProcessRunOptions?): ProcessHandle
	return (singleton :: any).process.create(args, options)
end

--- Terminates a process started with `create`.
function process.kill(handle: ProcessHandle)
	(singleton :: any).process.kill(handle)
end

return process
