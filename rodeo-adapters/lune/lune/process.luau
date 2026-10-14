-- @lune/process: Lune's own, from the Lune process behind this run
-- (`--adapter lune`), with this run's args and exit. `create` returns at once,
-- as Lune's does: the call that starts the child runs on its own thread, and
-- the child's members wait for it. A child that fails to start raises from
-- the first member called (and is reported on stderr).
local bridge = require("@rodeo-bridge")
local lune = bridge.module("lune", "@lune/process")
local rodeo = bridge.apis()

local function create(program: string, params: { string }?, options: { [string]: any }?)
	local child, failure
	local waiting: { thread } = {}
	task.spawn(function()
		local ok, result = pcall(lune.create, program, params, options)
		if ok then
			child = result
		else
			failure = result
			rodeo.stream.write(rodeo.io.stderr, `@lune/process.create failed: {result}\n`)
		end
		for _, thread in waiting do
			task.spawn(thread)
		end
	end)
	local function started()
		if child == nil and failure == nil then
			table.insert(waiting, coroutine.running())
			coroutine.yield()
		end
		if failure ~= nil then
			error(failure, 3)
		end
		return child
	end

	local function reader(name: string)
		return {
			read = function(_, chunkSize: number?): string?
				return started()[name]:read(chunkSize)
			end,
			readToEnd = function(_): string
				return started()[name]:readToEnd()
			end,
		}
	end

	return {
		stdin = {
			write = function(_, data: buffer | string)
				started().stdin:write(data)
			end,
			close = function(_)
				started().stdin:close()
			end,
		},
		stdout = reader("stdout"),
		stderr = reader("stderr"),
		kill = function(_)
			started():kill()
		end,
		status = function(_)
			return started():status()
		end,
	}
end

local process = table.clone(lune)
process.args = table.freeze(table.clone(rodeo.process.args))
process.exit = rodeo.process.exit
process.create = create
return table.freeze(process)
