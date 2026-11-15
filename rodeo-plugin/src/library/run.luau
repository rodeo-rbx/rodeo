--[[
	run.luau - one run, from its RunCommand to its ExecutionDone

	Everything a run does between the transport handing over its command and
	the transport carrying its messages back: the execution context, the
	runner, the RPC envelope, the completion message. Two transports use it:

	- Studio: MessageHandler, over the plugin's WebSocket to its backend.
	- ree (roblox-engine-emulator): library/ree.luau, over the engine's host
	  call to the `rodeo run` that started it.

	The transport is a `send` function that takes a `rodeoProto.PluginMessage`
	(rpc or done). For an rpc it also gets `respond`, which delivers that
	call's response to the run: a transport whose reply arrives with the send
	(ree's host call) answers through it, and one whose replies arrive later
	on their own (the WebSocket) fires the context's incoming event itself.
]]

local runtime = require(script.Parent.runtime)
local utils = require(script.Parent.utils)
local simulator = require(script.Parent.simulator)
local rodeoProto = require(script.Parent.Parent.Parent.proto.rodeo)
local runtimeProto = require(script.Parent.Parent.Parent.proto.runtime)

-- Delivers one RPC's response to the run: `{ type = "<method>", value =
-- <Response> } | { type = "error", value = <string> }`.
export type Respond = (res: any) -> ()

export type Options = {
	-- The plugin's verbose toggle; a run's own `verbose` turns it on too.
	verbose: boolean?,
	-- Carries a PluginMessage to the server. May throw (a string the wire
	-- can't carry): for an rpc the caller then gets the error at its call
	-- site.
	send: (message: any, respond: Respond?) -> (),
	-- Called once the run is over and its ExecutionDone has been sent.
	onComplete: (() -> ())?,
}

export type Run = {
	ctx: utils.ExecutionContext,
	kill: () -> (),
}

-- Start the run `run` (a RunCommand whose script is complete). Returns the
-- running run, or nil when it failed to start: its ExecutionDone has been
-- sent then, so the run client fails it with the reason instead of waiting
-- forever (issue #32).
local function start(run: any, options: Options): Run?
	local send = options.send
	local executionId: string = run.executionId

	-- End a run that failed to start; the client prints its error.
	local function fail(err: any)
		local done = rodeoProto.ExecutionDone.new({
			executionId = executionId,
			success = false,
			error = `run failed to start: {err}`,
		})
		send(rodeoProto.PluginMessage.new({ msg = { type = "done", value = done } }))
	end

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

	local created, ctx = pcall(utils.createExecutionContext, run.script, context, executionId, run.instancePath, run.scriptPath)
	if not created then
		fail(ctx)
		return nil
	end

	local started, killFn = pcall(runtime.executeInContext, ctx, context, {
		executionId = executionId,
		logFilter = logFilter,
		reloadRequires = run.reloadRequires,
		verbose = run.verbose or options.verbose,
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
			send(msg)
			-- Safety net: a capture that died mid-way leaves the simulator
			-- applied; the shim restores on its own success/error paths.
			pcall(simulator.restore, executionId)
			if options.onComplete then
				options.onComplete()
			end
		end,
		onRpc = function(msg)
			-- msg.callJson is canonical ClientRpcCall JSON minus executionId.
			-- Stamp executionId, decode to typed, wrap, hand to the transport.
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
				send(rpcMsg, respond)
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
		fail(killFn)
		return nil
	end

	return { ctx = ctx, kill = killFn }
end

return {
	start = start,
}
