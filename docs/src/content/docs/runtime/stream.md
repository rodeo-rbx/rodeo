---
title: stream
---

```luau
local stream = require("@rodeo/stream")
```
:::caution
These APIs are not finalized and may change in backwards incompatible ways.
:::

## Summary

| Entry | Description |
| :--- | :--- |
| [StreamHandle](#streamhandle) | Opaque stream identifier. Pass these to `stream.read`/`stream.write`/ |
| [close](#streamclose) | Closes `handle`. Subsequent reads/writes error. |
| [read](#streamread) | Reads from `handle`. Returns the next chunk as a string, or `nil` on EOF. |
| [readBytes](#streamreadbytes) | Reads all remaining bytes from `handle` as a `buffer`. Use for binary |
| [write](#streamwrite) | Writes `data` to `handle`. `data` is converted to a string via `tostring`. |
| [writeBytes](#streamwritebytes) | Writes the bytes in `data` to `handle`. Use for binary data; for text |

---

## Types

### StreamHandle

Opaque stream identifier. Pass these to `stream.read`/`stream.write`/

`stream.close`; you don't access `__handle` directly.

```luau
type StreamHandle = {
	__handle: string,
}
```

---

## Functions and Properties

### stream.close

Closes `handle`. Subsequent reads/writes error.

```luau
(handle: StreamHandle) -> ()
```

---

### stream.read

Reads from `handle`. Returns the next chunk as a string, or `nil` on EOF.

For a process's stdout or stderr, a read returns as soon as any output is available, up to `size` bytes (default 4096).

```luau
(handle: StreamHandle, size: number?) -> string?
```

---

### stream.readBytes

Reads all remaining bytes from `handle` as a `buffer`. Use for binary

data; for text use `read`.

```luau
(handle: StreamHandle) -> buffer
```

---

### stream.write

Writes `data` to `handle`. `data` is converted to a string via `tostring`.

```luau
(handle: StreamHandle, data: any) -> ()
```

---

### stream.writeBytes

Writes the bytes in `data` to `handle`. Use for binary data; for text

use `write`.

```luau
(handle: StreamHandle, data: buffer) -> ()
```

---
