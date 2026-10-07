--[[
	AppContext - Central state management for the Rodeo plugin

	This module provides:
	- Context creation and types
	- AppProvider component with all state logic
	- useAppContext hook for consuming the context

	All state flows through React context to child components.
]]

local React = require("@pkg/react")
local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")
local MessageHandler = require(script.Parent.MessageHandler)
local rodeoProto = require(script.Parent.Parent.Parent.Parent.proto.rodeo)
local constants = require(script.Parent.Parent.Parent.library.constants)
local reconnect = require(script.Parent.Parent.Parent.library.reconnect)
local studio = require(script.Parent.Parent.Parent.library.studio)

local e = React.createElement
local CONTEXT_BITS = constants.CONTEXT_BITS
local COLORS = constants.COLORS

local AppContext = React.createContext(nil)

-- Types
export type Settings = {
	verbose: boolean,
}

export type ProcessEntry = MessageHandler.ProcessEntry

export type AppContextValue = {
	-- Connection state
	connected: boolean,
	isExecuting: boolean,
	status: string,

	-- Runtime context
	contextBitset: number,

	-- Active processes
	activeProcesses: { [string]: ProcessEntry },

	-- Settings
	settings: Settings,

	-- Setters
	setVerbose: (verbose: boolean) -> (),
}

export type AppProviderProps = {
	plugin: Plugin,
	initialSettings: {
		port: number,
	},
	-- Called once, on the first successful connection to the backend.
	onConnected: (() -> ())?,
	children: any,
}

-- Helper function to compute runtime context bitset
local function getContextBitset(): number
	local bitset = 0

	if RunService:IsStudio() then
		bitset = bit32.bor(bitset, bit32.lshift(1, CONTEXT_BITS.isStudio))
	end
	if RunService:IsServer() then
		bitset = bit32.bor(bitset, bit32.lshift(1, CONTEXT_BITS.isServer))
	end
	if RunService:IsClient() then
		bitset = bit32.bor(bitset, bit32.lshift(1, CONTEXT_BITS.isClient))
	end
	if RunService:IsEdit() then
		bitset = bit32.bor(bitset, bit32.lshift(1, CONTEXT_BITS.isEdit))
	end
	if RunService:IsRunning() then
		bitset = bit32.bor(bitset, bit32.lshift(1, CONTEXT_BITS.isRunning))
	end

	return bitset
end

-- Hook for consuming the context
local function useAppContext(): AppContextValue
	local context = React.useContext(AppContext)
	if not context then
		error("useAppContext must be used within AppProvider")
	end
	return context
end

