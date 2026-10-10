-- @rodeo/roblox API: import / export model files via SerializationService,
-- plus bake (Luau-source data files).
-- Returns a factory: (rpc, fs, stream, normalize, serializeToLuau, runtime, createScene, sceneMotion, scenePrimitives, sceneNativeMotion) -> roblox module.
-- `runtime` is the runner's proto module, captured here rather than read from
-- a global at call time: several rodeo plugins can share this VM, and a global
-- would hold whichever build's runner set it last.

return function(rpc, fs, stream, normalize, serializeToLuau, runtime, createScene, sceneMotion, scenePrimitives, sceneNativeMotion)
	local roblox = {}
	local SerializationService = game:GetService("SerializationService")

	-- Deprecated names warn once per run and forward to the new ones. This
	-- factory is instantiated per execution, so the flag is per run.
	local warnedDeprecated: { [string]: boolean } = {}
	local function deprecated(old: string, new: string)
		if not warnedDeprecated[old] then
			warnedDeprecated[old] = true
			warn(`[rodeo] roblox.{old} is deprecated and will be removed in 2.0; use roblox.{new}`)
		end
	end

	function roblox.importInstances(path: string): { Instance }
		local h = fs.open(path, "r")
		local data = stream.readBytes(h)
		stream.close(h)
		return SerializationService:DeserializeInstancesAsync(data)
	end

	-- Content properties that can reference an in-memory EditableMesh or
	-- EditableImage (`Content.fromObject`), keyed by the class declaring them.
	-- SerializationService writes such a reference as empty, so an export
	-- that holds one "succeeds" and reloads as placeholder geometry or a blank
	-- image (issue #7). A name a Studio build lacks is skipped.
	local objectContentProperties: { [string]: { string } } = {
		MeshPart = { "MeshContent", "TextureContent" },
		SurfaceAppearance = {
			"ColorMapContent",
			"NormalMapContent",
			"RoughnessMapContent",
			"MetalnessMapContent",
			"EmissiveMaskContent",
		},
		Decal = { "TextureContent" },
		ImageLabel = { "ImageContent" },
		ImageButton = { "ImageContent", "HoverImageContent", "PressedImageContent" },
		BaseWrap = { "CageMeshContent" },
		WrapLayer = { "ReferenceMeshContent" },
	}
	local propertiesByClass: { [string]: { string } } = {}
	local function contentPropertiesOf(instance: Instance): { string }
		local cached = propertiesByClass[instance.ClassName]
		if cached then
			return cached
		end
		local names = {}
		for class, properties in objectContentProperties do
			if instance:IsA(class) then
				for _, name in properties do
					if pcall(function()
						return (instance :: any)[name]
					end) then
						table.insert(names, name)
					end
				end
			end
		end
		propertiesByClass[instance.ClassName] = names
		return names
	end

	-- `Instance.Property (ObjectClass)` for every in-memory content reference
	-- in `instances` and their descendants.
	local function objectContentIn(instances: { Instance }): { string }
		local found = {}
		local function visit(instance: Instance)
			for _, name in contentPropertiesOf(instance) do
				local ok, content = pcall(function()
					return (instance :: any)[name]
				end)
				if ok and typeof(content) == "Content" and content.SourceType == Enum.ContentSourceType.Object then
					local okClass, className = pcall(function()
						return content.Object.ClassName
					end)
					table.insert(found, `{instance:GetFullName()}.{name} ({if okClass then className else "Object"})`)
				end
			end
		end
		for _, root in instances do
			visit(root)
			for _, descendant in root:GetDescendants() do
				visit(descendant)
			end
		end
		return found
	end

	function roblox.exportInstances(path: string, instances: { Instance })
		local lost = objectContentIn(instances)
		if #lost > 0 then
			local shown = table.concat(lost, ", ", 1, math.min(#lost, 5))
			local more = if #lost > 5 then `, and {#lost - 5} more` else ""
			warn(
				`[rodeo] roblox.exportInstances({path}): .rbxm/.rbxmx cannot store in-memory EditableMesh/EditableImage `
					.. `content (Content.fromObject); {#lost} reference(s) export empty: {shown}{more}. `
					.. `Export those objects with roblox.exportEditableMesh / roblox.exportEditableImage.`
			)
		end
		local buf = SerializationService:SerializeInstancesAsync(instances)
		local h = fs.open(path, "w")
		stream.writeBytes(h, buf)
		-- Finalize in place of stream.close: consumes the handle, converts to
		-- XML when the extension is .rbxmx/.rbxlx, writes atomically.
		rpc.call("roblox_export", runtime.RobloxExportRequest.new({ handle = h.__handle }))
	end

	function roblox.import(path: string): { Instance }
		deprecated("import", "importInstances")
		return roblox.importInstances(path)
	end

	function roblox.export(path: string, instances: { Instance })
		deprecated("export", "exportInstances")
		roblox.exportInstances(path, instances)
	end

	-- Write `value` to `path` as a Luau module (`return <value>`), so the data
	-- can be required straight back into Studio. Roblox types round-trip via
	-- their constructors (vectors, CFrames, colors, enums, …) and buffers via
	-- `buffer.fromstring`; anything with no source representation (Instances,
	-- functions) becomes its `tostring`.
	--
	-- This is the same normalize → serialize → write path `--return <file>.luau`
	-- uses; that flag calls straight through here.
	function roblox.bake(path: string, value: any)
		local source = serializeToLuau(normalize(value))
		local parent = path:match("^(.+)[/\\][^/\\]+$")
		if parent and not fs.exists(parent) then
			fs.mkdir(parent)
		end
		local h = fs.open(path, "w")
		stream.write(h, source)
		stream.close(h)
	end

	-- Editable image codecs. EditableImage/EditableMesh are runtime Objects
	-- with no file representation (SerializationService writes an
	-- Object-backed Content as an empty reference), so these move their pixels
	-- between files and Studio directly: PNG on the host side, RGBA8 buffers
	-- on the Studio side, streamed in chunks over the same fs/stream RPCs
	-- export/import use.

	local AssetService = game:GetService("AssetService")
	local nextImageId = 0

	function roblox.exportEditableImage(path: string, image: EditableImage)
		local okClass, className = pcall(function()
			return (image :: any).ClassName
		end)
		if not okClass or className ~= "EditableImage" then
			error("exportEditableImage: expected an EditableImage", 2)
		end
		local size = image.Size
		local width, height = size.X, size.Y
		local pixels = image:ReadPixelsBuffer(Vector2.zero, size)
		local expected = width * height * 4
		if buffer.len(pixels) ~= expected then
			error(`exportEditableImage: pixel buffer is {buffer.len(pixels)} bytes, expected {expected} for {width}x{height}`, 2)
		end
		local h = fs.open(path, "w")
		stream.writeBytes(h, pixels)
		-- Finalize in place of stream.close: consumes the handle, encodes PNG.
		rpc.call("roblox_image_encode", runtime.RobloxImageEncodeRequest.new({
			handle = h.__handle,
			width = width,
			height = height,
		}))
	end

	function roblox.importEditableImage(path: string): EditableImage
		nextImageId += 1
		local handle = { __handle = `image:{nextImageId}:{path}` }
		local res = rpc.call("roblox_image_decode", runtime.RobloxImageDecodeRequest.new({
			path = path,
			handle = handle.__handle,
		}))
		local pixels = stream.readBytes(handle)
		stream.close(handle)
		local width, height = res.width, res.height
		local expected = width * height * 4
		if buffer.len(pixels) ~= expected then
			error(`importEditableImage: received {buffer.len(pixels)} bytes, expected {expected} for {width}x{height}`, 2)
		end
		local okCreate, image = pcall(function()
			return AssetService:CreateEditableImage({ Size = Vector2.new(width, height) })
		end)
		if not okCreate or not image then
			error(`importEditableImage: Studio refused an EditableImage of {width}x{height} ({path}): {tostring(image)}`, 2)
		end
		image:WritePixelsBuffer(Vector2.zero, image.Size, pixels)
		return image
	end

	-- Editable mesh codecs: glTF on the host, EditableMesh here, joined by the
	-- packed little-endian blob documented in rodeo-client/src/runtime/mesh.rs
	-- (BLOB LAYOUT). Vertices in the blob are glTF-style (every attribute per
	-- vertex, faces index vertices); EditableMesh attributes are per face
	-- corner, so export splits corners into unique (vertex, normal, uv, color)
	-- tuples and import binds each corner to its vertex's attribute ids.

	local MESH_MAGIC = 0x4853_4D52
	local MESH_VERSION = 1
	local MESH_F_NORMALS, MESH_F_UVS, MESH_F_COLORS, MESH_F_SKIN = 1, 2, 4, 8
	local MESH_NONE = 0xFFFFFFFF
	local nextMeshId = 0

	type MeshBlob = {
		positions: { Vector3 },
		normals: { Vector3 }?,
		uvs: { Vector2 }?,
		colors: { Color3 }?,
		alphas: { number }?,
		joints: { { number } }?,
		weights: { { number } }?,
		indices: { number },
		bones: { { name: string, parent: number?, cframe: CFrame, virtual: boolean } },
	}

	local function writeMeshBlob(data: MeshBlob): buffer
		local vcount, icount = #data.positions, #data.indices
		local flags = 0
		if data.normals then flags += MESH_F_NORMALS end
		if data.uvs then flags += MESH_F_UVS end
		if data.colors then flags += MESH_F_COLORS end
		if data.joints and data.weights then flags += MESH_F_SKIN end
		local size = 24 + vcount * 12 + icount * 4
		if data.normals then size += vcount * 12 end
		if data.uvs then size += vcount * 8 end
		if data.colors then size += vcount * 16 end
		if data.joints and data.weights then size += vcount * 32 end
		for _, b in ipairs(data.bones) do
			size += 8 + 48 + 4 + math.ceil(#b.name / 4) * 4
		end
		local buf = buffer.create(size)
		local o = 0
		local function u32(v: number) buffer.writeu32(buf, o, v); o += 4 end
		local function f32(v: number) buffer.writef32(buf, o, v); o += 4 end
		u32(MESH_MAGIC); u32(MESH_VERSION); u32(vcount); u32(icount); u32(flags); u32(#data.bones)
		for _, p in ipairs(data.positions) do f32(p.X); f32(p.Y); f32(p.Z) end
		if data.normals then for _, n in ipairs(data.normals) do f32(n.X); f32(n.Y); f32(n.Z) end end
		if data.uvs then for _, uv in ipairs(data.uvs) do f32(uv.X); f32(uv.Y) end end
		if data.colors then
			for i, c in ipairs(data.colors) do f32(c.R); f32(c.G); f32(c.B); f32((data.alphas :: { number })[i]) end
		end
		if data.joints and data.weights then
			for _, j in ipairs(data.joints) do for k = 1, 4 do u32(j[k]) end end
			for _, w in ipairs(data.weights) do for k = 1, 4 do f32(w[k]) end end
		end
		for _, i in ipairs(data.indices) do u32(i) end
		for _, b in ipairs(data.bones) do
			u32(if b.parent then b.parent else MESH_NONE)
			buffer.writeu8(buf, o, if b.virtual then 1 else 0); o += 4
			local comps = { b.cframe:GetComponents() }
			for k = 1, 12 do f32(comps[k]) end
			u32(#b.name)
			buffer.writestring(buf, o, b.name); o += math.ceil(#b.name / 4) * 4
		end
		return buf
	end

	local function readMeshBlob(buf: buffer): MeshBlob
		local o = 0
		local function u32(): number local v = buffer.readu32(buf, o); o += 4; return v end
		local function f32(): number local v = buffer.readf32(buf, o); o += 4; return v end
		if u32() ~= MESH_MAGIC then error("not a rodeo mesh blob", 0) end
		local version = u32()
		if version ~= MESH_VERSION then error(`unsupported mesh blob version {version}`, 0) end
		local vcount, icount, flags, bcount = u32(), u32(), u32(), u32()
		local data: MeshBlob = { positions = table.create(vcount), indices = table.create(icount), bones = table.create(bcount) }
		for i = 1, vcount do data.positions[i] = Vector3.new(f32(), f32(), f32()) end
		if bit32.band(flags, MESH_F_NORMALS) ~= 0 then
			local t = table.create(vcount); for i = 1, vcount do t[i] = Vector3.new(f32(), f32(), f32()) end; data.normals = t
		end
		if bit32.band(flags, MESH_F_UVS) ~= 0 then
			local t = table.create(vcount); for i = 1, vcount do t[i] = Vector2.new(f32(), f32()) end; data.uvs = t
		end
		if bit32.band(flags, MESH_F_COLORS) ~= 0 then
			local cs, as = table.create(vcount), table.create(vcount)
			for i = 1, vcount do cs[i] = Color3.new(f32(), f32(), f32()); as[i] = f32() end
			data.colors, data.alphas = cs, as
		end
		if bit32.band(flags, MESH_F_SKIN) ~= 0 then
			local js, ws = table.create(vcount), table.create(vcount)
			for i = 1, vcount do js[i] = { u32(), u32(), u32(), u32() } end
			for i = 1, vcount do ws[i] = { f32(), f32(), f32(), f32() } end
			data.joints, data.weights = js, ws
		end
		for i = 1, icount do data.indices[i] = u32() end
		for i = 1, bcount do
			local parent = u32()
			local virtual = buffer.readu8(buf, o) ~= 0; o += 4
			local c = table.create(12); for k = 1, 12 do c[k] = f32() end
			local nameLen = u32()
			local name = buffer.readstring(buf, o, nameLen); o += math.ceil(nameLen / 4) * 4
			data.bones[i] = {
				name = name,
				parent = if parent == MESH_NONE then nil else parent,
				cframe = CFrame.new(c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8], c[9], c[10], c[11], c[12]),
				virtual = virtual,
			}
		end
		return data
	end

	-- Writes an EditableMesh to `path` as glTF (.glb or .gltf). Geometry,
	-- per-corner normals/UVs/colors, and skinning (bones with bind poses, up to
	-- four influences per vertex) are written; FACS poses are not.
	local function meshToBlob(mesh: EditableMesh)
		local okClass, className = pcall(function()
			return (mesh :: any).ClassName
		end)
		if not okClass or className ~= "EditableMesh" then
			error("exportEditableMesh: expected an EditableMesh", 2)
		end

		-- Split face corners into unique (vertex, normal, uv, color) tuples.
		local tuples: { [string]: number } = {}
		local positions, normals, uvs, colors, alphas = {}, {}, {}, {}, {}
		local vertexOfTuple: { number } = {}
		local normalOfTuple: { number } = {}
		local indices: { number } = {}
		local hasNormals, hasUVs, hasColors = true, true, true
		local count = 0
		for _, faceId in ipairs(mesh:GetFaces()) do
			local vs = mesh:GetFaceVertices(faceId)
			if #vs ~= 3 then
				error(`exportEditableMesh: face {faceId} has {#vs} vertices; call mesh:Triangulate() first`, 2)
			end
			local ns = mesh:GetFaceNormals(faceId) or {}
			local us = mesh:GetFaceUVs(faceId) or {}
			local cs = mesh:GetFaceColors(faceId) or {}
			for k = 1, 3 do
				local v, n, u, c = vs[k], ns[k], us[k], cs[k]
				local nv = if n and n ~= 0 then mesh:GetNormal(n) else nil
				local uv = if u and u ~= 0 then mesh:GetUV(u) else nil
				local cv = if c and c ~= 0 then mesh:GetColor(c) else nil
				if not nv then hasNormals = false end
				if not uv then hasUVs = false end
				if not cv then hasColors = false end
				local key = `{v}|{n or 0}|{u or 0}|{c or 0}`
				local idx = tuples[key]
				if not idx then
					count += 1
					idx = count
					tuples[key] = idx
					positions[idx] = mesh:GetPosition(v)
					normals[idx] = nv
					uvs[idx] = uv
					colors[idx] = cv
					alphas[idx] = if cv then (mesh:GetColorAlpha(c) or 1) else 1
					vertexOfTuple[idx] = v
					normalOfTuple[idx] = n or 0
				end
				indices[#indices + 1] = idx - 1
			end
		end
		if count == 0 then
			error("exportEditableMesh: mesh has no faces", 2)
		end

		-- Bones, parents before children, remapped to 0-based blob indices.
		local bones = {}
		local boneIndex: { [number]: number } = {}
		local boneIds = mesh:GetBones()
		if #boneIds > 0 then
			local isBone: { [number]: boolean } = {}
			for _, id in ipairs(boneIds) do isBone[id] = true end
			local placed: { [number]: boolean } = {}
			local remaining = #boneIds
			while remaining > 0 do
				local progressed = false
				for _, id in ipairs(boneIds) do
					if not placed[id] then
						local p = mesh:GetBoneParent(id)
						local parent = if p and isBone[p] then p else nil
						if parent == nil or placed[parent] then
							placed[id] = true
							remaining -= 1
							progressed = true
							boneIndex[id] = #bones
							bones[#bones + 1] = {
								name = mesh:GetBoneName(id),
								parent = if parent then boneIndex[parent] else nil,
								cframe = mesh:GetBoneCFrame(id),
								virtual = mesh:GetBoneIsVirtual(id),
							}
						end
					end
				end
				if not progressed then
					error("exportEditableMesh: bone hierarchy contains a cycle", 2)
				end
			end
		end
		local joints, weights = nil, nil
		if #bones > 0 then
			local any = false
			local js, ws = table.create(count), table.create(count)
			for idx = 1, count do
				local v = vertexOfTuple[idx]
				local bs = mesh:GetVertexBones(v) or {}
				local bw = mesh:GetVertexBoneWeights(v) or {}
				if #bs > 4 then
					error(`exportEditableMesh: a vertex has {#bs} bone influences; glTF export supports at most 4`, 2)
				end
				local j, w = { MESH_NONE, MESH_NONE, MESH_NONE, MESH_NONE }, { 0, 0, 0, 0 }
				for k = 1, #bs do
					local bi = boneIndex[bs[k]]
					if bi ~= nil and (bw[k] or 0) > 0 then
						j[k] = bi
						w[k] = bw[k]
						any = true
					end
				end
				js[idx] = j
				ws[idx] = w
			end
			if any then joints, weights = js, ws end
		end

		local blob = writeMeshBlob({
			positions = positions,
			normals = if hasNormals then normals else nil,
			uvs = if hasUVs then uvs else nil,
			colors = if hasColors then colors else nil,
			alphas = if hasColors then alphas else nil,
			joints = joints,
			weights = weights,
			indices = indices,
			bones = bones,
		})
		return blob, { vertices = vertexOfTuple, normals = normalOfTuple }
	end

	function roblox.exportEditableMesh(path: string, mesh: EditableMesh): { string }
		local blob = meshToBlob(mesh)
		local h = fs.open(path, "w")
		stream.writeBytes(h, blob)
		-- Finalize in place of stream.close: consumes the handle, writes the
		-- file in the format the extension names. Returns what that format
		-- could not carry (empty for glTF; OBJ drops colors and skinning).
		local res = rpc.call("roblox_mesh_encode", runtime.RobloxMeshEncodeRequest.new({ handle = h.__handle }))
		return res.dropped or {}
	end

	-- Add `count` items one at a time and return their ids in order. The batch
	-- API is deliberately not used for attributes: the engine validates a
	-- BatchAdd of N items as if it added N vertices and fails with "Vertex
	-- count above limit" whenever vertexCount + N exceeds its cap, so a
	-- 32768-vertex mesh cannot batch its own 32768 UVs (issue #20). The
	-- per-element calls apply only the real per-mesh caps, and when one is hit
	-- the engine's own error is kept and annotated with how far the add got,
	-- so the message names the count without hardcoding a limit that may change.
	local function addEach(count: number, what: string, add: (number) -> number): { number }
		local ids = table.create(count)
		for i = 1, count do
			local ok, result = pcall(add, i)
			if not ok then
				error(`{tostring(result)} (adding {what} {i} of {count})`, 0)
			end
			ids[i] = result
		end
		return ids
	end

	local function meshFromBlob(blob: buffer, path: string)
		local data = readMeshBlob(blob)
		local boneIds: { number } = {}
		local attributes
		local mesh = assert(AssetService:CreateEditableMesh(), `importEditableMesh: {path}: editable mesh memory exhausted`)
		local ok, err = pcall(function()
			local vertexCount = #data.positions
			local vIds = addEach(vertexCount, "vertex", function(i)
				return mesh:AddVertex(data.positions[i])
			end)
			local nIds = if data.normals
				then addEach(vertexCount, "normal", function(i)
					return mesh:AddNormal(data.normals[i])
				end)
				else nil
			attributes = { vertices = vIds, normals = nIds or {} }
			local uIds = if data.uvs
				then addEach(vertexCount, "UV", function(i)
					return mesh:AddUV(data.uvs[i])
				end)
				else nil
			local cIds = if data.colors
				then addEach(vertexCount, "color", function(i)
					return mesh:AddColor(data.colors[i], data.alphas[i])
				end)
				else nil
			local faceCount = #data.indices // 3
			-- The k-th corner of face f, as an id from `ids` (indices are 0-based).
			local function corner(ids: { number }, f: number, k: number): number
				return ids[data.indices[(f - 1) * 3 + k] + 1]
			end
			local fIds = addEach(faceCount, "triangle", function(f)
				return mesh:AddTriangle(corner(vIds, f, 1), corner(vIds, f, 2), corner(vIds, f, 3))
			end)
			for f = 1, faceCount do
				if nIds then
					mesh:SetFaceNormals(fIds[f], { corner(nIds, f, 1), corner(nIds, f, 2), corner(nIds, f, 3) })
				end
				if uIds then
					mesh:SetFaceUVs(fIds[f], { corner(uIds, f, 1), corner(uIds, f, 2), corner(uIds, f, 3) })
				end
				if cIds then
					mesh:SetFaceColors(fIds[f], { corner(cIds, f, 1), corner(cIds, f, 2), corner(cIds, f, 3) })
				end
			end

			for i, b in ipairs(data.bones) do
				local props: { [string]: any } = { Name = b.name, CFrame = b.cframe, Virtual = b.virtual }
				if b.parent ~= nil then
					props.ParentId = boneIds[b.parent + 1]
				end
				boneIds[i] = mesh:AddBone(props)
			end
			if data.joints and data.weights then
				for i = 1, #vIds do
					local ids, ws = {}, {}
					for k = 1, 4 do
						local j, w = data.joints[i][k], data.weights[i][k]
						if j ~= MESH_NONE and w > 0 then
							ids[#ids + 1] = boneIds[j + 1]
							ws[#ws + 1] = w
						end
					end
					if #ids > 0 then
						mesh:SetVertexBones(vIds[i], ids)
						mesh:SetVertexBoneWeights(vIds[i], ws)
					end
				end
			end
		end)
		if not ok then
			mesh:Destroy()
			error(`importEditableMesh: {path}: {tostring(err)}`, 2)
		end
		return mesh, boneIds, attributes
	end

	-- Reads a .glb or .gltf into a new EditableMesh. Node transforms are baked
	-- into the geometry and all primitives merge into one mesh.
	function roblox.importEditableMesh(path: string): EditableMesh
		nextMeshId += 1
		local handle = { __handle = `mesh:{nextMeshId}:{path}` }
		rpc.call("roblox_mesh_decode", runtime.RobloxMeshDecodeRequest.new({
			path = path,
			handle = handle.__handle,
		}))
		local blob = stream.readBytes(handle)
		stream.close(handle)

		local mesh = meshFromBlob(blob, path)
		return mesh
	end

	createScene(roblox, rpc, fs, stream, runtime, meshToBlob, meshFromBlob, sceneMotion, scenePrimitives, sceneNativeMotion)

	-- Internal plumbing for the roblox.captureViewport shim (rodeo-pkg/rodeo/shims/
	-- roblox.luau), which composes these with the fs/process API. Not part of
	-- the typed public surface.

	-- Ask the plugin to select a simulator preset and/or a custom desktop device
	-- at width x height for this execution. Returns the resulting
	-- Camera.ViewportSize.
	function roblox._simulatorApply(
		device: string?,
		width: number?,
		height: number?,
		scalingMode: string?,
		pixelDensity: number?,
		orientation: string?,
		deviceForm: string?
	): (number, number)
		local res = rpc.call("roblox_simulator_apply", runtime.RobloxSimulatorApplyRequest.new({
			device = device,
			width = width,
			height = height,
			scalingMode = scalingMode,
			pixelDensity = pixelDensity,
			orientation = orientation,
			deviceForm = deviceForm,
		}))
		return res.width, res.height
	end

	function roblox._simulatorRestore()
		rpc.call("roblox_simulator_restore", runtime.RobloxSimulatorRestoreRequest.new({}))
	end

	-- Every capture: the run client snapshots the engine's capture directory
	-- before CaptureScreenshot (`begin`) and collects the frame the engine
	-- writes after the callback (`collect`): scale check, all-black refusal,
	-- resample to the viewport, PNG. See RobloxCaptureBeginRequest.
	function roblox._captureBegin(): string
		local res = rpc.call("roblox_capture_begin", runtime.RobloxCaptureBeginRequest.new({}))
		return res.token
	end

	function roblox._captureCollect(token: string, output: string, width: number, height: number, keepFrame: boolean): (number, number)
		local res = rpc.call("roblox_capture_collect", runtime.RobloxCaptureCollectRequest.new({
			token = token,
			output = output,
			width = width,
			height = height,
			keepFrame = keepFrame,
		}))
		return res.width, res.height
	end

	return roblox
end
