--[[
	MessageHandler.luau - WebSocket message handling for the Rodeo plugin

	Handles incoming messages from the CLI server: exec, kill, rpc:response, welcome.
	Tracks active processes and manages execution lifecycle.
]]

local HttpService = game:GetService("HttpService")

local runtime = require(script.Parent.Parent.Parent.library.runtime)
local utils = require(script.Parent.Parent.Parent.library.utils)
local constants = require(script.Parent.Parent.Parent.library.constants)
local mode_driver = require(script.Parent.Parent.Parent.library.mode_driver)
local simulator = require(script.Parent.Parent.Parent.library.simulator)
local rodeoProto = require(script.Parent.Parent.Parent.Parent.proto.rodeo)
local runtimeProto = require(script.Parent.Parent.Parent.Parent.proto.runtime)

export type ProcessEntry = {
	name: string,
	state: "running" | "done" | "error" | "killed",
}

type CreateHandlerArgs = {
	client: any,
	verbose: boolean,
	setIsExecuting: (boolean) -> (),
	setActiveProcesses: ({ [string]: ProcessEntry }) -> (),
	-- Backend build id from WelcomeMsg ("" if the backend predates it).
	setServerVersion: (string) -> (),
}

local function createHandler(args: CreateHandlerArgs): () -> ()
	local client = args.client
	local verbose = args.verbose
	local setIsExecuting = args.setIsExecuting
	local setActiveProcesses = args.setActiveProcesses
	local setServerVersion = args.setServerVersion

	-- Track active processes keyed by executionId
	local activeProcesses: { [string]: { ctx: utils.ExecutionContext, kill: (() -> ())? } } = {}

	-- UI-facing process entries
	local processEntries: { [string]: ProcessEntry } = {}

	-- Runs whose script arrived flagged `scriptContinues`: the RunCommand
	-- carried only the first piece, ScriptChunk messages carry the rest in
	-- order. Keyed by executionId; executed once the last chunk lands.
	local pendingRuns: { [string]: { run: any, parts: { string } } } = {}

	-- Still executing while any run remains.
	local function updateExecuting()
		if next(activeProcesses) == nil then
			setIsExecuting(false)
		end
	end

	-- End a run that failed to start. Without an ExecutionDone the run client
	-- waits forever (issue #32); the client prints its error.
	local function failRun(executionId: string, err: any)
		local done = rodeoProto.ExecutionDone.new({
			executionId = executionId,
			success = false,
			error = `run failed to start: {err}`,
		})
		local msg = rodeoProto.PluginMessage.new({ msg = { type = "done", value = done } })
		client:Send(HttpService:JSONEncode(msg:jsonEncode()))
		updateExecuting()
	end

	-- Start a run whose full script is assembled. Everything after script
	-- transport is unchanged from the pre-chunking single-message path.
	local function executeRun(run)
		setIsExecuting(true)

		local context = run.context or "plugin"
		local lf = run.logFilter
		local logFilter = {
			enableWarn = lf and lf.enableWarn,
			enableError = lf and lf.enableError,
			enableInfo = lf and lf.enableInfo,
			enableOutput = lf and lf.enableOutput,
			enableLogs = lf and lf.enableLogs,
		}
		local scriptArgs = run.scriptArgs or {}
		local executionId = run.executionId

		local created, ctx =
			pcall(utils.createExecutionContext, run.script, context, executionId, run.instancePath, run.scriptPath)
		if not created then
			failRun(executionId, ctx)
			return
		end

		local started, killFn = pcall(runtime.executeInContext, ctx, context, {
			executionId = executionId,
			logFilter = logFilter,
			reloadRequires = run.reloadRequires,
			verbose = run.verbose or verbose,
			scriptArgs = scriptArgs,
			returnFile = run.returnFile,
			showReturn = run.showReturn,
			outputFile = run.outputFile,
			profile = run.profile,
		}, {
			onComplete = function(result)
				local msg = rodeoProto.PluginMessage.new({
					msg = { type = "done", value = rodeoProto.ExecutionDone.new({
						executionId = executionId,
						success = result and result.success or false,
						error = result and result.error,
						errorInOutput = result and result.errorInOutput or false,
						returnValue = result and result.returnValue,
					}) },
				})
				client:Send(HttpService:JSONEncode(msg:jsonEncode()))
				-- Safety net: a capture that died mid-way leaves the simulator
				-- applied; the shim restores on its own success/error paths.
				pcall(simulator.restore, executionId)
				activeProcesses[executionId] = nil
				processEntries[executionId] = nil
				setActiveProcesses(table.clone(processEntries))
				updateExecuting()
			end,
			onRpc = function(msg)
				-- msg.callJson is canonical ClientRpcCall JSON minus executionId.
				-- Stamp executionId, decode to typed, wrap, encode to wire.
				--
				-- This runs on the outgoing-event thread, NOT the thread blocked
				-- in rpc.call's event:Wait(). If encoding/sending throws here
				-- (e.g. JSONEncode rejects a string field carrying non-UTF-8
				-- bytes — the wire is a UTF-8-locked WebSocket text frame), the
				-- error would otherwise be swallowed and the caller would hang
				-- forever. Catch it and deliver an error response for this id so
				-- rpc.call's event:Wait() returns and errors at the call site.
				msg.callJson.executionId = executionId
				local id = msg.callJson.id
				-- Responses are deferred: onRpc runs synchronously inside
				-- rpc.call's send() (BindableEvent:Fire is synchronous), so a
				-- direct Fire here would land before rpc.call reaches
				-- event:Wait() and be missed, hanging the caller forever.
				-- task.defer resumes after the caller is parked on Wait().
				local function respond(res)
					task.defer(function()
						ctx.incomingEvent:Fire({ type = "rpc:response", id = id, res = res })
					end)
				end
				local ok, err = pcall(function()
					local typedCall = runtimeProto.ClientRpcCall.jsonDecode(msg.callJson)
					local reqType = typedCall.req and typedCall.req.type
					-- Plugin-handled RPCs: device-simulator sessions for
					-- roblox.captureViewport. The service is PluginSecurity, so the
					-- run (LocalScript identity in client context) can't drive
					-- it; this handler runs at plugin identity in the same DOM.
					if reqType == "robloxSimulatorApply" then
						local req = typedCall.req.value
						task.spawn(function()
							local okApply, width, height = pcall(simulator.apply, executionId, req.device, req.width, req.height, req.scalingMode, req.pixelDensity, req.orientation, req.deviceForm)
							if okApply then
								respond({ type = "robloxSimulatorApply", value = { width = width, height = height } })
							else
								respond({ type = "error", value = tostring(width) })
							end
						end)
						return
					elseif reqType == "robloxSimulatorRestore" then
						task.spawn(function()
							local okRestore, restoreErr = pcall(simulator.restore, executionId)
							if okRestore then
								respond({ type = "robloxSimulatorRestore", value = {} })
							else
								respond({ type = "error", value = tostring(restoreErr) })
							end
						end)
						return
					end
					local rpcMsg = rodeoProto.PluginMessage.new({
						msg = { type = "rpc", value = typedCall },
					})
					client:Send(HttpService:JSONEncode(rpcMsg:jsonEncode()))
				end)
				if not ok then
					-- e.g. JSONEncode rejecting a string field carrying non-UTF-8
					-- bytes (the wire is a UTF-8-locked WebSocket text frame).
					respond({ type = "error", value = tostring(err) })
				end
			end,
		})
		if not started then
			-- An instance-path run renamed its original module; put it back.
			pcall(ctx.cleanup)
			failRun(executionId, killFn)
			return
		end

		activeProcesses[executionId] = { ctx = ctx, kill = killFn }
		processEntries[executionId] = {
			name = run.scriptPath or `inline-{executionId}`,
			state = "running",
		}
		setActiveProcesses(table.clone(processEntries))
	end

	-- Custom simulator devices persist in Studio settings; drop any left by a
	-- crashed capture before this plugin session starts making new ones.
	task.spawn(function()
		pcall(simulator.sweep)
	end)

	local connection = client.MessageReceived:Connect(function(message: string)
		local success, decoded = pcall(function()
			return HttpService:JSONDecode(message)
		end)

		if not success then
			if verbose then
				warn("[rodeo] Failed to decode message:", message)
			end
			return
		end

		-- Parse as ServerMessage proto (oneof field presence determines type)
		local serverMsg = rodeoProto.ServerMessage.jsonDecode(decoded)

		if serverMsg.msg and serverMsg.msg.type == "welcome" then
			local welcome = serverMsg.msg.value
			local serverVersion = welcome.version or ""
			setServerVersion(serverVersion)
			-- Display only: the backend is the enforcing side (it closes the
			-- socket on a mismatch). An older backend that doesn't check still
			-- gets a working plugin, as before.
			if serverVersion ~= constants.BUILD_ID then
				warn(`[rodeo] Version mismatch: plugin is v{constants.BUILD_ID}, server is v{serverVersion}`)
			end
		elseif serverMsg.msg and serverMsg.msg.type == "run" then
			local run = serverMsg.msg.value
			if run.scriptContinues then
				-- First piece of a chunked script — stash and wait for the
				-- ScriptChunk sequence before executing.
				pendingRuns[run.executionId] = { run = run, parts = { run.script } }
				return
			end
			executeRun(run)
		elseif serverMsg.msg and serverMsg.msg.type == "scriptChunk" then
			local chunk = serverMsg.msg.value
			local pending = pendingRuns[chunk.executionId]
			if not pending then
				warn(`rodeo: script chunk for unknown execution {chunk.executionId} — dropping`)
				return
			end
			table.insert(pending.parts, chunk.data)
			if chunk.isLast then
				pendingRuns[chunk.executionId] = nil
				pending.run.script = table.concat(pending.parts)
				pending.run.scriptContinues = nil
				executeRun(pending.run)
			end
		elseif serverMsg.msg and serverMsg.msg.type == "kill" then
			local kill = serverMsg.msg.value
			-- A kill can land while the script is still streaming in — drop the
			-- partial assembly so the run never starts.
			pendingRuns[kill.executionId] = nil
			local proc = activeProcesses[kill.executionId]
			pcall(simulator.restore, kill.executionId)
			if proc and proc.kill then
				proc.kill()
			else
				-- Unknown execution (already completed, or lost to a plugin
				-- reload). Still ack below: kill is idempotent, and a silent
				-- drop leaves the master's run entry alive forever — rodeo
				-- kill would report success while the run stays in state.
				warn(`rodeo: kill for unknown execution {kill.executionId} — acking anyway`)
			end
			local killedMsg = rodeoProto.PluginMessage.new({
				msg = { type = "killed", value = rodeoProto.ExecutionKilled.new({
					executionId = kill.executionId,
				}) },
			})
			client:Send(HttpService:JSONEncode(killedMsg:jsonEncode()))
			activeProcesses[kill.executionId] = nil
			processEntries[kill.executionId] = nil
			setActiveProcesses(table.clone(processEntries))
			updateExecuting()
		elseif serverMsg.msg and serverMsg.msg.type == "rpcResponse" then
			local rpcResp = serverMsg.msg.value  -- ClientRpcResponse
			-- Forward typed response to the runner. rpcResp.res is the tagged oneof variant:
			--   { type = "<method>", value = <Response> } | { type = "error", value = <string> }
			local proc = activeProcesses[rpcResp.executionId]
			if proc then
				proc.ctx.incomingEvent:Fire({
					type = "rpc:response",
					id = rpcResp.id,
					res = rpcResp.res,
				})
			end
		elseif serverMsg.msg and serverMsg.msg.type == "setTargetMode" then
			mode_driver.setTarget(serverMsg.msg.value.targetMode)
		end
	end)

	return function()
		connection:Disconnect()
	end
end

return {
	createHandler = createHandler,
}
