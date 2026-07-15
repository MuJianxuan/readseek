// SPDX-License-Identifier: Apache-2.0
// Copyright (C) Jarkko Sakkinen 2026

import { stat } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

import { tool, type Plugin, type PluginOptions, type ToolContext } from "@opencode-ai/plugin";

const require = createRequire(import.meta.url);
const readseekScript = require.resolve("@jarkkojs/readseek/bin/readseek.js");
const MAX_OUTPUT_BYTES = 32 * 1024 * 1024;

type RenamePlan = {
  summary: string;
  files: Set<string>;
};

type PresentationKind = "read" | "map" | "search" | "def" | "refs" | "hover" | "rename" | "check";

type Presentation = {
  title: string;
  metadata: Record<string, number>;
};

type ImagePolicy = "on" | "auto" | "off";
type ImageMode = "none" | "ocr" | "caption" | "objects";

function resolveImagePolicy(options: PluginOptions | undefined): ImagePolicy {
  const value = options?.imageMode;
  if (value === undefined) return "auto";
  if (value === "on" || value === "auto" || value === "off") return value;
  throw new Error('opencode-readseek imageMode must be "on", "auto", or "off"');
}

function isVisualFile(value: unknown): boolean {
  const output = record(value);
  const type = output.type;
  return typeof output.width === "number" || type === "application/pdf";
}

class SessionAnchors {
  #pathsBySession = new Map<string, Set<string>>();
  #renamePlans = new Map<string, RenamePlan>();

  mark(sessionID: string, filePath: string): void {
    let paths = this.#pathsBySession.get(sessionID);
    if (!paths) {
      paths = new Set<string>();
      this.#pathsBySession.set(sessionID, paths);
    }
    paths.add(filePath);
  }

