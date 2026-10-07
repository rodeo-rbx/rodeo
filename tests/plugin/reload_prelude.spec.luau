--!strict
-- Unit spec for rodeo-plugin/src/library/reload_prelude.luau, run under lune by
-- tests/plugin/reloadPrelude.test.ts. Covers where the prelude goes (after
-- the comment header, so hot comments and line numbers survive) and what the
-- injected wrapper does when the module runs: route each require through the
-- run's mapper, or fall back to the real require when there is none.
local luau = require("@lune/luau")
local prelude = require("../../rodeo-plugin/src/library/reload_prelude")

local PRELUDE = prelude.PRELUDE
local failures = 0
local count = 0

local function check(name: string, ok: boolean, detail: string?)
	count += 1
	if not ok then
		failures += 1
		print(`FAIL {name}{if detail then ": " .. detail else ""}`)
	end
end

-- Placement: `before` is the text that must stay ahead of the prelude.
local placements: { { name: string, source: string, before: string } } = {
	{ name = "plain code", source = "return 1", before = "" },
	{ name = "empty source", source = "", before = "" },
	{
		name = "hot comments stay in the header",
		source = "--!strict\n--!native\nreturn 1",
		before = "--!strict\n--!native\n",
	},
	{ name = "leading blank lines", source = "\n\n  return 1", before = "\n\n  " },
	{ name = "block comment", source = "--[[ a\nb ]] return 1", before = "--[[ a\nb ]] " },
	{ name = "leveled block comment", source = "--[==[ ]] ]=] ]==]\nreturn 1", before = "--[==[ ]] ]=] ]==]\n" },
	{
		name = "line comment that looks like a block",
		source = "--[ not a block\nreturn 1",
		before = "--[ not a block\n",
	},
	{ name = "byte order mark", source = "\239\187\191--!strict\nreturn 1", before = "\239\187\191--!strict\n" },
	{ name = "comments only", source = "-- nothing here\n", before = "-- nothing here\n" },
}

for _, case in placements do
	local injected = prelude.inject(case.source)
	local want = case.before .. PRELUDE .. string.sub(case.source, #case.before + 1)
	check(`placement: {case.name}`, injected == want, `got {string.format("%q", injected)}`)
	check(`idempotent: {case.name}`, prelude.inject(injected) == injected)
end

check("prelude has no newline", string.find(PRELUDE, "\n", 1, true) == nil)

-- Running injected modules. `script` and `_G` are stand-ins for the module's
-- own globals; `require` is the real one the wrapper must call.
type Env = { [string]: any }

local function fakeScript(attributes: { [string]: any }): any
	return {
		GetAttribute = function(_self, key: string)
			return attributes[key]
		end,
	}
end

local function run(source: string, env: Env): (boolean, any)
	local fn = luau.load(prelude.inject(source), { debugName = "Module", environment = env })
	return pcall(fn)
end

local stamped = fakeScript({ rodeoPlugin = "build-1", rodeoExecutionId = "abc" })

do
	-- With a mapper: the real require receives what the mapper returned, and
	-- the mapper is told which module is asking.
	local askedBy
	local env: Env = {
		script = stamped,
		require = function(x)
			return "required " .. tostring(x)
		end,
		_G = {
			__rodeo_plugins = {
				["build-1"] = {
					reload = {
						abc = function(x, from)
							askedBy = from
							return "copy of " .. tostring(x)
						end,
					},
				},
			},
		},
	}
	local ok, result = run("--!strict\nlocal RS = 'RS'\nreturn require(RS)", env)
	check("wrapper routes through the mapper", ok and result == "required copy of RS", tostring(result))
	check("mapper is told the requiring module", askedBy == stamped)
end

do
	-- No mapper for this run (a plain run, or the run already finished): the
	-- module keeps the real require.
	local env: Env = {
		script = stamped,
		require = function(x)
			return "required " .. tostring(x)
		end,
		_G = { __rodeo_plugins = { ["build-1"] = { reload = {} } } },
	}
	local ok, result = run("return require('X')", env)
	check("missing mapper falls back to require", ok and result == "required X", tostring(result))

	local unstamped: Env = {
		script = fakeScript({}),
		require = env.require,
		_G = {},
	}
	ok, result = run("return require('X')", unstamped)
	check("unstamped module falls back to require", ok and result == "required X", tostring(result))
end

do
	-- Line numbers are unchanged: an error on line 4 still says line 4.
	local env: Env = { script = fakeScript({}), require = function() end, _G = {}, error = error }
	local ok, err = run("--!strict\n-- comment\n\nerror('boom')", env)
	check("errors keep their line numbers", not ok and string.find(tostring(err), ":4:", 1, true) ~= nil, tostring(err))
end

if failures > 0 then
	error(`{failures} of {count} assertion(s) failed`)
end
print(`ok: {count} assertions`)
