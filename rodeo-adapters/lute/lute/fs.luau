-- @lute/fs shim — maps lute's handle-based fs onto rodeo's fs + stream RPCs.
-- Timestamps in stat are @lute/time Durations (since the Unix epoch), per
-- lute's FileMetadata.
--
-- Host handles are a reader OR a writer. "w+", "x+" and "a+" open a writer
-- (Lute's std writes files with "w+" and never reads them back), and reading
-- such a handle errors; "r+" can't be offered. link/symlink/watch have no host
-- RPC. Each unsupported case errors with a clear message rather than
-- misbehaving.
local g = _G.__rodeo_procs[_G.__rodeo_eid]
local time = require("@lute/time")

local fs = {}

local function isDirPath(path: string): boolean
	local ok, result = pcall(g.fs.type, path)
	return ok and result == "dir"
end

-- Handles opened with a "+" mode, by mode: rodeo opened them as writers.
local writeOnly = setmetatable({}, { __mode = "k" })
-- What readIntoBuffer read from a handle: lute's leaves the cursor where it
-- was, so the next read still returns it.
local unread = setmetatable({}, { __mode = "k" })

local function checkReadable(handle, fn: string)
	local mode = writeOnly[handle]
	if mode then
		error(`fs.{fn}: this handle was opened with "{mode}", which is write-only under rodeo (open with "r" to read)`, 3)
	end
end

function fs.open(path: string, mode: string?)
	local requested = mode or "r"
	local plus = string.sub(requested, -1) == "+"
	local m = if plus then string.sub(requested, 1, -2) else requested
	if m == "r" and plus then
		error(`fs.open: mode 'r+' is not supported under rodeo (a handle reads or writes, not both)`, 2)
	elseif m == "x" then
		if g.fs.exists(path) then
			error(`fs.open: '{path}' already exists (mode "{requested}")`, 2)
		end
		m = "w"
	elseif m ~= "r" and m ~= "w" and m ~= "a" then
		error(`fs.open: mode '{requested}' is not supported under rodeo (use "r", "w", "w+", "x", "x+", "a" or "a+")`, 2)
	end
	local handle = g.fs.open(path, m)
	if plus then
		writeOnly[handle] = requested
	end
	return handle
end

function fs.read(handle): string
	checkReadable(handle, "read")
	local content = unread[handle]
	if content then
		unread[handle] = nil
		return buffer.tostring(content)
	end
	return buffer.tostring(g.stream.readBytes(handle))
end

function fs.write(handle, contents: string | buffer)
	local buf = if typeof(contents) == "buffer" then contents else buffer.fromstring(contents :: string)
	g.stream.writeBytes(handle, buf)
end

function fs.close(handle)
	g.stream.close(handle)
end

-- No host-side seek: reads the remaining stream once and keeps it, then
-- copies the requested window. fileOffset is absolute from the start of the
-- file, so this is only valid before the handle's first fs.read.
function fs.readIntoBuffer(handle, buf: buffer, fileOffset: number, numBytes: number, bufferOffset: number?): number
	checkReadable(handle, "readIntoBuffer")
	local content = unread[handle] or g.stream.readBytes(handle)
	unread[handle] = content
	local available = buffer.len(content) - fileOffset
	local n = math.max(0, math.min(numBytes, available))
	if n > 0 then
		buffer.copy(buf, bufferOffset or 0, content, fileOffset, n)
	end
	return n
end

function fs.remove(path: string)
	g.fs.remove(path)
end

function fs.stat(path: string)
	local stat = g.fs.stat(path)
	-- The host's int64 fields (size, millisecond timestamps) arrive as strings.
	local function dur(millis: (number | string)?)
		return time.duration.milliseconds(tonumber(millis) or 0)
	end
	return {
		type = stat.type,
		permissions = { readonly = false },
		size = tonumber(stat.size) or 0,
		created = dur(stat.createdMillis),
		accessed = dur(stat.accessedMillis),
		modified = dur(stat.modifiedMillis),
	}
end

function fs.type(path: string): string
	local ok, result = pcall(g.fs.type, path)
	return if ok then result else "unknown"
end

local function parentOf(path: string): string?
	return string.match(path, "^(.*[^/\\])[/\\]+[^/\\]+[/\\]*$")
end

-- lute's mkdir makes one directory: its parent must exist (@std/fs's
-- createDirectory makes the parents itself when asked).
function fs.mkdir(path: string)
	local parent = parentOf(path)
	if parent and not isDirPath(parent) then
		error(`fs.mkdir: no such file or directory: '{parent}'`, 2)
	end
	g.fs.mkdir(path)
end

function fs.link(src: string, dest: string)
	error("fs.link is not supported under rodeo (no host RPC for hard links)", 2)
end

function fs.symlink(src: string, dest: string)
	error("fs.symlink is not supported under rodeo (no host RPC for symlinks)", 2)
end

function fs.watch(path: string, callback)
	error("fs.watch is not supported under rodeo (no host RPC for file watching)", 2)
end

function fs.exists(path: string): boolean
	return g.fs.exists(path)
end

-- lute copy/move handle directories; the host copy is file-only.
function fs.copy(src: string, dest: string)
	if isDirPath(src) then
		g.fs.mkdir(dest)
		for _, entry in g.fs.listdir(src) do
			fs.copy(src .. "/" .. entry.name, dest .. "/" .. entry.name)
		end
	else
		g.fs.copy(src, dest)
	end
end

local function removeTree(path: string)
	for _, entry in g.fs.listdir(path) do
		local child = path .. "/" .. entry.name
		if isDirPath(child) then
			removeTree(child)
		else
			g.fs.remove(child)
		end
	end
	g.fs.rmdir(path)
end

function fs.move(src: string, dest: string)
	fs.copy(src, dest)
	if isDirPath(src) then
		removeTree(src)
	else
		g.fs.remove(src)
	end
end

function fs.listdir(path: string)
	local entries = {}
	for _, entry in g.fs.listdir(path) do
		table.insert(entries, { name = entry.name, type = entry.type })
	end
	return entries
end

-- lute's rmdir removes an empty directory only.
function fs.rmdir(path: string)
	if #g.fs.listdir(path) > 0 then
		error(`fs.rmdir: directory not empty: '{path}'`, 2)
	end
	g.fs.rmdir(path)
end

return fs
