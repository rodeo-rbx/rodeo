import { tmpdir } from "node:os";
import { join } from "node:path";

/** `name` under the OS temp directory, with forward slashes. Safe to splice
 *  into Luau source as a string literal (a Windows path's backslashes would
 *  read as escape sequences), and every platform's file APIs accept it. A
 *  hardcoded `/tmp/...` resolves to `C:\tmp` on Windows, which need not exist. */
export function tempPath(name: string): string {
  return join(tmpdir(), name).replaceAll("\\", "/");
}
