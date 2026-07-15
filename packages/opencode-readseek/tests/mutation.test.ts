import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import type { ToolContext } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

function context(directory: string, permissions: string[] = []): ToolContext {
  return {
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
}

function spawnOutputs(outputs: unknown[]) {
  return spyOn(Bun, "spawn").mockImplementation(() => ({
    stdout: new Response(JSON.stringify(outputs.shift() ?? {})).body,
    stderr: new Response("").body,
    exited: Promise.resolve(0),
  }) as never);
}

describe("mutation tools", () => {
  afterEach(() => mock.restore());

  test("rejects stale edit anchors before asking to edit", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    await writeFile(path.join(directory, "file.ts"), "before\n");
    spawnOutputs([{ file: path.join(directory, "file.ts"), hashlines: [{ line: 1, hash: "def", text: "before" }] }]);
    const permissions: string[] = [];
    const edit = (await ReadSeekPlugin({} as never)).tool?.readseek_edit;
    if (!edit) throw new Error("plugin did not register readseek_edit");

    await expect(edit.execute({ path: "file.ts", edits: [{ set_line: { anchor: "1:abc", new_text: "after" } }] }, context(directory, permissions))).rejects.toThrow("stale anchor");

    expect(await readFile(path.join(directory, "file.ts"), "utf8")).toBe("before\n");
    expect(permissions).toEqual(["read"]);
  });

  test("applies verified edits and requests read then edit permission", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.ts");
    await writeFile(file, "before\nsecond\n");
    spawnOutputs([
      { file, hashlines: [{ line: 1, hash: "abc", text: "before" }] },
      { file, hashlines: [{ line: 1, hash: "def", text: "after" }] },
    ]);
    const permissions: string[] = [];
    const edit = (await ReadSeekPlugin({} as never)).tool?.readseek_edit;
    if (!edit) throw new Error("plugin did not register readseek_edit");

    await edit.execute({ path: "file.ts", edits: [{ set_line: { anchor: "1:abc", new_text: "after" } }] }, context(directory, permissions));

    expect(await readFile(file, "utf8")).toBe("after\nsecond\n");
    expect(permissions).toEqual(["read", "edit"]);
  });

  test("rejects overlapping anchored edits", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.ts");
    await writeFile(file, "first\nsecond\n");
    spawnOutputs([
      { file, hashlines: [{ line: 1, hash: "abc", text: "first" }] },
      { file, hashlines: [{ line: 2, hash: "def", text: "second" }] },
    ]);
    const permissions: string[] = [];
    const edit = (await ReadSeekPlugin({} as never)).tool?.readseek_edit;
    if (!edit) throw new Error("plugin did not register readseek_edit");

    await expect(edit.execute({
      path: "file.ts",
      edits: [
        { replace_lines: { start_anchor: "1:abc", end_anchor: "2:def", new_text: "replacement" } },
        { set_line: { anchor: "2:def", new_text: "other" } },
      ],
    }, context(directory, permissions))).rejects.toThrow("overlap");

    expect(await readFile(file, "utf8")).toBe("first\nsecond\n");
    expect(permissions).toEqual(["read"]);
  });

  test("writes whole files with edit permission", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "nested/file.txt");
    spawnOutputs([{ file, hashlines: [{ line: 1, hash: "abc", text: "content" }] }]);
    const permissions: string[] = [];
    const write = (await ReadSeekPlugin({} as never)).tool?.readseek_write;
    if (!write) throw new Error("plugin did not register readseek_write");

    await write.execute({ path: "nested/file.txt", content: "content\n" }, context(directory, permissions));

    expect(await readFile(file, "utf8")).toBe("content\n");
    expect(permissions).toEqual(["edit"]);
  });

  test("plans before atomically applying rename and supports dry-run", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.ts");
    const plan = { file, old_name: "before", new_name: "after", edits: [], conflicts: [], others: [], applied: false };
    const applied = { ...plan, applied: true };
    const spawn = spawnOutputs([plan, applied, plan]);
    const permissions: string[] = [];
    const renameTool = (await ReadSeekPlugin({} as never)).tool?.readseek_rename;
    if (!renameTool) throw new Error("plugin did not register readseek_rename");

    await renameTool.execute({ path: "file.ts", line: 1, to: "after" }, context(directory, permissions));
    await renameTool.execute({ path: "file.ts", line: 1, to: "after", apply: false }, context(directory, permissions));

    expect(spawn.mock.calls.map((call) => (call[0] as string[]).slice(1))).toEqual([
      ["rename", file, "--line", "1", "--to", "after"],
      ["rename", file, "--line", "1", "--to", "after", "--apply"],
      ["rename", file, "--line", "1", "--to", "after"],
    ]);
    expect(permissions).toEqual(["read", "edit", "read"]);
  });
});
