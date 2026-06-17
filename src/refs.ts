import type { ExtensionAPI, ToolRenderResultOptions } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";
import path from "node:path";
import { stat as fsStat } from "node:fs/promises";
import { defineToolPromptMetadata } from "./tool-prompt-metadata.js";
import { escapeControlCharsForDisplay } from "./hashline.js";
import { buildToolErrorResult } from "./readseek-value.js";
import { resolveToCwd } from "./path-utils.js";
import { isReadseekAvailable, readseekRefs, type ReadseekReference } from "./readseek-client.js";
import { buildRefsOutput, type RefsOutputFile, type RefsOutputLine } from "./refs-output.js";

import { clampLineToWidth, clampLinesToWidth, renderToolLabel, resolveRenderResultContext, summaryLine } from "./tui-render-utils.js";

type RefsParams = {
  name: string;
  path?: string;
  lang?: string;
  scope?: boolean;
  line?: number;
  column?: number;
  cached?: boolean;
  others?: boolean;
  ignored?: boolean;
};

const REFS_PROMPT_METADATA = defineToolPromptMetadata({
  promptUrl: new URL("../prompts/refs.md", import.meta.url),
  promptSnippet: "Find references to an identifier with readseek and return edit-ready anchors",
});

export function isRefsAvailable(): boolean {
  return isReadseekAvailable();
}

interface RefsToolOptions {
  onFileAnchored?: (absolutePath: string) => void;
}

function refsLine(reference: ReadseekReference): RefsOutputLine {
  return {
    line: reference.line,
    hash: reference.line_hash,
    anchor: `${reference.line}:${reference.line_hash}`,
    raw: reference.text,
    display: escapeControlCharsForDisplay(reference.text),
    enclosingSymbol: reference.enclosingSymbol,
  };
}

function groupReferences(references: ReadseekReference[], cwd: string): RefsOutputFile[] {
  const files = new Map<string, RefsOutputFile>();
  const seen = new Set<string>();
  for (const reference of references) {
    const abs = path.isAbsolute(reference.file) ? reference.file : path.resolve(cwd, reference.file);
    const dedupeKey = `${abs}:${reference.line}:${reference.column}`;
    if (seen.has(dedupeKey)) continue;
    seen.add(dedupeKey);
    let file = files.get(abs);
    if (!file) {
      file = { displayPath: path.relative(cwd, abs) || abs, path: abs, lines: [] };
      files.set(abs, file);
    }
    file.lines.push(refsLine(reference));
  }
  return [...files.values()];
}

