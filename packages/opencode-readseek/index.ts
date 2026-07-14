// SPDX-License-Identifier: Apache-2.0
// Copyright (C) Jarkko Sakkinen 2026

import { stat } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

import { tool, type Plugin, type ToolContext } from "@opencode-ai/plugin";

const require = createRequire(import.meta.url);
const readseekScript = require.resolve("@jarkkojs/readseek/bin/readseek.js");

class SessionAnchors {
  #pathsBySession = new Map<string, Set<string>>();
  #renamePlans = new Map<string, string>();

  mark(sessionID: string, filePath: string): void {
    let paths = this.#pathsBySession.get(sessionID);
    if (!paths) {
      paths = new Set<string>();
      this.#pathsBySession.set(sessionID, paths);
    }
    paths.add(filePath);
  }

  forget(filePath: string): void {
    for (const paths of this.#pathsBySession.values()) paths.delete(filePath);
  }

  planRename(sessionID: string, output: unknown): void {
    if (!output || typeof output !== "object") return;
    const { old_name: oldName, new_name: newName } = output as Record<string, unknown>;
    if (typeof oldName === "string" && typeof newName === "string") {
      this.#renamePlans.set(sessionID, `${oldName} -> ${newName}`);
    }
  }

  render(sessionID: string): string | undefined {
    const paths = this.#pathsBySession.get(sessionID);
    const sections: string[] = [];
    if (paths?.size) {
      sections.push(`## ReadSeek Anchors\nThe following files have fresh LINE:HASH anchors:\n${[...paths]
      .sort()
      .map((filePath) => `- ${filePath}`)
      .join("\n")}`);
    }
    const renamePlan = this.#renamePlans.get(sessionID);
    if (renamePlan) sections.push(`## Pending ReadSeek Rename Plan\n- ${renamePlan}`);
    return sections.length === 0 ? undefined : sections.join("\n\n");
  }
}

function resolvePath(directory: string, filePath: string): string {
  return path.resolve(directory, filePath);
}

function containsPath(directory: string, filePath: string): boolean {
  const relative = path.relative(directory, filePath);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
}

async function authorizeExternal(context: ToolContext, filePath: string): Promise<void> {
  if (containsPath(context.directory, filePath) || (context.worktree !== "/" && containsPath(context.worktree, filePath))) {
    return;
  }

  const info = await stat(filePath).catch(() => undefined);
  const parentDir = info?.isDirectory() ? filePath : path.dirname(filePath);
  const pattern = path.join(parentDir, "*").replaceAll("\\", "/");
  await context.ask({
    permission: "external_directory",
    patterns: [pattern],
    always: [pattern],
    metadata: { filepath: filePath, parentDir },
  });
}

async function authorizeRead(context: ToolContext, filePath: string): Promise<void> {
  await authorizeExternal(context, filePath);
  await context.ask({
    permission: "read",
    patterns: [path.relative(context.worktree, filePath).replaceAll("\\", "/")],
    always: ["*"],
    metadata: {},
  });
}

async function authorizeSearch(context: ToolContext, filePath: string, pattern: string): Promise<void> {
  await context.ask({
    permission: "grep",
    patterns: [pattern],
    always: ["*"],
    metadata: { pattern, path: filePath },
  });
  await authorizeExternal(context, filePath);
}

function optionalFlag(args: string[], enabled: boolean | undefined, flag: string): void {
  if (enabled) args.push(flag);
}

async function runReadSeek(directory: string, args: string[]): Promise<unknown> {
  const child = Bun.spawn([process.execPath, readseekScript, ...args], {
    cwd: directory,
    stderr: "pipe",
    stdout: "pipe",
  });
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  if (exitCode !== 0) throw new Error(stderr.trim() || `readseek exited with status ${exitCode}`);

  try {
    return JSON.parse(stdout) as unknown;
  } catch {
    throw new Error(`readseek returned invalid JSON: ${stdout.trim()}`);
  }
}

function render(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function collectFiles(value: unknown, files: Set<string>): void {
  if (Array.isArray(value)) {
    for (const item of value) collectFiles(item, files);
    return;
  }
  if (!value || typeof value !== "object") return;

  for (const [key, item] of Object.entries(value)) {
    if (key === "file" && typeof item === "string") files.add(item);
    else collectFiles(item, files);
  }
}

function identifiedName(value: unknown): string | undefined {
  if (!value || typeof value !== "object") return undefined;
  const identifier = (value as Record<string, unknown>).identifier;
  if (!identifier || typeof identifier !== "object") return undefined;
  const text = (identifier as Record<string, unknown>).text;
  return typeof text === "string" ? text : undefined;
}

function readseekTool(
  description: string,
  args: Record<string, any>,
  execute: (args: any, context: ToolContext) => Promise<unknown>,
) {
  return tool({
    description,
    args,
    async execute(args, context) {
      return render(await execute(args, context));
    },
  });
}

/**
 * Adds readseek's anchored reads and structural navigation without replacing
 * OpenCode's built-in file tools.
 */
export const ReadSeekPlugin: Plugin = async () => {
  const anchors = new SessionAnchors();
  const withSearchFlags = (args: string[], input: { cached?: boolean; others?: boolean; ignored?: boolean }) => {
    optionalFlag(args, input.cached, "--cached");
    optionalFlag(args, input.others, "--others");
    optionalFlag(args, input.ignored, "--ignored");
  };

  return {
    tool: {
      readseek_read: readseekTool(
        "Read a text file with stable LINE:HASH anchors. Use those anchors when describing a later edit.",
        {
          path: tool.schema.string().describe("Path relative to the project directory"),
          offset: tool.schema.number().int().positive().optional().describe("One-based starting line"),
          limit: tool.schema.number().int().positive().optional().describe("Maximum number of lines"),
        },
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          const args = ["read", input.offset === undefined ? filePath : `${filePath}:${input.offset}`];
          if (input.limit !== undefined) args.push("--end", String((input.offset as number) + (input.limit as number) - 1));
          return runReadSeek(context.directory, args);
        },
      ),
      readseek_map: readseekTool(
        "Build a structural symbol map for a source file.",
        { path: tool.schema.string().describe("Path relative to the project directory") },
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          return runReadSeek(context.directory, ["map", filePath]);
        },
      ),
      readseek_search: readseekTool(
        "Search source code using an ast-grep structural pattern. Results include LINE:HASH anchors.",
        {
          pattern: tool.schema.string().describe("AST pattern"),
          path: tool.schema.string().optional().describe("File or directory, defaulting to the project directory"),
          language: tool.schema.string().optional().describe("ast-grep language"),
          cached: tool.schema.boolean().optional(),
          others: tool.schema.boolean().optional(),
          ignored: tool.schema.boolean().optional(),
        },
        async (input, context) => {
          const target = resolvePath(context.directory, (input.path as string | undefined) ?? ".");
          await authorizeSearch(context, target, input.pattern as string);
          const args = ["search", target, input.pattern as string];
          if (input.language) args.push("--language", input.language as string);
          withSearchFlags(args, input);
          const result = await runReadSeek(context.directory, args);
          return result;
        },
      ),
      readseek_def: readseekTool(
        "Find structural definitions of a symbol. Results include LINE:HASH anchors.",
        {
          name: tool.schema.string().describe("Symbol name"),
          path: tool.schema.string().optional().describe("File or directory, defaulting to the project directory"),
          language: tool.schema.string().optional(),
          cached: tool.schema.boolean().optional(),
          others: tool.schema.boolean().optional(),
          ignored: tool.schema.boolean().optional(),
        },
        async (input, context) => {
          const target = resolvePath(context.directory, (input.path as string | undefined) ?? ".");
          await authorizeSearch(context, target, input.name as string);
          const args = ["def", target, "--format", "plain", input.name as string];
          if (input.language) args.push("--language", input.language as string);
          withSearchFlags(args, input);
          return runReadSeek(context.directory, args);
        },
      ),
      readseek_refs: readseekTool(
        "Find references to an identifier. Use scope with a cursor to identify a specific binding.",
        {
          name: tool.schema.string().describe("Identifier name"),
          path: tool.schema.string().optional().describe("File or directory, defaulting to the project directory"),
          language: tool.schema.string().optional(),
          scope: tool.schema.boolean().optional(),
          line: tool.schema.number().int().positive().optional(),
          column: tool.schema.number().int().positive().optional(),
          cached: tool.schema.boolean().optional(),
          others: tool.schema.boolean().optional(),
          ignored: tool.schema.boolean().optional(),
        },
        async (input, context) => {
          const target = resolvePath(context.directory, (input.path as string | undefined) ?? ".");
          await authorizeSearch(context, target, input.name as string);
          const args = ["refs", target, input.name as string];
          optionalFlag(args, input.scope as boolean | undefined, "--scope");
          if (input.line) args.push("--line", String(input.line));
          if (input.column) args.push("--column", String(input.column));
          if (input.language) args.push("--language", input.language as string);
          withSearchFlags(args, input);
          return runReadSeek(context.directory, args);
        },
      ),
      readseek_hover: readseekTool(
        "Identify the token and enclosing symbol at a source cursor.",
        {
          path: tool.schema.string().describe("Path relative to the project directory"),
          line: tool.schema.number().int().positive().describe("One-based cursor line"),
          column: tool.schema.number().int().positive().optional().describe("One-based cursor byte column"),
          language: tool.schema.string().optional(),
        },
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          const args = ["identify", `${filePath}:${input.line}`];
          if (input.column) args.push("--column", String(input.column));
          if (input.language) args.push("--language", input.language as string);
          return runReadSeek(context.directory, args);
        },
      ),
      readseek_rename: readseekTool(
        "Plan a binding-aware rename. This tool never writes files; apply the returned plan through OpenCode's normal edit tools.",
        {
          path: tool.schema.string().describe("Path relative to the project directory"),
          line: tool.schema.number().int().positive().describe("One-based cursor line of the binding"),
          column: tool.schema.number().int().positive().optional().describe("One-based cursor byte column"),
          to: tool.schema.string().min(1).describe("New binding name"),
          workspace: tool.schema.boolean().optional().describe("Include project-wide occurrences"),
        },
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          if (input.workspace) {
            const identifyArgs = ["identify", `${filePath}:${input.line}`];
            if (input.column) identifyArgs.push("--column", String(input.column));
            const name = identifiedName(await runReadSeek(context.directory, identifyArgs));
            if (!name) throw new Error("readseek could not identify a binding at the rename cursor");
            await authorizeSearch(context, context.directory, name);
          }
          const args = ["rename", filePath, "--line", String(input.line), "--to", input.to as string];
          if (input.column) args.push("--column", String(input.column));
          if (input.workspace) args.push("--workspace", context.directory);
          return runReadSeek(context.directory, args);
        },
      ),
      readseek_check: readseekTool(
        "Check a source file for parser errors and missing syntax.",
        { path: tool.schema.string().describe("Path relative to the project directory") },
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          return runReadSeek(context.directory, ["check", filePath]);
        },
      ),
    },
    event: async ({ event }) => {
      if (event.type !== "file.edited") return;
      anchors.forget(path.resolve(event.properties.file));
    },
    "tool.execute.before": async (input, output) => {
      if (input.tool === "readseek_rename" && output.args.apply === true) {
        throw new Error("readseek_rename only creates plans; apply its edits with OpenCode's edit tools");
      }
    },
    "tool.execute.after": async (input, output) => {
      if (!input.tool.startsWith("readseek_")) return;
      try {
        const result = JSON.parse(output.output) as unknown;
        if (input.tool === "readseek_rename") anchors.planRename(input.sessionID, result);

        const files = new Set<string>();
        collectFiles(result, files);
        for (const filePath of files) anchors.mark(input.sessionID, path.resolve(filePath));
      } catch {
        // A failed tool result has no valid anchors to retain.
      }
    },
    "experimental.session.compacting": async (input, output) => {
      const context = anchors.render(input.sessionID);
      if (context) output.context.push(context);
    },
  };
};