-- Provider component
local function AppProvider(props: AppProviderProps)
	-- This plugin's own backend: the port the installing backend baked into
	-- the plugin file (constants.PORT). The plugin never dials any other port.
	local host = constants.DEFAULT_HOST
	local myPort: number = props.initialSettings.port or constants.PORT

	local verbose, setVerbose = React.useState(constants.VERBOSE)

	-- WebSocket state
	local wsUrl, setWsUrl = React.useState(nil :: string?)
	local connected, setConnected = React.useState(false)
	-- Backend build id from the welcome message; nil until received.
	local serverVersion, setServerVersion = React.useState(nil :: string?)
	-- Mirrors serverVersion for the socket callbacks, whose closures predate
	-- the welcome. Set through noteServerVersion below.
	local serverVersionRef = React.useRef(nil :: string?)
	local function noteServerVersion(version: string?)
		serverVersionRef.current = version
		setServerVersion(version)
	end
	-- Seconds to wait before the next dial, set when an attempt fails or a
	-- connection drops, and read by the auto-connect effect. nil = first
	-- attempt.
	local retryDelayRef = React.useRef(nil :: number?)
	-- Consecutive failed attempts, for the back-off (reconnect.retryDelay).
	-- Reset when a socket opens.
	local failuresRef = React.useRef(0)
	-- Whether this DOM's /health probe can be believed (see reconnect.luau).
	-- A play client cannot make HTTP requests, so it never probes.
	local probeTrustRef = React.useRef(
		(if RunService:IsEdit() or RunService:IsServer() then "unknown" else "unusable") :: reconnect.ProbeTrust
	)
	local announcedRef = React.useRef(false)

	-- Execution state
	local isExecuting, setIsExecuting = React.useState(false)
	local activeProcesses, setActiveProcesses = React.useState({} :: { [string]: ProcessEntry })

	-- Derive status from connected + isExecuting (with rich text)
	local status = React.useMemo(function()
		if isExecuting then
			local hex = COLORS.executing:ToHex()
			return `<font color="#{hex}">Executing</font>`
		elseif connected and serverVersion ~= nil and serverVersion ~= constants.BUILD_ID then
			-- The backend refuses a mismatched plugin (see plugin_ws.rs), so
			-- this is what the user sees between the welcome and the close.
			local hex = COLORS.error:ToHex()
			local server = if serverVersion == "" then "older build" else `v{serverVersion}`
			return `<font color="#{hex}">Version mismatch (server {server})</font>`
		elseif connected then
			local hex = COLORS.success:ToHex()
			return `<font color="#{hex}">Connected</font>`
		else
			local hex = COLORS.error:ToHex()
			return `<font color="#{hex}">Disconnected</font>`
		end
	end, { isExecuting, connected, serverVersion }) :: string

	-- Store client in state so message handler effect can access it
	local client, setClient = React.useState(nil :: any)

	-- One connection attempt per wsUrl: probe the backend (edit and play-server
	-- DOMs, see reconnect.luau), open the WebSocket, and hold it until it
	-- closes. Clearing wsUrl ends the attempt; the auto-connect effect below
	-- starts the next one after the back-off.
	React.useEffect(function()
		if not wsUrl then
			setConnected(false)
			setClient(nil)
			setActiveProcesses({})
			return
		end

		-- Everything this attempt started can still report after the attempt
		-- is over: a Closed after the Error that already ended it, a probe
		-- answering after its timeout, any socket event after the cleanup
		-- below. `live` turns false at that point and they all do nothing, so a
		-- stale callback never tears down or re-times the attempt after it.
		local live = true
		local opened = false
		local probeResult: reconnect.ProbeResult? = nil
		local newClient: any = nil
		local disconnectStudioStateListener

		-- Ends the attempt and schedules the next. Error, Closed and the open
		-- deadline may all fire for one socket; only the first counts.
		local function fail()
			if not live then
				return
			end
			live = false
			failuresRef.current += 1
			-- A backend of another build closes (pre-1.5) or parks (1.5+) the
			-- socket after its welcome; that wait is longer (see reconnect.luau).
			local seen = serverVersionRef.current
			retryDelayRef.current =
				reconnect.retryDelay(failuresRef.current, seen ~= nil and seen ~= constants.BUILD_ID)
			setConnected(false)
			setWsUrl(nil)
			setClient(nil)
			setActiveProcesses({})
			-- Keep serverVersion: a mismatch closes or parks the socket and the
			-- status should stay readable while the plugin waits to retry.
		end

		local function openSocket()
			local success, created = pcall(function()
				return HttpService:CreateWebStreamClient(Enum.WebStreamClientType.WebSocket, {
					Url = wsUrl,
				})
			end)
			if not success then
				-- Once per outage, not once per retry.
				if failuresRef.current == 0 then
					warn("[rodeo] Error creating client:", created)
				end
				fail()
				return
			end
			newClient = created

			newClient.Opened:Connect(function()
				if not live then
					return
				end
				opened = true
				failuresRef.current = 0
				probeTrustRef.current = reconnect.afterOpen(probeTrustRef.current, probeResult)
				setConnected(true)
				if not announcedRef.current then
					announcedRef.current = true
					if props.onConnected then
						task.spawn(props.onConnected)
					end
				end

				-- Send state on connect and on every change
				local function sendState()
					local state = studio.getState()
					if not state then return end
					local msg = rodeoProto.PluginMessage.new({
						msg = { type = "studioState", value = rodeoProto.StudioStateMsg.new({
							domId = state.domId,
							studioId = state.studioId,
							mcpStudioId = state.mcpStudioId,
							isServer = state.isServer,
							isClient = state.isClient,
							isRunning = state.isRunning,
							isEdit = state.isEdit,
							mode = state.mode,
							domKind = state.domKind,
							pluginVersion = constants.BUILD_ID,
							gameName = state.gameName,
							placeId = state.placeId,
							gameId = state.gameId,
							clientInfo = state.clientInfo and rodeoProto.ClientInfo.new({
								name = state.clientInfo.name,
								userId = state.clientInfo.userId,
							}) or nil,
							-- Rodeo session identity, written by the launch's RunScript
							-- bootstrap as the `rodeoSession` Workspace attribute. Read at
							-- send time (always after connect, so the bootstrap has run);
							-- nil for a manual Studio. Master matches it to its
							-- StudioInstanceInfo and stamps DomConnection.session_guid on the
							-- first message.
							sessionGuid = game:GetService("Workspace"):GetAttribute("rodeoSession"),
						}) },
					})
					newClient:Send(HttpService:JSONEncode(msg:jsonEncode()))
				end
				sendState()
				disconnectStudioStateListener = studio.listen(sendState)
			end)

			newClient.Error:Connect(function(_code: string, _message: string)
				fail()
			end)

			newClient.Closed:Connect(function()
				fail()
			end)

			task.delay(reconnect.OPEN_TIMEOUT, function()
				if not opened then
					fail()
				end
			end)

			setClient(newClient)
		end

		if probeTrustRef.current == "unusable" then
			openSocket()
		else
			-- Both the request and its timeout settle the probe; whichever is
			-- first decides, and only while the attempt is still live.
			local function settle(result: reconnect.ProbeResult)
				if probeResult ~= nil or not live then
					return
				end
				probeResult = result
				local trust, dial = reconnect.afterProbe(probeTrustRef.current, result)
				probeTrustRef.current = trust
				if dial then
					openSocket()
				else
					fail()
				end
			end
			task.spawn(function()
				local ok = pcall(function()
					return HttpService:RequestAsync({
						Url = `http://{host}:{myPort}/health`,
						Method = "GET",
					})
				end)
				settle(if ok then "answered" else "refused")
			end)
			task.delay(reconnect.PROBE_TIMEOUT, settle, "timeout")
		end

		return function()
			live = false
			if disconnectStudioStateListener then
				disconnectStudioStateListener()
			end
			if newClient then
				newClient:Close()
			end
		end
	end, { wsUrl })

	-- Message handler effect (can depend on verbose without closing socket)
	React.useEffect(function()
		if not client then
			return
		end

		local cleanup = MessageHandler.createHandler({
			client = client,
			verbose = verbose,
			setIsExecuting = setIsExecuting,
			setActiveProcesses = setActiveProcesses,
			setServerVersion = noteServerVersion,
		})

		return cleanup
	end, { client, verbose })

	-- Verbose toggle handler
	local function handleVerboseToggle(newVerbose: boolean)
		setVerbose(newVerbose)
	end

	-- Auto-connect: always on. Dials this plugin's own backend and no other,
	-- behind two gates:
	--  * Ownership. A rodeo launch's RunScript bootstrap stamps the launching
	--    backend's port as the `rodeoPort` Workspace attribute. If it names a
	--    different port, this Studio belongs to another backend and this
	--    plugin stays dormant here: no socket, no UI. No attribute means a
	--    hand-opened Studio, which every running backend's plugin connects to.
	--    Before the bootstrap lands every installed plugin sees no attribute,
	--    so the check runs at dial time and again whenever the attribute
	--    changes.
	--  * Back-off, set when the last attempt failed or the connection dropped
	--    (retryDelayRef; see reconnect.luau).
	React.useEffect(function()
		-- Skip if already connected or a connection is in progress.
		if connected or wsUrl then
			return
		end

		local ws = game:GetService("Workspace")
		local shouldReconnect = true
		local function doConnect()
			if not shouldReconnect then
				return
			end
			local launchPort = ws:GetAttribute("rodeoPort")
			if launchPort ~= nil and launchPort ~= myPort then
				return
			end
			setWsUrl(`ws://{host}:{myPort}`)
		end

		-- Only the edit DOM races the RunScript bootstrap that stamps the launch
		-- attributes — `rodeoSession` (which launch/run this studio belongs to,
		-- sent on the WS handshake) and `rodeoPort` — worst case ~200ms after the
		-- plugin mounts, so its first attempt waits 1 s. Server/client DOMs are
		-- clones of the edit DataModel and inherit both, so they dial at once —
		-- one plugin connects per DOM, and this is on each DOM's launch→connect
		-- critical path. Retries use the back-off the last failure chose. It is
		-- read, not consumed: should this effect run again before the dial, it
		-- must not fall back to the first attempt's immediate dial.
		local delay = retryDelayRef.current or (if RunService:IsEdit() then 1 else 0)
		if delay > 0 then
			task.delay(delay, doConnect)
		else
			doConnect()
		end
		local attrConn = ws:GetAttributeChangedSignal("rodeoPort"):Connect(doConnect)

		return function()
			shouldReconnect = false
			attrConn:Disconnect()
		end
	end, { connected, wsUrl, myPort })

	-- Build context value (memoized)
	local contextValue = React.useMemo(function()
		return {
			-- Connection state
			connected = connected,
			isExecuting = isExecuting,
			status = status,

			-- Runtime context
			contextBitset = getContextBitset(),

			-- Active processes
			activeProcesses = activeProcesses,

			-- Settings (verbose only; connection is automatic)
			settings = {
				verbose = verbose,
			},

			-- Setters
			setVerbose = handleVerboseToggle,
		}
	end, {
		connected,
		isExecuting,
		status,
		activeProcesses,
		verbose,
		handleVerboseToggle,
	})

	return e(AppContext.Provider, {
		value = contextValue,
	}, props.children)
end

return {
	AppContext = AppContext,
	AppProvider = AppProvider,
	useAppContext = useAppContext,
}
