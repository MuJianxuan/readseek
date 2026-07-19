import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import type { ToolContext } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

function createContext(): ToolContext {
  return {
    sessionID: "session",
    messageID: "message",
    agent: "test",
    directory: "/repo",
    worktree: "/repo",
    abort: new AbortController().signal,
    metadata() {},
    async ask() {},
  };
}

describe("readseek_view", () => {
  afterEach(() => {
    mock.restore();
  });

  test("uses the shared cache and forwards document selectors", async () => {
    const spawn = spyOn(Bun, "spawn").mockImplementation(() => ({
      stdout: new Response("- page-2 [page] Page 2\n").body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    }) as never);
    const plugin = await ReadSeekPlugin({} as never);
    const view = plugin.tool?.readseek_view;
    if (!view) throw new Error("plugin did not register readseek_view");

    const result = await view.execute({
      path: "paper.pdf",
      node: "page-2",
      page: 2,
      kind: "heading",
      depth: 3,
      outline: true,
    }, createContext());
    if (typeof result === "string") throw new Error("expected a structured result");

    const viewArgs = spawn.mock.calls
      .map((call) => call[0] as string[])
      .find((args) => args.includes("view"));
    expect(viewArgs).toBeDefined();
    expect(viewArgs).toContain("--readseek-dir");
    expect(viewArgs).toContain("--node");
    expect(viewArgs).toContain("page-2");
    expect(viewArgs).toContain("--page");
    expect(viewArgs).toContain("2");
    expect(viewArgs).toContain("--kind");
    expect(viewArgs).toContain("heading");
    expect(viewArgs).toContain("--depth");
    expect(viewArgs).toContain("3");
    expect(viewArgs).toContain("--outline");
    expect(result.output).toBe("- page-2 [page] Page 2\n");
  });

  test("bounds large document output", async () => {
    const output = Array.from({ length: 2500 }, (_, index) => `line ${index}`).join("\n");
    spyOn(Bun, "spawn").mockImplementation(() => ({
      stdout: new Response(output).body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    }) as never);
    const plugin = await ReadSeekPlugin({} as never);
    const view = plugin.tool?.readseek_view;
    if (!view) throw new Error("plugin did not register readseek_view");

    const result = await view.execute({ path: "paper.pdf" }, createContext());
    if (typeof result === "string") throw new Error("expected a structured result");

    expect(result.output.split("\n").length).toBeLessThanOrEqual(2001);
    expect(result.output).toContain("[… document view truncated");
  });
});
