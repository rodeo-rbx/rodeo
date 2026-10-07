-- @lute/process conformance, per submodules/lute/definitions/process.luau.
local process = require("@lute/process")

local function check(cond: boolean, msg: string)
	if not cond then
		error(msg, 2)
	end
end

check(type(process.args) == "table", "args should be a table")
check(type(process.env) == "table", "env should be a table")
check(#process.cwd() > 0, "cwd empty")
check(#process.homedir() > 0, "homedir empty")
check(#process.execPath() > 0, "execPath empty")

-- run: ProcessResult shape (stdout/stderr/ok/exitcode)
local echoed = process.run({ "echo", "lute-run" })
check(echoed.ok == true, "run.ok wrong")
check(echoed.exitcode == 0, "run.exitcode wrong")
check(echoed.stdout == "lute-run\n", `run.stdout wrong: '{echoed.stdout}'`)
check(echoed.stderr == "", "run.stderr should be empty")

-- failing command reports ok = false and the code
local failed = process.run({ "sh", "-c", "exit 3" })
check(failed.ok == false, "failed run.ok wrong")
check(failed.exitcode == 3, `failed run.exitcode wrong: {failed.exitcode}`)

-- system: through the shell. Lute's default is $SHELL (%COMSPEC% on
-- Windows), and cmd's echo terminates with CRLF — a shell artifact, not an
-- adapter difference, so compare line-ending-agnostic.
local sys = process.system("echo lute-system")
local sysOut = (sys.stdout:gsub("\r\n", "\n"))
check(sys.ok and sysOut == "lute-system\n", `system output wrong: '{sys.stdout}'`)

-- system's `system` option picks the shell
local bash = process.system("echo $0", { system = "/bin/bash" })
check(bash.stdout == "/bin/bash\n", `system shell option ignored: '{bash.stdout}'`)

-- env adds to the inherited environment
local withEnv = process.run({ "sh", "-c", 'printf %s "$RODEO_LUTE_VAR"' }, { env = { RODEO_LUTE_VAR = "from env" } })
check(withEnv.stdout == "from env", `run env wrong: '{withEnv.stdout}'`)
local systemEnv = process.system('printf %s "$RODEO_LUTE_VAR"', { env = { RODEO_LUTE_VAR = "system env" } })
check(systemEnv.stdout == "system env", `system env wrong: '{systemEnv.stdout}'`)

-- stdio: "inherit" prints the output instead of returning it (the harness
-- checks it reached stdout), "none" discards it
local inherited = process.run({ "echo", "lute-inherit" }, { stdio = "inherit" })
check(inherited.ok and inherited.stdout == "", `inherit should leave stdout empty: '{inherited.stdout}'`)
local discarded = process.run({ "echo", "lute-none" }, { stdio = "none" })
check(discarded.ok and discarded.stdout == "", `none should leave stdout empty: '{discarded.stdout}'`)
check(not pcall(process.run, { "echo" }, { stdio = "tee" }), "an unknown stdio kind should error")

-- a child ended by a signal: Lute reports exitcode 0 and the signal number as a string
local killed = process.run({ "sh", "-c", "kill -9 $$" })
check(killed.ok == false, "killed run.ok wrong")
check(killed.exitcode == 0, `killed run.exitcode wrong: {killed.exitcode}`)
check(killed.signal == "9", `killed run.signal wrong: {killed.signal}`)
check(echoed.signal == nil, "signal should be nil for a normal exit")

-- unsupported surface errors clearly
check(not pcall(process.pid), "pid should error under rodeo")
check(not pcall(process.onSignal, "SIGINT", function() end), "onSignal should error under rodeo")

return true
