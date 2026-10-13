-- @lute/time shim — pure-Luau Instant/Duration matching lute's: frozen values
-- with arithmetic, comparison and tostring operators, at nanosecond precision.
-- Instants are monotonic via os.clock().

local NANOS_PER_SECOND = 1_000_000_000

-- A Duration is whole seconds plus whole nanoseconds (0 to 1e9 - 1), as
-- lute's C++ timespec is: conversions to ms/us and the subsec* getters
-- truncate, tostring prints "sec.nnnnnnnnn", and results never go negative.
local Duration

local function trunc(n: number): number
	return if n < 0 then math.ceil(n) else math.floor(n)
end

local function nanos(d): number
	return d.sec * NANOS_PER_SECOND + d.nsec
end

-- From a nanosecond count, truncated to a whole number like lute's int64 cast.
local function fromNanos(n: number)
	n = trunc(n)
	local sec = trunc(n / NANOS_PER_SECOND)
	return Duration(sec, n - sec * NANOS_PER_SECOND)
end

-- From seconds, like lute's getTimespecFromSeconds.
local function fromSeconds(seconds: number)
	return Duration(trunc(seconds), trunc(math.fmod(seconds, 1) * NANOS_PER_SECOND))
end

local function checkNonNegative(n: number)
	if n < 0 then
		error("duration cannot be negative", 3)
	end
end

local durationOps = {}

function durationOps.toNanoseconds(self): number
	return nanos(self)
end
function durationOps.toMicroseconds(self): number
	return self.sec * 1e6 + self.nsec // 1e3
end
function durationOps.toMilliseconds(self): number
	return self.sec * 1e3 + self.nsec // 1e6
end
function durationOps.toSeconds(self): number
	return self.sec + self.nsec / NANOS_PER_SECOND
end
function durationOps.toMinutes(self): number
	return durationOps.toSeconds(self) / 60
end
function durationOps.toHours(self): number
	return durationOps.toSeconds(self) / 3600
end
function durationOps.toDays(self): number
	return trunc(durationOps.toSeconds(self) / 86400)
end
function durationOps.toWeeks(self): number
	return trunc(durationOps.toSeconds(self) / 604800)
end
function durationOps.subsecnanos(self): number
	return self.nsec
end
function durationOps.subsecmicros(self): number
	return self.nsec // 1e3
end
function durationOps.subsecmillis(self): number
	return self.nsec // 1e6
end

local durationMeta = {
	__index = durationOps,
	__tostring = function(d): string
		return string.format("%d.%09d", d.sec, d.nsec)
	end,
	__add = function(a, b)
		local sec, nsec = a.sec + b.sec, a.nsec + b.nsec
		if nsec >= NANOS_PER_SECOND then
			sec, nsec = sec + 1, nsec - NANOS_PER_SECOND
		end
		return Duration(sec, nsec)
	end,
	__sub = function(a, b)
		local sec, nsec = a.sec - b.sec, a.nsec - b.nsec
		if nsec < 0 then
			sec, nsec = sec - 1, nsec + NANOS_PER_SECOND
		end
		return Duration(math.max(sec, 0), math.max(nsec, 0))
	end,
	__mul = function(a, factor: number)
		if factor ~= factor or factor == math.huge or factor < 0 then
			error("factor must be a finite non-negative number", 2)
		end
		return fromNanos(nanos(a) * factor)
	end,
	__div = function(a, factor: number)
		if factor ~= factor or factor == math.huge or factor <= 0 then
			error("factor must be a finite positive number", 2)
		end
		return fromNanos(nanos(a) / factor)
	end,
	__eq = function(a, b)
		return a.sec == b.sec and a.nsec == b.nsec
	end,
	__lt = function(a, b)
		return a.sec < b.sec or (a.sec == b.sec and a.nsec < b.nsec)
	end,
	__le = function(a, b)
		return a.sec < b.sec or (a.sec == b.sec and a.nsec <= b.nsec)
	end,
}

Duration = function(sec: number, nsec: number)
	return table.freeze(setmetatable({ sec = sec, nsec = nsec }, durationMeta))
end

local instantOps = {}

function instantOps.elapsed(self): number
	return os.clock() - self.clock
end

local instantMeta = {
	__index = instantOps,
	__sub = function(a, b)
		return fromNanos(math.max(a.clock - b.clock, 0) * NANOS_PER_SECOND)
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

-- lute's constructors: negative input errors, hours/days/weeks past int64
-- seconds error, and fractions below a nanosecond truncate.
local function tooLarge(n: number, secondsPerUnit: number)
	if n > 2 ^ 63 / secondsPerUnit then
		error("duration is too large", 3)
	end
end

time.duration = {
	create = function(seconds: number, subsecnanos: number)
		seconds, subsecnanos = trunc(seconds), trunc(subsecnanos)
		if seconds < 0 or subsecnanos < 0 then
			error("duration components cannot be negative", 2)
		end
		return Duration(seconds + subsecnanos // NANOS_PER_SECOND, subsecnanos % NANOS_PER_SECOND)
	end,
	nanoseconds = function(n: number)
		checkNonNegative(n)
		return fromNanos(n)
	end,
	microseconds = function(n: number)
		checkNonNegative(n)
		return fromNanos(n * 1e3)
	end,
	milliseconds = function(n: number)
		checkNonNegative(n)
		return fromNanos(n * 1e6)
	end,
	seconds = function(n: number)
		checkNonNegative(n)
		return fromSeconds(n)
	end,
	minutes = function(n: number)
		checkNonNegative(n)
		return fromSeconds(n * 60)
	end,
	hours = function(n: number)
		checkNonNegative(n)
		tooLarge(n, 3600)
		return fromSeconds(n * 3600)
	end,
	days = function(n: number)
		checkNonNegative(n)
		tooLarge(n, 86400)
		return fromSeconds(n * 86400)
	end,
	weeks = function(n: number)
		checkNonNegative(n)
		tooLarge(n, 604800)
		return fromSeconds(n * 604800)
	end,
}

return time