export function registerRefsTool(pi: ExtensionAPI, options: RefsToolOptions = {}) {
  const toolConfig = {
    callable: true,
    enabled: true,
    policy: "read-only" as const,
    readOnly: true,
    pythonName: "refs",
    defaultExposure: "opt-in" as const,
  };

  const tool = {
    name: "refs",
    label: "References",
    description: REFS_PROMPT_METADATA.description,
    promptSnippet: REFS_PROMPT_METADATA.promptSnippet,
    promptGuidelines: REFS_PROMPT_METADATA.promptGuidelines,
    parameters: Type.Object({
      name: Type.String({ description: "Identifier to find references for" }),
      path: Type.Optional(Type.String({ description: "Search path" })),
      lang: Type.Optional(Type.String({ description: "Language hint" })),
      scope: Type.Optional(Type.Boolean({ description: "Restrict to the binding under line/column (single file)" })),
      line: Type.Optional(Type.Number({ description: "One-based cursor line, used with scope" })),
      column: Type.Optional(Type.Number({ description: "One-based cursor byte column, used with scope" })),
      cached: Type.Optional(Type.Boolean({ description: "In a Git repository, search tracked/indexed files" })),
      others: Type.Optional(Type.Boolean({ description: "In a Git repository, search untracked files" })),
      ignored: Type.Optional(Type.Boolean({ description: "With others=true, include ignored untracked files" })),
    }),
    ptc: toolConfig,
    async execute(_toolCallId, params, signal, _onUpdate, ctx) {
      const p = params as RefsParams;
      if (p.ignored && !p.others) {
        return buildToolErrorResult("refs", "invalid-parameter", "Error: refs parameter 'ignored' requires 'others'");
      }
      if (p.scope && p.line === undefined) {
        return buildToolErrorResult("refs", "invalid-parameter", "Error: refs parameter 'scope' requires 'line'");
      }
      const searchPath = resolveToCwd(p.path ?? ".", ctx.cwd);

      try {
        await fsStat(searchPath);
      } catch (err: any) {
        if (err?.code === "ENOENT") {
          return buildToolErrorResult("refs", "path-not-found", `Error: path '${p.path ?? "."}' does not exist`, {
            path: p.path ?? searchPath,
          });
        }
        if (err?.code === "EACCES" || err?.code === "EPERM") {
          return buildToolErrorResult("refs", "permission-denied", `Error: permission denied for path '${p.path ?? "."}'`, {
            path: p.path ?? searchPath,
          });
        }
        const message = `Error: could not access path '${p.path ?? "."}': ${err?.message ?? String(err)}`;
        return buildToolErrorResult("refs", "fs-error", message, {
          path: p.path ?? searchPath,
          details: { fsCode: err?.code, fsMessage: err?.message },
        });
      }

      try {
        const references = await readseekRefs(searchPath, p.name, {
          scope: p.scope,
          line: p.line,
          column: p.column,
          language: p.lang,
          cached: p.cached,
          others: p.others,
          ignored: p.ignored,
          signal,
        });
        const files = groupReferences(references, ctx.cwd);
        const builtOutput = buildRefsOutput({ name: p.name, files });
        for (const file of files) {
          options.onFileAnchored?.(file.path);
        }
        return {
          content: [{ type: "text", text: builtOutput.text }],
          details: { readseekValue: builtOutput.readseekValue },
        };
      } catch (err: any) {
        const message = String(err?.message || err);
        const missingReadseek = err?.code === "ENOENT" || /Cannot find package|Cannot find module|no such file/i.test(message);
        return buildToolErrorResult(
          "refs",
          missingReadseek ? "readseek-not-installed" : "readseek-execution-error",
          message,
          missingReadseek ? { hint: "Run npm install to install @jarkkojs/readseek." } : {},
        );
      }
    },
    renderCall(args: any, theme: any, ...rest: any[]) {
      const context = rest[0] ?? {};
      let text = `${renderToolLabel(theme, "refs")} ${theme.fg("accent", args.name)}`;
      text += theme.fg("dim", ` in ${args.path ?? "."}`);
      if (args.lang) text += theme.fg("dim", ` (${args.lang})`);
      const flags = [args.scope && "scope", args.cached && "cached", args.others && "others", args.ignored && "ignored"].filter(Boolean);
      if (flags.length > 0) text += theme.fg("dim", ` [${flags.join(",")}]`);
      return new Text(clampLineToWidth(text, context.width), 0, 0);
    },
    renderResult(result: any, options: ToolRenderResultOptions, theme: any, ...rest: any[]) {
      const { isPartial, isError, expanded, cwd, width } = resolveRenderResultContext(options, rest);

      if (isPartial) return new Text(clampLinesToWidth([summaryLine("pending refs")], width).join("\n"), 0, 0);

      const content = result.content?.[0];
      const textContent = content?.type === "text" ? content.text : "";
      if (isError || result.isError) {
        const firstLine = textContent.split("\n")[0] || "Error";
        const body = expanded && textContent ? textContent : firstLine;
        return new Text(clampLinesToWidth(summaryLine(body).split("\n"), width).join("\n"), 0, 0);
      }
      const readseekValue = (result.details as any)?.readseekValue as
        | { tool: "refs"; files: Array<{ path: string; lines: any[] }> }
        | undefined;
      const files = readseekValue?.files ?? [];
      if (files.length === 0) return new Text(summaryLine("no references"), 0, 0);
      const fileCount = files.length;
      const totalRefs = files.reduce((sum: number, f: any) => sum + f.lines.length, 0);
      const refWord = totalRefs === 1 ? "reference" : "references";
      const fileWord = fileCount === 1 ? "file" : "files";
      let text = summaryLine(`${totalRefs} ${refWord} in ${fileCount} ${fileWord}`, { hidden: !expanded });
      if (expanded) {
        for (const file of files.slice(0, 20)) {
          const display = path.relative(cwd, file.path) || file.path;
          text += "\n" + theme.fg("dim", `  ${display} (${file.lines.length})`);
        }
        if (files.length > 20) text += "\n" + theme.fg("muted", `  … and ${files.length - 20} more files`);
      }
      return new Text(clampLinesToWidth(text.split("\n"), width).join("\n"), 0, 0);
    },
  } satisfies Parameters<ExtensionAPI["registerTool"]>[0] & { ptc: typeof toolConfig };

  pi.registerTool(tool);
  return tool;
}
