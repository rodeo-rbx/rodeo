--[[
	runtime.luau - Script execution runtime

	Handles executing scripts in server/client/plugin contexts,
	including log capture, result serialization, and cleanup.
	Require resolution and uncacheable module traversal are in require_resolver.luau.
]]

local Players = game:GetService("Players")
local ServerScriptService = game:GetService("ServerScriptService")

local constants = require(script.Parent.constants)
local createLogger = require(script.Parent.log)
local createRpc = require(script.Parent.Parent.Parent.shared.rpc)
local utils = require(script.Parent.utils)
local requireResolver = require(script.Parent.require_resolver)

export type LogFilter = {
	enableWarn: boolean?,
	enableError: boolean?,
	enableInfo: boolean?,
	enableOutput: boolean?,
	enableLogs: boolean?,
}

export type ExecutionParams = {
	executionId: string,
	logFilter: LogFilter?,
	reloadRequires: boolean?,
	verbose: boolean?,
	scriptArgs: { string }?,
	returnFile: string?,
	showReturn: boolean?,
	outputFile: string?,
	profile: boolean?,
}

export type ExecutionCallbacks = {
	onComplete: (result: { success: boolean, error: string?, errorInOutput: boolean?, returnValue: string? }?) -> (),
	onRpc: (msg: any) -> (),
}

type RunContext = "server" | "client" | "plugin" | "elevated" | "cmdbar"

-- Execute a module in a specific context (server/client/plugin)
-- ctx contains the runner, module, events, and cleanup function (created by utils.createExecutionContext)
local function executeInContext(ctx: utils.ExecutionContext, context: RunContext, params: ExecutionParams, callbacks: ExecutionCallbacks)
	-- Create RPC handle early so logger can use it
	local rpc = createRpc(function(msg)
		ctx.outgoingEvent:Fire(msg)
	end)
	ctx.incomingEvent.Event:Connect(function(msg)
		if msg.type == "rpc:response" and msg.id then
			rpc.handleResponse(msg)
		end
	end)
	local log = createLogger(rpc, if params.verbose then "debug" else "info")

	-- Run parameters reach the runner through its `params` BindableFunction,
	-- not attributes: with Workspace.NextGenerationReplication on, a game
	-- server rejects string attributes over 50 characters (issue #32), which
	-- the log filter's JSON alone exceeds, as can any path or script argument.
	-- The runner may run in another Luau VM (a play DOM's game VM for server
	-- and client runs); the table crosses as a copy.
	local runParams = {
		executionId = params.executionId,
		scriptArgs = params.scriptArgs or {},
		returnFile = params.returnFile or "",
		showReturn = params.showReturn or false,
		outputFile = params.outputFile or "",
		logFilter = params.logFilter or {},
		context = context,
		pluginBuild = constants.BUILD_ID,
		pluginKey = constants.PLUGIN_KEY,
	}
	ctx.paramsFunction.OnInvoke = function()
		return runParams
	end

	log.debug(`START`)
	log.debug(`ctx.module: {ctx.module:GetFullName()}`)
	log.debug(`context: {context}`)
	log.debug(`reloadRequires: {params.reloadRequires or false}`)

	-- Get module path BEFORE traversal (so we can re-resolve after if it gets cloned)
	local modulePath = requireResolver.getAbsoluteInstancePath(ctx.module)
	log.debug(`modulePath: {modulePath or "nil"}`)

	-- Instance requires use the VM's require cache by default — Roblox's own
	-- semantics, so a require resolves to the live module the surrounding game
	-- is already using. `reloadRequires` opts into re-evaluation: the traversal
	-- clones the require tree so this run gets its own freshly-initialized
	-- copies (test isolation, or picking up edits made since the modules were
	-- first required). Without it, a module whose Source changed since this DOM
	-- cached it, and what requires it, still comes back fresh (see
	-- setupChangedModuleReload).
	local traversalCleanup: (() -> ())? = nil
	if params.reloadRequires then
		log.debug(`Setting up uncachable require traversal...`)
		traversalCleanup = requireResolver.setupUncachableRequireModuleTraversal(ctx.module, params.executionId, log)
		log.debug(`Traversal setup complete`)
	else
		traversalCleanup = requireResolver.setupChangedModuleReload(ctx.module, params.executionId, log)
	end

	-- Re-resolve module via path (picks up cloned module if traversal cloned it)
	if modulePath then
		local resolvedModule = utils.resolveInstancePath(modulePath)
		if resolvedModule and resolvedModule:IsA("ModuleScript") then
			log.debug(`Re-resolved module: {resolvedModule:GetFullName()}`)
			ctx.runner:FindFirstChild("moduleRef").Value = resolvedModule
		end
	end

	-- The inlined @rodeo/* shims find their plugin's namespace and their run
	-- through these two attributes on the module they live in (`script`). Both
	-- have to come from the module: every plugin in a DOM shares one _G, and
	-- one plugin runs several runs at once, so a shared "current run" slot
	-- would be overwritten between a run's start and a late require. Both
	-- values stay well under NextGenerationReplication's 50-character limit
	-- for string attributes (a 12-hex run id; `<build>-<port>`).
	if ctx.module then
		ctx.module:SetAttribute("rodeoPlugin", constants.PLUGIN_KEY)
		ctx.module:SetAttribute("rodeoExecutionId", params.executionId)
	end

	-- Profiler label injection: stamp each frame with execution_id so dumps can be matched
	local profileConnection: RBXScriptConnection? = nil
	if params.profile then
		local RunService = game:GetService("RunService")
		local label = "rodeo:" .. params.executionId
		profileConnection = RunService.Heartbeat:Connect(function()
			debug.profilebegin(label)
			debug.profileend()
		end)
		log.debug(`Profiling enabled: {label}`)
	end

	-- Listen for outgoing RPC messages from script
	local outgoingConnection = ctx.outgoingEvent.Event:Connect(function(msg)
		if msg.type == "rpc" then
			callbacks.onRpc(msg)
		end
	end)

	local function doCleanup()
		if profileConnection then
			profileConnection:Disconnect()
			profileConnection = nil
		end
		if traversalCleanup then
			log.debug(`Calling traversalCleanup...`)
			traversalCleanup()
			log.debug(`traversalCleanup complete`)
		end
		log.debug(`Starting ctx.cleanup, ctx.module = {ctx.module:GetFullName()}`)
		ctx.cleanup()
		log.debug(`ctx.cleanup complete`)
	end

	-- Listen for results
	local resultsConnection
	resultsConnection = ctx.resultsEvent.Event:Connect(function(result)
		log.debug(`Results received, starting cleanup...`)
		resultsConnection:Disconnect()
		doCleanup()
		outgoingConnection:Disconnect()
		callbacks.onComplete(result)
	end)

	log.debug(`Starting execution...`)

	-- Trigger execution
	if context == "server" then
		-- Parent thin shim Script to SSS to trigger auto-execution
		ctx.shim.Parent = ServerScriptService
	elseif context == "client" then
		task.spawn(function()
			-- Parent thin shim LocalScript to PlayerScripts to trigger auto-execution
			ctx.shim.Parent = Players.LocalPlayer.PlayerScripts
		end)
	else -- plugin / elevated / cmdbar (runner handles the elevated MCP hop and the cmdbar bridge)
		task.spawn(function()
			require(ctx.runner)
		end)
	end

	-- Return a kill function that destroys the runner and cleans up
	return function()
		log.debug(`Kill requested, destroying runner...`)
		resultsConnection:Disconnect()
		doCleanup()
		outgoingConnection:Disconnect()
	end
end

return {
	executeInContext = executeInContext,
}
