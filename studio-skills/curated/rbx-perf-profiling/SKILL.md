---
name: rbx-perf-profiling
description: "Analyze MicroProfiler data with the LibMP Luau module. Use to investigate CPU and GPU bottlenecks, frame-time spikes and memory allocations by subsystem."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

# Roblox Performance Profiling

Let's learn how to profile Roblox games programmatically on clients, servers, and in Studio.

LibMP is a Luau module used to profile the performance of Roblox games. Profiling can be done in the game client's or server's scripts in real time (as we gather profiling information), or through Studio MCP's `execute_luau` command or the Studio command bar once the data has been collected and is static. The former is good for lightweight, targeted checks (as we should not influence frame time much with the checks themselves). The latter is better for deeper and longer analysis, allowing more frames, threads, and code scopes to be analyzed at once without affecting the client's frame time.

All information visually available in the MicroProfiler (MP) UI is accessible programmatically via LibMP. In Studio, MicroProfiler data is accessible in both Play and Edit modes.

**Limitations:** Currently, scope labels are not available (the text labels placed inside scope instances on the timeline by the Engine to describe processing details, not to be confused with scope/timer names/tags). Because of this, automatic script scopes are shown as "$Script" rather than "Script_MyScriptName". However, manually annotated scopes using `debug.profilebegin` / `debug.profileend` are displayed correctly. Also, only original/raw profiling data is provided, not derived data. There are no built-in average/min/max times; you must calculate these manually from the raw data if needed. Conversely, the previously unavailable history of counter values is now accessible.

### Sessions & Capturing

The main object is a `Session`. It can be created from Live Data (the data that the MicroProfiler currently holds and displays to the user). The MP can be enabled/disabled and capture can be activated/paused. When active, it collects a new frame each tick and evicts the oldest frame(s). A maximum of 256 recent frames can be held.

A Session tracking Live Data is supposed to be created once (not every frame) and reused as needed. When the MP is actively capturing, the first data retrieval call to LibMP will synchronize the Session with the MP data state in the Engine. When the MP is paused, using LibMP in the current and subsequent frames will not trigger a data sync; you will simply work with the data that was already synced. (A rare exception occurs if the MP is paused for a long time and an OS thread is deleted; in this case, the Session will sync on the next LibMP call).

A typical workflow when a frame time fluctuation is noticed is to wait briefly to collect a few more frames, then snapshot the MP data into a Luau buffer object. You can then create a new, independent Session from that buffer's data. This allows you to work with the exact same dataset indefinitely. This is especially useful when running LibMP in the Studio command bar or via MCP's execute_luau.

Of course, "frame time fluctuations" is a broad term. It can refer to generally poor performance, individual frame spikes, or sustained periods of slightly elevated frame times due to GPU bottlenecks. You can analyze not only total frame time but also individual code Scope times, or the time spent by groups of scopes (e.g., Physics). This can be measured as the sum of CPU core times or as real-world elapsed time (which differ during parallel execution). Analyze rare spikes separately from steady-state bottlenecks. A single 600 ms stall can dominate averages and hide the normal bottleneck. These concepts are common in game profiling and are not specific to LibMP. 

### Basic Entities

* **Frames:** The timeline is continuous, but it is divided into Frames for navigation. Frames have IDs that increment continuously, as well as absolute IDs used to find stitches (gaps) in the log recordings. Frames contain both CPU and GPU timestamps for their start and end points.
* **Timers (Scopes):** Sections of code instrumented so that entering and leaving them records timestamps in the profiling data. Each enter/leave pair forms a scope instance on the timeline. This includes both C++ Engine scopes (present by default) and user-added Luau scopes (present if executed at least once).
* **Groups:** Collections of Scopes, such as Physics, Scripts, Network, etc.
* **Threads:** OS threads. Each thread has its own parallel timeline and profiling data log, though known Scopes can appear in any thread. A typical unit of profiling is a Frame alongside its corresponding thread logs. The GPU thread is an exception: it is not an OS thread, but rather represents the execution of rendering processes on the GPU.
* **Counters:** Represent various Engine parameters at a specific moment in time (per Frame). They can show used/allocated memory per category (Physics, Luau heap, Render, etc.), allocations per second, or instance counts (e.g., scene objects processed). Counters are hierarchical: a base node might represent a total sum or act as a value-less container for child counters. The most specific counters are the leaf nodes.

