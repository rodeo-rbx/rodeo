---
title: process
---

```luau
local process = require("@rodeo/process")
```
:::caution
These APIs are not finalized and may change in backwards incompatible ways.
:::

## Summary

| Entry | Description |
| :--- | :--- |
| [ProcessHandle](#processhandle) | Handle to a process started with `create`. Fields are populated when |
| [ProcessResult](#processresult) | Result returned by `run` and `system`. |
| [ProcessRunOptions](#processrunoptions) | Options accepted by `run`, `system`, and `create`. All fields optional. |
| [StdioKind](#stdiokind) | How a stdio stream should be wired for a spawned process. `"default"` |
| [args](#processargs) | The command-line arguments passed to this rodeo execution after `--`. |
| [create](#processcreate) | Spawns `args` as a child process without waiting. Returns a handle with |
| [cwd](#processcwd) | Returns the current working directory. |
| [env](#processenv) | A read-only view of environment variables visible to this execution. |
| [execpath](#processexecpath) | Returns the path of the running rodeo binary. |
| [exit](#processexit) | Exits the current execution with the given code. |
| [homedir](#processhomedir) | Returns the user's home directory. |
| [kill](#processkill) | Terminates a process started with `create`. |
| [platform](#processplatform) | Returns the run client's OS: `"macos"`, `"windows"`, or `"linux"`. |
| [run](#processrun) | Runs `args` (or an existing handle), blocking until completion. Returns |
| [system](#processsystem) | Like `run`, but takes a single shell-style command string. |

---

## Types

### ProcessHandle

Handle to a process started with `create`. Fields are populated when

the corresponding stdio is `"piped"`; otherwise `nil`.

```luau
type ProcessHandle = {
	__rodeo_pid: string,
	stdin: stream.StreamHandle?,
	stdout: stream.StreamHandle?,
	stderr: stream.StreamHandle?,
}
```

---

### ProcessResult

Result returned by `run` and `system`.

`out` and `err` are the captured stdout and stderr, empty when `run` waits on a `create` handle.

```luau
type ProcessResult = {
	ok: boolean, -- true when the process exited with status 0
	exitcode: number, -- -1 when the process was terminated by a signal
	out: string,
	err: string,
}
```

---

### ProcessRunOptions

Options accepted by `run`, `system`, and `create`. All fields optional.

`stdio` applies to all three streams; per-stream overrides win.

`env` adds variables to the inherited environment.

`input` (`run` and `system` only) is written to the child's stdin, which is then closed; without it, a child that reads stdin gets end-of-file.

```luau
type ProcessRunOptions = {
	cwd: string?,
	stdio: StdioKind?,
	stdin: StdioKind?,
	stdout: StdioKind?,
	stderr: StdioKind?,
	env: { [string]: string }?,
	input: (string | buffer)?,
}
```

---

### StdioKind

How a stdio stream should be wired for a spawned process. `"default"`

uses the parent's stream, `"piped"` captures into a `StreamHandle`,

`"inherit"` passes through, `"none"` discards, `"tee"` mirrors to both

a capture and the parent's stream.

```luau
type StdioKind = "default" | "piped" | "inherit" | "none" | "tee"
```

---

## Functions and Properties

### process.args

The command-line arguments passed to this rodeo execution after `--`.

```luau
any) :: { string }
```

---

### process.create

Spawns `args` as a child process without waiting. Returns a handle with

stdin/stdout/stderr stream handles for live interaction.

```luau
(args: { string }, options: ProcessRunOptions?) -> ProcessHandle
```

---

### process.cwd

Returns the current working directory.

```luau
() -> string
```

---

### process.env

A read-only view of environment variables visible to this execution.

```luau
any) :: { [string]: string? }
```

---

### process.execpath

Returns the path of the running rodeo binary.

```luau
() -> string
```

---

### process.exit

Exits the current execution with the given code.

```luau
(code: number) -> ()
```

---

### process.homedir

Returns the user's home directory.

```luau
() -> string
```

---

### process.kill

Terminates a process started with `create`.

```luau
(handle: ProcessHandle) -> ()
```

---

### process.platform

Returns the run client's OS: `"macos"`, `"windows"`, or `"linux"`.

```luau
() -> string
```

---

### process.run

Runs `args` (or an existing handle), blocking until completion. Returns

captured stdout/stderr and exit code.

```luau
(argsOrHandle: { string } | ProcessHandle, options: ProcessRunOptions?) -> ProcessResult
```

---

### process.system

Like `run`, but takes a single shell-style command string.

```luau
(command: string, options: ProcessRunOptions?) -> ProcessResult
```

---
