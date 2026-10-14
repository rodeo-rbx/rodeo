--!strict
-- Wire format of the runtime bridge, shared by both ends: the script's side
-- in Studio (connection.luau) and the Lune/Lute process (server.luau). Pure Luau
-- over `buffer`, so it runs unchanged in Studio, Lune and Lute.
--
-- Plain data is copied: nil, booleans, numbers, strings (any bytes), buffers
-- and tables without a metatable (shared and cyclic references are
-- preserved, and so is frozen). Buffers and tables keep their identity: the
-- receiver holds one copy of each, and a copy sent back arrives as the
-- original (while its owner still has it). Each time one crosses, what it
-- arrives as takes the contents it was sent with (a frozen table keeps its
-- own). A value with identity or behavior (a function, userdata, a
-- table with a metatable) stays where it is and travels as a handle: its
-- owner's id for it, with its type name and, when its owner sends it, a
-- snapshot (see Side.describe; a value described earlier in the same message
-- goes without one). The receiving side turns a handle into a proxy, and a
-- proxy sent back travels as the same handle, so the owner gets its value.
-- Value types (a Lute Duration) are rebuilt as the receiver's own value.
--
-- A message is a kind byte followed by a tuple of values (count + values).

local codec = {}

-- Message kinds.
codec.REQUEST = 0
codec.OK = 1
codec.ERROR = 2

local T_NIL = 0
local T_FALSE = 1
local T_TRUE = 2
local T_NUMBER = 3
local T_STRING = 4
local T_BUFFER = 5
local T_TABLE = 6
local T_REF = 7
local T_HANDLE = 8
local T_VALUE = 9

-- How one end represents the other end's values and its own.
export type Side = {
	-- 0 for the runtime process, 1 for Studio; handles carry their owner.
	self: number,
	-- A value of this end that can't be copied: its id (the same id each
	-- time for the same value).
	register: (value: any) -> number,
	-- The value of this end registered as `id`.
	lookup: (id: number) -> any,
	-- The other end's value `id`: this end's proxy for it (the same proxy each
	-- time). `kind` is "function", "userdata" or "table"; `typeName` and
	-- `snapshot` are what the owner's `describe` gave, when it sent them.
	proxy: (id: number, kind: string, typeName: string, snapshot: any?) -> any,
	-- What the other end needs to use one of this end's values without asking
	-- (Studio can't wait for an answer inside a metamethod): its type name
	-- and a snapshot of its fields, tostring and length.
	describe: (value: any) -> (string, any?),
	-- If `value` is a proxy this end made, the other end's id for it.
	proxyId: (value: any) -> number?,
	-- Value types: `encodeValue` names one and gives its numeric fields, or
	-- returns nil for anything else; `decodeValue` rebuilds it.
	encodeValue: (value: any) -> (string?, { number }?),
	decodeValue: (name: string, fields: { number }) -> any,
	-- Which buffers and plain tables are copies of which (codec.identities()).
	identities: Identities,
}

export type Identities = {
	-- This end's id for its own buffer or table `t`.
	id: (t: any) -> number,
	-- This end's own buffer or table `id`, if it's still alive.
	own: (id: number) -> any?,
	-- The other end's id for what `t` is a copy of, if it's a copy.
	copyOf: (t: any) -> number?,
	-- This end's copy of the other end's buffer or table `id`, if it's still alive.
	copy: (id: number) -> any?,
	setCopy: (id: number, t: any) -> (),
}

-- Weak both ways: neither end keeps a value alive for the other.
function codec.identities(): Identities
	local ids: { [any]: number } = setmetatable({}, { __mode = "k" }) :: any
	local byId: { [number]: any } = setmetatable({}, { __mode = "v" }) :: any
	local copies: { [number]: any } = setmetatable({}, { __mode = "v" }) :: any
	local copyIds: { [any]: number } = setmetatable({}, { __mode = "k" }) :: any
	local nextId = 0
	return {
		id = function(t)
			local id = ids[t]
			if id == nil then
				nextId += 1
				id = nextId
				ids[t] = id
				byId[id] = t
			end
			return id
		end,
		own = function(id)
			return byId[id]
		end,
		copyOf = function(t)
			return copyIds[t]
		end,
		copy = function(id)
			return copies[id]
		end,
		setCopy = function(id, t)
			copies[id] = t
			copyIds[t] = id
		end,
	}
end