  forget(filePath: string): void {
    const absolutePath = path.resolve(filePath);
    for (const paths of this.#pathsBySession.values()) paths.delete(absolutePath);
    for (const [sessionID, plan] of this.#renamePlans) {
      if (plan.files.has(absolutePath)) this.#renamePlans.delete(sessionID);
    }
  }

  deleteSession(sessionID: string): void {
    this.#pathsBySession.delete(sessionID);
    this.#renamePlans.delete(sessionID);
  }

  planRename(sessionID: string, output: unknown): void {
    if (!output || typeof output !== "object") return;
    const record = output as Record<string, unknown>;
    const { file, old_name: oldName, new_name: newName, others } = record;
    if (typeof file !== "string" || typeof oldName !== "string" || typeof newName !== "string") return;

    const files = new Set([path.resolve(file)]);
    if (Array.isArray(others)) {
      for (const item of others) {
        if (!item || typeof item !== "object") continue;
        const otherFile = (item as Record<string, unknown>).file;
        if (typeof otherFile === "string") files.add(path.resolve(otherFile));
      }
    }
    this.#renamePlans.set(sessionID, { summary: `${oldName} -> ${newName}`, files });
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
    if (renamePlan) sections.push(`## Pending ReadSeek Rename Plan\n- ${renamePlan.summary}`);
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

async function runReadSeek(context: ToolContext, args: string[]): Promise<unknown> {
  context.abort.throwIfAborted();
  const child = Bun.spawn([process.execPath, readseekScript, ...args], {
    cwd: context.directory,
    killSignal: "SIGKILL",
    maxBuffer: MAX_OUTPUT_BYTES,
    signal: context.abort,
    stderr: "pipe",
    stdout: "pipe",
  });
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  context.abort.throwIfAborted();
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

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function items(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function initialTitle(kind: PresentationKind, args: any): string {
  switch (kind) {
    case "read":
      return `Read ${args.path}`;
    case "map":
      return `Map ${args.path}`;
    case "search":
      return `Search ${args.pattern}`;
    case "def":
      return `Find definition ${args.name}`;
    case "refs":
      return `Find references to ${args.name}`;
    case "hover":
      return `Identify ${args.path}:${args.line}`;
    case "rename":
      return `Rename ${args.path}:${args.line} to ${args.to}`;
    case "check":
      return `Check ${args.path}`;
  }
}

function resultPresentation(kind: PresentationKind, args: any, value: unknown): Presentation {
  const output = record(value);
  switch (kind) {
    case "read": {
      const startLine = output.start_line;
      const endLine = output.end_line;
      if (typeof startLine === "number" && typeof endLine === "number") {
        return {
          title: `Read ${args.path}:${startLine}-${endLine}`,
          metadata: { start_line: startLine, end_line: endLine, line_count: endLine - startLine + 1 },
        };
      }
      const width = output.width;
      const height = output.height;
      if (typeof width === "number" && typeof height === "number") {
        return { title: `Read ${args.path} (${width}x${height})`, metadata: { width, height } };
      }
      break;
    }
    case "map": {
      const symbols = items(output.symbols).length;
      return { title: `Mapped ${args.path} (${symbols} symbols)`, metadata: { symbols } };
    }
    case "search": {
      const results = items(output.results);
      const matches = results.reduce<number>((total, result) => total + items(record(result).matches).length, 0);
      return { title: `Found ${matches} matches`, metadata: { results: results.length, matches } };
    }
    case "def": {
      const locations = items(output.locations).length;
      return { title: `Found ${locations} definitions`, metadata: { locations } };
    }
    case "refs": {
      const references = items(output.references).length;
      return { title: `Found ${references} references`, metadata: { references } };
    }
    case "hover": {
      const identifier = record(output.identifier).text;
      const line = output.line;
      const column = output.column;
      const metadata: Record<string, number> = {};
      if (typeof line === "number") metadata.line = line;
      if (typeof column === "number") metadata.column = column;
      return { title: typeof identifier === "string" ? `Identified ${identifier}` : initialTitle(kind, args), metadata };
    }
    case "rename": {
      const oldName = output.old_name;
      const newName = output.new_name;
      const otherOutputs = items(output.others).map(record);
      const edits = otherOutputs.reduce((total, item) => total + items(item.edits).length, items(output.edits).length);
      const conflicts = otherOutputs.reduce(
        (total, item) => total + items(item.conflicts).length,
        items(output.conflicts).length,
      );
      const others = otherOutputs.length;
      const title =
        typeof oldName === "string" && typeof newName === "string"
          ? `Plan ${oldName} -> ${newName}`
          : initialTitle(kind, args);
      return { title, metadata: { edits, conflicts, others } };
    }
    case "check": {
      const errors = typeof output.error_count === "number" ? output.error_count : 0;
      const missing = typeof output.missing_count === "number" ? output.missing_count : 0;
      return {
        title: `Checked ${args.path} (${errors} errors, ${missing} missing)`,
        metadata: { error_count: errors, missing_count: missing },
      };
    }
  }
  return { title: initialTitle(kind, args), metadata: {} };
}

function readseekTool(
  description: string,
  args: Record<string, any>,
  kind: PresentationKind,
  execute: (args: any, context: ToolContext) => Promise<unknown>,
) {
  return tool({
    description,
    args,
    async execute(args, context) {
      const title = initialTitle(kind, args);
      context.metadata({ title });
      const result = await execute(args, context);
      const presentation = resultPresentation(kind, args, result);
      return { title: presentation.title, output: render(result), metadata: presentation.metadata };
    },
  });
}

/**
 * Adds readseek's anchored reads and structural navigation without replacing
 * OpenCode's built-in file tools.
 */
export const ReadSeekPlugin: Plugin = async (_input, options) => {
  const anchors = new SessionAnchors();
  const imagePolicy = resolveImagePolicy(options);
  const imageModes: readonly ImageMode[] = imagePolicy === "auto"
    ? ["none", "ocr", "caption", "objects"]
    : ["ocr", "caption", "objects"];
  const withSearchFlags = (args: string[], input: { cached?: boolean; others?: boolean; ignored?: boolean }) => {
    if (input.ignored && !input.others) throw new Error("ignored requires others");
    optionalFlag(args, input.cached, "--cached");
    optionalFlag(args, input.others, "--others");
    optionalFlag(args, input.ignored, "--ignored");
  };

  return {
    tool: {
      readseek_read: readseekTool(
        imagePolicy === "off"
          ? "Read text with stable LINE:HASH anchors. Image and PDF files are skipped."
          : `Read text with stable LINE:HASH anchors. For images and PDFs, explicitly select image: ${imageModes.join(", ")}; omitting image skips the file.`,
        {
          path: tool.schema.string().describe("Path relative to the project directory"),
          offset: tool.schema.number().int().positive().optional().describe("One-based starting line"),
          limit: tool.schema.number().int().positive().optional().describe("Maximum number of lines"),
          ...(imagePolicy === "off"
            ? {}
            : {
                image: tool.schema.enum(imageModes as [ImageMode, ...ImageMode[]]).optional()
                  .describe(`Image/PDF mode: ${imageModes.join(", ")}. Must be selected explicitly.`),
              }),
        },
        "read",
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          const image = input.image as ImageMode | undefined;
          if (imagePolicy === "off" && image !== undefined) throw new Error("image and PDF reads are disabled");
          if (imagePolicy === "on" && image === "none") throw new Error('image="none" requires imageMode="auto"');
          if (image === undefined) {
            const detection = await runReadSeek(context, ["detect", filePath]);
            if (isVisualFile(detection)) {
              return { file: filePath, skipped: true, reason: "image mode not selected" };
            }
          }
          const args = ["read", input.offset === undefined ? filePath : `${filePath}:${input.offset}`];
          if (input.limit !== undefined) args.push("--end", String((input.offset ?? 1) + input.limit - 1));
          if (image !== undefined) args.push("--image", image);
          return runReadSeek(context, args);
        },
      ),
      readseek_map: readseekTool(
        "Build a structural symbol map for a source file.",
        { path: tool.schema.string().describe("Path relative to the project directory") },
        "map",
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          return runReadSeek(context, ["map", filePath]);
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
        "search",
        async (input, context) => {
          const target = resolvePath(context.directory, (input.path as string | undefined) ?? ".");
          const args = ["search", target, input.pattern as string];
          if (input.language) args.push("--language", input.language as string);
          withSearchFlags(args, input);
          await authorizeSearch(context, target, input.pattern as string);
          const result = await runReadSeek(context, args);
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
        "def",
        async (input, context) => {
          const target = resolvePath(context.directory, (input.path as string | undefined) ?? ".");
          const args = ["def", target, "--format", "plain", input.name as string];
          if (input.language) args.push("--language", input.language as string);
          withSearchFlags(args, input);
          await authorizeSearch(context, target, input.name as string);
          return runReadSeek(context, args);
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
        "refs",
        async (input, context) => {
          if (input.scope && input.line === undefined) throw new Error("scope requires line");
          if (!input.scope && (input.line !== undefined || input.column !== undefined)) {
            throw new Error("line and column require scope");
          }
          const target = resolvePath(context.directory, (input.path as string | undefined) ?? ".");
          const args = ["refs", target, input.name as string];
          optionalFlag(args, input.scope as boolean | undefined, "--scope");
          if (input.line) args.push("--line", String(input.line));
          if (input.column) args.push("--column", String(input.column));
          if (input.language) args.push("--language", input.language as string);
          withSearchFlags(args, input);
          await authorizeSearch(context, target, input.name as string);
          return runReadSeek(context, args);
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
        "hover",
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          const args = ["identify", `${filePath}:${input.line}`];
          if (input.column) args.push("--column", String(input.column));
          if (input.language) args.push("--language", input.language as string);
          return runReadSeek(context, args);
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
        "rename",
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          if (input.workspace) {
            const identifyArgs = ["identify", `${filePath}:${input.line}`];
            if (input.column) identifyArgs.push("--column", String(input.column));
            const name = identifiedName(await runReadSeek(context, identifyArgs));
            if (!name) throw new Error("readseek could not identify a binding at the rename cursor");
            await authorizeSearch(context, context.directory, name);
          }
          const args = ["rename", filePath, "--line", String(input.line), "--to", input.to as string];
          if (input.column) args.push("--column", String(input.column));
          if (input.workspace) args.push("--workspace", context.directory);
          return runReadSeek(context, args);
        },
      ),
      readseek_check: readseekTool(
        "Check a source file for parser errors and missing syntax.",
        { path: tool.schema.string().describe("Path relative to the project directory") },
        "check",
        async (input, context) => {
          const filePath = resolvePath(context.directory, input.path as string);
          await authorizeRead(context, filePath);
          return runReadSeek(context, ["check", filePath]);
        },
      ),
    },
    event: async ({ event }) => {
      if (event.type === "file.edited" || event.type === "file.watcher.updated") {
        anchors.forget(event.properties.file);
        return;
      }
      if (event.type === "session.deleted") anchors.deleteSession(event.properties.info.id);
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

export default { id: "opencode-readseek", server: ReadSeekPlugin };
