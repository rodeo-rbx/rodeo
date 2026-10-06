-- Shared utilities for rodeo plugins

local ScriptEditorService = game:GetService("ScriptEditorService")

local constants = require(script.Parent.constants)

-- LuaSourceContainer.Source rejects strings at 200,000 bytes, even though
-- Studio's script editor and compiler support substantially larger sources.
-- UpdateSourceAsync writes through the editor API and avoids that property
-- assignment limit. Its callback may be retried, so keep it side-effect free.
local function updateSource(scriptInstance: LuaSourceContainer, source: string)
	ScriptEditorService:UpdateSourceAsync(scriptInstance, function()
		return source
	end)
end

local function resolveInstancePath(path: string): Instance?
	local resolver = Instance.new("ModuleScript")
	resolver.Name = "__rodeo_path_resolver"
	resolver.Source = "return " .. path
	resolver.Parent = game:GetService("ReplicatedStorage")

	local success, result = pcall(function()
		return require(resolver)
	end)

	resolver:Destroy()

	if success and typeof(result) == "Instance" then
		return result
	end
	return nil
end

local function makeIgnoreSuffix(executionId: string): string
	return constants.IGNORE_SUFFIX .. executionId
end

local function makeIgnoreSuffixPattern(executionId: string): string
	local suffix = makeIgnoreSuffix(executionId)
	return string.gsub(string.gsub(suffix, "%.", "%%."), "%-", "%%-") .. "$"
end

export type RunContext = "server" | "client" | "plugin" | "elevated" | "cmdbar"

-- i cloned because for some reason runner loses children and .rpc errors, this is an attempt to fully preserve
-- runner template for what seems to be an edge case from plugin hot reloading
local runnerTemplate = script.Parent:WaitForChild("runner"):Clone()
export type ExecutionContext = {
	runner: ModuleScript,
	shim: (Script | LocalScript)?,
	module: ModuleScript,
	originalInstance: ModuleScript?,
	resultsEvent: BindableEvent,
	incomingEvent: BindableEvent,
	outgoingEvent: BindableEvent,
	paramsFunction: BindableFunction,
	cleanup: () -> (),
}

