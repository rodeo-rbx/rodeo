-- @lute/fs shim — maps lute's handle-based fs onto rodeo's fs + stream RPCs.
-- Timestamps in stat are @lute/time Durations (since the Unix epoch), per
-- lute's FileMetadata.
--
-- Unsupported here: link/symlink/watch (no host RPC) and the read+write
-- handle modes (host handles are a reader OR a writer). Each errors with a
-- clear message rather than misbehaving.
--
-- NOTE: inlined as an IIFE with comments stripped (bundle.rs inline_shims).
-- No `--` inside string literals.
local g = _G.__rodeo_procs[_G.__rodeo_eid]
local time = require("@lute/time")

local fs = {}

local function isDirPath(path: string): boolean
	local ok, result = pcall(g.fs.type, path)
	return ok and result == "dir"
end

function fs.open(path: string, mode: string?)
	local m = mode or "r"
	if m == "x" then
		if g.fs.exists(path) then
			error(`fs.open: '{path}' already exists (mode "x")`, 2)
		end
		m = "w"
	elseif m ~= "r" and m ~= "w" and m ~= "a" then
		error(`fs.open: mode '{m}' is not supported under rodeo (handles are read OR write: use "r", "w", "x", or "a")`, 2)
	end
	return g.fs.open(path, m)
end

function fs.read(handle): string
	return buffer.tostring(g.stream.readBytes(handle))
end

function fs.write(handle, contents: string | buffer)
	local buf = if typeof(contents) == "buffer" then contents else buffer.fromstring(contents :: string)
	g.stream.writeBytes(handle, buf)
end

function fs.close(handle)
	g.stream.close(handle)
end

-- No host-side seek: reads the remaining stream once, then copies the
-- requested window. fileOffset is absolute from the start of the file, so
-- this is only valid as the handle's first read.
function fs.readIntoBuffer(handle, buf: buffer, fileOffset: number, numBytes: number, bufferOffset: number?): number
	local content = g.stream.readBytes(handle)
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
	local function dur(millis: number?)
		return time.duration.milliseconds(millis or 0)
	end
	return {
		type = stat.type,
		permissions = { readonly = false },
		size = stat.size or 0,
		created = dur(stat.createdMillis),
		accessed = dur(stat.accessedMillis),
		modified = dur(stat.modifiedMillis),
	}
end

function fs.type(path: string): string
	local ok, result = pcall(g.fs.type, path)
	return if ok then result else "unknown"
end

function fs.mkdir(path: string)
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

function fs.rmdir(path: string)
	removeTree(path)
end

return fs
