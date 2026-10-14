--!strict
-- The runtime end of the bridge, run by `lune run` or `lute run` for one
-- `rodeo run`. It dials the run client at $RODEO_BRIDGE_URL and serves the
-- requests of the script in Studio against this runtime's real libraries,
-- each in its own thread, so a call that waits (a request, a child process)
-- doesn't hold up the others.
--
-- Frames are `[u32 request id][message]` both ways (see codec.luau), after a
-- hello: a frame of id 0 alone, once this end's socket is open, before which
-- the run client sends nothing (Lute's client loses a frame that arrives with
-- the handshake). Requests:
--   require(name)               require(name)
--   call(f, ...)                f(...)
--   method(obj, key, ...)       obj:key(...), with obj sent back first so its
--                               snapshot refreshes
--   callout(f, index, ...)      f(...), and the index-th argument after the
--                               call (a buffer the function filled)
--   index(obj, key)             obj[key]
--   newindex(obj, key, value)   obj[key] = value
--   meta(name, a, b)            the operator `name` on a (and b)
--   pairs(obj)                  a copy of obj's keys and values
--   with(writes, op, ...)       the writes Studio made to this end's tables
--                               ({ table, key, value }), then the request op
--   poll()                      the next call this end makes to one of
--                               Studio's functions: (job, f, ...), once there
--                               is one
--   resolve(job, ok, ...)       that call's outcome: pcall's results

local codec = require("./codec")

local isLune, luneProcess = pcall(require, "@lune/process")
local isLute, luteProcess = pcall(require, "@lute/process")
assert(isLune or isLute, "the rodeo bridge runs under lune or lute")

local task: any = if isLune then require("@lune/task") else require("@lute/task")
local env: { [string]: string? } = if isLune then luneProcess.env else luteProcess.env
local url = env.RODEO_BRIDGE_URL
assert(url, "RODEO_BRIDGE_URL is not set")
env.RODEO_BRIDGE_URL = nil

-- Lute's Durations are a value type: they arrive as the receiver's own.
local duration: any = if isLute then require("@lute/time").duration else nil

-- This end's values that Studio holds handles to, by id (kept for the run:
-- Studio has no finalizers to say when it lets go).
local objects: { [number]: any } = {}
local objectIds: { [any]: number } = setmetatable({}, { __mode = "k" }) :: any
local nextId = 0
-- Studio's values this end holds: functions Studio passed in, by id.
local proxies: { [number]: any } = {}
local proxyIds: { [any]: number } = setmetatable({}, { __mode = "k" }) :: any

-- What Studio's proxy for each of this runtime's object types answers on its
-- own: the fields it reads (and refreshes with every method call), its
-- length, and the field that orders it. Anything else is a method. Keyed by
-- typeof, up to any "<...>" (Lune names its websocket "Websocket<WsStream>").
type Schema = { fields: { string }?, len: boolean?, order: string? }
local schemas: { [string]: Schema } = {
	-- @lune/regex
	RegexMatch = { fields = { "start", "finish", "len", "text" }, len = true },
	RegexCaptures = { len = true },
	-- @lune/datetime
	DateTime = { fields = { "unixTimestamp", "unixTimestampMillis" }, order = "unixTimestampMillis" },
	-- @lune/process
	Child = { fields = { "stdin", "stdout", "stderr" } },
	-- @lune/net
	Response = { fields = { "ok", "statusCode", "statusMessage", "headers", "body" } },
	Request = { fields = { "ip", "port", "method", "path", "query", "headers", "body" } },
	Tcp = { fields = { "localIp", "localPort", "remoteIp", "remotePort" } },
	Websocket = { fields = { "closeCode" } },
}

-- A table with a metatable crosses as a copy of its contents, with what its
-- iteration gives (when __iter makes that differ), the methods its __index
-- tables hold, and what its __tostring, __len and __call give. `open` marks
-- one whose members can't be listed (a locked metatable or an __index
-- function): any other key may be a method.
local function describeTable(value: { [any]: any }): any
	local fields = {}
	for k, v in next, value do
		fields[k] = v
	end
	local meta = getmetatable(value)
	local snapshot: { [string]: any } = { fields = fields, frozen = table.isfrozen(value) }
	if type(meta) ~= "table" then
		snapshot.open = true
		return snapshot
	end
	if rawget(meta, "__iter") ~= nil then
		local iteration = {}
		for k, v in value do
			table.insert(iteration, { k, v })
		end
		snapshot.iteration = iteration
	end
	local methods = {}
	local index = rawget(meta, "__index")
	for _ = 1, 8 do
		if type(index) ~= "table" then
			break
		end
		for k, v in index do
			if type(v) == "function" and fields[k] == nil then
				methods[k] = true
			end
		end
		local indexMeta = getmetatable(index)
		index = if type(indexMeta) == "table" then rawget(indexMeta, "__index") else nil
	end
	snapshot.methods = methods
	snapshot.open = type(index) == "function" and rawget(meta, "__iter") == nil
	if rawget(meta, "__tostring") ~= nil then
		snapshot.tostring = tostring(value)
	end
	if rawget(meta, "__len") ~= nil then
		snapshot.len = #value
	end
	snapshot.callable = rawget(meta, "__call") ~= nil
	return snapshot
