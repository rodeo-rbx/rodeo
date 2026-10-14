--!strict
-- @rodeo-bridge: what the adapter modules (`--adapter`) use to reach the real
-- Lune or Lute. Their libraries run in a process the run client starts for
-- this run; calls travel over the run's RPC connection (`adapter.call`), so
-- Studio opens no socket of its own. See connection.luau for how the
-- runtime's values look on this side.

local newConnection = require("@self/connection")

local bridge = {}

-- This run's rodeo APIs, as the @rodeo shims find them.
function bridge.apis(): any
	local plugins, key = _G.__rodeo_plugins, script and script:GetAttribute("rodeoPlugin")
	local ns = plugins and key and plugins[key]
	return if ns then ns.procs[script:GetAttribute("rodeoExecutionId")] else _G.__rodeo_procs[_G.__rodeo_eid]
end

-- Value types the runtimes send by value (a Lute Duration), registered by the
-- adapter module that owns the local type.
local valueTypes: { newConnection.ValueType } = {}

function bridge.valueType(valueType: newConnection.ValueType)
	table.insert(valueTypes, valueType)
end

local connections: { [string]: newConnection.Connection } = {}

local function connection(runtime: string): newConnection.Connection
	local existing = connections[runtime]
	if existing then
		return existing
	end
	local adapter = bridge.apis().adapter
	local created = newConnection(function(payload: buffer): buffer
		return adapter.call(runtime, payload)
	end, valueTypes)
	connections[runtime] = created
	return created
end

-- The runtime's module `name` (e.g. "@lute/fs"), as the real runtime returns it.
function bridge.module(runtime: string, name: string): any
	return connection(runtime).module(name)
end

-- Calls the runtime function `f`; returns the argument at `index` as the
-- runtime left it, then f's results. For functions that fill a buffer they're
-- given (arguments cross as copies).
function bridge.callOut(runtime: string, f: any, index: number, ...: any): ...any
	return connection(runtime).callOut(f, index, ...)
end

-- A table holding the keys and values of `remote`, a runtime table that
-- arrived as a proxy (Lute's process.env); writes to it are made on `remote`
-- too, before the script's next call to the runtime.
function bridge.mirror(runtime: string, remote: any): { [any]: any }
	return connection(runtime).mirror(remote)
end

return bridge
