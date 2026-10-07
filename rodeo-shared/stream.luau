-- @rodeo/stream API over typed RPC.
-- Public surface: read, write, close. The split (readChunk/readLine/readAll) lives at
-- the RPC layer only — `read` dispatches based on handle kind.
-- Factory: (rpc, runtime) -> stream. `runtime` is the caller's proto module,
-- passed in rather than read from _G: several rodeo plugins can share one VM.
return function(rpc, runtime)
	runtime = runtime or _G.__rodeo_runtime_proto -- standalone hosts (rodeo-shared/run.luau)
	local stream = {}

	local function readChunk(handle, size: number?): runtime.StreamReadChunkResponse
		return rpc.call("stream_read_chunk", runtime.StreamReadChunkRequest.new({
			handle = handle.__handle,
			size = size,
		}))
	end

	local function readLine(handle): runtime.StreamReadLineResponse
		return rpc.call("stream_read_line", runtime.StreamReadLineRequest.new({
			handle = handle.__handle,
		}))
	end

	local function readAll(handle): runtime.StreamReadAllResponse
		return rpc.call("stream_read_all", runtime.StreamReadAllRequest.new({
			handle = handle.__handle,
		}))
	end

	-- Public read: dispatches based on handle prefix convention.
	--   "stdin"                → readLine
	--   "file:..."             → readAll
	--   "proc:<pid>:stdout"    → readChunk (up to `size` bytes, nil at EOF)
	--   "proc:<pid>:stderr"    → readChunk (up to `size` bytes, nil at EOF)
	function stream.read(handle, size: number?): string?
		local h = handle.__handle
		if h == "stdin" then
			local ok, r = pcall(readLine, handle)
			if not ok then return nil end
			return r.data
		elseif h:sub(1, 5) == "file:" then
			local ok, r = pcall(readAll, handle)
			if not ok then return nil end
			return r.data
		elseif h:match("^proc:.+:std%w+$") then
			local ok, r = pcall(readChunk, handle, size)
			if not ok or r.eof then return nil end
			return r.data
		else
			return nil
		end
	end

	-- Each write is one RPC message to the host; oversized messages kill the
	-- transport instead of erroring, so large payloads are split into chunks.
	-- (repeat/until so an empty write still makes one call, matching the
	-- unchunked behavior.)
	--
	-- With the Rust hops raised to 16MiB envelopes (MAX_RPC_MESSAGE_SIZE in
	-- rodeo-proto), the binding limit is now the plugin↔backend WS hop's
	-- 16MB (16,777,216-byte) frames under worst-case ~2x JSON escaping
	-- inflation: 8,000,000 * 2 = 16,000,000 leaves ~777KB of headroom for
	-- the RPC wrapper (~170 bytes measured) and the handle path. An
	-- oversized chunk is over a cap BY CONSTRUCTION and deterministically
	-- kills the transport, so the margin errs conservative.
	local WRITE_CHUNK = 8_000_000

	function stream.write(handle, data)
		local s = tostring(data)
		local len = #s
		local offset = 1
		repeat
			local last = math.min(offset + WRITE_CHUNK - 1, len)
			if last < len then
				-- Cuts must land on codepoint boundaries: every hop (the RPC
				-- JSONEncode, WS text frames, serde on the host) rejects
				-- invalid UTF-8, and a chunk holding half a codepoint hangs
				-- the call. utf8.offset(0) gives the start of the codepoint
				-- containing the byte after the cut; if that codepoint
				-- straddles the cut, end the chunk just before it. (The
				-- offset guard keeps progress on non-UTF-8 data, which the
				-- transport rejects regardless.)
				local cpStart = utf8.offset(s, 0, last + 1)
				if cpStart ~= nil and cpStart > offset then
					last = cpStart - 1
				end
			end
			rpc.call("stream_write", runtime.StreamWriteRequest.new({
				handle = handle.__handle,
				data = string.sub(s, offset, last),
			}))
			offset = last + 1
		until offset > len
	end

	-- Same WS frame limit as WRITE_CHUNK, but bytes fields ride the WS/JSON
	-- hop base64-encoded — exactly 4/3 inflation, and base64 output is pure
	-- ASCII so no further JSON escaping applies: 12,000,000 * 4/3 =
	-- 16,000,000, the same ~777KB headroom. The raw chunk is far under the
	-- Rust hops' 16MiB envelopes. No boundary concerns — base64 splits
	-- cleanly anywhere, unlike UTF-8 (chunks are separate base64 payloads,
	-- not slices of one encoding).
	local WRITE_CHUNK_BYTES = 12_000_000

	function stream.writeBytes(handle, data: buffer)
		local len = buffer.len(data)
		if len <= WRITE_CHUNK_BYTES then
			rpc.call("stream_write_bytes", runtime.StreamWriteBytesRequest.new({
				handle = handle.__handle,
				data = data,
			}))
			return
		end
		local offset = 0
		while offset < len do
			local n = math.min(WRITE_CHUNK_BYTES, len - offset)
			local chunk = buffer.create(n)
			buffer.copy(chunk, 0, data, offset, n)
			rpc.call("stream_write_bytes", runtime.StreamWriteBytesRequest.new({
				handle = handle.__handle,
				data = chunk,
			}))
			offset += n
		end
	end

	-- Reads ride the same capped hops in the response direction, so pull in
	-- chunks sized like WRITE_CHUNK_BYTES and reassemble. eof=true on the
	-- response that exhausts the source (a short chunk), so a source of any
	-- size under the cap still costs a single round trip.
	local READ_CHUNK_BYTES = 12_000_000

	function stream.readBytes(handle): buffer
		local chunks: { buffer } = {}
		local total = 0
		repeat
			local r: runtime.StreamReadBytesResponse = rpc.call("stream_read_bytes", runtime.StreamReadBytesRequest.new({
				handle = handle.__handle,
				size = READ_CHUNK_BYTES,
			}))
			table.insert(chunks, r.data)
			total += buffer.len(r.data)
		until r.eof
		if #chunks == 1 then
			return chunks[1]
		end
		local out = buffer.create(total)
		local offset = 0
		for _, chunk in chunks do
			buffer.copy(out, offset, chunk)
			offset += buffer.len(chunk)
		end
		return out
	end

	function stream.close(handle)
		rpc.call("stream_close", runtime.StreamCloseRequest.new({
			handle = handle.__handle,
		}))
	end

	return stream
end
