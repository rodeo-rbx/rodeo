import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { connect } from "node:net";
import { spawnBackground, waitForOwnedStudio, type BackgroundProcess } from "../helpers.js";

const PORT = 46340;
const BACKEND_PORT = PORT + 1;

// Sends a WebSocket handshake to the studio backend and resolves with the
// response's status line.
function handshake(origin?: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const socket = connect(BACKEND_PORT, "127.0.0.1");
    let data = "";
    socket.on("connect", () => {
      socket.write(
        "GET / HTTP/1.1\r\n" +
          `Host: 127.0.0.1:${BACKEND_PORT}\r\n` +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n" +
          "Sec-WebSocket-Version: 13\r\n" +
          (origin === undefined ? "" : `Origin: ${origin}\r\n`) +
          "\r\n",
      );
    });
    socket.on("data", (chunk) => {
      data += chunk;
      const end = data.indexOf("\r\n");
      if (end !== -1) {
        socket.destroy();
        resolve(data.slice(0, end));
      }
    });
    socket.on("error", reject);
  });
}

describe("studio backend WebSocket origin (CLI)", () => {
  let bg: BackgroundProcess;

  beforeAll(async () => {
    bg = spawnBackground(["run", "--port", String(PORT), "--place"]);
    // The launched Studio's plugin goes through the same check, so its edit
    // DOM registering shows Studio's handshake carries no Origin.
    await waitForOwnedStudio(PORT);
  });
  afterAll(async () => { bg.kill(); await bg.exited; });

  it("accepts a handshake without an Origin, as Studio sends it", async () => {
    expect(await handshake()).toBe("HTTP/1.1 101 Switching Protocols");
  });

  it("refuses a handshake from a web page", async () => {
    expect(await handshake("https://example.com")).toBe("HTTP/1.1 403 Forbidden");
  });

  it("refuses a handshake from an opaque origin", async () => {
    // Sandboxed iframes and file:// pages send the literal origin "null".
    expect(await handshake("null")).toBe("HTTP/1.1 403 Forbidden");
  });
});
