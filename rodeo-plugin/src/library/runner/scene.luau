-- Scene assembly is separate from mesh/image codecs. All objects created by
-- import are caller-owned; failures destroy the entire partial result.
return function(roblox, rpc, fs, stream, runtime, meshToBlob, meshFromBlob, motion, primitives, nativeMotion)
	local AssetService = game:GetService("AssetService")
	local HttpService = game:GetService("HttpService")
	local mapProperties = {
		color = "ColorMapContent",
		normal = "NormalMapContent",
		roughness = "RoughnessMapContent",
		metalness = "MetalnessMapContent",
	}
	-- Scalar factors survive export even when Roblox previews an untextured
	-- material without allocating images or suppressing vertex colors.
	local materialPbr = {
		Plastic = { 0.55, 0 },
		SmoothPlastic = { 0.30, 0 },
		Glass = { 0.05, 0 },
		ForceField = { 0.10, 0 },
		Metal = { 0.35, 1 },
		DiamondPlate = { 0.45, 1 },
		CorrodedMetal = { 0.75, 1 },
		Foil = { 0.30, 1 },
		Wood = { 0.80, 0 },
		WoodPlanks = { 0.80, 0 },
		Fabric = { 0.90, 0 },
		Sand = { 0.95, 0 },
		Concrete = { 0.90, 0 },
		Brick = { 0.90, 0 },
		Cobblestone = { 0.90, 0 },
		Granite = { 0.75, 0 },
		Marble = { 0.40, 0 },
		Slate = { 0.85, 0 },
		Pebble = { 0.90, 0 },
		Rock = { 0.85, 0 },
		Basalt = { 0.85, 0 },
		Grass = { 0.95, 0 },
		LeafyGrass = { 0.95, 0 },
		Ground = { 0.95, 0 },
		Mud = { 0.90, 0 },
		Snow = { 0.85, 0 },
		Ice = { 0.15, 0 },
		Glacier = { 0.20, 0 },
		Salt = { 0.90, 0 },
		Limestone = { 0.85, 0 },
		Pavement = { 0.90, 0 },
		Cardboard = { 0.90, 0 },
		Carpet = { 0.95, 0 },
		Leather = { 0.60, 0 },
		Asphalt = { 0.90, 0 },
		Neon = { 0.70, 0 },
	}
	local function warnings(messages)
		local grouped, order = {}, {}
		for _, message in messages do
			local feature = message:match("^node %d+: (.*)") or message:match("^material [^:]+: (.*)") or message
			if not grouped[feature] then
				grouped[feature] = 0
				table.insert(order, feature)
			end
			grouped[feature] += 1
		end
		local result = {}
		for _, feature in order do
			local count = grouped[feature]
			table.insert(result, if count > 1 then `{feature} ({count} occurrences)` else feature)
		end
		return result
	end
	-- With Workspace.NextGenerationReplication on, a game server rejects
	-- string attributes over 50 characters (issue #32). Material names come
	-- from the source file and can be longer, so one is stored in pieces of at
	-- most 50 bytes that end on UTF-8 character boundaries: `name` holds the
	-- first, then `name2`, `name3`, ... Joint keys (a GUID and an index) fit.
	local ATTRIBUTE_STRING_LIMIT = 50
	local function setLongStringAttribute(instance, name, value)
		local piece, start = 1, 1
		repeat
			local stop = math.min(start + ATTRIBUTE_STRING_LIMIT, #value + 1)
			while stop <= #value and stop > start + 1 and bit32.band(string.byte(value, stop), 0xC0) == 0x80 do
				stop -= 1
			end
			instance:SetAttribute(if piece == 1 then name else `{name}{piece}`, string.sub(value, start, stop - 1))
			piece, start = piece + 1, stop
		until start > #value
	end
	local function getLongStringAttribute(instance, name)
		local value = instance:GetAttribute(name)
		local piece = 2
		while value ~= nil do
			local rest = instance:GetAttribute(`{name}{piece}`)
			if rest == nil then
				break
			end
			value ..= rest
			piece += 1
		end
		return value
	end
	local nextId = 0
	local function cframe(values)
		return CFrame.new(unpack(values))
	end
	local function components(cf)
		return { cf:GetComponents() }
	end
	local function checkPath(path)
		local lower = string.lower(path)
		assert(
			string.sub(lower, -4) == ".glb" or string.sub(lower, -5) == ".gltf",
			"editable scenes require .gltf or .glb"
		)
	end
	local function matrix(cf, scale)
		local p, x, y, z = cf.Position, cf.XVector * scale.X, cf.YVector * scale.Y, cf.ZVector * scale.Z
		return { { x.X, x.Y, x.Z, 0 }, { y.X, y.Y, y.Z, 0 }, { z.X, z.Y, z.Z, 0 }, { p.X, p.Y, p.Z, 1 } }
	end
	local function decompose(m)
		local x = Vector3.new(unpack(m[1]))
		local y = Vector3.new(unpack(m[2]))
		local z = Vector3.new(unpack(m[3]))
		local scale = Vector3.new(x.Magnitude, y.Magnitude, z.Magnitude)
		return CFrame.fromMatrix(Vector3.new(unpack(m[4])), x.Unit, y.Unit, z.Unit), scale
	end
	local function cleanup(objects)
		for i = #objects, 1, -1 do
			objects[i]:Destroy()
		end
	end
	local function listAt(map, key, value)
		if key ~= nil then
			local list = map[key] or {}
			map[key] = list
			table.insert(list, value)
		end
	end

	function roblox.importEditableScene(path, options)
		checkPath(path)
		if options then
			assert(options.animationRig == nil, "animationRig was removed; scene rigs are imported automatically")
			assert(options.animationSampleRate == nil, "animationSampleRate was removed; scene playback uses curves")
		end
		nextId += 1
		local handle = { __handle = `scene:{nextId}:{path}` }
		rpc.call("roblox_scene_decode", runtime.RobloxSceneDecodeRequest.new({ path = path, handle = handle.__handle }))
		local readOk, bytes = pcall(stream.readBytes, handle)
		stream.close(handle)
		if not readOk then
			error(bytes, 2)
		end
		local offset = 0
		local function chunk()
			local size = buffer.readu32(bytes, offset)
			offset += 4
			local result = buffer.create(size)
			buffer.copy(result, 0, bytes, offset, size)
			offset += size
			return result
		end
		local document = HttpService:JSONDecode(buffer.tostring(chunk()))
		if options and options.strict and #document.warnings > 0 then
			error(document.warnings[1], 2)
		end
		local scene = {
			roots = {},
			meshes = {},
			images = {},
			warnings = document.warnings,
			animations = {},
			morphs = {},
			morphWeights = {},
			sourceMap = { nodes = {}, primitives = {}, materials = {}, images = {}, joints = {} },
		}
		local owned = {}
		local function own(object)
			table.insert(owned, object)
			return object
		end
		local ok, result = pcall(function()
			local boneIds = {}
			for i, info in document.meshes do
				local mesh, ids, attributes = meshFromBlob(chunk(), path)
				scene.meshes[i], boneIds[i] = own(mesh), ids
				if #info.morph.targets > 0 or info.morph.tangents then
					table.insert(scene.morphs, motion.importMorph(mesh, attributes, info.morph))
				end
			end
			for i, info in document.images do
				local image = own(
					assert(
						AssetService:CreateEditableImage({ Size = Vector2.new(info.width, info.height) }),
						`image {i}: editable image memory exhausted`
					)
				)
				image:WritePixelsBuffer(Vector2.zero, image.Size, chunk())
				scene.images[i] = image
				listAt(scene.sourceMap.images, info.sourceIndex, image)
			end
			assert(offset == buffer.len(bytes), "scene packet contains trailing data")
			local nodes = {}
			local jointToken = HttpService:GenerateGUID(false)
			local jointSources = {}
			for _, info in document.meshes do
				for _, source in info.jointNodes do
					jointSources[source] = true
				end
			end
			for i, node in document.nodes do
				local nodeFrame = cframe(node.transform)
				local nodeInstance
				if node.className == "Attachment" then
					assert(#node.parts == 0, "Attachment node cannot carry geometry")
					nodeInstance = own(Instance.new("Attachment"))
					nodeInstance.CFrame = nodeFrame
				elseif #node.parts ~= 1 then
					nodeInstance = own(Instance.new("Model"))
					nodeInstance.WorldPivot = nodeFrame
				end
				for _, binding in node.parts do
					local meshIndex = binding.mesh + 1
					local mesh = scene.meshes[meshIndex]
					local info = document.meshes[meshIndex]
					local localFrame, scale = decompose(binding.matrix)
					local center = mesh:GetCenter()
					local origin = nodeFrame * localFrame
					local part = own(
						AssetService:CreateMeshPartAsync(
							Content.fromObject(mesh),
							{ CollisionFidelity = Enum.CollisionFidelity.Box }
						)
					)
					part.Name = if #node.parts == 1 then node.name else `Primitive{info.primitiveIndex or 0}`
					part.Anchored = true
					part.Material = Enum.Material.SmoothPlastic
					part.Size *= scale
					part.CFrame = origin * CFrame.new(center * scale)
					part.PivotOffset = part.CFrame:ToObjectSpace(nodeFrame)
					if nodeInstance then
						part.Parent = nodeInstance
					else
						nodeInstance = part
					end
					if binding.material ~= nil then
						local material = document.materials[binding.material + 1]
						local color = Color3.new(material.color[1], material.color[2], material.color[3])
						part.Color = color
						part.Transparency = 1 - material.color[4]
						part.DoubleSided = material.doubleSided
						local owner = part
						if next(material.maps) then
							local appearance = own(Instance.new("SurfaceAppearance"))
							appearance.Name, appearance.Color = material.name, color
							appearance.AlphaMode = Enum.AlphaMode.Transparency
							for channel, imageIndex in material.maps do
								appearance[mapProperties[channel]] = Content.fromObject(scene.images[imageIndex + 1])
							end
							appearance.Parent = part
							owner = appearance
						else
							-- Restore native identity only when the exporter supplied it.
							-- Roughness/metalness alone cannot distinguish wood from planks,
							-- or concrete from brick; never guess a textured native material.
							part.Material = if material.nativeMaterial
								then Enum.Material[material.nativeMaterial]
								elseif material.metalness >= 0.5 then Enum.Material.Metal
								else Enum.Material.SmoothPlastic
							if not material.nativeMaterial then
								table.insert(
									scene.warnings,
									"untextured PBR preview uses a native material approximation; original factors are preserved"
								)
							end
						end
						owner:SetAttribute("RodeoSceneEmissive", Color3.new(unpack(material.emissive)))
						owner:SetAttribute("RodeoSceneEmissiveStrength", material.emissiveStrength)
						owner:SetAttribute("RodeoSceneTransmission", material.transmission)
						setLongStringAttribute(owner, "RodeoSceneMaterialName", material.name)
						owner:SetAttribute("RodeoSceneRoughness", material.roughness)
						owner:SetAttribute("RodeoSceneMetalness", material.metalness)
						owner:SetAttribute("RodeoSceneMaterial", part.Material.Name)
						owner:SetAttribute("RodeoSceneNativeMaterial", material.nativeMaterial)
						listAt(scene.sourceMap.materials, material.sourceIndex, owner)
					end
					local bones, binds, poses = {}, {}, {}
					local indexOf = {}
					for j, id in boneIds[meshIndex] do
						indexOf[id] = j
					end
					for j, id in boneIds[meshIndex] do
						local bone = own(Instance.new("Bone"))
						bone.Name = mesh:GetBoneName(id)
						binds[j] = CFrame.new(-center) * mesh:GetBoneCFrame(id)
						poses[j] = if binding.poses[j] then CFrame.new(-center) * cframe(binding.poses[j]) else binds[j]
						local parentIndex = indexOf[mesh:GetBoneParent(id)]
						bone.CFrame = if parentIndex then binds[parentIndex]:ToObjectSpace(binds[j]) else binds[j]
						local pose = if parentIndex then poses[parentIndex]:ToObjectSpace(poses[j]) else poses[j]
						bone.Transform = bone.CFrame:ToObjectSpace(pose)
						bone.Parent = if parentIndex then bones[parentIndex] else part
						bones[j] = bone
						local source = info.jointNodes[j]
						if source ~= nil then
							bone:SetAttribute("RodeoSceneJoint", `{jointToken}:{source}`)
							listAt(
								scene.sourceMap.joints,
								source,
								{ mesh = mesh, boneId = id, bone = bone, part = part }
							)
						end
					end
					table.insert(scene.sourceMap.primitives, {
						nodeIndex = node.sourceIndex,
						meshIndex = info.meshIndex,
						primitiveIndex = info.primitiveIndex,
						skinIndex = info.skinIndex,
						materialIndex = if binding.material ~= nil
							then document.materials[binding.material + 1].sourceIndex
							else nil,
						mesh = mesh,
						part = part,
					})
					if #info.morph.targets > 0 then
						table.insert(
							scene.morphWeights,
							{ node = nodeInstance, part = part, weights = binding.weights }
						)
					end
					if #info.morph.targets > 0 or info.morph.tangents then
						part:SetAttribute("RodeoSceneData", true)
					end
				end
				nodeInstance.Name = node.name
				if node.sourceIndex ~= nil then
					scene.sourceMap.nodes[node.sourceIndex] = nodeInstance
					if jointSources[node.sourceIndex] then
						nodeInstance:SetAttribute("RodeoSceneJoint", `{jointToken}:{node.sourceIndex}`)
					end
				end
				nodes[i] = nodeInstance
				local m = node.world
				local x, y, z = Vector3.new(unpack(m[1])), Vector3.new(unpack(m[2])), Vector3.new(unpack(m[3]))
				local worldScale = Vector3.new(x.Magnitude, y.Magnitude, z.Magnitude)
				if x:Dot(y:Cross(z)) < 0 then
					worldScale *= Vector3.new(-1, 1, 1)
				end
				nodeInstance:SetAttribute("RodeoSceneWorldScale", worldScale)
				nodeInstance:SetAttribute("RodeoSceneLocalScale", Vector3.new(unpack(node.localScale)))
				if nodeInstance:IsA("Model") then
					nodeInstance:SetAttribute("RodeoSceneModelScale", nodeInstance:GetScale())
				end
			end
			for i, node in document.nodes do
				for _, child in node.children do
					local instance = nodes[child + 1]
					instance.Parent = nodes[i]
					if instance:IsA("Attachment") then
						instance.WorldCFrame = cframe(document.nodes[child + 1].transform)
					end
				end
			end
			local container = own(Instance.new("Model"))
			container.Name = document.name
			container.WorldPivot = CFrame.identity
			container:SetAttribute("RodeoSceneContainer", true)
			for _, root in document.roots do
				nodes[root + 1].Parent = container
			end
			scene.roots = { container }
			for _, animation in document.animations do
				local clip = {
					name = animation.name,
					sourceIndex = animation.sourceIndex,
					metadata = animation.metadata,
					channels = {},
				}
				for _, channel in animation.channels do
					channel.node = nodes[channel.node + 1]
					channel.node:SetAttribute("RodeoSceneData", true)
					table.insert(clip.channels, channel)
				end
				table.insert(scene.animations, clip)
			end
			if #scene.animations > 0 then
				motion.buildRig(scene, document, nodes, own)
			end
			scene.warnings = warnings(scene.warnings)
			if options and options.strict and #scene.warnings > 0 then
				error(scene.warnings[1])
			end
			return scene
		end)
		if not ok then
			cleanup(owned)
			error(`importEditableScene: {path}: {tostring(result)}`, 2)
		end
		return result
	end

	function roblox.exportEditableScene(path, input, options)
		checkPath(path)
		local scene = if input.roots then input else nil
		local roots = if scene then scene.roots else input
		local nativeClips = options and options.animationClips
		if nativeClips and #nativeClips == 0 then
			nativeClips = nil
		end
		assert(
			not (nativeClips and scene and scene.animations and #scene.animations > 0),
			"choose portable animations or native animationClips for one export"
		)
		local document =
			{ name = "Scene", roots = {}, nodes = {}, meshes = {}, images = {}, materials = {}, warnings = {} }
		local morphs, weightsFor = {}, {}
		if scene then
			for _, morph in scene.morphs or {} do
				assert(not morphs[morph.mesh], "duplicate morph record for a mesh")
				morphs[morph.mesh] = morph
			end
			for _, binding in scene.morphWeights or {} do
				weightsFor[binding.part] = binding.weights
			end
		end
		local meshBlobs, imageBlobs, temporary = {}, {}, {}
		local meshIds, imageIds, seen = {}, {}, {}
		local materialIds, loadedMeshes, primitiveMeshes = {}, {}, {}
		local jointNodes, skinBindings = {}, {}
		local function warning(instance, feature)
			if options and options.strict then
				error(`{instance:GetFullName()}: {feature}`)
			end
			table.insert(document.warnings, feature)
		end
		local function imageFor(content)
			if content.SourceType == Enum.ContentSourceType.None then
				return nil
			end
			local key = if content.SourceType == Enum.ContentSourceType.Object then content.Object else content
			if imageIds[key] ~= nil then
				return imageIds[key]
			end
			local sourcePath = options and options.imageSources and options.imageSources[key]
			if sourcePath then
				local id = #document.images
				table.insert(document.images, { width = 0, height = 0, path = sourcePath })
				table.insert(imageBlobs, buffer.create(0))
				imageIds[key] = id
				return id
			end
			local editable
			if content.SourceType == Enum.ContentSourceType.Object then
				editable = content.Object
			else
				editable = assert(AssetService:CreateEditableImageAsync(content), "image memory exhausted")
				table.insert(temporary, editable)
			end
			assert(editable:IsA("EditableImage"), "image content does not reference an EditableImage")
			local id = #document.images
			table.insert(document.images, { width = editable.Size.X, height = editable.Size.Y })
			table.insert(imageBlobs, editable:ReadPixelsBuffer(Vector2.zero, editable.Size))
			imageIds[key] = id
			return id
		end
		local function materialFor(part)
			local appearance = part:FindFirstChildOfClass("SurfaceAppearance")
			local color = if appearance then appearance.Color else part.Color
			local material = {
				name = if appearance
					then appearance.Name
					else getLongStringAttribute(part, "RodeoSceneMaterialName") or part.Name,
				color = { color.R, color.G, color.B, 1 - part.Transparency },
				doubleSided = part:IsA("MeshPart") and part.DoubleSided,
				alphaMode = if part.Transparency == 0 then "OPAQUE" else "BLEND",
			}
			local maps = {}
			if appearance then
				for channel, property in mapProperties do
					maps[channel] = imageFor(appearance[property])
				end
				if appearance.AlphaMode == Enum.AlphaMode.Transparency and maps.color ~= nil then
					material.alphaMode = "BLEND"
				end
				if appearance.AlphaMode == Enum.AlphaMode.Overlay then
					warning(appearance, "Overlay alpha compositing is unsupported; exported as transparency")
				end
				if appearance.EmissiveMaskContent.SourceType ~= Enum.ContentSourceType.None then
					warning(appearance, "emissive channels are unsupported")
				end
			elseif part:IsA("MeshPart") then
				maps.color = imageFor(part.TextureContent)
			end
			if next(maps) then
				material.maps = maps
			end
			local owner = appearance or part
			local native = materialPbr[part.Material.Name] or { 0.70, 0 }
			local savedMaterial = owner:GetAttribute("RodeoSceneMaterial")
			local keepFactors = savedMaterial == part.Material.Name
			material.roughness = if maps.roughness
				then 1
				elseif keepFactors then owner:GetAttribute("RodeoSceneRoughness")
				else native[1] * (1 - 0.9 * part.Reflectance)
			material.metalness = if maps.metalness
				then 1
				elseif keepFactors then owner:GetAttribute("RodeoSceneMetalness")
				else math.max(native[2], part.Reflectance)
			if not appearance then
				material.nativeMaterial = if keepFactors
					then owner:GetAttribute("RodeoSceneNativeMaterial")
					else part.Material.Name
			end
			if keepFactors then
				local emission = owner:GetAttribute("RodeoSceneEmissive")
				if emission then
					material.emissive = { emission.R, emission.G, emission.B }
				end
				material.emissiveStrength = owner:GetAttribute("RodeoSceneEmissiveStrength")
				material.transmission = owner:GetAttribute("RodeoSceneTransmission")
			elseif part.Material == Enum.Material.Neon then
				material.emissive = { color.R, color.G, color.B }
				material.emissiveStrength = 3
				warning(part, "Neon glow is approximated by glTF emission")
			elseif part.Material == Enum.Material.Glass or part.Material == Enum.Material.ForceField then
				material.transmission = 0.9
				warning(part, `{part.Material.Name} refraction is approximated by glTF transmission`)
			end
			if part.MaterialVariant ~= "" then
				warning(part, "MaterialVariant maps are not exported")
			end
			local keyParts = {
				tostring(color),
				tostring(part.Transparency),
				tostring(material.doubleSided),
				material.alphaMode,
				tostring(material.roughness),
				tostring(material.metalness),
				tostring(material.nativeMaterial),
				tostring(
					if appearance then appearance.Name else getLongStringAttribute(part, "RodeoSceneMaterialName")
				),
			}
			for _, channel in { "color", "normal", "roughness", "metalness" } do
				table.insert(keyParts, tostring(maps[channel]))
			end
			table.insert(
				keyParts,
				HttpService:JSONEncode({ material.emissive, material.emissiveStrength, material.transmission })
			)
			local key = table.concat(keyParts, "|")
			if materialIds[key] ~= nil then
				return materialIds[key]
			end
			local id = #document.materials
			materialIds[key] = id
			table.insert(document.materials, material)
			return id
		end
		local function geometryFor(part)
			local primitiveShape, legacyMesh
			local editable
			if part:IsA("MeshPart") then
				local content = part.MeshContent
				if content.SourceType == Enum.ContentSourceType.Object then
					editable = content.Object
				else
					local key = content
					editable = loadedMeshes[key]
					if not editable then
						editable = assert(
							AssetService:CreateEditableMeshAsync(content),
							`{part:GetFullName()}: mesh memory exhausted`
						)
						loadedMeshes[key] = editable
						table.insert(temporary, editable)
					end
				end
			else
				local shape, legacy = primitives.kind(part)
				primitiveShape, legacyMesh = shape, legacy
				editable = primitiveMeshes[shape]
				if not editable then
					editable = primitives.create(shape)
					primitiveMeshes[shape] = editable
					table.insert(temporary, editable)
				end
			end
			assert(
				editable:IsA("EditableMesh"),
				`{part:GetFullName()}: mesh content does not reference an EditableMesh`
			)
			local id = meshIds[editable]
			if id == nil then
				id = #document.meshes
				local morph = morphs[editable]
				local blob, corners = meshToBlob(editable)
				table.insert(
					document.meshes,
					{ jointNodes = {}, morph = if morph then motion.exportMorph(morph, corners) else nil }
				)
				table.insert(meshBlobs, blob)
				meshIds[editable] = id
			end
			local native = if part:IsA("MeshPart") then part.MeshSize else Vector3.one
			local scale = Vector3.new(
				if native.X > 0 then part.Size.X / native.X else 1,
				if native.Y > 0 then part.Size.Y / native.Y else 1,
				if native.Z > 0 then part.Size.Z / native.Z else 1
			)
			if primitiveShape == "Cylinder" then
				local diameter = math.min(scale.Y, scale.Z)
				scale = Vector3.new(scale.X, diameter, diameter)
			elseif primitiveShape == "Ball" and not legacyMesh then
				scale = Vector3.one * math.min(scale.X, scale.Y, scale.Z)
			end
			if legacyMesh then
				if legacyMesh:IsA("SpecialMesh") and legacyMesh.TextureId ~= "" then
					warning(part, "legacy primitive mesh textures are unsupported")
				end
				if legacyMesh.VertexColor ~= Vector3.one then
					warning(part, "legacy mesh vertex tint is unsupported")
				end
				scale *= legacyMesh.Scale
			end
			local origin = part.CFrame
				* CFrame.new((if legacyMesh then legacyMesh.Offset else Vector3.zero) - editable:GetCenter() * scale)
			local binding = {
				mesh = id,
				material = materialFor(part),
				matrix = matrix(part:GetPivot():ToObjectSpace(origin), scale),
				poses = {},
				weights = weightsFor[part],
			}
			local ids = editable:GetBones()
			if #ids > 0 then
				local bones = {}
				for _, descendant in part:GetDescendants() do
					if descendant:IsA("Bone") and descendant:FindFirstAncestorWhichIsA("MeshPart") == part then
						assert(
							not bones[descendant.Name],
							`{part:GetFullName()}: duplicate Bone name {descendant.Name}`
						)
						bones[descendant.Name] = descendant
					end
				end
				assert(
					next(bones) == nil or (scale - Vector3.one).Magnitude < 1e-4,
					`{part:GetFullName()}: scaled posed skins are unsupported`
				)
				-- Match meshToBlob's parent-first order, which is stable even when
				-- the engine's bone IDs are sparse or returned in another order.
				local present, placed, ordered = {}, {}, {}
				for _, id in ids do
					present[id] = true
				end
				while #ordered < #ids do
					local before = #ordered
					for _, id in ids do
						local parent = editable:GetBoneParent(id)
						if not placed[id] and (not present[parent] or placed[parent]) then
							placed[id] = true
							table.insert(ordered, id)
						end
					end
					assert(#ordered > before, "bone hierarchy contains a cycle")
				end
				local bound = {}
				for _, id in ordered do
					local bone = bones[editable:GetBoneName(id)]
					table.insert(
						binding.poses,
						if bone
							then components(origin:ToObjectSpace(bone.TransformedWorldCFrame))
							else components(editable:GetBoneCFrame(id))
					)
					table.insert(bound, bone or false)
				end
				table.insert(skinBindings, { binding = binding, bones = bound, part = part })
			end
			return binding
		end
		local function visit(instance, parentFrame)
			assert(not seen[instance], `exportEditableScene: roots overlap at {instance:GetFullName()}`)
			seen[instance] = #document.nodes
			local frame = parentFrame
			if instance:IsA("Model") or instance:IsA("BasePart") then
				frame = instance:GetPivot()
			elseif instance:IsA("Attachment") then
				frame = instance.WorldCFrame
			elseif not instance:IsA("Folder") then
				warning(instance, `{instance.ClassName} behavior is unsupported; exported hierarchy only`)
			end
			local index = #document.nodes
			local node = {
				name = instance.Name,
				className = if instance:IsA("Attachment") then "Attachment" else nil,
				transform = components(frame),
				children = {},
				parts = {},
			}
			local sourceScale = instance:GetAttribute("RodeoSceneWorldScale")
			if sourceScale then
				local modelScale = instance:GetAttribute("RodeoSceneModelScale")
				if modelScale then
					sourceScale *= instance:GetScale() / modelScale
				end
				node.world = matrix(frame, sourceScale)
				local localScale = instance:GetAttribute("RodeoSceneLocalScale")
				node.localScale = { localScale.X, localScale.Y, localScale.Z }
			elseif scene or nativeClips then
				node.world = matrix(frame, Vector3.one)
			end
			if not scene and instance:GetAttribute("RodeoSceneData") then
				warning(instance, "animation/morph data omitted; pass the full EditableScene to preserve it")
			end
			table.insert(document.nodes, node)
			local jointKey = instance:GetAttribute("RodeoSceneJoint")
			if jointKey then
				jointNodes[jointKey] = index
			end
			if instance:IsA("BasePart") then
				table.insert(node.parts, geometryFor(instance))
			end
			for _, child in instance:GetChildren() do
				if
					child:IsA("SurfaceAppearance")
					or child:IsA("Bone")
					or child:IsA("DataModelMesh")
					or child:GetAttribute("RodeoSceneHelper")
					or (
						nativeClips
						and (
							child:IsA("JointInstance")
							or child:IsA("Constraint")
							or child:IsA("AnimationController")
							or child:IsA("Humanoid")
							or child:IsA("AnimationClip")
							or child:IsA("Animation")
							or child:IsA("BaseScript")
						)
					)
				then
					continue
				end
				table.insert(node.children, visit(child, frame))
			end
			return index
		end
		local handle
		local ok, result = pcall(function()
			for _, root in roots do
				if
					not nativeClips and (root:GetAttribute("RodeoSceneContainer") or root:GetAttribute("RodeoSceneRig"))
				then
					document.name = root.Name
					for _, child in root:GetChildren() do
						if not child:GetAttribute("RodeoSceneHelper") then
							table.insert(document.roots, visit(child, CFrame.identity))
						end
					end
				else
					table.insert(document.roots, visit(root, CFrame.identity))
				end
			end
			local assignedJointPoses = {}
			for _, record in skinBindings do
				local nodes, poses = {}, {}
				for _, bone in record.bones do
					local key = if bone then bone:GetAttribute("RodeoSceneJoint") else nil
					local index = key and jointNodes[key]
					if index == nil then
						break
					end
					local pose = bone.TransformedWorldCFrame
					local assigned = assignedJointPoses[index]
					-- Shared imported skeletons may be edited independently after
					-- cloning a part. Give divergent poses separate glTF joints.
					if
						assigned
						and (
							(assigned.Position - pose.Position).Magnitude > 1e-4
							or assigned.XVector:Dot(pose.XVector) < 0.99999
							or assigned.YVector:Dot(pose.YVector) < 0.99999
						)
					then
						break
					end
					table.insert(nodes, index)
					table.insert(poses, pose)
				end
				if #nodes == #record.bones then
					record.binding.jointNodes = nodes
					for i, index in nodes do
						assignedJointPoses[index] = poses[i]
						document.nodes[index + 1].transform = components(poses[i])
						if document.nodes[index + 1].world then
							document.nodes[index + 1].world = matrix(poses[i], Vector3.one)
						end
					end
				end
			end
			if scene then
				document.animations = motion.exportAnimations(scene, seen)
			end

			if nativeClips then
				nativeMotion(
					document,
					seen,
					skinBindings,
					nativeClips,
					options.animationSampleRate or 60,
					matrix,
					warning
				)
			end

			local chunks = { buffer.fromstring(HttpService:JSONEncode(document)) }
			for _, blob in meshBlobs do
				table.insert(chunks, blob)
			end
			for _, blob in imageBlobs do
				table.insert(chunks, blob)
			end
			handle = fs.open(path, "w")
			for _, blob in chunks do
				local length = buffer.create(4)
				buffer.writeu32(length, 0, buffer.len(blob))
				stream.writeBytes(handle, length)
				stream.writeBytes(handle, blob)
			end
			rpc.call("roblox_scene_encode", runtime.RobloxSceneEncodeRequest.new({ handle = handle.__handle }))
			return warnings(document.warnings)
		end)
		cleanup(temporary)
		if not ok then
			if handle then
				-- close normally commits raw transport bytes. Discard instead;
				-- encode may already have consumed the handle, which is harmless.
				pcall(
					rpc.call,
					"stream_close",
					runtime.StreamCloseRequest.new({ handle = handle.__handle, discard = true })
				)
			end
			error(`exportEditableScene: {path}: {tostring(result)}`, 2)
		end
		return result
	end
end
