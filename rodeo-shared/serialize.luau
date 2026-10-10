-- Luau table serializer: converts normalized values to Luau source code
-- Returns the serializeToLuau function

local EncodingService = game:GetService("EncodingService")

-- The escape for every byte, as gsub's replacement table. Luau strings are
-- bytes and the file must require back to the same ones: control bytes aren't
-- valid inside a literal (a raw NUL reads as a malformed string), and bytes
-- that aren't UTF-8 can't cross rodeo's UTF-8-only file transport.
local ESCAPES = {}
for byte = 0, 255 do
	ESCAPES[string.char(byte)] = string.format("\\x%02X", byte)
end
ESCAPES["\\"] = "\\\\"
ESCAPES['"'] = '\\"'
ESCAPES["\n"] = "\\n"
ESCAPES["\t"] = "\\t"
ESCAPES["\r"] = "\\r"

-- Printable ASCII other than `"` and `\` is written as is, as is UTF-8 text
-- beyond ASCII, so it stays readable. A string that isn't valid UTF-8 has
-- every byte outside printable ASCII escaped. In the classes, `]` comes first
-- (where it's literal) and the ranges are ]-~, space-!, #-[ and 0x80-0xFF.
local KEEP_TEXT = "[^]-~ -!#-[\128-\255]"
local KEEP_ASCII = "[^]-~ -!#-[]"
local function escapeString(str: string): string
	return (string.gsub(str, if utf8.len(str) then KEEP_TEXT else KEEP_ASCII, ESCAPES))
end

local function getIndent(depth)
	return string.rep("\t", depth)
end

local function isArray(t)
	local count = 0
	for _ in t do
		count += 1
	end
	for i = 1, count do
		if t[i] == nil then
			return false
		end
	end
	return true
end

local function getSortedKeys(t)
	local keys = {}
	for k in t do
		table.insert(keys, k)
	end
	-- Preserve original key types — stringifying here loses integer keys, which
	-- then mis-index the table later as `value[tostring(k)]` (= nil), dropping
	-- the value. Sort deterministically: group by type name, then by value
	-- within a type, so mixed number/string keys don't error in table.sort.
	table.sort(keys, function(a, b)
		local ta, tb = type(a), type(b)
		if ta == tb then
			return a < b
		end
		return ta < tb
	end)
	return keys
end

-- Luau constructor emitters for tagged type structs
local TYPE_EMITTERS = {
	Vector3 = function(v)
		return "vector.create(" .. table.concat(v, ", ") .. ")"
	end,
	Vector2 = function(v)
		return "Vector2.new(" .. table.concat(v, ", ") .. ")"
	end,
	CFrame = function(v)
		return "CFrame.new(" .. table.concat(v, ", ") .. ")"
	end,
	Color3 = function(v)
		return "Color3.new(" .. table.concat(v, ", ") .. ")"
	end,
	UDim = function(v)
		return "UDim.new(" .. table.concat(v, ", ") .. ")"
	end,
	UDim2 = function(v)
		return "UDim2.new(" .. table.concat(v, ", ") .. ")"
	end,
	NumberRange = function(v)
		return "NumberRange.new(" .. table.concat(v, ", ") .. ")"
	end,
	Rect = function(v)
		return "Rect.new(" .. table.concat(v, ", ") .. ")"
	end,
	-- Already a fully-qualified path; emit it unquoted so it round-trips as an
	-- EnumItem rather than a string that merely looks like one.
	EnumItem = function(v)
		return v
	end,
	-- normalize carries the bytes as base64.
	buffer = function(v)
		local bytes = buffer.tostring(EncodingService:Base64Decode(buffer.fromstring(v)))
		return 'buffer.fromstring("' .. escapeString(bytes) .. '")'
	end,
}

-- Forward declaration (mutually recursive with `walk`); annotated so the
-- analyzer knows the concatenations below operate on strings.
local emitValue: (value: any, depth: number) -> string

-- One table entry: indent + optional key prefix + value + trailing comma.
local function walk(key, value, depth)
	local lua = getIndent(depth)
	if key ~= nil then
		if type(key) == "number" then
			-- Luau supports numeric keys: `[100] = ...`. Emitting `["100"]`
			-- would change the key's type on round-trip.
			lua = lua .. "[" .. tostring(key) .. "] = "
		elseif type(key) == "string" then
			lua = lua .. '["' .. escapeString(key) .. '"] = '
		else
			lua = lua .. '["' .. escapeString(tostring(key)) .. '"] = '
		end
	end
	return lua .. emitValue(value, depth) .. ",\n"
end

-- The value expression alone — no indent, no key, no trailing comma. Shared
-- by table entries and the top-level value, so a scalar or a tagged Roblox
-- type at the root emits `return 42` / `return vector.create(...)` instead of
-- being dropped from an empty table literal.
function emitValue(value, depth)
	local valueType = typeof(value)
	if valueType == "string" then
		return '"' .. escapeString(value) .. '"'
	elseif valueType == "number" or valueType == "boolean" then
		return tostring(value)
	elseif value == nil then
		return "nil"
	elseif valueType == "table" then
		-- Tagged Roblox type struct?
		local taggedType = value.type
		local emitter = if type(taggedType) == "string" then TYPE_EMITTERS[taggedType] else nil
		if emitter then
			return emitter(value.value)
		end
		local lua = "{\n"
		if isArray(value) then
			for _, v in ipairs(value) do
				lua = lua .. walk(nil, v, depth + 1)
			end
		else
			for _, k in ipairs(getSortedKeys(value)) do
				lua = lua .. walk(k, value[k], depth + 1)
			end
		end
		return lua .. getIndent(depth) .. "}"
	end
	-- Unreachable: normalize() stringifies every other type first.
	return "nil"
end

local function serializeToLuau(value)
	return "return " .. emitValue(value, 0) .. "\n"
end

return serializeToLuau
