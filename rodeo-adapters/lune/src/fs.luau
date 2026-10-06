-- @lune/fs shim — maps to rodeo's fs + stream RPCs.
-- Conformance target: lune's own fs tests (tests/adapters/lune/fs/).
local g = _G.__rodeo_procs[_G.__rodeo_eid]

-- Byte streams rather than the string stream: file contents are arbitrary
-- binary, and the string path is UTF-8-lossy on the wire.
local function readFile(path: string): string
	local h = g.fs.open(path, "r")
	local content = g.stream.readBytes(h)
	g.stream.close(h)
	return buffer.tostring(content)
end

-- lune accepts strings and buffers.
local function writeFile(path: string, content: string | buffer)
	local h = g.fs.open(path, "w")
	local buf = if typeof(content) == "buffer" then content else buffer.fromstring(content :: string)
	g.stream.writeBytes(h, buf)
	g.stream.close(h)
end

local function isFile(path: string): boolean
	local ok, result = pcall(g.fs.type, path)
	return ok and result == "file"
end

local function isDir(path: string): boolean
	local ok, result = pcall(g.fs.type, path)
	return ok and result == "dir"
end

local function readDir(path: string): { string }
	local entries = g.fs.listdir(path)
	local names = {}
	for _, entry in entries do
		table.insert(names, entry.name)
	end
	return names
end

local function removeFile(path: string)
	g.fs.remove(path)
end

-- lune's removeDir removes contents too; the host rmdir is non-recursive.
local function removeDir(path: string)
	for _, entry in g.fs.listdir(path) do
		local child = path .. "/" .. entry.name
		if isDir(child) then
			removeDir(child)
		else
			g.fs.remove(child)
		end
	end
	g.fs.rmdir(path)
end

local function writeDir(path: string)
	g.fs.mkdir(path)
end

-- lune's copy handles directories recursively; the host copy is file-only.
local function copy(from: string, to: string)
	if isDir(from) then
		g.fs.mkdir(to)
		for _, entry in g.fs.listdir(from) do
			copy(from .. "/" .. entry.name, to .. "/" .. entry.name)
		end
	else
		g.fs.copy(from, to)
	end
end

local function move(from: string, to: string)
	copy(from, to)
	if isDir(from) then
		removeDir(from)
	else
		g.fs.remove(from)
	end
end

-- lune metadata: `exists = false` for missing paths (never throws), kind,
-- and DateTime timestamps — Studio's native DateTime satisfies
-- typeof(x) == "DateTime" and value equality, so mint real ones from the
-- stat millis.
local function toDateTime(millis: number?): DateTime?
	if millis == nil then
		return nil
	end
	return DateTime.fromUnixTimestampMillis(millis)
end

local function metadata(path: string)
	local ok, stat = pcall(g.fs.stat, path)
	if not ok then
		return { exists = false }
	end
	return {
		exists = true,
		kind = stat.type,
		permissions = { readOnly = false },
		createdAt = toDateTime(stat.createdMillis),
		modifiedAt = toDateTime(stat.modifiedMillis),
		accessedAt = toDateTime(stat.accessedMillis),
	}
end

-- Members the shim does not provide raise an error naming them, instead of
-- indexing to nil and failing later as "attempt to call a nil value".
return setmetatable({
	readFile = readFile,
	writeFile = writeFile,
	isFile = isFile,
	isDir = isDir,
	readDir = readDir,
	removeFile = removeFile,
	removeDir = removeDir,
	writeDir = writeDir,
	copy = copy,
	move = move,
	metadata = metadata,
}, {
	__index = function(_, key)
		error(`@lune/fs.{key} is not supported by rodeo's Lune adapter`, 2)
	end,
})
