---
name: rbx-unit-test
description: "Write, run and debug Luau unit tests for ModuleScripts in Roblox Studio. Use only when asked to add tests, improve coverage, or run or debug tests."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

# Unit Test

## TOOLS

- `script_search`: Find scripts by name.
- `script_grep`: Search script content for keywords.
- `script_read`: Read script contents.
- `search_game_tree`: Browse the game hierarchy.
- `inspect_instance`: Get detailed properties and attributes of an instance.
- `execute_luau`: Run Luau to query game state and to run tests. RETURN values, do not print, except that the test run itself prints its results to the console by design.
- `multi_edit`: Create and edit scripts.
- `start_stop_play`: Start or stop a Play session. Tests run only in Play.
- `get_console_output`: Read the console. This is how test results are collected.

## WHAT IS UNIT TESTABLE

- Only ModuleScripts. LocalScripts and ServerScripts are not unit testable because they cannot be invoked in isolation. When useful logic lives in a LocalScript or ServerScript, recommend extracting it into a ModuleScript so it can be tested.
- A unit test exercises one module in isolation. It must not depend on real network calls, DataStores, HttpService, other players, or wall-clock time. Replace those with stubs (see ISOLATING DEPENDENCIES).
- Tests run on the server DataModel during Play. Client-only APIs are unavailable. Keep testable logic free of client-only assumptions.

## CHOOSING THE APPROACH: DETECT, THEN CONFORM

Before writing anything, check whether the place already uses a test framework, and conform to it. Use the built-in harness (LAYOUT onward) only when none is found.

1. Detect with `script_grep` and `script_search`:
   - Jest-Lua: a `JestGlobals` or `Jest` ModuleScript (usually under `Packages`), or spec modules that `require(... JestGlobals)`. Grep for `JestGlobals`. This is the framework Roblox uses internally and the most likely modern choice.
   - TestEZ: a `TestEZ` ModuleScript, a `TestBootstrap` reference, or spec modules using `describe`/`it`/`expect` that return a function. Grep for `TestEZ`.
   - Existing specs in any form: ModuleScripts named `*.spec` or `*_spec`. Grep for `.spec` and `describe(`.
2. If a framework is present, conform to it. Match its idiom, its spec naming, and where its existing specs live (colocated with the module under test is common; do not force them into ServerStorage). Run the suite through the project's existing runner. Never install Wally or other dependencies yourself. If a framework is referenced but its package is missing, report that and stop rather than working around it.
3. If nothing is found, use the built-in harness described below.

The same intent maps across all three, so reuse the test design without relearning assertions:
| Intent | Built-in | Jest-Lua | TestEZ |
| --- | --- | --- | --- |
| equal | `expect.equal(a, b)` | `expect(a).toBe(b)` | `expect(a).to.equal(b)` |
| deep equal | `expect.deepEqual(a, b)` | `expect(a).toEqual(b)` | compare fields |
| near (float) | `expect.near(a, b, t)` | `expect(a).toBeCloseTo(b)` | assert the delta |
| throws | `expect.throws(fn)` | `expect(fn).toThrow()` | `expect(fn).to.throw()` |
| truthy | `expect.truthy(v)` | `expect(v).toBeTruthy()` | `expect(v).to.be.ok()` |
| falsy | `expect.falsy(v)` | `expect(v).toBeFalsy()` | `expect(v).to.never.be.ok()` |

