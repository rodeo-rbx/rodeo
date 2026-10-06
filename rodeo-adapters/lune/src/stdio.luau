-- @lune/stdio shim — maps to rodeo's stream.write(io.stderr/stdout, ...)
local g = _G.__rodeo_procs[_G.__rodeo_eid]

-- Members the shim does not provide (color, style, format, readLine,
-- readToEnd, prompt) raise an error naming them, instead of indexing to nil
-- and failing later as "attempt to call a nil value".
return setmetatable({
	ewrite = function(s: string)
		g.stream.write(g.io.stderr, s)
	end,
	write = function(s: string)
		g.stream.write(g.io.stdout, s)
	end,
}, {
	__index = function(_, key)
		error(`@lune/stdio.{key} is not supported by rodeo's Lune adapter`, 2)
	end,
})