Entities possess numeric IDs and are referenced by them. If an entity has a string name, helper functions can find its ID by name or search mask. IDs start from 1 - a value of 0 means the absence of an entity. Entities also have descriptors containing their ID, name, and specific parameters (such as a Timer's color).

### Iterators & Frame Boundaries

Iterators are a very powerful mechanism for processing performance data. Thread log iterators can process all thread log data, or be configured to process only specific scopes in specific threads within a specific frame range. At each step, they provide a detailed state, tracking the stack of scopes as we enter and leave them over time.

They can also be reused multiple times by rewinding to a new frame range. If a new frame range starts exactly where the previous one ended, the iterator continues smoothly without a cold start, preserving its last state. This is crucial for the following reason:

Scopes can cross frame boundaries. A single visual frame might contain the tail of a scope that started in the previous frame, and the beginning of another scope that finishes in a subsequent frame. In such cases, the scope time for the current frame is the sum of the partial scope durations that fit within that frame's borders. Do not attribute a scope instance solely to the frame where its exit entry appears. Instead, treat each scope instance as a time span and distribute its duration across every frame it overlaps.

When cold-starting on the first frame, you may encounter scope instances that started beforehand; thus, you will only see exit entries without their corresponding enter entries. Conversely, in the final frame, scopes may begin (showing an enter entry) but finish outside the range, lacking an exit entry.

Middle frames do not have these problems because the iterator tracks the current scope stack. This stack is preserved across frame boundaries, and the iterator explicitly marks the timestamps when scopes on the stack cross those borders. Even the corner cases described above can be successfully resolved using hints from the log iterator. It indicates when an exit entry causes a stack underflow and provides timestamps for when the next frame boundary is reached (even at the end of the last frame) and when each thread's read finishes within the frame, allowing you to calculate exactly what portion of a scope fits within the analyzed frame.

The iterator progresses frame by frame, and thread by thread within each frame. 

**Important Iterator Behaviors:**
* **GPU vs. CPU Timestamps:** When iterating over the GPU thread, be aware that timestamp origins and scales may differ from CPU timestamps - do not mix them. Additionally, a frame's overall GPU start and finish times may be too broad compared to the actual frame time, or may even contain inaccurate data, such as an end time earlier than the start time, or a frame without log entries where the next or previous frame spans multiple frames' duration and data. Still, continuous (almost gapless) sections of scopes within the GPU thread correctly represent GPU execution time, so the time spent within those sections should be used as your reference. Alternatively, frames with inaccurate GPU boundaries may be excluded from GPU-related analysis along with the surrounding frames. On some platforms, the GPU thread may be absent or empty.
* **Event Entries:** Besides scope enter/leave entries, there are Event entries representing metadata for scope instances (like memory allocations or network events); however, these are not currently supported.
* **Stitch Detection:** The log iterator detects stitches in frame capturing - based on absolute frame IDs - which occur if capturing is paused and resumed later.
* **Warm-ups:** When processing paused/snapshotted data (where real-time performance overhead is less critical), you can start iterating one frame early (if this frames exist). This "warms up" the scope stack before analyzing your target frame range, so you don't need to run a procedure to recover the initial stack state.

**Timing Types:**
Scope/Timer total times are inclusive, meaning the execution time of child scopes is included in their parent's time. If multiple instances of the same scope occur in the same frame across several threads, you can measure:
1. Total wall-time from the start of the first instance to the end of the last.
2. Real-world CPU active work time (excluding inactive periods between instances).
3. The sum of CPU core times spent (which can exceed real-world elapsed time due to parallelization).
Inclusive group totals are usually work volume, not frame duration.
Exact-stack total times can be used to identify wall-time bottlenecks in non-parallel cases. If the same exact stack runs concurrently on multiple threads, summing all instances gives work volume / occupancy across threads, not necessarily critical-path wall time. For real bottleneck diagnosis, inspect per-frame critical path and per-thread spans.

### Counters & Caching

The Counter iterator helps navigate the hierarchy of counters, which is more useful than a flat list. You can iterate over all counters, or specify certain counters as root elements to iterate only through them and their children. The iterator tracks the parent stack, its absolute depth, and its relative depth (relative to the specified root counter).

Using a counter ID, you can retrieve its last sample (where a sample consists of a `uint32` frame number and a `double` value) or all of its samples. Fetching the last sample is computationally cheaper. However, if you have already retrieved all samples and the MP data has not updated (which is typical for same-frame access during active capture, or indefinitely when paused), the data is cached, making subsequent accesses cheap. 

The same applies to basic entity descriptors (Threads, Timers, etc.). They are considered long-term data; once retrieved, assuming they haven't been changed or appended (which happens rarely), you will receive the cached data. Thus, executing `FetchTimerDesc(timerId)` frequently is perfectly fine.

### Data Retrieval: `Get***` vs `Fetch***`

The Luau LibMP API is a wrapper around a WebAssembly (WASM) module, and the returned structures utilize FlatBuffers. You will find `Get***` and `Fetch***` methods for the same data:

* **`Get***`** returns a FlatBuffer view (a Luau object/table) with getter functions to access original, non-copied data.
* **`Fetch***`** creates a native Luau table/struct containing a deep copy of each field.

Some `Get***` methods return a simple scalar value (e.g. the ID of a thread) and therefore do not have a corresponding `Fetch***` counterpart.

It is generally recommended to use `Fetch***` for simplicity, despite the slight performance overhead. Retrieving data via `Get***` is typically faster, especially when accessing only one field instead of the entire object. However, **a `Get***` view is strictly temporary and is invalidated by ANY subsequent `Get***` or `Fetch***` call on the Session object.** Therefore, fields must be accessed immediately after the `Get***` call. Accessing them later will result in reading garbage data. *(Note: The `session:WithDataLock()` method exists specifically to keep these views valid safely if necessary).* Additionally, in certain cases, `Get***` objects can actually be less performant than `Fetch***`. For example, accessing a string field from a `Get***` descriptor creates a new Luau string allocation every time. `Fetch***` allocates the string only once.

Arrays returned by `Get***` methods are exception - they persist until disposed. Be especially careful with them - you must dispose the returned arrays later. Also, if their elements are not simple scalars (e.g., arrays of IDs), retrieving individual elements from the array falls under the rules described above.

Another exception is Iterator State objects - you get them once via `iter:GetState()`, and they persist and are updated as long as the iterator exists.

### Notes

Sessions, Iterators and Arrays must be disposed of manually by calling the `Dispose()` method. Special attention should be given to Arrays - store them in a variable immediately after creating them with a `Get***` method (e.g., `GetCounterSamples`) so you can call `Dispose()` later. Otherwise, they will leak.

If you frequently query arrays and LibMP.GetMemUsed() reports an increasing amount of memory, it is almost certainly a leak.

The names of Scopes/Timers are typically self-explanatory, but Roblox provides a reference on their website. *Note: these names are sometimes referred to as Tags or even Labels, although the latter is not technically correct (In the Engine internals, a Label refers to an auxiliary string attached to a scope instance, not to its name).*

- *Human-friendly reference:* https://create.roblox.com/docs/performance-optimization/microprofiler/tag-table
- *AI-readable source:* https://create.roblox.com/docs/performance-optimization/microprofiler/tag-table.md

*(When writing scripts for deep capture analysis, monitor your script's execution time. If it takes excessively long, look for optimizations in your Luau script to speed up future runs).*

Lastly, you can manually save a dump to a file from the MicroProfiler top menu via "Dump" --> "Dump in binary format". This captures exactly the same data exposed to you in-game, including the frame limit you specified. The dump (.gprx file) can later be accessed in command-line Luau (Lute) using the same LibMP module described here.

Examples of usage in scripts, along with documentation for the API methods and structures, are provided below.

LibMP download: https://github.com/Roblox/libmp/releases | Lute download: https://github.com/luau-lang/lute/releases


---

## Special notes for AI agents

In Studio, do not force a switch to Play mode if MicroProfiler data has already been captured and is static.

Use Studio MCP's `execute_luau` tool to run code snippets for interacting with MicroProfiler.

When mentioning frame IDs to the user, provide both the regular ID and the absolute ID, so the frame can be located programmatically and in the UI.

Always set the frame limit before operating on frame data. A user may also use LibMP in their game and configure a very low frame limit. If you do not override it, that limit will be inherited by default. Note, however, that changing the frame limit from its previous value clears the history of counter values.

**Data freezing: pausing and snapshotting**
* **No freezing (Live data):** Best for in-game scripts performing lightweight checks on real-time data without interrupting the ongoing capture.
* **Pausing MP:** Best for in-game scripts where your analysis consists of self-contained tasks executed entirely within a single frame.
* **Snapshotting:** Best for all other cases, including external script snippets or in-game analysis, where iteration over entities spans multiple frames.
* **Combined (pause + snapshot):** Use this only when you want the on-screen MP UI to visually freeze on the exact data you are analyzing via an external snippet. Make sure to unpause MP when finished.

Do not use `OpenFromLiveData` in `execute_luau`, open the session from a snapshot.

If a game is suspected to be GPU-bound because the GPU frame time is close to the CPU frame time, this does not automatically mean the workload is actually GPU-bound. The true active work time of the GPU can only be determined by examining the GPU thread scopes.


---

## LibMP -- minimalistic Luau example

```
-- local LibMP = require("@rbx/LibMP") -- This shortcut works ONLY for Studio MCP/Assistant and the Studio command bar. For all other use cases, you must manually add the LibMP module to your place.
-- local LibMP = require("./LibMP") -- For command-line Luau environment (Lute). Useful for parsing dump
local LibMP = require("@game/ReplicatedStorage/LibMP") -- For Roblox Engine environment

local session = LibMP.Session.OpenFromLiveData()
-- local session = LibMP.Session.OpenFromFile("path/to/file")

local counterId = session:FindCounterId("**/Luau/heap")
local counterSample = session:FetchLastCounterSample(counterId)
if counterSample ~= nil then
    print(counterSample.Value)
end

-- A simple iterator that captures scopes on exit and retrieves their enter/exit timestamps.
-- It does not respect frame boundaries, so scopes that cross frame boundaries are detected
-- only in the frame where they exit.
-- This approach is sufficient for simple use cases, but it is not sufficient for working with individual frames or for precise per-frame analysis of what strictly fits within the frame bounds.
-- Also, cross-frame scopes that start before startFrame or end after endFrame are never captured.
local function iterateLogSimple(iter, startFrame, endFrame, process_scope)
    -- We obtain a reference to the iterator state just once and can read it on each iteration step.
    local iterState = iter:GetState()
    iter:RewindTo(startFrame, endFrame)

    while iter:Step() do
        if iterState:IsExit() and not iterState:ThreadStackIsUnderflowed() and not iterState:ThreadStackWasOverflowed() then
            -- Get the element that was just popped from the scope stack
            -- (This is safe because the stack did not overflow in the previous step and the element exists).
            -- Stack elements are indexed from 0.
            -- Depth is the number of scopes currently on the stack.
            local depth = iterState:ThreadStackDepth()
            local el = iter:GetCurrentThreadStackElement(depth)
            
            -- el:EnterTimestamp() returns the original scope start time and is not tied to
            -- the current frame start time. Use el:EnterFrameId() to check which frame it originated in.
            -- Also, instead of using iterState:ThreadStackWasOverflowed(), we can check whether 'el' is not nil.
            process_scope(
                iterState:FrameId(),
                iterState:ThreadId(),
                depth,
                iterState:TimerId(),
                el:EnterTimestamp(),
                iterState:Timestamp()
            )
        end
    end
end

local iter = session:CreateLogIterator() -- Iterate all frames, all threads, and all their entries by default
iterateLogSimple(iter, 0, 0, function(frameId, threadId, depth, timerId, enterTimestamp, exitTimestamp)
    -- print(frameId, threadId, depth, timerId, enterTimestamp, exitTimestamp)
end)
```


---

## LibMP -- advanced Luau example

```
local LibMP = require("@game/ReplicatedStorage/LibMP")

-- LibMP module version info 
print("LibMP.Versions.Library = " .. LibMP.Versions.Library)
print("LibMP.Versions.DataFormat = " .. LibMP.Versions.DataFormat)

-- Is the API ready to use
print("LibMP.Control:IsBackendReady() = " .. (LibMP.Control:IsBackendReady() and 1 or 0))

-- When saving snapshots for deeper analysis, setting the frame limit to 256 (max) can be useful.
-- The actual number of frames captured may be lower, as it depends on how many frames fit into the MicroProfiler buffers.
LibMP.Control:SetFrameLimit(128)
-- local snapshotBuf = LibMP.Control:CaptureToBufferSync()
-- print("snapshot buf len = " .. buffer.len(snapshotBuf))
-- local session = LibMP.Session.OpenFromBuffer(snapshotBuf)

LibMP.Control:ShowUI(true) -- Show the MicroProfiler UI so we can visually inspect frame times (optional, Studio-only)
LibMP.Control:EnableProfiler(true)
LibMP.Control:EnableCapture(true) -- true to activate perf data capture / false to pause capture

local session = LibMP.Session.OpenFromLiveData()
if not session then
    error("Failed to initialize session from live data.")
end

print("session:GetDataFormatVersion() = " .. session:GetDataFormatVersion())
print("session:IsValid() = ", session:IsValid())

local cid = session:FindCounterId("**/Luau/heap")
-- local counterId = session:FindCounterId("/root/child", false)
-- "leaf$" - "parent/leaf$" - "parent/leaf_template_*$"
-- "node/child" -- "node/*/child" -- "node/**/child" -- "**/node/child" -- "*/root_child/child"
-- Counter Find*** methods accept a search mask similar to Linux file paths. If you specify just a name (without a mask),
-- the node can be anywhere in the hierarchy. If you want to find a specific leaf node,
-- mark its name with a '$' at the end. Root-anchored node paths start with '/'
-- Use * (single wildcard) as a placeholder for exactly one node level.
--    ** (recursive wildcard) acts as a placeholder for any number of nested nodes.
-- You can specify multiple search patterns at once by separating them with the '|' symbol.

local timerIds = session:FindTimerIds("*render*|*job*|*physics*|*script*", false) -- Find*** methods for timers and threads accept simple '*' masks.
local workerIds = session:FindThreadIds("*worker A") -- Case-insensitive by default, so the second argument can be omitted.
-- local workerIds = session:FindThreadIds("*worker A|*worker B|*worker F", false)
-- local threadIds = { session:GetThreadIdGpu() }
-- Find*** methods are slow, so use them once and store the returned value.

local iter = session:CreateLogIterator()

iter:Configure({
    --SkipGpuThreads = true,
    SkipEvents = true,
    EmitThreadFrameEnd = true,
    --GroupIds = {},
    --SkipTimerIds = {},
    TimerIds = timerIds,
    ThreadIds = workerIds,
    StartFrameId = 0, -- zero means start from the first available frame (default)
    EndFrameId = 0, -- zero means stop at the last available frame (default)
})

-- An enhanced iterator that attributes portions of cross-frame scopes to each frame in which they were active, making it suitable for precise per-frame analysis.
-- However, this approach cannot reconstruct the depths of the scope stack elements at the very beginning of startFrame. For use cases where only scope duration is important, this is sufficient.
-- As a workaround for stack reconstruction, you can start iterating one frame earlier (if it exists) to "warm up" the scope stack before entering the first frame.
-- iter's LogIteratorConfig must have EmitThreadFrameEnd set to true.
local function iterateLogFrameLocal(session, iter, startFrame, endFrame, process_scope)
    local iterState = iter:GetState()
    iter:RewindTo(startFrame, endFrame)
    -- Note: iter:Rewind() resets the iterator to its configured state and restores
    -- the config's start and end frame IDs. It also resets thread scope stacks.

    while iter:Step() do
        if iterState:IsThreadFrameEnd() then
            -- The current thread log has reached its end for the current frame,
            -- so we can scan the current scope stack to account for the portions
            -- of those scopes that belong to the current frame.
            
            local exitTimestamp = iterState:Timestamp()
            local maxLocalDepth = iterState:ThreadStackDepth()
            
            if maxLocalDepth > 0 then
                for localDepth = maxLocalDepth - 1, 0, -1 do
                    -- If the current thread stack is overflowed, nil elements will be returned first,
                    -- followed by the actual elements.
                    
                    local el = iter:GetCurrentThreadStackElement(localDepth)
                    if el then
                        process_scope(
                            iterState:FrameId(),
                            iterState:ThreadId(),
                            localDepth,
                            el:EnterTimerId(),
                            el:EnterTimestampFrameLocal(),
                            exitTimestamp
                        )
                    end
                end
            end
        
        elseif iterState:IsExit() and not iterState:ThreadStackWasOverflowed() then
            local depth = iterState:ThreadStackDepth()
            local enterTimestamp
            
            if not iterState:ThreadStackIsUnderflowed() then
                -- Use el:EnterTimestampFrameLocal() to account only for the portion
                -- of the total scope time that belongs to the current frame.
                -- It is clamped to the current frame start time:
                --   * if the scope started before the current frame, it returns the current frame start time;
                --   * if the scope started during the current frame, it returns the same value as el:EnterTimestamp().
                
                local el = iter:GetCurrentThreadStackElement(depth)
                enterTimestamp = el:EnterTimestampFrameLocal()
            else
                -- If the scope stack is underflowed, this means that we found a scope exit without previously visiting its corresponding enter entry (because the enter entry occurred before the specified frame range).
                -- Since we are only interested in the portion of time that the scope spent in the current frame, we use the frame start time as the enter timestamp.
                -- We use a simple approach here. For a more optimized solution, see the more complex example below.
                
                local frameDesc = session:FetchFrameDesc(iterState:FrameId())
                enterTimestamp = if iterState:IsGpuThread() then frameDesc.TickStartGpu else frameDesc.TickStartCpu
            end
            
            process_scope(
                iterState:FrameId(),
                iterState:ThreadId(),
                depth,
                iterState:TimerId(),
                enterTimestamp,
                iterState:Timestamp()
            )
        end
    end
end

-- An advanced iterator that performs a two-pass full reconstruction of all scopes found in the frames, including the initial state and correct scope depths,
-- making it useful for the most demanding use cases, such as analyzing isolated, non-connected frames or the first frame in the sequence.
-- If there are extremely long scopes that start before startFrame and end after endFrame (so they are never mentioned in the iterated portion of the log), they will not be recovered.
-- If stack depth values are not needed, the second pass can be removed, making the implementation similar to iterateLogFrameLocal, but more optimized.
-- iter's LogIteratorConfig must have EmitThreadFrameEnd set to true.
local function iterateLogFullReconstruct(session, iter, startFrame, endFrame, process_scope)
    local scopes = {}

    local frameStartTickCpu = 0
    local frameStartTickGpu = 0
    local frameStartTickCurThread = 0

    local frameId = 0
    local threadId = 0
    local st = nil

    local iterState = iter:GetState()
    iter:RewindTo(startFrame, endFrame)

    while iter:Step() do
        -- IsFrameBoundary() is a special notification, as it is not related to any specific thread.
        if iterState:IsFrameBoundary() then
            -- If iterState:FrameId() is greater than the last frame id we requested, we have reached the end.
            -- iterState:FrameId() is continuously incrementing and represents how frames are stored
            -- in the profiler. iterState:FrameAbsoluteId() can contain gaps if the profiler was not always
            -- capturing, since it represents real rendered frame numbers.
            frameId = iterState:FrameId()

            -- Natively captures the start of the current frame (or the virtual N+1 end-frame start timestamp)
            frameStartTickCpu = iterState:Timestamp() -- CPU time
            frameStartTickGpu = iterState:TimestampGpu() -- GPU time

            -- If you pause and resume MicroProfiler capturing, the capture
            -- will contain stitches. These are automatically detected, and the scope stack is reset.
            -- You might want to reset your implementation-specific state here.
            -- if iterState:IsFrameStitching() then
            -- end

            -- IsFrameBoundary() is the last chance to consider scope states across all threads
            -- before moving to the next frame. Use iter:GetThreadStackState(threadId) and
            -- iter:GetThreadStackElement(threadId, depth) for this.
            
            continue
        end

        if iterState:ThreadChanged() then
            -- The IsFrameBoundary() notification is fired before ThreadChanged(),
            -- so we can remember the frame's CPU and GPU start times there
            -- and select the proper one here (depending on whether the current thread is a CPU or GPU thread).
            -- Alternatively, we can always rely on session:FetchFrameDesc(iterState:FrameId()) for CPU/GPU frame boundaries,
            -- but that is less performant.
            
            frameStartTickCurThread = if iterState:IsGpuThread() then frameStartTickGpu else frameStartTickCpu
            threadId = iterState:ThreadId()
            st = iter:GetThreadStackState(threadId)
        end

        -- ThreadChanged() and FrameChanged() are not separate notifications; they are part of the state when iterating over regular log entries (enter/exit scope entries).
        -- if iterState:FrameChanged() then
        -- end

        -- if iterState:IsEvent() then
        -- end

        if iterState:IsThreadFrameEnd() then
            local exitTimestamp = iterState:Timestamp()
            local maxLocalDepth = iterState:ThreadStackDepth()
            local currentAbsDepth = st:DepthAbsolute()

            -- Append active stack elements with their absolute depths
            if maxLocalDepth > 0 then
                for localDepth = maxLocalDepth - 1, 0, -1 do
                    local el = iter:GetCurrentThreadStackElement(localDepth)
                    if el then
                        local elAbsDepth = currentAbsDepth - (maxLocalDepth - localDepth)

                        table.insert(scopes, {
                            absDepth = elAbsDepth,
                            timerId = el:EnterTimerId(),
                            enterTimestamp = el:EnterTimestampFrameLocal(), -- Natively clamped
                            exitTimestamp = exitTimestamp,
                        })
                    end
                end
            end
            
            -- End of frame for this thread: normalize depths and process scopes
            local virtualZero = st:DepthAbsoluteMin()
            
            for _, scope in ipairs(scopes) do
                process_scope(
                    frameId,
                    threadId,
                    scope.absDepth - virtualZero,
                    scope.timerId,
                    scope.enterTimestamp,
                    scope.exitTimestamp
                )
            end
            
            -- Clear the buffer for the next thread in this frame
            table.clear(scopes)

        elseif iterState:IsExit() and not iterState:ThreadStackWasOverflowed() then
            local exitTimestamp = iterState:Timestamp()
            local currentAbsDepth = st:DepthAbsolute()
            
            -- Underflow fallback defaults to our perfectly tracked frameStartTickCurThread
            local enterTimestamp = frameStartTickCurThread 
            
            if not iterState:ThreadStackIsUnderflowed() then
                local localDepth = iterState:ThreadStackDepth() -- Same as st:Depth()
                local el = iter:GetCurrentThreadStackElement(localDepth)
                enterTimestamp = el:EnterTimestampFrameLocal() -- Natively clamped!
            end
            
            -- Append the scope with its absolute depth
            table.insert(scopes, {
                absDepth = currentAbsDepth,
                timerId = iterState:TimerId(),
                enterTimestamp = enterTimestamp,
                exitTimestamp = exitTimestamp
            })
        end
    end
end

-- A practical hybrid iterator that performs full stack reconstruction only for the first frame,
-- and then uses a single-pass iterator for the remaining frames.
-- Uncovered case: if there is frame stitching between the start and end frames
-- (i.e., two separate capture segments stitched together, e.g. due to pausing and resuming the capture),
-- then full stack reconstruction will not be performed again upon stitch detection.
local function iterateLogOptimal(session, iter, startFrame, endFrame, process_scope)
    iterateLogFullReconstruct(session, iter, startFrame, startFrame, process_scope)
    if endFrame > startFrame then
        -- A new iterator pass starts from the frame immediately following the final frame
        -- of the previous pass, so per-thread scope stacks will be preserved
        -- as if we had performed a single seamless pass.
        iterateLogFrameLocal(session, iter, startFrame + 1, endFrame, process_scope)
    end
end

function processScope(frameId, threadId, depth, timerId, enterTimestamp, exitTimestamp)
    -- print(frameId, threadId, depth, timerId, enterTimestamp, exitTimestamp)
    -- local threadName = session:FetchThreadDesc(threadId).ThreadName
    -- local timerName = session:FetchTimerDesc(timerId).TimerName 
end

local function renderMiniUi()
    local frameIdMin = session:GetFrameIdMin()
    local frameIdMax = session:GetFrameIdMax()
    local globalDesc = session:FetchGlobalDesc()
    local tickToMs = globalDesc.TickToMsCpu
    -- Use TickToMsGpu for GPU thread time conversion

    if frameIdMin == 0 and frameIdMax == 0 then
        return -- No frames available
    end

    local maxFramesN = 5
    local frameIdStart = math.max(frameIdMin, frameIdMax - maxFramesN + 1)

    -- Print last N frame times --------
    for i = frameIdStart, frameIdMax do
        -- Use FetchFrameDesc for simplicity or when reused in multiple places in the code.
        -- Use GetFrameDesc for speed when accessing only a few fields immediately.
        local frameDesc = session:GetFrameDesc(i)
        local duration = frameDesc:TickEndCpu() - frameDesc:TickStartCpu()
        local frameTimeMs = duration * tickToMs
        -- print(frameTimeMs)
    end
    
    -- Process scopes ------------------
    iterateLogOptimal(session, iter, frameIdStart, frameIdMax, processScope)

    -- Fetch counter samples -----------
    local samples = session:GetCounterSamples(cid)
    local s1 = samples:FetchSampleByFrameId(frameIdMax)
    if s1 ~= nil then
        -- print(s1.Value)
    end
    local sampleArray1 = samples:FetchAll()
    if #sampleArray1 > 0 then
        -- print(sampleArray1[1].FrameId)
    end
    samples:Dispose() -- Don't forget to dispose of the wrapped array object!
    samples = nil

    local sampleArray2 = session:FetchCounterSamples(cid)
    if #sampleArray2 > 0 then
        -- print(sampleArray2[1].FrameId)
    end

    local sampleLast1 = session:GetLastCounterSample(cid)
    if sampleLast1 ~= nil then
        -- print(sampleLast1:Value())
    end
    local sampleLast2 = session:FetchLastCounterSample(cid)
    if sampleLast2 ~= nil then
        -- print(sampleLast2.FrameId)
        -- print(sampleLast2.Value)
    end
end

local function iterateCountersSubtrees(session, rootIds)
    local cIter = session:CreateCounterIterator()
    local state = cIter:GetState()
    
    local config = {
        RootCounterIds = rootIds -- If an empty array is provided, we iterate over all counters.
    }
    
    cIter:Configure(config)
    
    while cIter:Step() do
        -- Indent based on relative tree depth from our chosen roots
        local indent = string.rep("  ", state:RelativeLevel())
        
        local counterId = state:CounterId()
        local name = session:FetchCounterDesc(counterId).CounterName
        local isLeaf = state:IsLeaf() and "Yes" or "No"
        
        local info = string.format(
            "(Lvl: %d Rel_Lvl: %d Root_Lvl: %d, Leaf: %s, ID: %d)", 
            state:Level(), state:RelativeLevel(), state:RootLevel(), isLeaf, counterId
        )
        
        -- Build the stack path string
        local pathStr = ""
        if state:HierarchyChanged()  then
            local pathSize = state:Level() + 1
            local pathArr = {}
            
            for i = 0, state:Level() do
                table.insert(pathArr, cIter:GetStackElement(i))
            end
            
            pathStr = string.format(" [Path Size: %d] %s", pathSize, table.concat(pathArr, "|"))
        end
        
        print(indent .. "- " .. name .. " " .. info .. pathStr)
    end

    cIter:Dispose()
end

local function iterateCounters()
    -- local customRoots = {}
    local rootCounterId = session:FindCounterId("**/Luau")
    local customRoots = {rootCounterId}
    iterateCountersSubtrees(session, customRoots)
end

local RunService = game:GetService("RunService")
RunService.Heartbeat:Connect(function(deltaTime)
    renderMiniUi()
end)

iterateCounters()
renderMiniUi()

-- iter:Dispose() -- Disposing of the log iterator first, while the session still exists.
-- session:Dispose()
-- iter = nil
-- session = nil
```


---

## LibMP API Documentation

LibMP is a universal MicroProfiler API. The Luau library is a wrapper around the WebAssembly C-API and utilizes FlatBuffers for highly efficient data serialization and extraction.

Syntax Note: All API methods are dynamic (called via `:`), with the exception of constructors like `Session.OpenFrom***` and direct global methods like `LibMP.***`, which use a dot (`.`).

---

### 1. Core Library (`LibMP`)

* **`LibMP.IsCliMode`** `(boolean)`: Returns `true` if the environment is a CLI.
* **`LibMP.GetMemUsed()`** `-> number`: Returns the current memory usage of the underlying WASM instance.
* **`LibMP.Versions`**: Instance of the `Versions` class. Contains `Versions.Library` and `Versions.DataFormat` `(number)`.
* **`LibMP.Control`**: A global `Control` instance. This maps exactly to **`LibMP.Control.Global`**.

---

### 2. Main Classes

#### `Control`
* **`:IsBackendReady()`** `-> boolean`: Checks if the API is ready to use (the remote is accessible and versions are compatible).
* **`:IsBackendAccessible()`** `-> boolean`: Checks if the remote API is accessible.
* **`:IsBackendVersionCompatible()`** `-> boolean`: Checks version capability.
* **`:EnableProfiler(enable: boolean)`** `-> boolean`: Toggles the profiler.
* **`:EnableCapture(enable: boolean)`** `-> boolean`: Activates/pauses data capture.
* **`:SetFrameLimit(num: number)`** `-> boolean`: Sets the rolling frame limit.
* **`:ShowUI(show: boolean)`** `-> boolean`: Toggles the on-screen UI (Studio-only feature).
* **`:CaptureToBufferSync()`** `-> buffer`: Captures the accumulated live data (collected frames) into a newly created buffer and returns it.
* **`:UseControlChannel(use: boolean)`** `-> boolean`: Enables or disables the control channel.

---

#### `Session`
Represents a MicroProfiler session - the main object you interact with. Contains internal caching mechanisms to optimize repetitive reads.

> **Important Return Type Distinction:**
> *   Methods prefixed with **`Get`** return a **FlatBuffer (FB) object view** (e.g., `FB view (TimerDesc)`). These views do not contain raw fields; you must call their getter methods to retrieve data (e.g., `obj:TimerName()`). Views must be accessed immediately, as they are invalidated by any subsequent `Get***` or `Fetch***` call. Array equivalents return wrapped arrays (e.g., `StructArray<FB.U64Sample>`) that persist until manually disposed. Iterator state returned via `:GetState()` persists and is updated as long as the iterator exists.
> *   Methods prefixed with **`Fetch`** return a **native Luau table** deeply unpacked from the FlatBuffer (e.g., `Luau table (TimerDesc)`). Properties are directly accessible fields (e.g., `obj.TimerName`). Array equivalents return standard Luau arrays of these tables (e.g., `Array of Luau tables (U64Sample)`).

##### Constructors
* **`Session.OpenFromMem(data: string | buffer)`** `-> Session?`: Opens a session from a string or buffer. Note that this method creates an internal copy of the provided string or buffer before working with it.
* **`Session.OpenFromBuffer(b: buffer)`** `-> Session?`: Opens a session from a raw Luau buffer object.
* **`Session.OpenFromSlot(id: number)`** `-> Session?`: Opens a session directly from a MicroProfiler data slot.
* **`Session.OpenFromFile(path: string, [cacheSize: number])`** `-> Session?`: Opens a session from a file.
* **`Session.OpenFromLiveData()`** `-> Session?`: Opens from data slot 0 in Roblox Engine, or from a file path provided via command-line arguments in CLI mode.
* **`Session.OpenFromExternalProvider(io: table, [cacheSize: number])`** `-> Session?`: Advanced constructor requiring a custom IO handler.

##### Lifecycle & State
* **`:Dispose()`** `-> nil`: Cleans up and unregisters the session.
* **`:IsValid()`** `-> boolean`: Checks if the session is valid.
* **`:GetDataFormatVersion()`** `-> number`: Returns the session's data format version.
* **`:GetObjSize()`** `-> number`: Returns the size of the last accessed object. If caching is enabled (default behavior), this will not reflect the size of objects that were cached and later accessed again.
* **`:WithDataLock(callback: function)`** `-> (boolean, any)`: Safely locks the first accessed object data, executes the callback, and automatically unlocks (returning success status and callback result). This is only required in specific cases where you retrieve a data view via a `Get***` method and need to ensure it does not expire or get overwritten after subsequent `Get***` calls. **Mechanism:** The data view returned by the *first* `Get***` method executed within the lock helper will persist and remain valid until the callback finishes and the lock is released.

##### Synchronization & Caching
* **`:SyncWithDataSource()`** `-> boolean`: Synchronizes the session with its active data source. Live data sessions call this method automatically when you first access their data after it has been updated (e.g., a new game frame arrives).
* **`:SetSyncRequired()`** `-> nil`: Marks the session as requiring a sync on the next data access.
* **`:SetClearCacheOnSyncRequired()`** `-> nil`: Flags the cache to be cleared upon the next synchronization.
* **`:SyncIfRequired()`** `-> nil`: Executes a sync if `SetSyncRequired()` was previously called.
* **`:EnableObjectCache(enabled: boolean)`** `-> nil`: Toggles the Luau-side and C-side object caching for descriptors. Caching is enabled by default.
* **`:ClearObjectCache()`** `-> nil`: Force-clears the cache.
* **`:ClearLuauCacheBucket(bucketIdx: number)`** `-> nil`: Clears a specific bucket in the Luau cache (1 = ShortTerm, 2 = LongTerm).

##### Core Getters / Fetchers
* **Global & General**: 
    * `:GetGlobalDesc()` `-> FB view (GlobalDesc)`
    * `:FetchGlobalDesc()` `-> Luau table (GlobalDesc)`
    * `:GetGeneralInfo()` `-> FB view (GeneralInfo)`
    * `:FetchGeneralInfo()` `-> Luau table (GeneralInfo)`
* **Groups**: 
    * `:GetGroupIdMax()` `-> number`
    * `:GetGroupDesc(groupId: number)` `-> FB view (GroupDesc)`
    * `:FetchGroupDesc(groupId: number)` `-> Luau table (GroupDesc)`
* **Timers**: 
    * `:GetTimerIds()` `-> ScalarArray<number>`
    * `:FetchTimerIds()` `-> Array of numbers`
    * `:GetTimerDesc(timerId: number)` `-> FB view (TimerDesc)`
    * `:FetchTimerDesc(timerId: number)` `-> Luau table (TimerDesc)`
* **Counters**: 
    * `:GetCounterIds()` `-> ScalarArray<number>`
    * `:FetchCounterIds()` `-> Array of numbers`
    * `:GetCounterDesc(counterId: number)` `-> FB view (CounterDesc)`
    * `:FetchCounterDesc(counterId: number)` `-> Luau table (CounterDesc)`
* **Threads**: 
    * `:GetThreadIds()` `-> ScalarArray<number>`
    * `:FetchThreadIds()` `-> Array of numbers`
    * `:GetThreadIdGpu()` `-> number`
    * `:GetThreadDesc(threadId: number)` `-> FB view (ThreadDesc)`
    * `:FetchThreadDesc(threadId: number)` `-> Luau table (ThreadDesc)`
* **Frames**: 
    * `:GetFrameIdMin()` `-> number`
    * `:GetFrameIdMax()` `-> number`
    * `:GetFrameDesc(frameId: number)` `-> FB view (FrameDesc)`
    * `:FetchFrameDesc(frameId: number)` `-> Luau table (FrameDesc)`
* **Data Samples**: 
    * `:GetCounterSamples(counterId: number)` `-> CounterSampleArray<FB.CounterSample>`
    * `:FetchCounterSamples(counterId: number)` `-> Array of Luau tables (CounterSample)`
    * `:GetLastCounterSample(counterId: number)` `-> FB view (CounterSample)`
    * `:FetchLastCounterSample(counterId: number)` `-> Luau table (CounterSample)`
    * `:GetPlaceIdSamples()` `-> StructArray<FB.U64Sample>`
    * `:FetchPlaceIdSamples()` `-> Array of Luau tables (U64Sample)`
    * `:GetUtcTimestampSamples()` `-> StructArray<FB.U64Sample>`
    * `:FetchUtcTimestampSamples()` `-> Array of Luau tables (U64Sample)`

##### Search & Filtering
* **`:FindGroupId(nameMask: string, caseSensitive: boolean)`** `-> number`
* **`:FindGroupIds(nameMask: string, caseSensitive: boolean)`** `-> Array of numbers`
* **`:FindTimerId(nameMask: string, caseSensitive: boolean)`** `-> number`
* **`:FindTimerIds(nameMask: string, caseSensitive: boolean)`** `-> Array of numbers`
* **`:GetTimerIdsByGroupId(groupId: number)`** `-> ScalarArray<number>`
* **`:FetchTimerIdsByGroupId(groupId: number)`** `-> Array of numbers`
* **`:GetTimerIdsByGroupNames(nameMask: string, caseSensitive: boolean)`** `-> ScalarArray<number>`
* **`:FetchTimerIdsByGroupNames(nameMask: string, caseSensitive: boolean)`** `-> Array of numbers`
* **`:FindCounterId(nameMask: string, caseSensitive: boolean)`** `-> number`
* **`:FindCounterIds(nameMask: string, caseSensitive: boolean)`** `-> Array of numbers`
* **`:FindThreadId(nameMask: string, caseSensitive: boolean)`** `-> number`
* **`:FindThreadIds(nameMask: string, caseSensitive: boolean)`** `-> Array of numbers`

##### Iterators
* **`:CreateLogIterator()`** `-> LogIterator`
* **`:CreateCounterIterator()`** `-> CounterIterator`

---

#### `LogIterator`
* **`:Configure(configObj: Luau table (LogIteratorConfig))`** `-> nil`: Accepts a Luau table matching `LogIteratorConfig`.
* **`:Step()`** `-> boolean`: Advances the iterator. Returns false if finished.
* **`:Rewind()`** `-> nil`: Resets the iterator.
* **`:RewindTo(frameIdMin: number, frameIdMax: number)`** `-> nil`: Rewinds the iterator to a specific frame ID range.
* **`:GetState()`** `-> FB view (LogIteratorState)`: Returns a wrapped `LogIteratorState` struct.
* **`:GetThreadStackState(threadId: number)`** `-> FB view (LogIteratorThreadStackState)`: Returns a wrapped stack state for a specific thread.
* **`:GetThreadStackElement(threadId: number, stackLevel: number)`** `-> FB view (LogIteratorThreadStackElement)`: Returns a specific stack element for a thread. stackLevel is 0-based.
* **`:GetCurrentThreadStackElement(stackLevel: number)`** `-> FB view (LogIteratorThreadStackElement)`: Returns a wrapped element from the current thread's stack. stackLevel is 0-based.
* **`:Dispose()`** `-> nil`: Cleans up the iterator's resources.

---

#### `CounterIterator`
* **`:Configure(configObj: Luau table (CounterIteratorConfig))`** `-> nil`: Accepts a Luau table matching `CounterIteratorConfig`.
* **`:Step()`** `-> boolean`: Advances the iterator.
* **`:Rewind()`** `-> nil`: Resets the iterator.
* **`:GetState()`** `-> FB view (CounterIteratorState)`: Returns a wrapped `CounterIteratorState`.
* **`:GetStackElement(stackLevel: number)`** `-> number`: Returns the counter ID at the specified stack level. stackLevel is 0-based.
* **`:Dispose()`** `-> nil`: Cleans up the iterator's resources.

---

#### Array Classes
Returned by many `Session` list getters. Contains either a scalar value or FlatBuffer elements.
* **`ScalarArray<T>`**: Wraps an array of scalar elements (like `number`).
* **`StructArray<T>`**: Wraps an array of FB structs (like `FB.U64Sample`).
* **`TableArray<T>`**: Wraps an array of FB tables.
* **`CounterSampleArray<T>`**: Specialized wrapper for an array of `FB.CounterSample` elements.

**Common Methods:**
* **`:Size()`** `-> number`: Returns the number of elements.
* **`:Get(index: number)`** `-> FB view | number`: Returns the element/wrapped FB object at the 0-based index.
* **`:Fetch(index: number)`** `-> Luau table | number`: Returns the unpacked native Luau table or scalar value at the index.
* **`:FetchAll()`** `-> Array of Luau tables | Array of numbers`: Returns a 1-based Luau table containing all deeply unpacked items.
* **`:Dispose()`** `-> nil`: Releases the array's resources.

*Specific to `CounterSampleArray`:*
* **`:GetSampleByFrameId(frameId: number)`** `-> FB view (CounterSample)`
* **`:FetchSampleByFrameId(frameId: number)`** `-> Luau table (CounterSample)`
* **`:GetLastSample()`** `-> FB view (CounterSample)`
* **`:FetchLastSample()`** `-> Luau table (CounterSample)`

---

### 3. Unpacked FlatBuffer Tables

When using `Fetch***` methods or providing configuration objects to Iterators, the returned/accepted Native Luau tables mimic these FlatBuffer definitions. 

#### Core Descriptors

**`GlobalDesc`**
* `TickToMsCpu`: number (float)
* `TickToMsGpu`: number (float)

**`GeneralInfo`**
* `PlatformInfoJson`: string

**`GroupDesc`**
* `GroupId`: number (uint)
* `GroupName`: string
* `Color`: number (uint)
* `IsGpu`: boolean

**`TimerDesc`**
* `TimerId`: number (uint)
* `TimerName`: string
* `GroupId`: number (uint)
* `Color`: number (uint)
* `IsUserTimer`: boolean

**`CounterDesc`**
* `CounterId`: number (uint)
* `CounterName`: string
* `Parent`: number (uint)
* `Sibling`: number (uint)
* `FirstChild`: number (uint)
* `Level`: number (ubyte)

**`ThreadDesc`**
* `ThreadId`: number (uint)
* `ThreadName`: string
* `BufferSize`: number (uint)
* `IsGpu`: boolean

**`FrameDesc`**
* `FrameId`: number (uint)
* `FrameAbsoluteId`: number (uint)
* `TickStartCpu`: number (ulong)
* `TickEndCpu`: number (ulong)
* `TickStartGpu`: number (ulong)
* `TickEndGpu`: number (ulong)
* `LabelOffsetMin`: number (ulong)
* `LabelOffsetMax`: number (ulong)
* `IsPaused`: boolean
* `IsIncomplete`: boolean
* `ThreadLogs`: Array of `ThreadLogDescr` objects.

#### Data & Samples

**`CounterSample`**
* `FrameId`: number (uint)
* `Value`: number (double)

**`U64Sample`**
* `FrameId`: number (uint)
* `Value`: number (ulong)

**`ThreadLogDescr`**
* `ThreadId`: number (uint)
* `LogEntriesNum`: number (uint)
* `LogStartLocator`: number (ulong)
* `IsRemoved`: boolean

#### Iterators & State

**`LogIteratorConfig`**
* `StartFrameId`: number (uint, default: 0)
* `EndFrameId`: number (uint, default: 0)
* `SkipGpuThreads`: boolean
* `SkipEvents`: boolean
* `SkipPausedFrames`: boolean
* `SkipFrameBoundaries`: boolean
* `SkipTimestampNormalization`: boolean
* `SkipThreadStackResume`: boolean
* `SkipThreadStackDepthSync`: boolean
* `EmitThreadFrameEnd`: boolean
* `ThreadIds`: Array of numbers
* `GroupIds`: Array of numbers
* `TimerIds`: Array of numbers
* `SkipTimerIds`: Array of numbers

**`LogIteratorThreadStackState`**
* `Depth`: number (uint)
* `DepthAbsolute`: number (int)
* `DepthAbsoluteMin`: number (int)
* `IsUnderflowed`: boolean
* `IsOverflowed`: boolean
* `WasOverflowed`: boolean

**`LogIteratorState`**
* `Started`: boolean
* `Finished`: boolean
* `ThreadStackDepth`: number (uint)
* `ThreadStackIsUnderflowed`: boolean
* `ThreadStackIsOverflowed`: boolean
* `ThreadStackWasOverflowed`: boolean
* `ThreadPrevTimestamp`: number (ulong)
* `FrameId`: number (uint)
* `FrameAbsoluteId`: number (uint)
* `FrameDescLocator`: number (ulong)
* `ThreadId`: number (uint)
* `IsGpuThread`: boolean
* `FrameChanged`: boolean
* `ThreadChanged`: boolean
* `IsFrameStitching`: boolean
* `EntryLocator`: number (ulong)
* `TimerId`: number (uint)
* `Timestamp`: number (ulong)
* `TimestampGpu`: number (ulong)
* `EntryTypeRaw`: number (uint)
* `EventType`: number (uint)
* `IsUnknown`: boolean
* `IsEnter`: boolean
* `IsExit`: boolean
* `IsLabel`: boolean
* `IsEvent`: boolean
* `IsThreadFrameEnd`: boolean
* `IsFrameBoundary`: boolean

**`LogIteratorThreadStackElement`**
* `EnterFrameId`: number (uint)
* `EnterTimerId`: number (uint)
* `EnterEntryLocator`: number (ulong)
* `EnterTimestamp`: number (ulong)
* `EnterTimestampFrameLocal`: number (ulong)
* `CustomData`: `CustomData16b` object (`B0` through `B15`)

**`CounterIteratorConfig`**
* `RootCounterIds`: Array of numbers

**`CounterIteratorState`**
* `Started`: boolean
* `Finished`: boolean
* `CounterId`: number (uint)
* `ParentId`: number (uint)
* `Level`: number (uint)
* `RelativeLevel`: number (uint)
* `RootLevel`: number (uint)
* `IndexInSiblings`: number (uint)
* `IsLeaf`: boolean
* `HierarchyChanged`: boolean
* `CounterStackIsOverflowed`: boolean

---

### 4. Known Issues

`GeneralInfo.PlatformInfoJson` - not implemented; always empty  
`TimerDesc.IsUserTimer` - not implemented; always false  
