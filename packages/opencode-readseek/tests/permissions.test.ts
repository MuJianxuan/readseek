import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";

import { mkdir, mkdtemp, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import type { ToolContext } from "@opencode-ai/plugin";

import { ReadSeekPlugin } from "../index.ts";

type AskInput = Parameters<ToolContext["ask"]>[0];

function createContext(asks: AskInput[], rejectPermission: string): ToolContext {
  return {
    sessionID: "session",
    messageID: "message",
    agent: "test",
    directory: "/repo/project",
    worktree: "/repo",
    abort: new AbortController().signal,
    metadata() {},
    async ask(input) {
      asks.push(input);
      if (input.permission === rejectPermission) throw new Error("permission denied");
    },
  };
}

async function tools() {
  const plugin = await ReadSeekPlugin({} as never);
  if (!plugin.tool) throw new Error("plugin did not register tools");
  return plugin.tool;
}

describe("OpenCode permissions", () => {
  afterEach(() => {
    mock.restore();
  });

  test("requests read permission for project files", async () => {
    const asks: AskInput[] = [];
    const context = createContext(asks, "read");

    await expect((await tools()).readseek_read.execute({ path: "src/main.ts" }, context)).rejects.toThrow(
      "permission denied",
    );

    expect(asks).toEqual([
      {
        permission: "read",
        patterns: ["project/src/main.ts"],
        always: ["*"],
        metadata: {},
      },
    ]);
  });

  test("allows files elsewhere in the worktree without an external grant", async () => {
    const asks: AskInput[] = [];
    const context = createContext(asks, "read");

    await expect((await tools()).readseek_map.execute({ path: "../shared.ts" }, context)).rejects.toThrow(
      "permission denied",
    );

    expect(asks.map((ask) => ask.permission)).toEqual(["read"]);
    expect(asks[0]?.patterns).toEqual(["shared.ts"]);
  });

  test("requests an external-directory grant before reading outside the worktree", async () => {
    const asks: AskInput[] = [];
    const context = createContext(asks, "read");

    await expect((await tools()).readseek_check.execute({ path: "/outside/file.ts" }, context)).rejects.toThrow(
      "permission denied",
    );

    expect(asks.map((ask) => ask.permission)).toEqual(["external_directory", "read"]);
    expect(asks[0]?.patterns).toEqual(["/outside/*"]);
  });

  test("resolves symlinks before granting worktree access", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "opencode-readseek-"));
    try {
      const worktree = path.join(directory, "worktree");
      const project = path.join(worktree, "project");
      const external = path.join(directory, "external");
      await mkdir(project, { recursive: true });
      await mkdir(external);
      await symlink(external, path.join(project, "linked"), process.platform === "win32" ? "junction" : "dir");

      const asks: AskInput[] = [];
      const context = {
        ...createContext(asks, "external_directory"),
        directory: project,
        worktree,
      };
      await expect(
        (await tools()).readseek_check.execute({ path: "linked/file.ts" }, context),
      ).rejects.toThrow("permission denied");

      expect(asks.map((ask) => ask.permission)).toEqual(["external_directory"]);
      expect(asks[0]?.patterns).toEqual([path.join(external, "*").replaceAll("\\", "/")]);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test("does not treat the filesystem root as a non-git worktree grant", async () => {
    const asks: AskInput[] = [];
    const context = { ...createContext(asks, "external_directory"), worktree: "/" };

    await expect((await tools()).readseek_read.execute({ path: "/outside/file.ts" }, context)).rejects.toThrow(
      "permission denied",
    );

    expect(asks.map((ask) => ask.permission)).toEqual(["external_directory"]);
  });

  test("requests grep and external-directory grants for external searches", async () => {
    const asks: AskInput[] = [];
    const context = createContext(asks, "external_directory");

    await expect(
      (await tools()).readseek_search.execute({ pattern: "fn $NAME()", path: "/outside/src" }, context),
    ).rejects.toThrow("permission denied");

    expect(asks.map((ask) => ask.permission)).toEqual(["grep", "external_directory"]);
    expect(asks[0]?.patterns).toEqual(["fn $NAME()"]);
    expect(asks[1]?.patterns).toEqual(["/outside/*"]);
  });

  test("checks the identified name before planning a workspace rename", async () => {
    const asks: AskInput[] = [];
    const context = createContext(asks, "grep");
    const spawn = spyOn(Bun, "spawn").mockReturnValue({
      stdout: new Response(JSON.stringify({ identifier: { text: "currentName" } })).body,
      stderr: new Response("").body,
      exited: Promise.resolve(0),
    } as never);

    await expect(
      (await tools()).readseek_rename.execute(
        { path: "src/main.ts", line: 10, to: "nextName", workspace: true },
        context,
      ),
    ).rejects.toThrow("permission denied");

    expect(spawn).toHaveBeenCalledTimes(1);
    expect(asks.map((ask) => ask.permission)).toEqual(["read", "grep"]);
    expect(asks[1]?.patterns).toEqual(["currentName"]);
  });
});
