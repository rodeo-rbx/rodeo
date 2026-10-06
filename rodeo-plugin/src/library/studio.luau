--[[
    How this works:

    Plugins will spawn for every VM
    We use studio MCP to execute luau to fire bindables + remotes to disperse the studio ID
]]

--!strict
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local StudioTestService = game:GetService("StudioTestService")

local constants = require(script.Parent.constants)
local dom_role = require(script.Parent.dom_role)

-- Static (never change for this VM's lifetime)
local isServer = RunService:IsServer()
local isClient = RunService:IsClient()
-- Role in the unifier exchange. Derived from DOM kind, not from the two
-- network flags above: a Team Create edit DOM reports IsClient() only, yet
-- must still host (see dom_role.luau).
local role = dom_role.derive({
	isEdit = RunService:IsEdit(),
	isServer = isServer,
	isClient = isClient,
})

-- RODEO_UNIFIER: cross-VM identity exchange
local RODEO_UNIFIER: Folder
local RODEO_UNIFIER_REMOTE: RemoteEvent
local RODEO_UNIFIER_BINDABLE: BindableEvent
local RODEO_UNIFIER_MODULE: ModuleScript

if role.hosts then
	RODEO_UNIFIER = ReplicatedStorage:FindFirstChild("RODEO_UNIFIER") :: Folder
	if not RODEO_UNIFIER then
		RODEO_UNIFIER = Instance.new("Folder")
		RODEO_UNIFIER.Name = "RODEO_UNIFIER"
		RODEO_UNIFIER.Parent = ReplicatedStorage
	end
	-- Rodeo scratch, not place content: never serialize it. In a Team Create
	-- session ReplicatedStorage is a synced, autosaved DataModel, and in plain
	-- edit this keeps `--save` output free of it. Play DataModels are clones
	-- of the edit DataModel and drop non-archivable instances, so the play
	-- server recreates its own (the FindFirstChild above then misses).
	RODEO_UNIFIER.Archivable = false

	RODEO_UNIFIER_REMOTE = RODEO_UNIFIER:FindFirstChild("RemoteEvent") :: RemoteEvent
	if not RODEO_UNIFIER_REMOTE then
		RODEO_UNIFIER_REMOTE = Instance.new("RemoteEvent")
		RODEO_UNIFIER_REMOTE.Name = "RemoteEvent"
		RODEO_UNIFIER_REMOTE.Parent = RODEO_UNIFIER
	end

	RODEO_UNIFIER_BINDABLE = RODEO_UNIFIER:FindFirstChild("BindableEvent") :: BindableEvent
	if not RODEO_UNIFIER_BINDABLE then
		RODEO_UNIFIER_BINDABLE = Instance.new("BindableEvent")
		RODEO_UNIFIER_BINDABLE.Name = "BindableEvent"
		RODEO_UNIFIER_BINDABLE.Parent = RODEO_UNIFIER
	end

	RODEO_UNIFIER_MODULE = RODEO_UNIFIER:FindFirstChild("ModuleScript") :: ModuleScript
	if not RODEO_UNIFIER_MODULE then
		RODEO_UNIFIER_MODULE = Instance.new("ModuleScript")
		RODEO_UNIFIER_MODULE.Source = "return game:GetService('HttpService'):GenerateGUID(false)"
		RODEO_UNIFIER_MODULE.Name = "ModuleScript"
		RODEO_UNIFIER_MODULE.Parent = RODEO_UNIFIER
	end
elseif role.joins then
	RODEO_UNIFIER = ReplicatedStorage:WaitForChild("RODEO_UNIFIER") :: Folder
	RODEO_UNIFIER_REMOTE = RODEO_UNIFIER:WaitForChild("RemoteEvent") :: RemoteEvent
	RODEO_UNIFIER_BINDABLE = RODEO_UNIFIER:WaitForChild("BindableEvent") :: BindableEvent
	RODEO_UNIFIER_MODULE = RODEO_UNIFIER:WaitForChild("ModuleScript") :: ModuleScript
end

-- The DOM id. The shared RODEO_UNIFIER ModuleScript's source is a GenerateGUID
-- call, and Luau caches module results per VM — so each DOM's require executes
-- it fresh and mints a distinct id. This yields exactly one id per DOM only
-- because rodeo has a single entry-point VM per DOM (the plugin in edit, the
-- runner in server/client). A second require site in another VM of the same
-- DOM would mint a second id and break the invariant.
local domId = require(RODEO_UNIFIER_MODULE)

-- Canonical studio id: one per Studio *process*, shared across all its DOMs.
-- The edit DOM ALWAYS mints a fresh id as a Workspace attribute — a persisted
-- attribute in the place file identifies a dead process (or another live one,
-- if the file is open twice), so it must never be trusted in edit. The play
-- server/client DataModels are clones of the edit DataModel (verified: a
-- runtime-set Workspace attribute survives the play-solo clone), so they
-- inherit and reuse it. This is the stable studio identity — independent of
-- the launch `session_guid` (only owned/launched studios have one) and of
-- StudioMCP's flaky id.
-- Keyed by this plugin's backend port: several rodeo plugins can share a
-- Studio, and each edit-DOM plugin mints its own id at load. One shared
-- attribute would hold whichever plugin wrote last, and the play clones would
-- inherit that — one Studio reporting two ids to one backend.
local RODEO_STUDIO_ID_ATTR = `rodeoStudioId_{constants.PORT}`
local studioId: string = ""
do
	local ws = game:GetService("Workspace")
	if RunService:IsEdit() then
		studioId = game:GetService("HttpService"):GenerateGUID(false)
		ws:SetAttribute(RODEO_STUDIO_ID_ATTR, studioId)
	else
		local existing = ws:GetAttribute(RODEO_STUDIO_ID_ATTR)
		if type(existing) == "string" and existing ~= "" then
			studioId = existing
		end
	end
end

-- Session kind ("test" | "play") of a session rodeo started: mode_driver.luau
-- and the multiplayer start in rodeo-client pass test args tagged with it, and
-- a play DataModel reads them back. Known at load, so mode_driver can end a
-- session of the wrong kind before (or without) any client's CanLeaveTest
-- verdict — a multiplayer test with no clients never gets one. nil for
-- sessions started some other way (Studio's own Play buttons) or if
-- GetTestArgs fails. The reported State.mode keeps waiting for the verdict:
-- a solo test's server reports "test" only once its player has joined, and
-- runs routed to it rely on that.
local taggedSessionKind: ("test" | "play")? = nil
if not RunService:IsEdit() then
	local ok, args = pcall(function()
		return StudioTestService:GetTestArgs()
	end)
	if ok and type(args) == "table" then
		local kind = (args :: any).rodeoSession
		if kind == "test" or kind == "play" then
			taggedSessionKind = kind
		end
	end
end

-- Inputs: mutable values updated by events. The Heartbeat reads these to compute state.
-- `*mcpStudioId*` values are StudioMCP's own ids (propagated via the RODEO_UNIFIER
-- event channels by rodeo-cli's reconciliation loop). They are NOT the canonical
-- master-assigned studio_id — that's tracked server-side.
local inputs = {
	-- From BindableEvent (MCP → edit/server VM)
	bindableMcpStudioId = nil :: string?,
	-- From RemoteEvent (cross-VM)
	serverDomId = nil :: string?,
	serverMcpStudioId = nil :: string?,
	clientDomIds = {} :: { [string]: Player },
	clientMcpStudioIds = {} :: { [string]: Player },
	-- Session kind ("test" | "play") reported by a client via CanLeaveTest and
	-- propagated to the server, which can't call CanLeaveTest itself.
	sessionKind = nil :: ("test" | "play")?,
	-- Players
	players = {} :: { [Player]: boolean },
}

-- Current state (replaced immutably on change)
local currentState: State? = nil

export type State = {
	domId: string,
	-- Canonical studio identity — same value for every DOM of one Studio
	-- process (minted by the edit DOM, inherited by play children via clone).
	studioId: string,
	-- StudioMCP's assigned id for this Studio (set via reconciliation). Used
	-- only for elevated-context routing (set_active_studio); NOT the identity.
	mcpStudioId: string?,

	isServer: boolean,
	isClient: boolean,
	isRunning: boolean,
	isEdit: boolean,

	mode: "edit" | "run" | "test" | "play",
	domKind: "edit" | "server" | "client",

	gameName: string,
	placeId: number,
	gameId: number,

	clientInfo: {
		name: string,
		userId: number,
	}?,

	clientDomIds: { [string]: {
		name: string,
		userId: number,
	} }?,
}

-- Compute a fresh state from current inputs + RunService
local function computeState(): State
	local isRunning = RunService:IsRunning()
	local isEdit = RunService:IsEdit()
	local isRunMode = RunService:IsRunMode()

	local clientMcpStudioId = if role.joins then inputs.bindableMcpStudioId else next(inputs.clientMcpStudioIds)
	local serverMcpStudioId = inputs.serverMcpStudioId or clientMcpStudioId or inputs.bindableMcpStudioId

	-- test vs play (running, non-run-mode) comes from `inputs.sessionKind`, set
	-- from StudioTestService:CanLeaveTest() — the client computes it once running
	-- and both stores it locally and propagates it to the server (which can't
	-- call CanLeaveTest itself). Defaults to "play" until set — a DOM with no
	-- verdict yet can't be a solo test.
	local sessionKind: "test" | "play" = inputs.sessionKind or "play"

	local mode: "edit" | "run" | "test" | "play" = if isEdit then "edit" elseif isRunMode then "run" else sessionKind

	local clientDomIds = nil
	if role.serves then
		local ids = {}
		for id, player in inputs.clientDomIds do
			ids[id] = {
				name = player.Name,
				userId = player.UserId,
			}
		end
		clientDomIds = ids
	end

	local domKind: "edit" | "server" | "client" = role.domKind

	return {
		domId = domId,
		studioId = studioId,
		mcpStudioId = if role.joins then clientMcpStudioId else serverMcpStudioId,

		isServer = isServer,
		isClient = isClient,
		isRunning = isRunning,
		isEdit = isEdit,

		mode = mode,
		domKind = domKind,

		gameName = game.Name,
		placeId = game.PlaceId,
		gameId = game.GameId,

		clientInfo = if role.joins and Players.LocalPlayer
			then {
				name = Players.LocalPlayer.Name,
				userId = Players.LocalPlayer.UserId,
			}
			else nil,

		clientDomIds = clientDomIds,
	}
end

local function deepEqual(a: any, b: any): boolean
	if type(a) ~= type(b) then
		return false
	end
	if type(a) ~= "table" then
		return a == b
	end
	for k, v in a do
		if not deepEqual(v, (b :: any)[k]) then
			return false
		end
	end
	for k in b do
		if (a :: any)[k] == nil then
			return false
		end
	end
	return true
end

local function stateChanged(a: State?, b: State): boolean
	if a == nil then
		return true
	end
	return not deepEqual(a, b)
end

-- Listeners
local stateListeners: { [number]: () -> () } = {}
local nextListenerId = 1

local function listen(fn: () -> ()): () -> ()
	local id = nextListenerId
	stateListeners[id] = fn
	nextListenerId += 1
	return function()
		stateListeners[id] = nil
	end
end

local function notifyListeners()
	for _, fn in stateListeners do
		fn()
	end
end

-- Heartbeat: compute fresh state, diff, swap + notify
RunService.Heartbeat:Connect(function()
	local newState = computeState()
	if stateChanged(currentState, newState) then
		currentState = newState
		notifyListeners()
	end
end)

-- Event handlers: just update inputs, Heartbeat picks them up

if role.serves then
	RODEO_UNIFIER_REMOTE.OnServerEvent:Connect(function(player, action, id)
		if action == "register_client_dom_id" then
			inputs.clientDomIds[id] = player
		elseif action == "studio_id_from_client" then
			-- "id" is the client's mcp_studio_id (from reconciliation)
			inputs.clientMcpStudioIds[id] = player
		elseif action == "session_kind" then
			-- "id" is "test" | "play" — the client's CanLeaveTest verdict
			inputs.sessionKind = id
		end
	end)

	local function playerAdded(player: Player)
		inputs.players[player] = true
		RODEO_UNIFIER_REMOTE:FireClient(player, "register_server_dom_id", domId)
	end

	for _, player in Players:GetPlayers() do
		playerAdded(player)
	end
	Players.PlayerAdded:Connect(playerAdded)
	Players.PlayerRemoving:Connect(function(player)
		inputs.players[player] = nil
	end)
end

if role.joins then
	RODEO_UNIFIER_REMOTE.OnClientEvent:Connect(function(action, id)
		if action == "register_server_dom_id" then
			inputs.serverDomId = id
		elseif action == "studio_id_from_server" then
			-- "id" is the server's mcp_studio_id (from reconciliation)
			inputs.serverMcpStudioId = id
		end
	end)
	RODEO_UNIFIER_REMOTE:FireServer("register_client_dom_id", domId)

	local ok, canLeave = pcall(function()
		return StudioTestService:CanLeaveTest()
	end)
	local kind = if ok and not canLeave then "test" else "play"
	inputs.sessionKind = kind
	RODEO_UNIFIER_REMOTE:FireServer("session_kind", kind)
end

RODEO_UNIFIER_BINDABLE.Event:Connect(function(id)
	-- "id" is this VM's mcp_studio_id (from reconciliation's unify_code)
	inputs.bindableMcpStudioId = id
end)

return {
	domId = domId,
	listen = listen,
	getState = function(): State?
		return currentState
	end,
	-- This play DataModel's session kind for mode_driver: the start tag, else
	-- the client verdict, else nil while unknown — unlike State.mode, which
	-- falls back to "play". mode_driver must not end a session on that guess.
	getSessionKind = function(): ("test" | "play")?
		return taggedSessionKind or inputs.sessionKind
	end,
}
