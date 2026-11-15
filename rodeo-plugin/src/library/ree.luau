--[[
	ree.luau - rodeo's entry in ree (roblox-engine-emulator)

	`rodeo run --ree` starts `ree` with rodeo's harness
	(rodeo-client/src/ree/Rodeo.luau), which mounts this plugin's modules and
	calls this entry with the harness Context. One run per engine process:
	the entry says hello, gets its RunCommand back, runs it with the same
	library/run that Studio's MessageHandler uses, and ends the engine's run
	once the ExecutionDone is out.

	The transport is the engine's host call (`Context.HostCall`): one
	PluginMessage out as JSON, the engine waits, and the host's ServerMessage
	comes back (empty for a message that has no reply). The engine does
	nothing else while it waits and its clocks stand, so the script's calls to
	the host (`@rodeo/fs`, `@rodeo/io`, ...) take no time in the run.
]]

local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")

local constants = require(script.Parent.constants)
local rodeoProto = require(script.Parent.Parent.Parent.proto.rodeo)
local runner = require(script.Parent.run)

return function(Context: any)
	local hostCall = Context.HostCall
	if type(hostCall) ~= "function" then
		error(
			"this ree has no host call (Context.HostCall), which `rodeo run --ree` runs a script through: "
				.. "it needs a rodeo-rbx/roblox-engine-emulator newer than 0.2.0-rodeo.1",
			0
		)
	end

	-- One PluginMessage to the host; its ServerMessage, or nil for no reply.
	local function call(message: any): any?
		local reply = hostCall(HttpService:JSONEncode(message:jsonEncode()))
		if reply == "" then
			return nil
		end
		return rodeoProto.ServerMessage.jsonDecode(HttpService:JSONDecode(reply))
	end

	-- Hello: which build this is and what kind of DataModel the engine made.
	-- The reply is the run.
	local running = RunService:IsRunning()
	local server = running and RunService:IsServer()
	local hello = call(rodeoProto.PluginMessage.new({
		msg = {
			type = "studioState",
			value = rodeoProto.StudioStateMsg.new({
				domId = "ree",
				studioId = "ree",
				isServer = server,
				isClient = running and not server,
				isRunning = running,
				isEdit = not running,
				mode = if running then "test" else "edit",
				domKind = if not running then "edit" elseif server then "server" else "client",
				pluginVersion = constants.BUILD_ID,
			}),
		},
	}))
	local run = if hello and hello.msg and hello.msg.type == "run" then hello.msg.value else nil
	if not run then
		error("rodeo's ree entry: the host answered the hello without a run", 0)
	end

	local over = false
	local exiting = false
	local finished = Instance.new("BindableEvent")
	local proc: runner.Run? = nil

	-- process.exit: the script's thread is parked for good after its notify
	-- and the run client ends the run with a kill. A Studio's plugin gets
	-- that kill over its socket, some frames later; here the entry ends the
	-- run itself, once the lines the script logged before exiting have left:
	-- LogService delivers a frame's lines after its Heartbeat handlers, so
	-- that is one Heartbeat and the resumption after it.
	local function exit()
		RunService.Heartbeat:Wait()
		task.wait()
		if proc then
			proc.kill()
		end
		call(rodeoProto.PluginMessage.new({
			msg = { type = "killed", value = rodeoProto.ExecutionKilled.new({ executionId = run.executionId }) },
		}))
		over = true
		finished:Fire()
	end

	proc = runner.start(run, {
		send = function(message, respond)
			local reply = call(message)
			if respond and reply and reply.msg and reply.msg.type == "rpcResponse" then
				respond(reply.msg.value.res)
			end
			local sent = message.msg
			local req = if sent and sent.type == "rpc" then sent.value.req else nil
			if req and req.type == "processExit" and not exiting then
				exiting = true
				task.spawn(exit)
			end
		end,
		onComplete = function()
			over = true
			finished:Fire()
		end,
	})
	if proc and not over then
		finished.Event:Wait()
	end
	finished:Destroy()

	-- ree ends its run at the frame a script prints exactly this line. The
	-- run's log capture is disconnected by now, so it stays out of the run's
	-- output.
	print("@done")
end
