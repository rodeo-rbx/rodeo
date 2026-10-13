-- @lune/serde shim — maps to HttpService JSON encode/decode
local HttpService = game:GetService("HttpService")

-- Members the shim does not provide (compress, decompress, hash, hmac)
-- raise an error naming them, instead of indexing to nil and failing later
-- as "attempt to call a nil value".
return setmetatable({
	encode = function(format: string, value: any): string
		if format ~= "json" then
			error(`@lune/serde.encode format "{format}" is not supported by rodeo's Lune adapter (only "json" is)`, 2)
		end
		return HttpService:JSONEncode(value)
	end,
	decode = function(format: string, data: string): any
		if format ~= "json" then
			error(`@lune/serde.decode format "{format}" is not supported by rodeo's Lune adapter (only "json" is)`, 2)
		end
		return HttpService:JSONDecode(data)
	end,
}, {
	__index = function(_, key)
		error(`@lune/serde.{key} is not supported by rodeo's Lune adapter`, 2)
	end,
})
