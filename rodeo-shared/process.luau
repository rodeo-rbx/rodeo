-- @rodeo/process API over typed RPC.
-- Factory: (rpc, scriptArgs, hostInfo, runtime) -> process. `runtime` is the
-- caller's proto module, passed in rather than read from _G: several rodeo
-- plugins can share one VM. (Type annotations below name it for readers only.)

-- Apply proto3 defaults: absent fields = zero values
local function applyRunDefaults(resp: runtime.ProcessRunResponse): runtime.ProcessRunResponse
	return {
		ok = if resp.ok ~= nil then resp.ok else false,
		exitcode = resp.exitcode or 0,
		out = resp.out or "",
		err = resp.err or "",
		signal = resp.signal,
	}
end

-- `input` may be given as a string; on the wire it is bytes.
local function wireOptions(options: any?): any?
	if options and type(options.input) == "string" then
		options = table.clone(options)
		options.input = buffer.fromstring(options.input)
	end
	return options
end

return function(rpc, scriptArgs, hostInfo, runtime)
	local process = {}

	-- Properties
	process.args = scriptArgs

	-- Read-only env proxy
	process.env = setmetatable({}, {
		__index = function(_, key)
			return hostInfo.env[key]
		end,
		__newindex = function()
			error("process.env is read-only in Rodeo (runs remotely)")
		end,
	})

	function process.cwd(): string
		return hostInfo.cwd
	end

	function process.homedir(): string
		return hostInfo.homedir
	end

	function process.execpath(): string
		return hostInfo.execpath
	end

	function process.platform(): string
		return hostInfo.platform or ""
	end

	function process.exit(code: number)
		rpc.notify("process_exit", runtime.ProcessExitRequest.new({ code = code }))
		coroutine.yield()
	end

	function process.run(argsOrHandle, options: any?)
		local resp: runtime.ProcessRunResponse
		if type(argsOrHandle) == "table" and argsOrHandle.__rodeo_pid then
			resp = rpc.call("process_run_handle", runtime.ProcessRunHandleRequest.new({
				pid = argsOrHandle.__rodeo_pid,
			}))
		else
			resp = rpc.call("process_run", runtime.ProcessRunRequest.new({
				args = argsOrHandle,
				options = wireOptions(options),
			}))
		end
		return applyRunDefaults(resp)
	end

	function process.system(command: string, options: any?)
		local resp: runtime.ProcessRunResponse = rpc.call("process_system", runtime.ProcessSystemRequest.new({
			command = command,
			options = wireOptions(options),
		}))
		return applyRunDefaults(resp)
	end

	function process.create(args: { string }, options: any?)
		local result: runtime.ProcessCreateResponse = rpc.call("process_create", runtime.ProcessCreateRequest.new({
			args = args,
			options = wireOptions(options),
		}))
		local handle: any = { __rodeo_pid = result.pid }
		if result.stdinHandle then
			handle.stdin = { __handle = result.stdinHandle }
		end
		if result.stdoutHandle then
			handle.stdout = { __handle = result.stdoutHandle }
		end
		if result.stderrHandle then
			handle.stderr = { __handle = result.stderrHandle }
		end
		return handle
	end

	function process.kill(handle)
		rpc.call("process_kill", runtime.ProcessKillRequest.new({ pid = handle.__rodeo_pid }))
	end

	return process
end
