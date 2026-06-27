import type { ExtensionAPI, ToolRenderResultOptions } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import path from "node:path";
import { defineToolPromptMetadata } from "./tool-prompt-metadata.js";
import { statSearchPathOrError } from "./stat-search-path.js";
import { buildReadSeekLineWithHash, buildToolErrorResult } from "./readseek-value.js";
import { resolveToCwd } from "./path-utils.js";
import { classifyReadSeekFailure, readseekRefs, type ReadSeekReference } from "./readseek-client.js";
import { buildRefsOutput, type RefsOutputFile, type RefsOutputLine } from "./refs-output.js";
import type { FileAnchoredCallback } from "./tool-types.js";
import { readseekGitSearchParams, validateIgnoredRequiresOthers } from "./readseek-params.js";
import { registerReadSeekTool } from "./register-tool.js";

import { renderAnchoredFilesResult, renderReadSeekSearchCall } from "./tui-render-utils.js";

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

interface RefsToolOptions {
  onFileAnchored?: FileAnchoredCallback;
}

/**
 * Inputs for executing the references tool without registering it.
 */
export interface ExecuteRefsOptions {
  params: unknown;
  signal: AbortSignal | undefined;
  cwd: string;
  onFileAnchored?: FileAnchoredCallback;
}

function refsLine(reference: ReadSeekReference): RefsOutputLine {
  return {
    ...buildReadSeekLineWithHash(reference.line, reference.line_hash, reference.text),
    enclosingSymbol: reference.enclosingSymbol,
  };
}

function groupReferences(references: ReadSeekReference[], cwd: string): RefsOutputFile[] {
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

function isReadSeekCursorValidationFailure(message: string): boolean {
  return (
    /line and column must be greater than zero/i.test(message) ||
    /line \d+ not found/i.test(message) ||
    /column \d+ exceeds maximum column \d+ for line \d+/i.test(message)
  );
}

/**
 * Executes identifier reference lookup and returns readseek-anchored matches.
 */
export async function executeRefs(opts: ExecuteRefsOptions): Promise<any> {
  const { params, signal, cwd, onFileAnchored } = opts;
  const p = params as RefsParams;
  const ignoredError = validateIgnoredRequiresOthers("refs", p);
  if (ignoredError) return ignoredError;
  if (p.scope && p.line === undefined) {
    return buildToolErrorResult("refs", "invalid-parameter", "Error: refs parameter 'scope' requires 'line'");
  }
  const searchPath = resolveToCwd(p.path ?? ".", cwd);

  const statResult = await statSearchPathOrError("refs", p.path, searchPath);
  if (!statResult.ok) return statResult.error;

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
    const files = groupReferences(references, cwd);
    const builtOutput = buildRefsOutput({ name: p.name, files });
    for (const file of files) {
      onFileAnchored?.(file.path);
    }
    return {
      content: [{ type: "text", text: builtOutput.text }],
      details: { readseekValue: builtOutput.readseekValue },
    };
  } catch (err: any) {
    const failure = classifyReadSeekFailure(err);
    if (p.scope && isReadSeekCursorValidationFailure(failure.message)) {
      return buildToolErrorResult("refs", "invalid-parameter", failure.message);
    }
    return buildToolErrorResult("refs", failure.code, failure.message, failure.hint ? { hint: failure.hint } : {});
  }
}

export function registerRefsTool(pi: ExtensionAPI, options: RefsToolOptions = {}) {
  const tool = registerReadSeekTool(pi, {
    policy: "read-only",
    pythonName: "refs",
    defaultExposure: "opt-in",
  }, {
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
      ...readseekGitSearchParams(),
    }),
    async execute(_toolCallId, params, signal, _onUpdate, ctx) {
      return executeRefs({ params, signal, cwd: ctx.cwd, onFileAnchored: options.onFileAnchored });
    },
    renderCall(args: any, theme: any, ...rest: any[]) {
      return renderReadSeekSearchCall(args, theme, rest, {
        label: "refs",
        accent: args.name,
        flags: [args.scope && "scope", args.cached && "cached", args.others && "others", args.ignored && "ignored"],
      });
    },
    renderResult(result: any, options: ToolRenderResultOptions, theme: any, ...rest: any[]) {
      return renderAnchoredFilesResult(result, options, theme, rest, {
        pendingLabel: "pending refs",
        emptyLabel: "no references",
        unitSingular: "reference",
        unitPlural: "references",
      });
    },
  });
  return tool;
}
