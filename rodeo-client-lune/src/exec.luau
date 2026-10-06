-- Shared runCode plumbing for all three tiers (client / studio / dom).
-- Mirrors rodeo-client-ts/src/client.ts `daemonRunCode`.

local process = require("@lune/process")
local serde = require("@lune/serde")
local task = require("@lune/task")

local run = require("./run")
local uuid = require("./util/uuid")

type Daemon = {
	call: (method: string, params: any?) -> any,
	registerStream: (streamId: string, cb: (string, { [string]: any }) -> ()) -> (),
	unregisterStream: (streamId: string) -> (),
	shutdown: () -> (),
}

type ProcessedSource = {
	script: string,
	scriptPath: string?,
	instancePath: string?,
}

local function processSource(opts: run.RunCodeOpts): ProcessedSource
	local args = { "__process_source" }
	if opts.file then
		table.insert(args, opts.file)
		if opts.sourcemap then
			table.insert(args, "--sourcemap")
			table.insert(args, opts.sourcemap)
		end
	elseif opts.source then
		table.insert(args, "--source")
		table.insert(args, opts.source)
	else
		error("runCode requires either `source` or `file`")
	end
	local result = process.exec("rodeo", args)
	if not result.ok then
		error(`source processing failed: {result.stderr}`)
	end
	return serde.decode("json", result.stdout) :: ProcessedSource
end

-- `method` selects the tier (client./studio./dom.runCode); `target` carries
-- the tier's identifier ({}, { studioHandle = h }, or { domId = id }).
local function runViaDaemon(
	daemon: Daemon,
	method: string,
	target: { [string]: any },
	opts: run.RunCodeOpts
): run.RunResult
	local processed = processSource(opts)

	-- Local tag for the default profile-dir name only — NOT the run id.
	local profileTag = uuid.v4()
	local profileDir: string? = nil
	if opts.profile ~= nil then
		profileDir = if opts.profile ~= "" then opts.profile else `.rodeo/.temp/profiles/{profileTag}`
	end

	local streamId = uuid.v4()
	local bufferedOutput = {}

	local co = coroutine.running()
	assert(co, "runCode must run in a coroutine")

	local done = false
	local finalResult: run.RunResult? = nil
	local finalError: string? = nil

	daemon.registerStream(streamId, function(m, params)
		if done then
			return
		end
		if m == "stream.data" then
			local kind = params.kind
			if kind == "stdout" or kind == "stderr" then
				table.insert(bufferedOutput, tostring(params.chunk or ""))
			end
		elseif m == "stream.done" then
			daemon.unregisterStream(streamId)
			done = true
			local r = params.result or {}
			local output = (type(r.output) == "string" and #r.output > 0) and r.output or table.concat(bufferedOutput)
			local parsedReturn: any = nil
			if type(r.returnValue) == "string" and #r.returnValue > 0 then
				local ok, decoded = pcall(serde.decode, "json", r.returnValue)
				if ok then
					parsedReturn = decoded
				end
			end
			finalResult = {
				ok = r.ok == true,
				output = output,
				exitCode = r.exitCode or 0,
				executionId = if type(r.executionId) == "string" then r.executionId else nil,
				["return"] = parsedReturn,
				error = if type(r.error) == "string" then r.error else nil,
			}
			task.spawn(co)
		elseif m == "stream.error" then
			daemon.unregisterStream(streamId)
			done = true
			finalError = tostring(params.error or "run failed")
			task.spawn(co)
		end
	end)

	local params: { [string]: any } = {
		streamId = streamId,
		source = processed.script,
		mode = opts.mode,
		domKind = opts.domKind,
		context = opts.context,
		showReturn = opts.showReturn == true,
		reloadRequires = opts.reloadRequires == true,
		verbose = opts.verbose == true,
		scriptArgs = opts.scriptArgs or {},
		profileDir = profileDir,
		returnFile = opts.returnFile,
		instancePath = processed.instancePath,
		scriptPath = processed.scriptPath,
		logFilter = opts.logFilter,
	}
	for k, v in target do
		params[k] = v
	end

	local callOk, callErr = pcall(daemon.call, method, params)
	if not callOk then
		daemon.unregisterStream(streamId)
		error(callErr, 2)
	end

	if not done then
		coroutine.yield()
	end

	if finalError then
		error(finalError, 2)
	end
	return assert(finalResult, "runCode completed with no result")
end

return {
	runViaDaemon = runViaDaemon,
}
