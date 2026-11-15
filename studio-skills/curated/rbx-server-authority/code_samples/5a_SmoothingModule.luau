-- ReplicatedStorage.SmoothingModule
-- Can be used to improve the visual appearance of client side prediction
-- by smoothing out the position jumps associated with mispredictions.
-- ie: it turns sudden movements into smooth movements.
-- This can be used by placing SmoothMe Client Script under a Part and setting the Target attribute
-- on that script to the part to smoothly track.
-- The smoothing engages if the difference in position the rendered object and the target object
-- is between 'SmoothEngageMin' and 'SmoothEngageMax'
-- This means it will usually be disabled, and the parent object will be in the exact
-- same location as the target object.
-- Tune EngageMin, EngageMax, Time and Disengage to values correct for your game
-- (based on the speed and acceleration of the movement).
-- Smoothing can be disabled for one frame via 'DisableForNextFrame'
-- in order to not smooth intentional jumps like teleports or respawns.
-- Default values are provided.
local TweenService:TweenService = game:GetService("TweenService")

export type SmoothData = {
	velocity:Vector3,
	position_offset:Vector3,
	is_smoothing:boolean,
	params:SmoothParams,
	last_target:Vector3,
	target_delta:Vector3,
	projected_delta:Vector3,
}

export type SmoothParams = {
	DisableOneFrame:boolean,
	Always:boolean,
	Never:boolean,
	VisualizeDebug:boolean,
	EngageMin:number,
	EngageMax:number,
	ReEngageMin:number,
	ReEngageMax:number,
	Disengage:number,
	Time:number,
}
export type SmoothParameters = {
	DisableOneFrame:boolean?,
	Always:boolean?,
	Never:boolean?,
	VisualizeDebug:boolean?,
	EngageMin:number?,
	EngageMax:number?,
	ReEngageMin:number?,
	ReEngageMax:number?,
	Disengage:number?,
	Time:number?,
	Fancy:boolean?,
}

local smoothDefaults:SmoothParams = {
	DisableOneFrame = true,
	Always = false,
	Never = false,
	VisualizeDebug = false,
	EngageMin = 1.0,
	EngageMax = 30.0,
	ReEngageMin = 10.0,
	ReEngageMax = 30.0,
	Disengage = 0.05,
	Time = 0.1,
	Fancy = true,
}

local module = {}

function module.new(parameters:SmoothParameters?):SmoothData
	local s:SmoothData = {
		velocity = Vector3.zero,
		position = Vector3.zero,
		position_offset = Vector3.zero,
		is_smoothing = false,
		smooth_velocity = Vector3.zero,
		params = table.clone(smoothDefaults),
		last_target = Vector3.zero
	}
	if parameters ~= nil then
		for k,v in parameters do
			s.params[k] = v
		end
	end
	return s
end

local function projectVector(vectorA:Vector3, vectorB:Vector3):Vector3
	local dotProduct = vectorA:Dot(vectorB)
	local bMagnitudeSquared = vectorB.Magnitude^2
	if bMagnitudeSquared == 0 then
		return Vector3.new(0, 0, 0)
	end
	local projection = (dotProduct / bMagnitudeSquared) * vectorB
	return projection
end

function module.SmoothPosition(position:Vector3, targetPosition:Vector3, data:SmoothData, deltaTime:number):Vector3
	local position_delta = (position - targetPosition)
	local position_delta_magnitude = position_delta.Magnitude
	local target_delta = data.last_target - targetPosition
	if data.params.DisableOneFrame then
		data.is_smoothing = false
		data.params.DisableOneFrame = false
	elseif data.params.Always then
		data.is_smoothing = true
		data.position_offset = position_delta
	elseif data.params.Never then
		data.is_smoothing = false
	elseif not data.is_smoothing and position_delta.Magnitude >= data.params.EngageMin and position_delta.Magnitude < data.params.EngageMax then
		data.is_smoothing = true
		data.position_offset = position_delta
		data.last_target = targetPosition
	elseif data.is_smoothing then
		if target_delta.Magnitude >= data.params.ReEngageMin then
			data.is_smoothing = true
			print("REENGAGE", target_delta.Magnitude)
			data.position_offset = position_delta
			data.last_target = targetPosition
		end
	end
	target_delta = data.last_target - targetPosition
	
	if data.is_smoothing then
		if data.params.Fancy then
			if target_delta.Magnitude > 0.0 and target_delta:Dot(data.position_offset) > 0 then
				local projected_delta = projectVector(target_delta, data.position_offset.Unit)
				data.position_offset = data.position_offset - projected_delta
			end
		end
		
		data.position_offset, data.velocity = TweenService:SmoothDamp(data.position_offset, Vector3.zero, data.velocity, data.params.Time, 1000.0, deltaTime)
		position = targetPosition + data.position_offset		
		if not data.params.Always and data.position_offset.Magnitude < data.params.Disengage then
			--print("DISENGAGE", data.position_offset.Magnitude)
			data.is_smoothing = false
		end
	end

	if not data.is_smoothing then
		data.velocity = Vector3.zero
		data.position_offset = Vector3.zero
		position = targetPosition
	end
	data.last_target = targetPosition
	
	return position
end
return module