local function createExecutionContext(source: string, context: RunContext, executionId: string, instancePath: string?, scriptPath: string?): ExecutionContext
	-- Determine module name for stack traces.
	-- Priority: instancePath → scriptPath → inline-<executionId>. Never the
	-- source itself: Instance.Name caps at 200,000 chars, so naming a run by
	-- its content made large inline scripts fail before executing (issue #16).
	local moduleName = `inline-{executionId}`
	if instancePath then
		local lastSegment = string.match(instancePath, '%["([^"]+)"%]$')
		if lastSegment then
			moduleName = lastSegment
		end
	elseif scriptPath then
		moduleName = scriptPath
	end

	-- Create or clone user module
	local module: ModuleScript
	local originalInstance: ModuleScript? = nil
	if instancePath then
		local resolved = resolveInstancePath(instancePath)
		if resolved and resolved:IsA("ModuleScript") then
			originalInstance = resolved
			-- Clone returns nil for an instance with Archivable off. Fail
			-- before the rename below, which nothing would undo.
			local clone = originalInstance:Clone()
			if not clone then
				error(`can't run {instancePath}: its Archivable is off, and the run executes a clone of it`, 0)
			end
			module = clone

			-- Rename original so clone takes precedence in require resolution
			originalInstance.Name = originalInstance.Name .. makeIgnoreSuffix(executionId)
			module.Parent = originalInstance.Parent
		else
			module = Instance.new("ModuleScript")
			module.Name = moduleName
			updateSource(module, source)
		end
	else
		module = Instance.new("ModuleScript")
		module.Name = moduleName
		updateSource(module, source)
	end

	-- Create nil-parented runner module from template
	local runner = Instance.new("ModuleScript")
	runner.Name = "@rodeo/runner"
	runner.Source = runnerTemplate.Source

	-- BindableEvents as children of runner (init.luau accesses via script.results etc.)
	local resultsEvent = Instance.new("BindableEvent")
	resultsEvent.Name = "results"
	resultsEvent.Parent = runner

	local incomingEvent = Instance.new("BindableEvent")
	incomingEvent.Name = "incoming"
	incomingEvent.Parent = runner

	local outgoingEvent = Instance.new("BindableEvent")
	outgoingEvent.Name = "outgoing"
	outgoingEvent.Parent = runner

	-- The runner invokes this once at startup for its run parameters (see
	-- runtime.executeInContext for why they don't travel as attributes).
	local paramsFunction = Instance.new("BindableFunction")
	paramsFunction.Name = "params"
	paramsFunction.Parent = runner

	-- Clone and nil-parent each dependency module with @rodeo/ names for clean stack traces
	local sharedRoot = script.Parent.Parent.Parent.shared

	local depModules = {}
	local function cloneDep(template: ModuleScript, name: string): ModuleScript
		local m = template:Clone()
		m.Name = name
		-- Studio reuses compiled code for identical source, together with the
		-- chunk name it was first compiled under. Every same-build rodeo plugin
		-- in this Studio carries these modules, so their errors would name
		-- whichever plugin's file compiled them first. A trailing comment
		-- unique to this plugin keeps the clones apart, so errors read
		-- `@rodeo/<name>:<line>`; it adds no lines before the code.
		m.Source = template.Source .. "\n-- " .. constants.PLUGIN_KEY
		table.insert(depModules, m)
		return m
	end

	local rpcMod = cloneDep(sharedRoot.rpc, "@rodeo/rpc")
	local limitsMod = cloneDep(sharedRoot.limits, "@rodeo/limits")
	local loggingMod = cloneDep(sharedRoot.logging, "@rodeo/logging")
	local normalizeMod = cloneDep(sharedRoot.normalize, "@rodeo/normalize")
	local serializeMod = cloneDep(sharedRoot.serialize, "@rodeo/serialize")
	local fsMod = cloneDep(sharedRoot.fs, "@rodeo/fs")
	local ioMod = cloneDep(sharedRoot.io, "@rodeo/io")
	local processMod = cloneDep(sharedRoot.process, "@rodeo/process")
	local streamMod = cloneDep(sharedRoot.stream, "@rodeo/stream")
	local robloxMod = cloneDep(runnerTemplate.roblox, "@rodeo/roblox")
	local sceneMod = cloneDep(runnerTemplate.scene, "@rodeo/scene")
	local scenePrimitivesMod = cloneDep(runnerTemplate.scenePrimitives, "@rodeo/scenePrimitives")
	local sceneNativeMotionMod = cloneDep(runnerTemplate.sceneNativeMotion, "@rodeo/sceneNativeMotion")
	local sceneMotionMod = cloneDep(runnerTemplate.sceneMotion, "@rodeo/sceneMotion")

	-- Build refs Folder with ObjectValues (runner init.luau resolves deps via script.refs)
	local refs = Instance.new("Folder")
	refs.Name = "refs"
	refs.Parent = runner

	local function addRef(name: string, value: Instance)
		local ov = Instance.new("ObjectValue")
		ov.Name = name
		ov.Value = value
		ov.Parent = refs
	end

	-- Clone the entire proto folder so its ModuleScripts are cross-VM reachable.
	-- Plugin-tree ModuleScripts aren't requirable from the server VM; cloned instances
	-- (parented to the runner, which is nil-parented) cross VM boundaries via ObjectValue.
	-- We clone the whole folder because runtime.luau uses `script.Parent.proto` to reach
	-- its sibling proto/ helpers folder — cloning runtime.luau alone would break that.
	local protoFolder = sharedRoot.proto:Clone()
	protoFolder.Parent = runner

	addRef("module", module)
	addRef("rpc", rpcMod)
	addRef("limits", limitsMod)
	addRef("runtimeProto", protoFolder.runtime)
	addRef("logging", loggingMod)
	addRef("normalize", normalizeMod)
	addRef("serialize", serializeMod)
	addRef("fs", fsMod)
	addRef("io", ioMod)
	addRef("process", processMod)
	addRef("stream", streamMod)
	addRef("roblox", robloxMod)
	addRef("scene", sceneMod)
	addRef("sceneMotion", sceneMotionMod)
	addRef("scenePrimitives", scenePrimitivesMod)
	addRef("sceneNativeMotion", sceneNativeMotionMod)

	-- Create thin shim Script/LocalScript for server/client (must be parented to trigger execution)
	local shim: (Script | LocalScript)? = nil
	if context == "server" or context == "client" then
		shim = Instance.new(if context == "server" then "Script" else "LocalScript")
		shim.Name = "__rodeo_runner_" .. executionId
		shim.Source = "require(script.refs.runner.Value)"
		local shimRefs = Instance.new("Folder")
		shimRefs.Name = "refs"
		shimRefs.Parent = shim
		local runnerOV = Instance.new("ObjectValue")
		runnerOV.Name = "runner"
		runnerOV.Value = runner
		runnerOV.Parent = shimRefs
	end

	local cleanup = function()
		module:Destroy()
		runner:Destroy()
		for _, dep in depModules do
			dep:Destroy()
		end
		if shim then
			shim:Destroy()
		end
		if originalInstance then
			originalInstance.Name = string.gsub(originalInstance.Name, makeIgnoreSuffixPattern(executionId), "")
		end
	end

	return {
		runner = runner,
		shim = shim,
		module = module,
		originalInstance = originalInstance,
		resultsEvent = resultsEvent,
		incomingEvent = incomingEvent,
		outgoingEvent = outgoingEvent,
		paramsFunction = paramsFunction,
		cleanup = cleanup,
	}
end

return {
	resolveInstancePath = resolveInstancePath,
	createExecutionContext = createExecutionContext,
	makeIgnoreSuffix = makeIgnoreSuffix,
	makeIgnoreSuffixPattern = makeIgnoreSuffixPattern,
}
