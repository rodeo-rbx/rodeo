-- @rodeo/roblox shim. import/export come from the factory-built singleton
-- (transport tier); captureViewport is implemented here in the shim (composition
-- tier) — it only needs the public fs/stream/process API plus engine services
-- and the singleton's internal capture plumbing, so it rides the inlined
-- bundle and works over any backend the singleton fronts.
--
-- NOTE: this file is inlined into user scripts as an IIFE with all comments
-- stripped (see bundle.rs inline_shims). Do not put `--` inside string
-- literals.
local plugins, key = _G.__rodeo_plugins, script and script:GetAttribute("rodeoPlugin")
local ns = plugins and key and plugins[key]
local g = if ns then ns.procs[script:GetAttribute("rodeoExecutionId")] else _G.__rodeo_procs[_G.__rodeo_eid]
local process = g.process
local roblox = g.roblox

type CaptureOptions = {
	cframe: CFrame?,
	fov: number?,
	focus: CFrame?,
	settle: number?,
	device: string?,
	viewportSize: Vector2?,
	resample: boolean?,
	scalingMode: string?,
	pixelDensity: number?,
	orientation: string?,
	deviceForm: string?,
}

type CaptureInfo = {
	width: number,
	height: number,
}


-- Capture a Studio screenshot and write it to a stable path. `output` ending
-- in .png is the exact file path; any other value is a directory the
-- auto-named .png lands in; nil defaults to .rodeo/.temp/captures (next to
-- the profiles and logs rodeo already keeps under .rodeo/.temp, which is the
-- directory projects gitignore, rather than a new top-level directory).
--
-- The frame comes from the file the engine writes for every capture into
-- its per-user capture directory, never from an EditableImage: promoting the
-- capture's temporary texture is refused in play DOMs and capped at 8192
-- pixels a side everywhere, which on a 2x display rules out anything past a
-- 4096 viewport, and it means reading the whole RGBA frame through Luau. The
-- run client snapshots that directory before the capture and accepts exactly
-- one complete PNG that appears afterwards at the viewport's size, erroring
-- on ambiguity rather than guessing. It then verifies the frame against the
-- viewport, refuses an all-black frame (a frame Studio did not render),
-- resamples to exactly the viewport (the engine renders at the display
-- scale, 2x on Retina) and writes the PNG.
--
-- `device` / `viewportSize` drive Studio's device simulator for the shot via
-- plugin-handled RPCs (the service is PluginSecurity; the plugin acts in this
-- DOM at its own identity). Camera and simulator state are restored on every
-- exit path. Returns the absolute output path and the image size.
-- An error raised inside the plugin reaches the shim as "RPC error [<method>]:
-- <message>"; the message is the engine's own and is what the caller should see.
local function engineMessage(err: unknown): string
	return (tostring(err):gsub("^RPC error %[[%w_]+%]: ", ""))
end