type Writer = {
	buf: buffer,
	pos: number,
	tables: { [any]: number },
	tableCount: number,
	described: { [any]: boolean },
}
type Reader = { buf: buffer, pos: number }

local function reserve(w: Writer, n: number)
	local size = buffer.len(w.buf)
	if w.pos + n <= size then
		return
	end
	while w.pos + n > size do
		size *= 2
	end
	local grown = buffer.create(size)
	buffer.copy(grown, 0, w.buf, 0, w.pos)
	w.buf = grown
end

local function writeu8(w: Writer, v: number)
	reserve(w, 1)
	buffer.writeu8(w.buf, w.pos, v)
	w.pos += 1
end

local function writeu32(w: Writer, v: number)
	reserve(w, 4)
	buffer.writeu32(w.buf, w.pos, v)
	w.pos += 4
end

local function writef64(w: Writer, v: number)
	reserve(w, 8)
	buffer.writef64(w.buf, w.pos, v)
	w.pos += 8
end

local function writestring(w: Writer, s: string)
	writeu32(w, #s)
	reserve(w, #s)
	buffer.writestring(w.buf, w.pos, s)
	w.pos += #s
end

local function writebuffer(w: Writer, b: buffer)
	local n = buffer.len(b)
	writeu32(w, n)
	reserve(w, n)
	buffer.copy(w.buf, w.pos, b, 0, n)
	w.pos += n
end

-- Which end owns a buffer or table, and that end's id for it.
local function writeidentity(w: Writer, side: Side, value: any)
	local copyOf = side.identities.copyOf(value)
	if copyOf then
		writeu8(w, 1 - side.self)
		writeu32(w, copyOf)
	else
		writeu8(w, side.self)
		writeu32(w, side.identities.id(value))
	end
end

local function kindOf(value: any): string
	local t = type(value)
	return if t == "function" then "function" elseif t == "userdata" then "userdata" else "table"
end

local function encode(w: Writer, side: Side, value: any)
	local t = type(value)
	if value == nil then
		writeu8(w, T_NIL)
	elseif value == false then
		writeu8(w, T_FALSE)
	elseif value == true then
		writeu8(w, T_TRUE)
	elseif t == "number" then
		writeu8(w, T_NUMBER)
		writef64(w, value)
	elseif t == "string" then
		writeu8(w, T_STRING)
		writestring(w, value)
	elseif t == "buffer" then
		writeu8(w, T_BUFFER)
		writeidentity(w, side, value)
		writebuffer(w, value)
	elseif t == "thread" then
		error("a thread can't be passed between Studio and the runtime", 0)
	else
		local remoteId = side.proxyId(value)
		if remoteId then
			writeu8(w, T_HANDLE)
			writeu8(w, 1 - side.self)
			writeu32(w, remoteId)
			writestring(w, kindOf(value))
			writestring(w, "")
			writeu8(w, 0)
			return
		end
		local name, fields = side.encodeValue(value)
		if name then
			writeu8(w, T_VALUE)
			writestring(w, name)
			local list = fields :: { number }
			writeu8(w, #list)
			for _, field in list do
				writef64(w, field)
			end
			return
		end
		if t == "table" and getmetatable(value) == nil then
			local index = w.tables[value]
			if index then
				writeu8(w, T_REF)
				writeu32(w, index)
				return
			end
			local count = 0
			for _ in value do
				count += 1
			end
			w.tableCount += 1
			w.tables[value] = w.tableCount
			writeu8(w, T_TABLE)
			writeidentity(w, side, value)
			writeu8(w, if table.isfrozen(value) then 1 else 0)
			writeu32(w, w.tableCount)
			writeu32(w, count)
			for k, v in value do
				encode(w, side, k)
				encode(w, side, v)
			end
			return
		end
		writeu8(w, T_HANDLE)
		writeu8(w, side.self)
		writeu32(w, side.register(value))
		writestring(w, kindOf(value))
		if w.described[value] then
			-- Its snapshot is being written: it contains itself.
			writestring(w, typeof(value))
			writeu8(w, 0)
			return
		end
		w.described[value] = true
		local typeName, snapshot = side.describe(value)
		writestring(w, typeName)
		if snapshot == nil then
			writeu8(w, 0)
		else
			writeu8(w, 1)
			encode(w, side, snapshot)
		end
	end
end

local function readu8(r: Reader): number
	local v = buffer.readu8(r.buf, r.pos)
	r.pos += 1
	return v
end

local function readu32(r: Reader): number
	local v = buffer.readu32(r.buf, r.pos)
	r.pos += 4
	return v
end

local function readf64(r: Reader): number
	local v = buffer.readf64(r.buf, r.pos)
	r.pos += 8
	return v
end

local function readstring(r: Reader): string
	local n = readu32(r)
	local s = buffer.readstring(r.buf, r.pos, n)
	r.pos += n
	return s
end

-- What a buffer or table that crossed arrives as: this end's own (when it
-- comes back) or its copy of the other end's, nil the first time.
local function existing(side: Side, owner: number, identity: number): any?
	return if owner == side.self then side.identities.own(identity) else side.identities.copy(identity)
end

local function readbuffer(r: Reader, side: Side): buffer
	local owner = readu8(r)
	local identity = readu32(r)
	local n = readu32(r)
	local b = existing(side, owner, identity)
	if b == nil or buffer.len(b) ~= n then
		b = buffer.create(n)
		if owner ~= side.self then
			side.identities.setCopy(identity, b)
		end
	end
	buffer.copy(b, 0, r.buf, r.pos, n)
	r.pos += n
	return b
end

local function decode(r: Reader, side: Side, tables: { [number]: any }): any
	local tag = readu8(r)
	if tag == T_NIL then
		return nil
	elseif tag == T_FALSE then
		return false
	elseif tag == T_TRUE then
		return true
	elseif tag == T_NUMBER then
		return readf64(r)
	elseif tag == T_STRING then
		return readstring(r)
	elseif tag == T_BUFFER then
		return readbuffer(r, side)
	elseif tag == T_TABLE then
		local owner = readu8(r)
		local identity = readu32(r)
		local frozen = readu8(r) == 1
		local index = readu32(r)
		local count = readu32(r)
		-- This end's own table coming back is itself; the other end's is this
		-- end's copy of it. Either takes the contents sent (unless frozen).
		local tbl = existing(side, owner, identity)
		local fill = tbl == nil or not table.isfrozen(tbl)
		if tbl == nil then
			tbl = {}
			if owner ~= side.self then
				side.identities.setCopy(identity, tbl)
			end
		elseif fill then
			table.clear(tbl)
		end
		tables[index] = tbl
		for _ = 1, count do
			local k = decode(r, side, tables)
			local v = decode(r, side, tables)
			if fill and k ~= nil then
				tbl[k] = v
			end
		end
		if fill and frozen then
			table.freeze(tbl)
		end
		return tbl
	elseif tag == T_REF then
		return tables[readu32(r)]
	elseif tag == T_HANDLE then
		local owner = readu8(r)
		local id = readu32(r)
		local kind = readstring(r)
		local typeName = readstring(r)
		local snapshot = if readu8(r) == 1 then decode(r, side, tables) else nil
		if owner == side.self then
			return side.lookup(id)
		end
		return side.proxy(id, kind, typeName, snapshot)
	elseif tag == T_VALUE then
		local name = readstring(r)
		local count = readu8(r)
		local fields = table.create(count)
		for i = 1, count do
			fields[i] = readf64(r)
		end
		return side.decodeValue(name, fields)
	end
	error(`bridge message is corrupt: unknown value tag {tag}`, 0)
end

-- A message: `kind` (REQUEST, OK or ERROR) and `n` values.
function codec.encode(side: Side, kind: number, n: number, ...: any): buffer
	-- One table numbering for the whole message, so a table that appears in
	-- two values arrives as one table.
	local w: Writer = { buf = buffer.create(256), pos = 0, tables = {}, tableCount = 0, described = {} }
	writeu8(w, kind)
	writeu32(w, n)
	for i = 1, n do
		encode(w, side, (select(i, ...)))
	end
	local out = buffer.create(w.pos)
	buffer.copy(out, 0, w.buf, 0, w.pos)
	return out
end

-- The kind, values and value count of a message (from `offset` in `buf`).
function codec.decode(side: Side, buf: buffer, offset: number?): (number, { any }, number)
	local r: Reader = { buf = buf, pos = offset or 0 }
	local kind = readu8(r)
	local n = readu32(r)
	local values = table.create(n)
	local tables = {}
	for i = 1, n do
		values[i] = decode(r, side, tables)
	end
	return kind, values, n
end

return codec