end

local function describe(value: any): (string, any?)
	local typeName = typeof(value)
	if type(value) == "table" then
		return typeName, describeTable(value)
	end
	local snapshot: { [string]: any } = {}
	local schema = schemas[string.match(typeName, "^[^<]+") or typeName]
	local fields, fieldNames = {}, {}
	if schema then
		for _, field in schema.fields or {} do
			fieldNames[field] = true
			fields[field] = value[field]
		end
		if schema.len then
			snapshot.len = #value
		end
		if schema.order then
			snapshot.order = value[schema.order]
		end
	end
	snapshot.fields = fields
	snapshot.fieldNames = fieldNames
	local ok, text = pcall(tostring, value)
	if ok then
		snapshot.tostring = text
	end
	return typeName, snapshot
end

-- Calls to Studio's functions: Studio long-polls for them (ops.poll), runs
-- each, and hands back its outcome (ops.resolve), which resumes the caller.
local calls: { any } = {}
local pollers: { thread } = {}
local callers: { [number]: thread } = {}
local nextJob = 0

local function callStudio(f: any, ...: any): ...any
	if not coroutine.isyieldable() then
		error("a Studio function can't be called here: the runtime calls it where it can't wait for Studio", 2)
	end
	nextJob += 1
	local job = nextJob
	callers[job] = coroutine.running()
	local call = table.pack(job, f, ...)
	local poller = table.remove(pollers, 1)
	if poller then
		task.spawn(poller, call)
	else
		table.insert(calls, call)
	end
	local outcome = table.pack(coroutine.yield())
	if outcome[1] then
		return table.unpack(outcome, 2, outcome.n)
	end
	error(outcome[2], 0)
end

local function studioValue(id: number, kind: string): any
	if kind == "function" then
		local f
		f = function(...)
			return callStudio(f, ...)
		end
		return f
	end
	local value = newproxy(true)
	getmetatable(value).__index = function()
		error("a Studio value can't be used in the runtime", 2)
	end
	return value
end

local side: codec.Side = {
	self = 0,
	identities = codec.identities(),
	register = function(value)
		local id = objectIds[value]
		if id then
			return id
		end
		nextId += 1
		objects[nextId] = value
		objectIds[value] = nextId
		return nextId
	end,
	lookup = function(id)
		return objects[id]
	end,
	proxy = function(id, kind)
		local value = proxies[id]
		if value == nil then
			value = studioValue(id, kind)
			proxies[id] = value
			proxyIds[value] = id
		end
		return value
	end,
	proxyId = function(value)
		return proxyIds[value]
	end,
	describe = describe,
	encodeValue = function(value)
		if duration and typeof(value) == "duration" then
			-- tostring is exact ("sec.nnnnnnnnn"); toSeconds would round.
			local sec, nsec = string.match(tostring(value), "^(%d+)%.(%d+)$")
			return "Duration", { tonumber(sec) :: number, tonumber(nsec) :: number }
		end
		return nil
	end,
	decodeValue = function(name, fields)
		if name == "Duration" and duration then
			return duration.create(fields[1], fields[2])
		end
		error(`unknown value type '{name}' from Studio`, 0)
	end,
}

local binary: { [string]: (any, any) -> any } = {
	__add = function(a, b)
		return a + b
	end,
	__sub = function(a, b)
		return a - b
	end,
	__mul = function(a, b)
		return a * b
	end,
	__div = function(a, b)
		return a / b
	end,
	__idiv = function(a, b)
		return a // b
	end,
	__mod = function(a, b)
		return a % b
	end,
	__pow = function(a, b)
		return a ^ b
	end,
	__concat = function(a, b)
		return a .. b
	end,
	__eq = function(a, b)
		return a == b
	end,
	__lt = function(a, b)
		return a < b
	end,
	__le = function(a, b)
		return a <= b
	end,
	__unm = function(a)
		return -a
	end,
	__len = function(a)
		return #a
	end,
	__tostring = function(a)
		return tostring(a)
	end,
}

