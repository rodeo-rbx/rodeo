--[[
	MessageHandler.luau - WebSocket message handling for the Rodeo plugin

	Handles incoming messages from the CLI server: exec, kill, rpc:response, welcome.
	Tracks active processes and manages execution lifecycle.
]]

local HttpService = game:GetService("HttpService")

local runner = require(script.Parent.Parent.Parent.library.run)
local constants = require(script.Parent.Parent.Parent.library.constants)
local mode_driver = require(script.Parent.Parent.Parent.library.mode_driver)
local server_version = require(script.Parent.Parent.Parent.library.server_version)
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
	-- The last one set, which outlives this handler: a verbose toggle
	-- recreates the handler on the same socket, after its welcome.
	getServerVersion: () -> string?,
}

local function createHandler(args: CreateHandlerArgs): () -> ()
	local client = args.client
	local verbose = args.verbose
	local setIsExecuting = args.setIsExecuting
	local setActiveProcesses = args.setActiveProcesses
	local setServerVersion = args.setServerVersion
	local getServerVersion = args.getServerVersion

	-- Track active processes keyed by executionId
	local activeProcesses: { [string]: runner.Run } = {}

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

	-- A PluginMessage to the backend, as one WebSocket text frame.
	local function send(message)
		client:Send(HttpService:JSONEncode(message:jsonEncode()))
	end

	-- Why this plugin won't run the server's scripts, or nil if it will. The
	-- backend is the enforcing side: since 1.4 it routes no runs to a plugin
	-- of another build (it closes or parks the socket, or
	-- RODEO_SKIP_VERSION_CHECK made it accept on purpose). A server older than
	-- that check routes runs here regardless, so the plugin refuses them
	-- itself (see server_version.luau). The server's welcome always precedes
	-- its first run, so the version read here is this connection's.
	local function refusal(): string?
		local serverVersion = getServerVersion()
		if serverVersion == nil or serverVersion == constants.BUILD_ID then
			return nil
		end
		if not server_version.predatesHandshake(serverVersion) then
			return nil
		end
		return server_version.refusal({
			build = constants.BUILD_ID,
			version = constants.VERSION,
			file = constants.PLUGIN_FILE,
		}, serverVersion)
	end

	-- Answer a run this plugin won't execute, so its CLI fails it with the
	-- reason instead of waiting forever. The message goes out as a stderr
	-- write, which every rodeo CLI prints (old ones never print
	-- ExecutionDone.error), and as the error of the ExecutionDone that ends
	-- the run, marked as already in the output so a current CLI doesn't print
	-- it twice.
	local function refuseRun(executionId: string, message: string)
		local write = runtimeProto.ClientRpcCall.jsonDecode({
			id = "",
			executionId = executionId,
			streamWrite = { handle = "stderr", data = `\27[31m[rodeo] {message}\27[0m\n` },
		})
		client:Send(HttpService:JSONEncode(rodeoProto.PluginMessage.new({
			msg = { type = "rpc", value = write },
		}):jsonEncode()))
		local done = rodeoProto.PluginMessage.new({
			msg = { type = "done", value = rodeoProto.ExecutionDone.new({
				executionId = executionId,
				success = false,
				error = message,
				errorInOutput = true,
			}) },
		})
		client:Send(HttpService:JSONEncode(done:jsonEncode()))
	end

	-- Start a run whose full script is assembled. Everything after script
	-- transport is library/run's: the same code runs a run in ree.
	local function executeRun(run)
		setIsExecuting(true)

		local executionId = run.executionId
		local proc = runner.start(run, {
			verbose = verbose,
			send = send,
			onComplete = function()
				activeProcesses[executionId] = nil
				processEntries[executionId] = nil
				setActiveProcesses(table.clone(processEntries))
				updateExecuting()
			end,
		})
		if not proc then
			-- Failed to start; its ExecutionDone is already on the wire.
			updateExecuting()
			return
		end

		activeProcesses[executionId] = proc
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
			if serverVersion ~= constants.BUILD_ID then
				local refusing = if refusal()
					then " (predates rodeo's plugin version check), so this plugin refuses its runs"
					else ""
				warn(`[rodeo] Version mismatch: plugin is v{constants.BUILD_ID}, server is v{serverVersion}{refusing}`)
			end
		elseif serverMsg.msg and serverMsg.msg.type == "run" then
			local run = serverMsg.msg.value
			local refused = refusal()
			if refused then
				refuseRun(run.executionId, refused)
				return
			end
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
			local target = serverMsg.msg.value
			mode_driver.setTarget(target.targetMode, target.numPlayers)
		end
	end)

	-- A transition the edit DOM gave up on goes to the master, which fails
	-- the runs queued behind it with this reason.
	local function reportModeFailure(targetMode: string, err: string)
		local msg = rodeoProto.PluginMessage.new({
			msg = { type = "modeFailed", value = rodeoProto.ModeTransitionFailed.new({
				targetMode = targetMode,
				error = err,
			}) },
		})
		local ok, sendErr = pcall(function()
			client:Send(HttpService:JSONEncode(msg:jsonEncode()))
		end)
		if not ok then
			warn(`[rodeo] could not report the mode transition failure: {sendErr}`)
		end
	end
	mode_driver.setReporter(reportModeFailure)

	return function()
		connection:Disconnect()
		mode_driver.setReporter(nil)
	end
end

return {
	createHandler = createHandler,
}
