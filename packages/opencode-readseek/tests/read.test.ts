import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import type { ToolContext } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

function createContext(abort = new AbortController().signal): ToolContext {
  return {
    sessionID: "session",
    messageID: "message",
    agent: "test",
    directory: "/repo",
    worktree: "/repo",
    abort,
    metadata() {},
    async ask() {},
  };
}

describe("readseek_read", () => {
  afterEach(() => {
    mock.restore();
  });

  test("constructs ranges for every offset and limit combination", async () => {
    const spawn = spyOn(Bun, "spawn").mockImplementation(
      () =>
        ({
          stdout: new Response("{}").body,
          stderr: new Response("").body,
          exited: Promise.resolve(0),
        }) as never,
    );
    const plugin = await ReadSeekPlugin({} as never);
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");
    const context = createContext();

    await read.execute({ path: "file.ts" }, context);
    await read.execute({ path: "file.ts", limit: 5 }, context);
    await read.execute({ path: "file.ts", offset: 3 }, context);
    await read.execute({ path: "file.ts", offset: 3, limit: 5 }, context);

    const args = spawn.mock.calls.map((call) => (call[0] as string[]).slice(2));
    expect(args).toEqual([
      ["read", "/repo/file.ts"],
      ["read", "/repo/file.ts", "--end", "5"],
      ["read", "/repo/file.ts:3"],
      ["read", "/repo/file.ts:3", "--end", "7"],
    ]);
  });

  test("passes cancellation and output limits to the subprocess", async () => {
    const controller = new AbortController();
    const spawn = spyOn(Bun, "spawn").mockReturnValue({
      stdout: new Response("{}").body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    } as never);
    const plugin = await ReadSeekPlugin({} as never);
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    await read.execute({ path: "file.ts" }, createContext(controller.signal));

    expect(spawn.mock.calls[0]?.[1]).toMatchObject({
      cwd: "/repo",
      killSignal: "SIGKILL",
      maxBuffer: 32 * 1024 * 1024,
      signal: controller.signal,
      stderr: "pipe",
      stdout: "pipe",
    });
  });

  test("does not spawn when already cancelled", async () => {
    const controller = new AbortController();
    controller.abort(new Error("cancelled"));
    const spawn = spyOn(Bun, "spawn");
    const plugin = await ReadSeekPlugin({} as never);
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    await expect(read.execute({ path: "file.ts" }, createContext(controller.signal))).rejects.toThrow("cancelled");
    expect(spawn).not.toHaveBeenCalled();
  });
});
