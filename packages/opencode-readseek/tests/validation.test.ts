import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import type { ToolContext } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

const context: ToolContext = {
  sessionID: "session",
  messageID: "message",
  agent: "test",
  directory: "/repo",
  worktree: "/repo",
  abort: new AbortController().signal,
  metadata() {},
  async ask() {},
};

describe("tool argument validation", () => {
  afterEach(() => {
    mock.restore();
  });

  test("requires others when ignored files are requested", async () => {
    const spawn = spyOn(Bun, "spawn");
    const tools = (await ReadSeekPlugin({} as never)).tool;
    if (!tools) throw new Error("plugin did not register tools");

    await expect(tools.readseek_search.execute({ pattern: "$A", ignored: true }, context)).rejects.toThrow(
      "ignored requires others",
    );
    expect(spawn).not.toHaveBeenCalled();
  });

  test("requires a line for scoped references", async () => {
    const spawn = spyOn(Bun, "spawn");
    const tools = (await ReadSeekPlugin({} as never)).tool;
    if (!tools) throw new Error("plugin did not register tools");

    await expect(tools.readseek_refs.execute({ name: "value", scope: true }, context)).rejects.toThrow(
      "scope requires line",
    );
    expect(spawn).not.toHaveBeenCalled();
  });

  test("rejects cursor fields without scoped references", async () => {
    const spawn = spyOn(Bun, "spawn");
    const tools = (await ReadSeekPlugin({} as never)).tool;
    if (!tools) throw new Error("plugin did not register tools");

    await expect(tools.readseek_refs.execute({ name: "value", line: 10 }, context)).rejects.toThrow(
      "line and column require scope",
    );
    expect(spawn).not.toHaveBeenCalled();
  });
});
