--!strict
-- The Studio end of one runtime's bridge, over a `transport` that takes a
-- request message and returns the reply (in a run: the adapter RPC).
--
-- The runtime's values arrive as proxies, which never wait inside a
-- metamethod (Roblox doesn't allow it: "thread is not yieldable"). A function
-- becomes a function that calls it. A table with a metatable becomes a table
-- holding its contents, whose other members are its methods. A userdata
-- becomes a userdata whose fields, tostring, length and order come from the
-- snapshot the runtime sends with it, and any other key is a method. Both are
-- refreshed whenever they cross again (every method call sends its object
-- back), and a method call is an ordinary call that may wait.
--
-- Studio's functions the runtime is given are called back over a long poll:
-- while the runtime holds any, a thread here waits on a `poll` request, which
-- the runtime answers with each call it makes to one.

local codec = require("./codec")

export type ValueType = {
	name: string,
	is: (value: any) -> boolean,
	fields: (value: any) -> { number },
	new: (fields: { number }) -> any,
}

export type Connection = {
	request: (op: string, ...any) -> ...any,
	module: (name: string) -> any,
	-- Calls `f` with `...`; returns the argument at `index` as the runtime
	-- left it (a buffer `f` filled), then `f`'s results.
	callOut: (f: any, index: number, ...any) -> ...any,
	-- A table holding a runtime table's keys and values; writes to it are made
	-- on the runtime's table too.
	mirror: (remote: any) -> { [any]: any },
}

type Snapshot = {
	fields: { [any]: any }?,
	fieldNames: { [any]: boolean }?,
	tostring: string?,
	len: number?,
	order: any?,
}

local function new(transport: (payload: buffer) -> buffer, valueTypes: { ValueType }): Connection
	local objects: { [number]: any } = {}
	local objectIds: { [any]: number } = setmetatable({}, { __mode = "k" }) :: any
	local nextId = 0
	local proxies: { [number]: any } = {}
	local proxyIds: { [any]: number } = setmetatable({}, { __mode = "k" }) :: any
	local snapshots: { [any]: Snapshot } = setmetatable({}, { __mode = "k" }) :: any
	-- Writes to runtime tables not yet made there: { table, key, value }.
	local writes: { { any } } = {}
	local polling = false

	local side: codec.Side
	local startPolling: () -> ()

	local function request(op: string, ...: any): ...any
		local message
		if #writes > 0 then
			local pending = writes
			writes = {}
			message = codec.encode(side, codec.REQUEST, select("#", ...) + 3, "with", pending, op, ...)
		else
			message = codec.encode(side, codec.REQUEST, select("#", ...) + 1, op, ...)
		end
		local kind, values, n = codec.decode(side, transport(message))
		if kind == codec.ERROR then
			error(values[1], 0)
		end
		return table.unpack(values, 1, n)
	end

	-- One set of metamethods for every proxy: Luau compares two values with
	-- __eq/__lt/__le only when both have the same function.
	local states: { [any]: { typeName: string, kind: string, methods: { [any]: (...any) -> ...any } } } =
		setmetatable({}, { __mode = "k" }) :: any

	-- The function calling method `key` of the object it's called on.
	local function stub(stubs: { [any]: any }, key: any): any
		local method = stubs[key]
		if method == nil then
			method = function(self, ...)
				-- The object comes back first, refreshing it.
				return select(2, request("method", self, key, ...))
			end
			stubs[key] = method
		end
		return method
	end

	local function index(proxy: any, key: any): any
		local snapshot = snapshots[proxy]
		if snapshot.fieldNames and snapshot.fieldNames[key] then
			return (snapshot.fields :: any)[key]
		end
		return stub(states[proxy].methods, key)
	end

	local function typeNameOf(value: any): string
		local state = states[value]
		return if state then state.typeName else typeof(value)
	end

	local function order(value: any): any
		local snapshot = snapshots[value]
		local key = snapshot and snapshot.order
		if key == nil then
			error(`attempt to compare {typeNameOf(value)} values`, 3)
		end
		return key
	end

	local metamethods = {
		__index = index,
		__newindex = function(proxy, key)
			error(`can't set '{tostring(key)}' on a {typeNameOf(proxy)} from Studio`, 2)
		end,
		__iter = function(proxy)
			return next, snapshots[proxy].fields or {}
		end,
		__tostring = function(proxy)
			return snapshots[proxy].tostring or typeNameOf(proxy)
		end,
		__len = function(proxy)
			local len = snapshots[proxy].len
			if len == nil then
				error(`attempt to get length of a {typeNameOf(proxy)} value`, 2)
			end
			return len
		end,
		__eq = function(a, b)
			local sa, sb = snapshots[a], snapshots[b]
			return sa ~= nil and sb ~= nil and sa.order ~= nil and sa.order == sb.order
		end,
		__lt = function(a, b)
			return order(a) < order(b)
		end,
		__le = function(a, b)
			return order(a) <= order(b)
		end,
		__call = function(proxy, ...)
			return request("call", proxy, ...)
		end,
	}

	local function object(kind: string, typeName: string): any
		local proxy = newproxy(true)
		local meta = getmetatable(proxy)
		for name, f in metamethods do
			meta[name] = f
		end
		meta.__metatable = "The metatable is locked"
		states[proxy] = { typeName = typeName, kind = kind, methods = {} }
		return proxy
	end

	type TableState = {
		methods: { [any]: boolean },
		open: boolean,
		stubs: { [any]: any },
		tostring: string?,
		len: number?,
		-- What iterating it gives, as { key, value } pairs, and those values
		-- by key, when the runtime's __iter makes that differ from its contents.
		iteration: { { any } }?,
		iterated: { [any]: any },
	}
	local tableStates: { [any]: TableState } = setmetatable({}, { __mode = "k" }) :: any

	-- A table proxy's contents become the snapshot's. A frozen one stays as
	-- it first arrived: the runtime's can't change either.
	local function fill(t: any, snapshot: any)
		local fields = snapshot.fields or {}
		for k in next, t do
			if fields[k] == nil then
				rawset(t, k, nil)
			end
		end
		for k, v in fields do
			rawset(t, k, v)
		end
		local state = tableStates[t]
		state.methods = snapshot.methods or {}
		state.open = snapshot.open == true
		state.tostring = snapshot.tostring
		state.len = snapshot.len
		state.iteration = snapshot.iteration
		state.iterated = {}
		for _, pair in snapshot.iteration or {} do
			state.iterated[pair[1]] = pair[2]
		end
		if snapshot.frozen then
			table.freeze(t)
		end
	end

	local function newTable(snapshot: any): any
		local state: TableState = { methods = {}, open = false, stubs = {}, iterated = {} }
		local meta: { [string]: any } = {
			__index = function(_, key)
				local value = state.iterated[key]
				if value ~= nil then
					return value
				elseif state.methods[key] or state.open then
					return stub(state.stubs, key)
				end
				return nil
			end,
		}
		if snapshot and snapshot.iteration ~= nil then
			meta.__iter = function()
				local i = 0
				local iteration = state.iteration or {}
				return function()
					i += 1
					local pair = iteration[i]
					if pair then
						return pair[1], pair[2]
					end
					return nil
				end
			end
		end
		if snapshot and snapshot.tostring ~= nil then
			meta.__tostring = function()
				return state.tostring
			end
		end
		if snapshot and snapshot.len ~= nil then
			meta.__len = function()
				return state.len
			end
		end
		if snapshot and snapshot.callable then
			meta.__call = function(t, ...)
				return request("call", t, ...)
			end
		end
		local t = setmetatable({}, meta)
		tableStates[t] = state
		return t
	end

	side = {
		self = 1,
		identities = codec.identities(),
		register = function(value)
			local id = objectIds[value]
			if id then
				return id
			end
			nextId += 1
			objects[nextId] = value
			objectIds[value] = nextId
			if type(value) == "function" then
				startPolling()
			end
			return nextId
		end,
		lookup = function(id)
			return objects[id]
		end,
		proxy = function(id, kind, typeName, snapshot)
			local proxy = proxies[id]
			if proxy == nil then
				if kind == "function" then
					local f
					f = function(...)
						return request("call", f, ...)
					end
					proxy = f
				elseif kind == "table" then
					proxy = newTable(snapshot)
				else
					proxy = object(kind, if typeName ~= "" then typeName else kind)
					snapshots[proxy] = {}
				end
				proxies[id] = proxy
				proxyIds[proxy] = id
			end
			if snapshot ~= nil then
				if kind == "table" then
					if not table.isfrozen(proxy) then
						fill(proxy, snapshot)
					end
				elseif kind ~= "function" then
					snapshots[proxy] = snapshot
				end
			end
			return proxy
		end,
		proxyId = function(value)
			return proxyIds[value]
		end,
		describe = function(value)
			return typeof(value), nil
		end,
		encodeValue = function(value)
			for _, valueType in valueTypes do
				if valueType.is(value) then
					return valueType.name, valueType.fields(value)
				end
			end
			return nil
		end,
		decodeValue = function(name, fields)
			for _, valueType in valueTypes do
				if valueType.name == name then
					return valueType.new(fields)
				end
			end
			error(`the runtime sent a {name}, which this script can't represent`, 0)
		end,
	}

	-- Answers the runtime's calls to Studio's functions, each on its own
	-- thread, until the bridge stops (the run ended).
	startPolling = function()
		if polling then
			return
		end
		polling = true
		task.spawn(function()
			while true do
				local ok, call = pcall(function()
					return table.pack(request("poll"))
				end)
				if not ok then
					return
				end
				task.spawn(function()
					local results = table.pack(pcall(call[2], table.unpack(call, 3, call.n)))
					pcall(request, "resolve", call[1], table.unpack(results, 1, results.n))
				end)
			end
		end)
	end

	return {
		request = request,
		module = function(name: string): any
			return request("require", name)
		end,
		callOut = function(f: any, index: number, ...: any): ...any
			return request("callout", f, index, ...)
		end,
		mirror = function(remote: any): { [any]: any }
			local values = request("pairs", remote)
			return setmetatable({}, {
				__index = values,
				__newindex = function(_, key, value)
					values[key] = value
					table.insert(writes, { remote, key, value })
				end,
				__iter = function()
					return next, values
				end,
			})
		end,
	}
end

return new
