import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import type { ToolContext, ToolResult } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

function context(titles: string[]): ToolContext {
  return {
    sessionID: "session",
    messageID: "message",
    agent: "test",
    directory: "/repo",
    worktree: "/repo",
    abort: new AbortController().signal,
    metadata(input) {
      if (input.title) titles.push(input.title);
    },
    async ask() {},
  };
}

function presented(result: ToolResult): Exclude<ToolResult, string> {
  if (typeof result === "string") throw new Error("tool returned an unstructured result");
  return result;
}

describe("OpenCode presentation", () => {
  afterEach(() => {
    mock.restore();
  });

  test("publishes live and final titles without changing JSON output", async () => {
    const titles: string[] = [];
    const output = { file: "/repo/file.ts", start_line: 2, end_line: 4, hashlines: [] };
    let call = 0;
    spyOn(Bun, "spawn").mockImplementation(() => ({
      stdout: new Response(JSON.stringify(call++ === 0 ? {} : output)).body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    }) as never);
    const read = (await ReadSeekPlugin({} as never)).tool?.readseek_read;
    if (!read) throw new Error("plugin did not register readseek_read");

    const result = presented(await read.execute({ path: "file.ts", offset: 2, limit: 3 }, context(titles)));

    expect(titles).toEqual(["Read file.ts"]);
    expect(result.title).toBe("Read file.ts:2-4");
    expect(result.metadata).toEqual({ start_line: 2, end_line: 4, line_count: 3 });
    expect(JSON.parse(result.output)).toEqual(output);
  });

  test("summarizes structural search counts", async () => {
    spyOn(Bun, "spawn").mockReturnValue({
      stdout: new Response(
        JSON.stringify({ results: [{ file: "/repo/a.ts", matches: [{}, {}] }, { file: "/repo/b.ts", matches: [{}] }] }),
      ).body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    } as never);
    const search = (await ReadSeekPlugin({} as never)).tool?.readseek_search;
    if (!search) throw new Error("plugin did not register readseek_search");

    const result = presented(await search.execute({ pattern: "$A", path: "src" }, context([])));

    expect(result.title).toBe("Found 3 matches");
    expect(result.metadata).toEqual({ results: 2, matches: 3 });
  });

  test("summarizes rename plans and conflicts", async () => {
    spyOn(Bun, "spawn").mockReturnValue({
      stdout: new Response(
        JSON.stringify({
          file: "/repo/a.ts",
          old_name: "before",
          new_name: "after",
          edits: [{}, {}],
          conflicts: [{}],
          others: [{ file: "/repo/b.ts", edits: [{}], conflicts: [{}, {}] }],
        }),
      ).body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    } as never);
    const rename = (await ReadSeekPlugin({} as never)).tool?.readseek_rename;
    if (!rename) throw new Error("plugin did not register readseek_rename");

    const result = presented(await rename.execute({ path: "a.ts", line: 3, to: "after" }, context([])));

    expect(result.title).toBe("Plan before -> after");
    expect(result.metadata).toEqual({ edits: 3, conflicts: 3, others: 1 });
  });
});
