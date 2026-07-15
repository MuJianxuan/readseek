import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { ToolContext, ToolResult } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

describe("readseek_grep", () => {
  afterEach(() => mock.restore());

  test("returns CLI-generated anchors for literal text matches", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.txt");
    await writeFile(file, "alpha\nbeta.*\ngamma\n");
    spyOn(Bun, "spawn").mockImplementation(() => ({
      stdout: new Response(JSON.stringify({ file, hashlines: [{ line: 2, hash: "abc", text: "beta.*" }] })).body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    }) as never);
    const permissions: string[] = [];
    const context: ToolContext = {
      sessionID: "session",
      messageID: "message",
      agent: "test",
      directory,
      worktree: directory,
      abort: new AbortController().signal,
      metadata() {},
      async ask(input) {
        permissions.push(input.permission);
      },
    };
    const grep = (await ReadSeekPlugin({} as never)).tool?.readseek_grep;
    if (!grep) throw new Error("plugin did not register readseek_grep");

    const result = await grep.execute({ path: "file.txt", pattern: "beta.*", literal: true }, context) as Exclude<ToolResult, string>;
    const output = JSON.parse(result.output) as { results: { file: string; matches: unknown[]; hashlines: unknown[] }[] };

    expect(output.results).toEqual([{ file, matches: [{ line: 2, hash: "abc", text: "beta.*" }], hashlines: [{ line: 2, hash: "abc", text: "beta.*" }] }]);
    expect(permissions).toEqual(["grep"]);
  });
});
