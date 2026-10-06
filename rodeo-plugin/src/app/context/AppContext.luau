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
	-- Seconds to wait before the next dial, set by the close/error handlers
	-- and consumed by the auto-connect effect. nil = first attempt.
	local retryDelayRef = React.useRef(nil :: number?)
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
			-- A backend older than that check keeps the socket, and this
			-- plugin refuses its runs instead (MessageHandler).
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

	-- WebSocket client creation effect
	React.useEffect(function()
		if not wsUrl then
			setConnected(false)
			setClient(nil)
			setActiveProcesses({})
			return
		end

		local success, newClient = xpcall(function()
			return HttpService:CreateWebStreamClient(Enum.WebStreamClientType.WebSocket, {
				Url = wsUrl,
			})
		end, function(err)
			warn("[rodeo] Error creating client:", err)
			setConnected(false)
		end)

		if not success then
			setWsUrl(nil)
			return
		end

		local disconnectStudioStateListener

		newClient.Opened:Connect(function()
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

		-- Back-off before the next dial. A backend of another build closes the
		-- socket (pre-1.5) or parks it (1.5+); redialing every second at a
		-- backend that will never take this plugin is pure churn, so wait 30 s.
		-- Otherwise: 1 s in the edit DOM (as before); 5 s in server/client DOMs,
		-- which used to redial with no delay at all.
		local function scheduleRetry()
			local seen = serverVersionRef.current
			local mismatch = seen ~= nil and seen ~= constants.BUILD_ID
			retryDelayRef.current = if mismatch then 30 elseif RunService:IsEdit() then 1 else 5
		end

		newClient.Error:Connect(function(code: string, message: string)
			-- warn("[rodeo] Socket error:", code, message)
			scheduleRetry()
			setWsUrl(nil)
		end)

		newClient.Closed:Connect(function()
			scheduleRetry()
			setConnected(false)
			setWsUrl(nil)
			setClient(nil)
			setActiveProcesses({})
			-- Keep serverVersion: a mismatch closes or parks the socket and the
			-- status should stay readable while the plugin waits to retry.
		end)

		setClient(newClient)

		return function()
			if disconnectStudioStateListener then
				disconnectStudioStateListener()
			end
			newClient:Close()
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
			getServerVersion = function()
				return serverVersionRef.current
			end,
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
	--  * Back-off, set by the close/error handlers (retryDelayRef).
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
		-- critical path. Retries use the back-off the last close chose.
		local delay = retryDelayRef.current or (if RunService:IsEdit() then 1 else 0)
		retryDelayRef.current = nil
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
