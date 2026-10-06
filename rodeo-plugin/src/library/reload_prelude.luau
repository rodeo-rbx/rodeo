--!strict
-- The `require` wrapper that --reload-requires puts at the top of every module
-- a run loads from its own copy. Pure string handling: no Roblox globals, so
-- it runs under lune (tests/plugin/reload_prelude.spec.luau) as well as in
-- Studio.
--
-- A reload run swaps fresh copies in for the modules it finds by evaluating
-- require(...) arguments ahead of time (require_resolver.luau). An argument
-- that reads a local (`local RS = game:GetService("ReplicatedStorage")` then
-- `require(RS.Foo)`) or is computed can't be evaluated outside its module, so
-- those requires used to reach the cached module silently (issue #33). The
-- wrapper settles them when they're called: it hands the argument to this
-- run's mapper (published by the runner, found through the run id and plugin
-- key stamped on the module), which swaps in a fresh copy of a module the run
-- hasn't copied yet and returns it. String paths go back to the real
-- `require`, called from inside the module so they still resolve relative to
-- it; the mapper only swaps the copy in first.
--
-- The text has no newline and goes after the leading comments, so line
-- numbers in errors are unchanged and `--!strict` / `--!native` /
-- `--!optimize` stay in the header, where Luau reads them.

local PRELUDE = "local require = (function(r, s) "
	.. "local p = _G.__rodeo_plugins "
	.. 'local n = p and p[s:GetAttribute("rodeoPlugin")] '
	.. 'local m = n and n.reload and n.reload[s:GetAttribute("rodeoExecutionId")] '
	.. "if not m then return r end "
	.. "return function(x) return r(m(x, s)) end "
	.. "end)(require, script);"

-- Byte index where `source`'s code starts: past a UTF-8 byte order mark,
-- whitespace, and line and block comments. #source + 1 when there is no code.
local function codeStart(source: string): number
	local n = #source
	local i = 1
	if string.sub(source, 1, 3) == "\239\187\191" then
		i = 4
	end
	while i <= n do
		local space = string.match(source, "^[ \t\r\n\f\v]+", i)
		if space then
			i += #space
		elseif string.sub(source, i, i + 1) == "--" then
			local level = string.match(source, "^%[(=*)%[", i + 2)
			if level then
				local close = "]" .. level .. "]"
				local stop = string.find(source, close, i + 4 + #level, true)
				if not stop then
					return n + 1
				end
				i = stop + #close
			else
				local newline = string.find(source, "\n", i + 2, true)
				if not newline then
					return n + 1
				end
				i = newline + 1
			end
		else
			break
		end
	end
	return i
end

-- `source` with the prelude in front of its first statement. Idempotent: a
-- copy of a copy (two reload runs swapping the same module) keeps one prelude.
local function inject(source: string): string
	local at = codeStart(source)
	if string.sub(source, at, at + #PRELUDE - 1) == PRELUDE then
		return source
	end
	return string.sub(source, 1, at - 1) .. PRELUDE .. string.sub(source, at)
end

return {
	PRELUDE = PRELUDE,
	codeStart = codeStart,
	inject = inject,
}