To run a detected framework inside Studio: start Play, then invoke its entry point with `execute_luau` (for Jest-Lua, the project's `runCLI` script pointed at the spec root; for TestEZ, `require(path.TestEZ).TestBootstrap:run({ specRoot })`), and read results from `get_console_output`.

Everything in TEST THE CONTRACT NOT THE IMPLEMENTATION, TEST CASE CONVENTIONS, ISOLATING DEPENDENCIES, and DEBUGGING FAILURES (contract-first design, Arrange-Act-Assert, one behavior per case, isolation, boundary and error coverage, determinism, dependency injection) applies regardless of framework. Only the harness, file layout, and run command change.

## BUILT-IN HARNESS (fallback when no framework is detected)

Place all test infrastructure under ServerStorage so it never replicates to clients:

```
ServerStorage
  UnitTest (Folder)
    RunUnitTest (ModuleScript)   -- entry point: discovers, filters, times, runs cases
    Cases (Folder)
      <ScriptName>_Test (ModuleScript)   -- one per module under test
ServerScriptService
  UnitTestRunner (Script, Disabled)   -- manual run affordance for humans
```

Before creating any instance (Folder, ModuleScript, or Script), check whether a child of that name already exists under the same parent and reuse it. Never create duplicate names under one parent.

## TEST THE CONTRACT, NOT THE IMPLEMENTATION

This is the most important rule, and the easiest to break when tests are written from existing code. Do not read what the function returns and assert that. Doing so only proves the code does what it already does, which silently locks in any bug as expected behavior and gives false confidence. Derive the function's intended contract first, then write tests against that contract, assuming the implementation may be wrong.

- Derive the contract from authoritative signals, in this order: explicit docstrings, comments, and type annotations; the function and parameter names; how existing callers use it; and the user's or design's stated intent. Read the body to understand inputs and branches, not to decide what "correct" means.
- Write each assertion to the expected contractual result, not the observed one.
- A test that fails because the function is genuinely incorrect is a success, not a problem to fix. Never change an expected value to match a result you believe is wrong, and never weaken or delete such a test to make the suite green. A failing test the user can argue about is worth more than a passing test the user would disagree with.
- Report any such failure explicitly as a suspected implementation bug, naming the input, the expected contractual result, and the actual result, so the user can confirm or correct the contract.
- When the intended contract is genuinely ambiguous and no authoritative signal resolves it, do not invent one. State the ambiguity and ask, rather than guessing and producing tests that are merely wrong in a new way.

## TEST CASE CONVENTIONS

- Place each case ModuleScript under `ServerStorage.UnitTest.Cases`, named `<ScriptName>_Test`, where `<ScriptName>` is the module under test.
- A case module returns a single function that receives a context `t`, where `t.test(name, fn)` runs one case in isolation and `t.expect` holds the matchers. Register every assertion through `t.test`.
- Put a one line comment in front of each case describing what it verifies.
- Structure each case as Arrange, Act, Assert: build inputs, call the function once, then assert on the result.
- Test one behavior per case. Prefer several focused cases over one case with many unrelated assertions.
- Name cases by behavior, not by function name. "returns zero for an empty list" beats "test sum".
- Test observable behavior through the public API. Do not assert on private internals, which change without changing behavior.
- Keep cases independent and order independent. Build fresh inputs inside each case. Never rely on state left by another case.
- Cover the normal path, boundaries (empty, nil, zero, negative, very large, duplicate), and error paths (invalid input should fail predictably, verified with `t.expect.throws`).
- Keep cases deterministic. Do not depend on `os.time`, `math.random`, `tick`, or real waits. Inject or seed those so results repeat.
- Aim for meaningful coverage of public behavior and branches rather than a percentage. Do not add cases that assert nothing.

## ISOLATING DEPENDENCIES

- Prefer modules that receive dependencies as arguments or through a constructor such as `Module.new(deps)`, so a test can pass a stub in place of a real service.
- A stub is a plain table that satisfies the interface the module uses and, where useful, records how it was called.

```lua
-- Fake DataStore: serves canned reads and records writes.
local function makeFakeStore(initial)
	local data = initial or {}
	return {
		calls = {},
		GetAsync = function(_, key) return data[key] end,
		SetAsync = function(self, key, value)
			table.insert(self.calls, { key = key, value = value })
			data[key] = value
		end,
	}
end
```

- `require` caches a module per session, so module level state persists across cases within a run, and across repeated runs in the same Play session. For stateful modules, use a constructor that returns a fresh instance per case, or expose a reset function called at the start of each case. To clear all cached state, stop and restart Play.

## RUNNER: RunUnitTest

Create this ModuleScript at `ServerStorage.UnitTest.RunUnitTest` if it does not exist:

```lua
-- RunUnitTest: discovers and runs unit test cases under ServerStorage.UnitTest.Cases.
-- Each case module returns: function(t) ... end, using t.test(name, fn) and t.expect.
-- Usage: require(ServerStorage.UnitTest.RunUnitTest)(filter, timeout)
--   filter   optional string; runs only case modules whose name contains it
--   timeout  optional seconds per case (default 5)
local ServerStorage = game:GetService("ServerStorage")

local function fmt(v)
	if type(v) == "string" then return string.format("%q", v) end
	return tostring(v)
end

local expect = {}
function expect.equal(actual, expected)
	if actual ~= expected then
		error(string.format("expected %s, got %s", fmt(expected), fmt(actual)), 2)
	end
end
function expect.truthy(value)
	if not value then error(string.format("expected truthy, got %s", fmt(value)), 2) end
end
function expect.falsy(value)
	if value then error(string.format("expected falsy, got %s", fmt(value)), 2) end
end
function expect.near(actual, expected, tolerance) -- use for floating point results
	tolerance = tolerance or 1e-6
	if type(actual) ~= "number" or math.abs(actual - expected) > tolerance then
		error(string.format("expected %s within %s of %s", fmt(actual), fmt(tolerance), fmt(expected)), 2)
	end
end
function expect.throws(fn) -- asserts fn raises; returns the error message
	local ok, err = pcall(fn)
	if ok then error("expected the function to throw, but it returned normally", 2) end
	return err
end
function expect.deepEqual(actual, expected)
	local function eq(a, b)
		if a == b then return true end
		if type(a) ~= "table" or type(b) ~= "table" then return false end
		for k, v in pairs(a) do if not eq(v, b[k]) then return false end end
		for k in pairs(b) do if a[k] == nil then return false end end
		return true
	end
	if not eq(actual, expected) then error("tables are not deeply equal", 2) end
end

-- Runs one test in isolation with a yield-based timeout.
-- A case that never yields (for example a tight infinite loop) cannot be
-- preempted; the timeout only bounds cases that yield while waiting.
local function runOne(fn, timeout)
	local done, ok, err = false, false, nil
	local cpuStart = os.clock()
	task.spawn(function()
		ok, err = pcall(fn)
		done = true
	end)
	local waited = 0
	while not done and waited < timeout do
		waited += task.wait()
	end
	local elapsed = waited > 0 and waited or (os.clock() - cpuStart)
	if not done then
		return "timeout", elapsed, string.format("exceeded %.1fs", timeout)
	elseif ok then
		return "pass", elapsed, nil
	else
		return "fail", elapsed, tostring(err)
	end
end

return function(filter, timeout)
	timeout = timeout or 5
	local totals = { run = 0, passed = 0, failed = 0 }
	local sessionStart = os.clock()

	for _, module in ipairs(ServerStorage.UnitTest.Cases:GetDescendants()) do
		if module:IsA("ModuleScript") and (filter == nil or string.find(module.Name, filter, 1, true)) then
			local okRequire, caseFn = pcall(require, module)
			if not okRequire or type(caseFn) ~= "function" then
				totals.run += 1; totals.failed += 1
				warn(string.format("[FAIL] %s | case did not return a function: %s", module.Name, tostring(caseFn)))
			else
				local t = { expect = expect }
				function t.test(name, fn)
					totals.run += 1
					local status, elapsed, message = runOne(fn, timeout)
					if status == "pass" then
						totals.passed += 1
						print(string.format("[PASS] %s > %s (%.3fs)", module.Name, name, elapsed))
					else
						totals.failed += 1
						warn(string.format("[%s] %s > %s (%.3fs) | %s", string.upper(status), module.Name, name, elapsed, message))
					end
				end
				local okRun, runErr = pcall(caseFn, t)
				if not okRun then
					totals.run += 1; totals.failed += 1
					warn(string.format("[FAIL] %s | error while building cases: %s", module.Name, tostring(runErr)))
				end
			end
		end
	end

	print(string.format("[SUMMARY] %d run, %d passed, %d failed, %.3fs", totals.run, totals.passed, totals.failed, os.clock() - sessionStart))
	return totals
end
```

## MANUAL RUNNER: UnitTestRunner

Create this `Script` at `ServerScriptService.UnitTestRunner` if it does not exist, and set `Disabled = true` by default so it does not auto-run on every Play:

```lua
-- Manual entry point. Enable this Script and press Play to run all unit tests.
local ServerStorage = game:GetService("ServerStorage")
require(ServerStorage.UnitTest.RunUnitTest)()
```

Before running tests yourself with `execute_luau`, confirm this Script is disabled in edit mode so the suite does not run twice.

## EXAMPLE CASE

A ModuleScript named `Inventory_Test` under `ServerStorage.UnitTest.Cases`:

```lua
-- Tests for ReplicatedStorage.Modules.Inventory.
-- Covers add and remove behavior, capacity limits, and invalid input.
return function(t)
	local Inventory = require(game.ReplicatedStorage.Modules.Inventory)
	local expect = t.expect

	-- A fresh instance per case keeps state isolated.
	local function makeInventory()
		return Inventory.new({ capacity = 2 })
	end

	-- Adds a single item and reports it as present.
	t.test("adds an item", function()
		local inv = makeInventory()
		inv:add("sword")
		expect.truthy(inv:has("sword"))
	end)

	-- Rejects items beyond the configured capacity.
	t.test("enforces capacity", function()
		local inv = makeInventory()
		inv:add("a")
		inv:add("b")
		expect.throws(function() inv:add("c") end)
	end)

	-- Removing an item that is not present is a safe no-op.
	t.test("remove of a missing item is safe", function()
		local inv = makeInventory()
		inv:remove("ghost")
		expect.falsy(inv:has("ghost"))
	end)
end
```

## RUNNING TESTS

1. Confirm the case modules and `RunUnitTest` are saved, and `UnitTestRunner` is disabled.
2. Start Play with `start_stop_play`.
3. Run with `execute_luau` using `datamodel_type="Server"`:

```lua
local ServerStorage = game:GetService("ServerStorage")
return require(ServerStorage.UnitTest.RunUnitTest)(nil, 5)
```

Pass a filter string as the first argument to run a subset (for example `("Inventory")`). 4. Read the console with `get_console_output`. Per-test lines and the `[SUMMARY]` line appear there. The call also returns the totals table. 5. Interpret results: the run passes only if there are zero `[FAIL]` and zero `[TIMEOUT]` lines. Use those lines and any stack traces to locate problems.

## WORKFLOW

1. Detect. Check for an existing framework (see CHOOSING THE APPROACH) and decide whether to conform to it or use the built-in harness.
2. Plan. Identify the module under test and the behaviors that need coverage.
3. Read the code. Use `script_read` and `script_grep` on both the module and any existing tests for it to see what is already covered.
4. Assess gaps. Decide which untested behaviors, boundaries, and error paths to add. If coverage is already adequate, say so rather than adding redundant cases.
5. Write. In the detected framework's idiom and location, or per TEST CASE CONVENTIONS for the built-in harness (creating `RunUnitTest` and `UnitTestRunner` first if missing).
6. Run. Through the framework's runner, or follow RUNNING TESTS for the built-in harness.
7. Diagnose any failure before changing anything (see DEBUGGING FAILURES).
8. Fix the correct layer, then rerun, narrowing to the failing case while iterating (the filter argument for the built-in harness, or the framework's focus mechanism such as `it.only`).
9. Report. Give a brief summary of what was covered and the final counts. Call out separately any tests that fail because the implementation appears to contradict its contract, listing them as suspected bugs (input, expected, actual) rather than folding them in with broken tests. If all pass, do not repeat the console output, since the main agent reads it anyway. If a failure could not be resolved, state what was attempted and what remains.

## DEBUGGING FAILURES

- Read the `[FAIL]` or `[TIMEOUT]` line and stack trace to find the failing case and assertion. Re-run with the filter argument to isolate it.
- Decide whether the fault is in the test or in the module. A test is wrong only when its expected value does not match the function's intended contract. If the expectation reflects the contract but the module disagrees, the module is at fault, not the test. Confirm the intended contract from docstrings, types, callers, or the user before assuming.
- When the module is at fault, do not silently rewrite the test to pass. Report it as a suspected implementation bug (input, expected contractual result, actual result). Fix the module only if correcting it is in scope and the contract is confirmed; otherwise leave the test failing for the user to confirm.
- When the test itself is genuinely wrong (its expectation misreads the contract), correct the expectation. Never weaken or delete a valid contract assertion only to make the suite pass.
- A `[TIMEOUT]` usually means the case is waiting on something that never completes, or the module yields longer than expected. Check injected waits and async dependencies.
- For an intermittent failure, suspect a nondeterministic dependency (time, randomness, ordering, or shared state) and isolate it.
- If a case cannot be resolved after a few focused attempts, leave it visibly failing rather than hiding it, and report it.
