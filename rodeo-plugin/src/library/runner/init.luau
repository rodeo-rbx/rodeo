local HttpService = game:GetService("HttpService")
local LogService = game:GetService("LogService")

local resultsEvent = script.results
local incomingEvent = script.incoming
local outgoingEvent = script.outgoing

-- Resolve dependencies via ObjectValue refs (all nil-parented for clean stack traces)
local refs = script.refs
local function ref(name: string): any
	return refs[name].Value
end

local module = ref("module")

-- Run parameters, from the plugin (runtime.executeInContext)
local params = script.params:Invoke()
local executionId: string = params.executionId
local scriptArgs: { string } = params.scriptArgs
local returnFile: string = params.returnFile
local showReturn: boolean = params.showReturn
local outputFile: string = params.outputFile
local logFilter = params.logFilter

-- Wire up RPC using shared transport-agnostic module + BindableEvent transport
local createRpc = require(ref("rpc"))
local rpc = createRpc(function(msg)
	outgoingEvent:Fire(msg)
end)
incomingEvent.Event:Connect(function(msg)
	if msg.type == "rpc:response" and msg.id then
		rpc.handleResponse(msg)
	end
end)

-- Runtime proto module — required directly via ref (works across VM boundaries
-- for run:server/client) and handed to every factory explicitly.
local runtime = require(ref("runtimeProto"))

