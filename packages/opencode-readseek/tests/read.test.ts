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

    const commands = spawn.mock.calls.map((call) => call[0] as string[]);
    for (const command of commands) expect(command[0]).toMatch(/readseek(?:\.exe)?$/);
    const args = commands.map((command) => command.slice(1));
    expect(args).toEqual([
      ["detect", "/repo/file.ts"],
      ["read", "/repo/file.ts", "--end", "2000"],
      ["detect", "/repo/file.ts"],
      ["read", "/repo/file.ts", "--end", "5"],
      ["detect", "/repo/file.ts"],
      ["read", "/repo/file.ts:3", "--end", "2002"],
      ["detect", "/repo/file.ts"],
      ["read", "/repo/file.ts:3", "--end", "7"],
    ]);
  });

  test("passes an explicitly selected auto image mode", async () => {
    const outputs = [
      JSON.stringify({ type: "image/png", width: 10, height: 10 }),
      "{}",
    ];
    const spawn = spyOn(Bun, "spawn").mockImplementation(
      () => ({
        stdout: new Response(outputs.shift() ?? "{}").body,
        stderr: new Response("").body,
        exited: Promise.resolve(0),
      }) as never,
    );
    const plugin = await ReadSeekPlugin({} as never, { imageMode: "auto" });
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    await read.execute({ path: "figure.png", image: "none" }, createContext());

    expect((spawn.mock.calls[1]?.[0] as string[]).slice(1)).toEqual([
      "read", "/repo/figure.png", "--image", "none",
    ]);
  });

  test("skips a detected visual file when image mode is omitted", async () => {
    const spawn = spyOn(Bun, "spawn").mockReturnValue({
      stdout: new Response('{"type":"image/png","width":10,"height":10}').body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    } as never);
    const plugin = await ReadSeekPlugin({} as never, { imageMode: "auto" });
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    const result = await read.execute({ path: "figure.png" }, createContext());

    expect(spawn).toHaveBeenCalledTimes(1);
    expect(JSON.parse((result as { output: string }).output)).toMatchObject({
      skipped: true,
      reason: "image mode not selected",
    });
  });

  test("rejects none when imageMode is on", async () => {
    const spawn = spyOn(Bun, "spawn");
    const plugin = await ReadSeekPlugin({} as never, { imageMode: "on" });
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    await expect(read.execute({ path: "figure.png", image: "none" }, createContext())).rejects.toThrow(
      'image="none" requires imageMode="auto"',
    );
    expect(spawn).not.toHaveBeenCalled();
  });

  test("rejects invalid plugin imageMode", async () => {
    await expect(ReadSeekPlugin({} as never, { imageMode: "force" })).rejects.toThrow(
      'imageMode must be "on", "auto", or "off"',
    );
  });

  test("rejects page selection for non-PDF files", async () => {
    const spawn = spyOn(Bun, "spawn").mockReturnValue({
      stdout: new Response('{"type":"text/plain"}').body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    } as never);
    const plugin = await ReadSeekPlugin({} as never);
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    await expect(read.execute({ path: "file.txt", page: 2 }, createContext())).rejects.toThrow(
      "page applies to PDF reads only",
    );
    expect(spawn).toHaveBeenCalledTimes(1);
  });

  test("bounds a PDF read to one page and returns image attachments", async () => {
    const outputs = [
      JSON.stringify({ type: "application/pdf", format: "pdf", pages: 3 }),
      JSON.stringify({
        format: "pdf",
        pages: 3,
        markdown: "<!-- readseek:page 2 -->\nPage 2\n",
        images: [{
          page: 2,
          width: 10,
          height: 20,
          mime: "image/png",
          mode: "none",
          encoding: "base64",
          data: "pixel",
        }],
      }),
    ];
    const spawn = spyOn(Bun, "spawn").mockImplementation(
      () => ({
        stdout: new Response(outputs.shift() ?? "{}").body,
        stderr: new Response("").body,
        exited: Promise.resolve(0),
      }) as never,
    );
    const plugin = await ReadSeekPlugin({} as never, { imageMode: "auto" });
    const read = plugin.tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    const result = await read.execute(
      { path: "paper.pdf", image: "none", page: 2 },
      createContext(),
    );
    if (typeof result === "string") throw new Error("expected a structured result");

    const readArgs = spawn.mock.calls[1]?.[0] as string[];
    expect(readArgs).toContain("--page");
    expect(readArgs).toContain("2");
    expect(JSON.parse(result.output).images[0]).toEqual({
      page: 2,
      width: 10,
      height: 20,
      mime: "image/png",
      mode: "none",
    });
    expect(result.attachments).toEqual([{
      type: "file",
      mime: "image/png",
      url: "data:image/png;base64,pixel",
      filename: "pdf-page-2-image-1.png",
    }]);
  });

  test("passes cancellation and output limits to the subprocess", async () => {
    const controller = new AbortController();
    const spawn = spyOn(Bun, "spawn").mockImplementation(
      () => ({
        stdout: new Response("{}").body,
        stderr: new Response("").body,
        exited: Promise.resolve(0),
      }) as never,
    );
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
