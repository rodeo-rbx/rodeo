-- @lute/time shim — pure-Luau Instant/Duration matching lute's definitions:
-- frozen values with locked metatables, arithmetic and comparison operators,
-- nanosecond precision (as precise as double-held nanos allow). Instants are
-- monotonic via os.clock().
--
-- NOTE: inlined as an IIFE with comments stripped (bundle.rs inline_shims).
-- No `--` inside string literals.

local Duration

local durationOps = {}

local function durNanos(d): number
	return d.nanos
end

function durationOps.toNanoseconds(self): number
	return durNanos(self)
end
function durationOps.toMicroseconds(self): number
	return durNanos(self) / 1e3
end
function durationOps.toMilliseconds(self): number
	return durNanos(self) / 1e6
end
function durationOps.toSeconds(self): number
	return durNanos(self) / 1e9
end
function durationOps.toMinutes(self): number
	return durNanos(self) / (60 * 1e9)
end
function durationOps.toHours(self): number
	return durNanos(self) / (3600 * 1e9)
end
function durationOps.toDays(self): number
	return durNanos(self) / (86400 * 1e9)
end
function durationOps.toWeeks(self): number
	return durNanos(self) / (7 * 86400 * 1e9)
end
function durationOps.subsecnanos(self): number
	return durNanos(self) % 1e9
end
function durationOps.subsecmicros(self): number
	return math.floor(durNanos(self) % 1e9) / 1e3
end
function durationOps.subsecmillis(self): number
	return math.floor(durNanos(self) % 1e9) / 1e6
end

local durationMeta = {
	__index = durationOps,
	__add = function(a, b)
		return Duration(durNanos(a) + durNanos(b))
	end,
	__sub = function(a, b)
		return Duration(durNanos(a) - durNanos(b))
	end,
	__mul = function(a, k: number)
		return Duration(durNanos(a) * k)
	end,
	__div = function(a, k: number)
		return Duration(durNanos(a) / k)
	end,
	__eq = function(a, b)
		return durNanos(a) == durNanos(b)
	end,
	__lt = function(a, b)
		return durNanos(a) < durNanos(b)
	end,
	__le = function(a, b)
		return durNanos(a) <= durNanos(b)
	end,
}

Duration = function(nanos: number)
	return table.freeze(setmetatable({ nanos = nanos }, durationMeta))
end

local instantOps = {}

function instantOps.elapsed(self): number
	return os.clock() - self.clock
end

local instantMeta = {
	__index = instantOps,
	__sub = function(a, b)
		return Duration((a.clock - b.clock) * 1e9)
	end,
	__eq = function(a, b)
		return a.clock == b.clock
	end,
	__lt = function(a, b)
		return a.clock < b.clock
	end,
	__le = function(a, b)
		return a.clock <= b.clock
	end,
}

local time = {}

function time.now()
	return table.freeze(setmetatable({ clock = os.clock() }, instantMeta))
end

function time.since(instant): number
	return instant:elapsed()
end

time.duration = {
	create = function(seconds: number, subsecnanos: number)
		return Duration(seconds * 1e9 + subsecnanos)
	end,
	nanoseconds = function(n: number)
		return Duration(n)
	end,
	microseconds = function(n: number)
		return Duration(n * 1e3)
	end,
	milliseconds = function(n: number)
		return Duration(n * 1e6)
	end,
	seconds = function(n: number)
		return Duration(n * 1e9)
	end,
	minutes = function(n: number)
		return Duration(n * 60 * 1e9)
	end,
	hours = function(n: number)
		return Duration(n * 3600 * 1e9)
	end,
	days = function(n: number)
		return Duration(n * 86400 * 1e9)
	end,
	weeks = function(n: number)
		return Duration(n * 7 * 86400 * 1e9)
	end,
}

return time