-- This plugin instance's namespace in _G. Every plugin in a DOM's VM shares
-- one _G, and this runner may be in a VM the plugin itself never touched (a
-- play DOM's game VM for run:server/client), so the runner creates the table
-- when needed. Per-plugin state lives here: `procs` (run id → the run's API
-- tables, read by the inlined @rodeo/* shims) and `cmdbar` (results the launch
-- bootstrap's bridge hands back). Never a "current run": one plugin runs
-- several runs at once, so per-run identity comes from the attributes the
-- runtime stamps on each run's module.
local pluginKey: string = params.pluginKey
_G.__rodeo_plugins = _G.__rodeo_plugins or {}
local ns = _G.__rodeo_plugins[pluginKey]
if not ns then
	ns = { procs = {}, cmdbar = {} }
	_G.__rodeo_plugins[pluginKey] = ns
end

-- Fetch host info once, share across factories
local hostInfo = rpc.call("process_get_info", runtime.ProcessGetInfoRequest.new({}))

-- Create RPC-backed API implementations via factories
local fsApi = require(ref("fs"))(rpc, runtime)
local processApi = require(ref("process"))(rpc, scriptArgs, hostInfo, runtime)
local streamApi = require(ref("stream"))(rpc, runtime)
-- Required before `apis` so roblox.bake can close over them (the return-value
-- handling below uses the same pair directly for its JSON form).
local normalize = require(ref("normalize"))
local serializeToLuau = require(ref("serialize"))
local apis = {
	fs = fsApi,
	io = require(ref("io"))(rpc, runtime),
	process = processApi,
	stream = streamApi,
	roblox = require(ref("roblox"))(
		rpc,
		fsApi,
		streamApi,
		normalize,
		serializeToLuau,
		runtime,
		require(ref("scene")),
		require(ref("sceneMotion")),
		require(ref("scenePrimitives")),
		require(ref("sceneNativeMotion"))
	),
}

-- Publish for the inlined @rodeo/* shims: they read ns.procs[run id] through
-- the attributes on their module. The flat `_G.__rodeo_procs` / `__rodeo_eid`
-- pair stays as a compatibility mirror for projects that vendor rodeo-pkg —
-- its singleton reads them and has no way to learn this plugin's key.
ns.procs[executionId] = apis
_G.__rodeo_procs = _G.__rodeo_procs or {}
_G.__rodeo_procs[executionId] = apis
_G.__rodeo_eid = executionId

local limits = require(ref("limits"))
local setupLogging = require(ref("logging"))

-- Setup log capture (LogService → stdout stream, optional output file)
local logConnection, outputFileStream = setupLogging(LogService, apis.fs, apis.stream, apis.io, logFilter, outputFile)

-- Execute user module
local context: string = params.context

local success, result = xpcall(function()
	if context == "elevated" then
		local TestService = game:GetService("TestService")
		local RunService = game:GetService("RunService")
		-- StudioMCP's execute_luau requires the datamodel_type its command bar
		-- should run in. This DOM knows its own DataModel via RunService; map it
		-- to StudioMCP's enum (Edit / Server / Client). Without it the call is
		-- rejected with "datamodel_type is required".
		local datamodelType = if RunService:IsEdit() then "Edit" elseif RunService:IsClient() then "Client" else "Server"
		local refName = "RODEO_" .. executionId
		local ref = Instance.new("ObjectValue")
		ref.Name = refName
		ref.Value = module
		ref.Parent = TestService
		-- Wrapper xpcall's the require so StudioMCP's outer script frame ("AssistantCommand")
		-- never becomes part of the trace. On failure, return the trace as data with a sentinel
		-- prefix; the runner re-raises it below with level=0 so only inner frames surface.
		-- On success, JSON-encode the result inside the elevated wrapper so callers
		-- receive the original typed value (boolean, number, table, etc.) instead of
		-- StudioMCP's lossy string stringification. The runner decodes it back on
		-- receipt before normalize+wire-encode.
		local code = [[
			local HttpService = game:GetService("HttpService")
			local ref = game:GetService("TestService"):FindFirstChild("]] .. refName .. [[")
			local ok, result = xpcall(function()
				return require(ref.Value)
			end, function(err)
				return tostring(err) .. "\n" .. debug.traceback(nil, 2)
			end)
			if not ok then return "__RODEO_ELEVATED_ERROR__" .. tostring(result) end
			local jsonOk, jsonResult = pcall(HttpService.JSONEncode, HttpService, result)
			if jsonOk then return "__RODEO_ELEVATED_JSON__" .. jsonResult end
			return tostring(result) -- non-JSON-encodable (Instance, Vector3, etc.) → string fallback
		]]
		local r: runtime.McpCallResponse = rpc.call(
			"mcp_call",
			runtime.McpCallRequest.new({
				tool = "execute_luau",
				argumentsJson = HttpService:JSONEncode({ code = code, datamodel_type = datamodelType }),
			})
		)
		ref:Destroy()
		if typeof(r.result) == "string" then
			if string.sub(r.result, 1, 24) == "__RODEO_ELEVATED_ERROR__" then
				error(string.sub(r.result, 25), 0)
			end
			if string.sub(r.result, 1, 23) == "__RODEO_ELEVATED_JSON__" then
				return HttpService:JSONDecode(string.sub(r.result, 24))
			end
		end
		return r.result
	elseif context == "cmdbar" then
		-- Command-bar identity without StudioMCP. The launch bootstrap (a
		-- `-runScriptFile`, itself run at command-bar identity) parents a
		-- BindableFunction under CoreGui whose OnInvoke keeps that identity
		-- when this plugin thread invokes it — so only rodeo-launched Studios
		-- have one. The edit DOM's identities share this Luau VM, so the bridge
		-- hands the raw result back through this plugin's namespace in _G
		-- rather than the Bindable return (which deep-copies tables and rejects
		-- functions): return values behave exactly like the plugin context's.
		local bridge = game:GetService("CoreGui"):FindFirstChild("rodeoCmdbar")
		if not bridge then
			error(
				"cmdbar context needs a rodeo-launched Studio: the launch bootstrap installs the "
					.. "command-bar bridge, and this Studio has none (opened by hand, or launched by an "
					.. "older rodeo). Use --context elevated instead.",
				0
			)
		end
		-- The bridge belongs to the backend that launched this Studio. Only that
		-- backend's plugin is active here, so this holds unless a stale plugin
		-- file of another build is dialing this port.
		local bridgeBuild = bridge:GetAttribute("rodeoBuild")
		local pluginBuild = params.pluginBuild
		if bridgeBuild ~= nil and pluginBuild ~= nil and bridgeBuild ~= pluginBuild then
			error(
				`cmdbar bridge belongs to rodeo build {bridgeBuild}, this plugin is {pluginBuild}: `
					.. "the Studio was launched by another rodeo serve — run against that serve's port.",
				0
			)
		end
		bridge:Invoke(module, executionId)
		local slot = ns.cmdbar[executionId]
		if not slot then
			error("cmdbar bridge returned no result for this run", 0)
		end
		ns.cmdbar[executionId] = nil
		if not slot.ok then
			error(slot.result, 0)
		end
		return slot.result
	end
	return require(module)
end, debug.traceback)

task.wait(0.2) -- Allow final logs to flush
logConnection:Disconnect()
if outputFileStream then
	apis.stream.close(outputFileStream)
end

-- Handle return value output. `returnValueJson` is the canonical wire-format
-- (JSON-encoded normalized result); it flows over the WS as
-- `ExecutionDone.return_value` only when no return file was given — a return
-- file captures the value instead, keeping it off the done message (a single
-- unchunkable hop whose downstream transports cap at a few MB). File and
-- stdout writes are size-unbounded: stream.write chunks large payloads.

local returnValueJson: string? = nil
local capError: string? = nil
if success and result ~= nil then
	local isLuauReturnFile = returnFile ~= "" and string.match(returnFile, "%.luau?$") ~= nil

	-- The JSON form backs the wire field, --show-return, and non-.luau return
	-- files; .luau return files go through roblox.bake instead, so the
	-- (potentially huge) normalize + encode is skipped when nothing consumes it.
	local json: string? = nil
	if not isLuauReturnFile or showReturn then
		json = HttpService:JSONEncode(normalize(result))
	end

	if returnFile ~= "" then
		if isLuauReturnFile then
			-- Same normalize → serialize → write path as the public API; one
			-- implementation so the flag and roblox.bake can't drift.
			apis.roblox.bake(returnFile, result)
		else
			local f = apis.fs.open(returnFile, "w")
			apis.stream.write(f, json :: string)
			apis.stream.close(f)
		end
	end

	if showReturn then
		apis.stream.write(apis.io.stdout, (json :: string) .. "\n")
	end

	if returnFile == "" then
		-- The wire field rides the done message — a single unchunkable hop
		-- with a hard transport cap; an oversized value would kill the
		-- backend↔master stream. File writes and --show-return stdout are
		-- chunked and size-unbounded, so cap only the wire copy.
		local j = json :: string
		local sizeMsg = string.format(
			"return value too large for the wire (%.1f MB > %d MB)",
			#j / 1048576,
			limits.MAX_WIRE_RETURN_BYTES / 1048576
		)
		if #j <= limits.MAX_WIRE_RETURN_BYTES then
			returnValueJson = j
		elseif showReturn then
			-- stdout already carried the full value; just flag the omission.
			-- ExecutionDone.error is dropped by every client, so stderr is
			-- the one channel users actually see.
			apis.stream.write(
				apis.io.stderr,
				"\27[33m" .. sizeMsg .. "; omitted from result.return — use --return <path> to capture it\27[0m\n"
			)
		else
			capError = sizeMsg .. "; write it to a file with --return <path>"
			apis.stream.write(apis.io.stderr, "\27[31m" .. capError .. "\27[0m\n")
		end
	end
end

-- Clean up process-scoped _G entry
ns.procs[executionId] = nil
_G.__rodeo_eid = nil
_G.__rodeo_procs[executionId] = nil

resultsEvent:Fire({
	success = success and capError == nil,
	error = if not success then tostring(result) elseif capError ~= nil then capError else nil,
	returnValue = returnValueJson,
})

return nil
