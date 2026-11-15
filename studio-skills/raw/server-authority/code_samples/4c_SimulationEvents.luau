-- !strict
-- Simulation Events Module Script
-- ReplicatedStorage/SimulationEvents (ModuleScript)
-- By decoupling the renderer from the simulation we gain greater power over the rendering, allowing us to hide network artifacts.
-- See #smoothing section for one example of smoothing out visual artifacts from sudden unpredicted movement.
-- Another technique is to have the simulation produce 'Simulation Events' that are then rendered client side.
-- Simulation Events are simply small chunks of server authoritative data that are predicted to have happened, and ultimately confirmed by server data or deleted.
-- The insight in this design is that SimulationEvents are just more State.
-- This system is a way you can procedurally write _more state_ into the simulation with an ergonomic API.
-- It can be used to capture data from a moment in the simulation so that it can be rendered effectively on the client.
-- An example is a rocket exploding. We want to render an explosion effect on the clients. We dont want to use server authority for the cosmetic effect of the explosion, the server doesnt need to know
-- where each chunk of visual debris is. The explosion _effect_ is just cosmetic, it doesnt affect the simulation. So instead of creating all the explosion pieces inside the simulation and using server authority to correct it,
-- we encapsulate just the right subset of data that the renderer needs to be able to show an explosion effect. We do this through the SimulationEvents module script.
local module = {}
local CollectionService = game:GetService("CollectionService")
local RunService = game:GetService("RunService")
local utils = require(game.ReplicatedStorage.Utilities.Utils)
local frameCount = 0
local clientInitialized = false
local eventsParent = utils.sharedInstance(game.Workspace, "Folder", "SimulationEvents")
assert(eventsParent ~= nil)
local eventDestroyTime = 3.0

type EventData = {[string]:any}
type EventDataMap = {
	[Instance]:EventData
}

module.RenderEventType = table.freeze({
	New = 1,
	Change = 2,
	Delete = 3,
})
export type RenderEventType = number

export type RenderEventArgs = {
	instance:Instance,
	attributes:any,
	attributesPrevious:any,
	attributesDiff:any,
	type:RenderEventType,
}

export type RenderEventCallback = (RenderEventArgs)->()

local renderingEvents:EventDataMap = {}
local callbackMap:{[string]:RenderEventCallback} = {}
local destroyedEvents:{Instance} = {}

function module.CreateEvent(tag:string, data:any)
	local event = Instance.new("Folder")
	assert(eventsParent ~= nil)
	event.Name = string.format("%s_%d", tag, math.round(time() * 60))
	event.Parent = eventsParent
	for key, value in data do
		event:SetAttribute(key, value)
	end
	
	event:SetAttribute("tag", tag)
	
	if RunService:IsServer() then
		task.delay(eventDestroyTime, function()
			event:Destroy()
		end)
	else
		task.defer(function()
			game.RunService:SetPredictionMode(event, Enum.PredictionMode.On)
		end)
	end
	return event
end

function module.RenderEvents(tag:string, callback:RenderEventCallback)
	assert(RunService:IsClient())
	if not clientInitialized then
		clientInitialized = true
		module.InitializeClient()
	end
	callbackMap[tag] = callback
end

function module.InitializeClient()
	assert(RunService:IsClient())
	
	local function tableDiff(a:any, b:any):any
		local diff:any = nil
		if b == nil then return a end
		for k, v in a do
			if b[k] ~= v then
				if diff == nil then
					diff = {}
				end
				diff[k] = v
			end
		end
		return diff
	end

	local function render(deltaTime:number)
		frameCount = frameCount + 1
		
		-- Look for new events
		for _, event:Instance in eventsParent:GetChildren() do
			if renderingEvents[event] == nil then
				local attributes = event:GetAttributes()
				renderingEvents[event] = attributes
				local tag:string = attributes["tag"]
				local callback:RenderEventCallback = callbackMap[tag]
				if callback ~= nil then
					callback({
						instance = event,
						attributes = attributes,
						attributesPrevious = nil,
						attributesDiff = attributes,
						type = module.RenderEventType.New
					})
				end
			end
		end
		
		-- Look for changed and deleted events
		for event:Instance, previousAttributes:EventData in renderingEvents do
			local attributes = event:GetAttributes()
			local tag = attributes["tag"]
			local callback:RenderEventCallback = callbackMap[tag]
			if callback == nil then
				continue
			end
			
			if event.Parent == nil then
				-- deleted event
				callback({
					instance = event,
					attributes = nil,
					attributesPrevious = previousAttributes,
					attributesDiff = {},
					type = module.RenderEventType.Delete
				})
				renderingEvents[event] = nil
			else
				local diff = tableDiff(attributes, previousAttributes)
				if diff ~= nil then
					-- changed event
					callback({
						instance = event,
						attributes = attributes,
						attributesPrevious = previousAttributes,
						attributesDiff = diff,
						type = module.RenderEventType.Change
					})
					renderingEvents[event] = attributes
				end
			end
		end
	end
	
	RunService:BindToRenderStep("renderEvents", Enum.RenderPriority.Character.Value, render)
end

return module
