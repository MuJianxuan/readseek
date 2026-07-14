import { describe, expect, test } from "bun:test";

import { ReadSeekPlugin } from "../index.ts";

async function plugin() {
  return ReadSeekPlugin({} as never);
}

async function recordResult(
  hooks: Awaited<ReturnType<typeof plugin>>,
  sessionID: string,
  tool: string,
  result: unknown,
): Promise<void> {
  await hooks["tool.execute.after"]?.(
    { tool, sessionID, callID: "call", args: {} },
    { title: "", output: JSON.stringify(result), metadata: {} },
  );
}

async function compact(hooks: Awaited<ReturnType<typeof plugin>>, sessionID: string): Promise<string> {
  const output = { context: [] as string[] };
  await hooks["experimental.session.compacting"]?.({ sessionID }, output);
  return output.context.join("\n");
}

describe("session state", () => {
  test("file edits clear anchors and affected rename plans across sessions", async () => {
    const hooks = await plugin();
    await recordResult(hooks, "first", "readseek_read", { file: "/repo/a.ts", hash: "1234" });
    await recordResult(hooks, "first", "readseek_rename", {
      file: "/repo/a.ts",
      old_name: "before",
      new_name: "after",
    });
    await recordResult(hooks, "second", "readseek_read", { file: "/repo/a.ts", hash: "1234" });

    await hooks.event?.({ event: { type: "file.edited", properties: { file: "/repo/a.ts" } } });

    expect(await compact(hooks, "first")).toBe("");
    expect(await compact(hooks, "second")).toBe("");
  });

  test("watcher changes invalidate workspace plans through their other files", async () => {
    const hooks = await plugin();
    await recordResult(hooks, "session", "readseek_rename", {
      file: "/repo/main.ts",
      old_name: "before",
      new_name: "after",
      others: [{ file: "/repo/use.ts" }],
    });

    await hooks.event?.({
      event: { type: "file.watcher.updated", properties: { file: "/repo/use.ts", event: "change" } },
    });

    expect(await compact(hooks, "session")).not.toContain("before -> after");
  });

  test("unrelated changes preserve rename plans", async () => {
    const hooks = await plugin();
    await recordResult(hooks, "session", "readseek_rename", {
      file: "/repo/main.ts",
      old_name: "before",
      new_name: "after",
    });

    await hooks.event?.({ event: { type: "file.edited", properties: { file: "/repo/unrelated.ts" } } });

    expect(await compact(hooks, "session")).toContain("before -> after");
  });

  test("session deletion releases only that session's state", async () => {
    const hooks = await plugin();
    await recordResult(hooks, "deleted", "readseek_read", { file: "/repo/deleted.ts", hash: "1234" });
    await recordResult(hooks, "surviving", "readseek_read", { file: "/repo/surviving.ts", hash: "5678" });

    await hooks.event?.({
      event: { type: "session.deleted", properties: { info: { id: "deleted" } } } as never,
    });

    expect(await compact(hooks, "deleted")).toBe("");
    expect(await compact(hooks, "surviving")).toContain("/repo/surviving.ts");
  });
});
