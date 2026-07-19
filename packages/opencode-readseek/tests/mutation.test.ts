import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import type { ToolContext } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

type AskInput = Parameters<ToolContext["ask"]>[0];

function context(
  directory: string,
  permissions: string[] = [],
  asks: AskInput[] = [],
  abort: AbortSignal = new AbortController().signal,
): ToolContext {
  return {
    sessionID: "session",
    messageID: "message",
    agent: "test",
    directory,
    worktree: directory,
    abort,
    metadata() {},
    async ask(input) {
      permissions.push(input.permission);
      asks.push(input);
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

  test("limits the post-edit preview to nearby lines", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "large.ts");
    const content = Array.from({ length: 1_000 }, (_, index) => `line ${index + 1}`).join("\n") + "\n";
    await writeFile(file, content);
    const spawn = spawnOutputs([
      { file, hashlines: [{ line: 750, hash: "abc", text: "line 750" }] },
      { file, start_line: 747, end_line: 786, hashlines: [] },
    ]);
    const edit = (await ReadSeekPlugin({} as never)).tool?.readseek_edit;
    if (!edit) throw new Error("plugin did not register readseek_edit");

    await edit.execute(
      { path: "large.ts", edits: [{ set_line: { anchor: "750:abc", new_text: "changed" } }] },
      context(directory),
    );

    expect((spawn.mock.calls.at(-1)?.[0] as string[]).slice(1)).toEqual([
      "read",
      `${file}:747`,
      "--end",
      "786",
    ]);
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

  test("creates empty files", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "empty.txt");
    spawnOutputs([{ file, hashlines: [] }]);
    const write = (await ReadSeekPlugin({} as never)).tool?.readseek_write;
    if (!write) throw new Error("plugin did not register readseek_write");

    await write.execute({ path: "empty.txt", content: "" }, context(directory));

    expect(await readFile(file, "utf8")).toBe("");
  });

  test("preserves CRLF, bare CR, and UTF-8 BOM content", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const cases = [
      { name: "crlf.ts", before: "first\r\nsecond\r\nthird\r\n", replacement: "changed\nadded", after: "first\r\nchanged\r\nadded\r\nthird\r\n" },
      { name: "cr.ts", before: "first\rsecond\rthird\r", replacement: "changed\nadded", after: "first\rchanged\radded\rthird\r" },
      { name: "bom.ts", before: "\uFEFFfirst\nsecond\n", replacement: "changed", after: "\uFEFFfirst\nchanged\n" },
    ];
    const outputs = cases.flatMap(({ name }) => {
      const file = path.join(directory, name);
      return [
        { file, hashlines: [{ line: 2, hash: "abc", text: "second" }] },
        { file, hashlines: [{ line: 2, hash: "def", text: "changed" }] },
      ];
    });
    spawnOutputs(outputs);
    const edit = (await ReadSeekPlugin({} as never)).tool?.readseek_edit;
    if (!edit) throw new Error("plugin did not register readseek_edit");

    for (const item of cases) {
      const file = path.join(directory, item.name);
      await writeFile(file, item.before);
      await edit.execute({
        path: item.name,
        edits: [{ set_line: { anchor: "2:abc", new_text: item.replacement } }],
      }, context(directory));
      expect(await readFile(file, "utf8")).toBe(item.after);
    }
  });

  test("rejects malformed edit variants before I/O", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const spawn = spyOn(Bun, "spawn");
    const edit = (await ReadSeekPlugin({} as never)).tool?.readseek_edit;
    if (!edit) throw new Error("plugin did not register readseek_edit");
    const permissions: string[] = [];
    const invalidEdits = [
      [{ set_line: { anchor: "1:abc", new_text: "x" }, insert_after: { anchor: "1:abc", new_text: "y" } }],
      [{ unknown: { anchor: "1:abc", new_text: "x" } }],
      [{ set_line: { anchor: "1:abc", new_text: "x", extra: true } }],
      [{ set_line: { anchor: "1:abc" } }],
      [{ set_line: { anchor: "0:abc", new_text: "x" } }],
    ];

    for (const edits of invalidEdits) {
      await expect(edit.execute({ path: "file.ts", edits } as never, context(directory, permissions))).rejects.toThrow();
    }
    expect(spawn).not.toHaveBeenCalled();
    expect(permissions).toEqual([]);
  });

  test("provides a bounded preview for oversized diffs", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "large.txt");
    const content = Array.from({ length: 1_000 }, (_, index) => `${index}:${"x".repeat(100)}`).join("\n");
    spawnOutputs([{ file, hashlines: [] }]);
    const asks: AskInput[] = [];
    const write = (await ReadSeekPlugin({} as never)).tool?.readseek_write;
    if (!write) throw new Error("plugin did not register readseek_write");

    await write.execute({ path: "large.txt", content }, context(directory, [], asks));

    const diff = String(asks.find((ask) => ask.permission === "edit")?.metadata.diff ?? "");
    expect(diff).toContain("diff truncated:");
    expect(diff).toContain("added lines omitted");
    expect(Buffer.byteLength(diff)).toBeLessThanOrEqual(32 * 1024);
  });

  test("plans before applying a verified rename and supports dry-run", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.ts");
    const plan = { file, old_name: "before", new_name: "after", plan_hash: "plan123", edits: [], conflicts: [], others: [], applied: false };
    const applied = { ...plan, applied: true };
    const spawn = spawnOutputs([plan, applied, plan]);
    const permissions: string[] = [];
    const renameTool = (await ReadSeekPlugin({} as never)).tool?.readseek_rename;
    if (!renameTool) throw new Error("plugin did not register readseek_rename");

    await renameTool.execute({ path: "file.ts", line: 1, to: "after" }, context(directory, permissions));
    await renameTool.execute({ path: "file.ts", line: 1, to: "after", apply: false }, context(directory, permissions));

    expect(spawn.mock.calls.map((call) => (call[0] as string[]).slice(1))).toEqual([
      ["rename", file, "--line", "1", "--to", "after"],
      ["rename", file, "--line", "1", "--to", "after", "--plan-hash", "plan123", "--apply"],
      ["rename", file, "--line", "1", "--to", "after"],
    ]);
    expect(permissions).toEqual(["read", "edit", "read"]);
  });

  test("authorizes the exact workspace rename plan", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.ts");
    const other = path.join(directory, "other.ts");
    const plan = {
      file,
      old_name: "before",
      new_name: "after",
      plan_hash: "workspace-plan",
      edits: [{}],
      conflicts: [],
      others: [{ file: other, edits: [{}], conflicts: [] }],
      applied: false,
    };
    spawnOutputs([{ identifier: { text: "before" } }, plan, { ...plan, applied: true }]);
    const asks: AskInput[] = [];
    const renameTool = (await ReadSeekPlugin({} as never)).tool?.readseek_rename;
    if (!renameTool) throw new Error("plugin did not register readseek_rename");

    await renameTool.execute(
      { path: "file.ts", line: 1, to: "after", workspace: true },
      context(directory, [], asks),
    );

    expect(asks.map((ask) => ask.permission)).toEqual(["read", "grep", "edit"]);
    expect(asks[2]?.patterns).toEqual(["file.ts", "other.ts"]);
    expect(asks[2]?.patterns).not.toContain("**");
  });

  test("does not interrupt an authorized rename apply", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    const file = path.join(directory, "file.ts");
    const plan = {
      file,
      old_name: "before",
      new_name: "after",
      plan_hash: "plan123",
      edits: [{}],
      conflicts: [],
      others: [],
      applied: false,
    };
    const controller = new AbortController();
    let callCount = 0;
    const spawn = spyOn(Bun, "spawn").mockImplementation(() => {
      const output = callCount++ === 0 ? plan : { ...plan, applied: true };
      if (callCount === 2) controller.abort();
      return {
        stdout: new Response(JSON.stringify(output)).body,
        stderr: new Response("").body,
        exited: Promise.resolve(0),
      } as never;
    });
    const renameTool = (await ReadSeekPlugin({} as never)).tool?.readseek_rename;
    if (!renameTool) throw new Error("plugin did not register readseek_rename");

    const result = await renameTool.execute(
      { path: "file.ts", line: 1, to: "after" },
      context(directory, [], [], controller.signal),
    );
    expect(result).toMatchObject({ title: "Renamed before -> after" });

    const applyOptions = spawn.mock.calls[1]?.[1] as { signal?: AbortSignal };
    expect(applyOptions.signal).toBeUndefined();
  });
});