local function capture(output: string?, options: CaptureOptions?): (string, CaptureInfo)
	local resolved: CaptureOptions = options or {}

	local viewportSize = resolved.viewportSize
	if viewportSize ~= nil then
		if typeof(viewportSize) ~= "Vector2" then
			error("roblox.captureViewport: viewportSize must be a Vector2", 2)
		end
		local w, h = math.floor(viewportSize.X + 0.5), math.floor(viewportSize.Y + 0.5)
		if w < 1 or h < 1 then
			error(`roblox.captureViewport: viewportSize must be at least 1x1 (got {w}x{h})`, 2)
		end
		viewportSize = Vector2.new(w, h)
	end
	if resolved.device ~= nil and type(resolved.device) ~= "string" then
		error("roblox.captureViewport: device must be a simulator preset id string", 2)
	end
	if resolved.resample ~= nil and type(resolved.resample) ~= "boolean" then
		error("roblox.captureViewport: resample must be a boolean", 2)
	end
	-- Simulator overrides, applied for the shot and restored after like the
	-- camera fields. They need a device to act on, so device or viewportSize
	-- must come with them; the plugin validates the names and the engine's
	-- own limits (density 72..10000, orientation on mobile forms) propagate.
	for _, name in { "scalingMode", "orientation", "deviceForm" } do
		local value = (resolved :: any)[name]
		if value ~= nil and type(value) ~= "string" then
			error(`roblox.captureViewport: {name} must be an enum item name string`, 2)
		end
	end
	if resolved.pixelDensity ~= nil and type(resolved.pixelDensity) ~= "number" then
		error("roblox.captureViewport: pixelDensity must be a number (DPI)", 2)
	end
	local simulatorOptions = resolved.scalingMode ~= nil or resolved.pixelDensity ~= nil or resolved.orientation ~= nil or resolved.deviceForm ~= nil
	if simulatorOptions and resolved.device == nil and viewportSize == nil then
		error("roblox.captureViewport: scalingMode, pixelDensity, orientation and deviceForm drive the device simulator; pass device or viewportSize with them", 2)
	end
	if resolved.scalingMode == "FitToWindow" then
		error("roblox.captureViewport: FitToWindow renders the device at whatever size fits the Studio window, not at its resolution, so there is no frame to capture; use ActualResolution or ScaleToPhysicalSize", 2)
	end
	-- Default: the image is exactly the viewport, so UI offsets map 1:1 onto
	-- pixels. `resample = false` keeps the engine's frame at its rendered size
	-- (the viewport times the display scale, or whatever a caller-driven
	-- simulator produced) for the largest image the engine will give.
	local keepFrame = resolved.resample == false
	if output then
		output = output:gsub("\\", "/")
	end

	local camera = workspace.CurrentCamera
	if not camera then
		error("roblox.captureViewport: no viewport camera in this context (use plugin context, or client context in a running session)", 2)
	end

	local CaptureService = game:GetService("CaptureService")
	local HttpService = game:GetService("HttpService")
	local RunService = game:GetService("RunService")
	local platform = process.platform()

	local savedType = camera.CameraType
	local savedCFrame = camera.CFrame
	local savedFocus = camera.Focus
	local savedFov = camera.FieldOfView
	local connection: RBXScriptConnection? = nil
	local simulatorApplied = false
	-- With the simulator driving the shot, the frame the engine renders is the
	-- device's full resolution (rotated in portrait), which for a preset with
	-- safe-area insets is larger than Camera.ViewportSize; the plugin reports it.
	local frameSize: Vector2? = nil

	local function captureAndWrite(): (string, CaptureInfo)
		if resolved.device or viewportSize then
			-- Limits (width 7680, height 4320, unknown preset) are the engine's:
			-- its message propagates as this call's error, minus the rpc
			-- layer's prefix. The plugin has already put the simulator back.
			simulatorApplied = true
			local okSim, frameW, frameH = pcall(
				roblox._simulatorApply,
				resolved.device,
				viewportSize and viewportSize.X,
				viewportSize and viewportSize.Y,
				resolved.scalingMode,
				resolved.pixelDensity,
				resolved.orientation,
				resolved.deviceForm
			)
			if not okSim then
				error(`roblox.captureViewport: {engineMessage(frameW)}`, 0)
			end
			frameSize = Vector2.new(frameW, frameH)
		end

		local cframe = resolved.cframe
		local fov = resolved.fov
		local focus = resolved.focus

		if cframe then
			camera.CameraType = Enum.CameraType.Scriptable
			camera.CFrame = cframe
		end
		if fov then
			camera.FieldOfView = fov
		end

		if cframe or fov or focus then
			connection = RunService.RenderStepped:Connect(function()
				if cframe then
					camera.CFrame = cframe
				end
				if focus then
					camera.Focus = focus
				elseif cframe then
					camera.Focus = CFrame.new(cframe.Position + cframe.LookVector * 100)
				end
				if fov then
					camera.FieldOfView = fov
				end
			end)
		end

		task.wait(resolved.settle or 0)

		-- The logical size the frame is rendered at; the finalize step verifies
		-- the frame against it and resamples to exactly this size. Without the
		-- simulator that is the window's viewport.
		local viewport = frameSize or camera.ViewportSize
		local width, height = math.floor(viewport.X + 0.5), math.floor(viewport.Y + 0.5)

		local outputPath
		if output and output:lower():sub(-4) == ".png" then
			outputPath = output
		else
			local directory = output or ".rodeo/.temp/captures"
			if directory:sub(-1) == "/" then
				directory = directory:sub(1, -2)
			end
			outputPath = directory .. "/capture-" .. HttpService:GenerateGUID(false):lower() .. ".png"
		end
		if outputPath:sub(1, 1) ~= "/" and not outputPath:match("^%a:") then
			local cwd = (process.cwd():gsub("\\", "/"))
			if cwd:sub(-1) == "/" then
				cwd = cwd:sub(1, -2)
			end
			outputPath = cwd .. "/" .. outputPath
		end

		-- The frame comes from the engine's capture file (see the header): the
		-- run client snapshots the capture directory now, so the file that
		-- appears after the callback is unmistakably this capture's.
		local captureToken = roblox._captureBegin()

		-- The wait for the callback has no deadline: a large frame or a slow
		-- GPU can take well over 10s (the simulator's largest frame takes about
		-- 7s here), and a timer only invented failures. The engine raises its
		-- own errors for what cannot work (limits, the server DOM), and those
		-- propagate. The one case that never completes, a minimized Studio on
		-- Windows, is documented; killing the run abandons it.
		local contentId: string? = nil
		local okShot, shotErr = pcall(function()
			CaptureService:CaptureScreenshot(function(id: string)
				contentId = id
			end)
		end)
		if not okShot then
			error(`roblox.captureViewport: {engineMessage(shotErr)}`, 0)
		end
		while not contentId do
			task.wait()
		end

		local outWidth, outHeight = roblox._captureCollect(captureToken, outputPath, width, height, keepFrame)
		return outputPath, { width = outWidth, height = outHeight }
	end

	local ok, result, info = pcall(captureAndWrite)
	if simulatorApplied then
		pcall(roblox._simulatorRestore)
	end
	if connection then
		connection:Disconnect()
	end
	camera.CameraType = savedType
	camera.CFrame = savedCFrame
	camera.Focus = savedFocus
	camera.FieldOfView = savedFov

	if not ok then
		error(result, 0)
	end
	return result, info
end

roblox.captureViewport = capture

-- Deprecated name; warns once per run, then forwards.
local warnedCapture = false
roblox.capture = function(output: string?, options: CaptureOptions?): (string, CaptureInfo)
	if not warnedCapture then
		warnedCapture = true
		warn("[rodeo] roblox.capture is deprecated and will be removed in 2.0; use roblox.captureViewport")
	end
	return capture(output, options)
end

return roblox