local ops: { [string]: (...any) -> ...any } = {}

function ops.require(name: string)
	return require(name)
end

function ops.call(f, ...)
	return f(...)
end

function ops.method(obj, key, ...)
	return obj, obj[key](obj, ...)
end

function ops.callout(f, index: number, ...)
	local args = table.pack(...)
	local results = table.pack(f(...))
	return args[index], table.unpack(results, 1, results.n)
end

function ops.index(obj, key)
	return obj[key]
end

function ops.newindex(obj, key, value)
	obj[key] = value
end

function ops.meta(name: string, a, b)
	local operator = binary[name]
	if not operator then
		error(`unknown operator '{name}'`, 0)
	end
	return operator(a, b)
end

function ops.pairs(obj)
	local copy = {}
	for k, v in obj do
		copy[k] = v
	end
	return copy
end

local function dispatch(op: any, ...: any): ...any
	local f = ops[op]
	if not f then
		error(`unknown bridge request '{tostring(op)}'`, 0)
	end
	return f(...)
end

function ops.with(writes: { { any } }, op, ...)
	for _, write in writes do
		write[1][write[2]] = write[3]
	end
	return dispatch(op, ...)
end

function ops.poll()
	local call = table.remove(calls, 1)
	if call == nil then
		table.insert(pollers, coroutine.running())
		call = coroutine.yield()
	end
	return table.unpack(call, 1, call.n)
end

function ops.resolve(job: number, ...)
	local caller = callers[job]
	callers[job] = nil
	-- Resumed by the runtime's scheduler, so whatever waits on the thread
	-- finishing (Lute sends an HTTP handler's response then) sees it finish.
	-- A thread the runtime dropped meanwhile is left alone: Lute resets a
	-- websocket upgrade handler that waits.
	if caller and coroutine.status(caller) == "suspended" then
		task.defer(caller, ...)
	end
end

-- Errors raised by a runtime function carry this file's position as their
-- caller, and Lune's a traceback of its own scheduler; the script in Studio
-- called it, so keep only the message.
local function message(err: any): string
	local text = if type(err) == "string" then err else tostring(err)
	text = string.gsub(text, "^[^\n]-server%.luau:%d+: ", "")
	return (string.gsub(text, "\nstack traceback:.*$", ""))
end

local send: (frame: buffer) -> ()

local function serve(frame: buffer)
	local id = buffer.readu32(frame, 0)
	local ok, reply = pcall(function()
		local _, values, n = codec.decode(side, frame, 4)
		local results = table.pack(dispatch(table.unpack(values, 1, n)))
		return codec.encode(side, codec.OK, results.n, table.unpack(results, 1, results.n))
	end)
	if not ok then
		reply = codec.encode(side, codec.ERROR, 1, message(reply))
	end
	local out = buffer.create(4 + buffer.len(reply))
	buffer.writeu32(out, 0, id)
	buffer.copy(out, 4, reply)
	send(out)
end

-- Tells the run client this end is ready for requests.
local function hello()
	send(buffer.create(4))
end

local function asBuffer(message: string | buffer): buffer
	return if type(message) == "string" then buffer.fromstring(message) else message
end

if isLune then
	local socket = require("@lune/net").socket(url)
	send = function(frame)
		socket:send(frame, true)
	end
	hello()
	while true do
		local message = socket:next()
		if message == nil then
			break
		end
		task.spawn(serve, asBuffer(message))
	end
	-- rodeo is gone: end, even with servers or timers still running.
	luneProcess.exit(0)
else
	-- Lute calls these from outside any script thread, and a thread started
	-- there can't require (it has no requiring context), so they only queue
	-- the message for a thread started here, which serves it.
	local inbox: { buffer } = {}
	local closed = false
	local waiting: thread? = nil
	local function wake()
		local thread = waiting
		waiting = nil
		if thread then
			task.spawn(thread)
		end
	end
	local socket = require("@lute/net/client").websocket(url, {
		onmessage = function(message)
			table.insert(inbox, asBuffer(message))
			wake()
		end,
		onclose = function()
			closed = true
			wake()
		end,
		onerror = function()
			closed = true
			wake()
		end,
		onopen = function()
			hello()
		end,
	})
	send = function(frame)
		socket:send(frame)
	end
	task.spawn(function()
		while not closed do
			local message = table.remove(inbox, 1)
			if message then
				task.spawn(serve, message)
			else
				waiting = coroutine.running()
				coroutine.yield()
			end
		end
		-- rodeo is gone: end, even with servers or timers still running.
		luteProcess.exit(0)
	end)
end
