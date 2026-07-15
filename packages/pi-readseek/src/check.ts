import { readFile } from "node:fs/promises";

import type { ExtensionAPI, ToolRenderResultOptions } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";

import { classifyReadSeekFailure, readSeekCheck, type ReadSeekCheckOutput } from "./readseek-client.js";
import { buildToolErrorResult } from "./readseek-value.js";
import { defineToolPromptMetadata } from "./tool-prompt-metadata.js";
import { formatFsError } from "./fs-error.js";
import { resolveToCwd } from "./path-utils.js";
import { filePathParam, registerReadSeekTool } from "./register-tool.js";
import { clampLineToWidth, clampLinesToWidth, linkToolPath, renderErrorResult, renderPendingResult, resolveRenderResultContext, summaryLine } from "./tui-render-utils.js";

const CHECK_PROMPT_METADATA = defineToolPromptMetadata({
  promptUrl: new URL("../prompts/check.md", import.meta.url),
  promptSnippet: "Check a source file for parser errors and missing syntax",
});

export async function executeCheck(opts: {
  params: unknown;
  signal: AbortSignal | undefined;
  cwd: string;
}): Promise<any> {
  const { params, signal, cwd } = opts;
  const p = params as { path: string };
  const filePath = resolveToCwd(p.path, cwd);

  let content: string;
  try {
    content = await readFile(filePath, "utf-8");
  } catch (err: any) {
    const { code, message } = formatFsError(err, "check-error");
    return buildToolErrorResult("check", code, message, { path: p.path });
  }

  try {
    const output = await readSeekCheck(filePath, content, { signal });
    const lines = output.diagnostics.map((diagnostic) =>
      `${diagnostic.kind}: lines ${diagnostic.start_line}-${diagnostic.end_line}`,
    );
    const summary = output.errorCount === 0 && output.missingCount === 0
      ? "No syntax errors found."
      : `${output.errorCount} error(s), ${output.missingCount} missing syntax node(s)`;
    return {
      content: [{ type: "text", text: [summary, ...lines].join("\n") }],
      details: {
        readSeekValue: { tool: "check", ok: true, path: filePath, output },
      },
    };
  } catch (err: any) {
    const failure = classifyReadSeekFailure(err);
    return buildToolErrorResult("check", failure.code, failure.message, failure.hint ? { hint: failure.hint } : {});
  }
}

export function registerCheckTool(pi: ExtensionAPI) {
  return registerReadSeekTool(pi, {
    name: "readSeek_check",
    label: "Check",
    description: CHECK_PROMPT_METADATA.description,
    promptSnippet: CHECK_PROMPT_METADATA.promptSnippet,
    promptGuidelines: CHECK_PROMPT_METADATA.promptGuidelines,
    parameters: Type.Object({ path: filePathParam() }),
    async execute(_toolCallId, params, signal, _onUpdate, ctx) {
      return executeCheck({ params, signal, cwd: ctx.cwd });
    },
    renderCall(args: any, theme: any, ...rest: any[]) {
      const context = rest[0] ?? {};
      const cwd = context.cwd ?? process.cwd();
      const displayPath = typeof args?.path === "string" ? args.path : "?";
      const text = `${theme.fg("toolTitle", theme.bold("check"))} ${linkToolPath(theme.fg("accent", displayPath), displayPath, cwd)}`;
      return new Text(clampLineToWidth(text, context.width), 0, 0);
    },
    renderResult(result: any, options: ToolRenderResultOptions, theme: any, ...rest: any[]) {
      const { isPartial, isError, expanded, width } = resolveRenderResultContext(options, rest);
      if (isPartial) return renderPendingResult("pending check", width, theme);
      const textContent = result.content?.[0]?.type === "text" ? result.content[0].text : "";
      if (isError || result.isError) {
        return renderErrorResult(textContent, { expanded, width, fallback: "check failed", theme });
      }
      const output = (result.details as any)?.readSeekValue?.output as ReadSeekCheckOutput | undefined;
      const issueCount = (output?.errorCount ?? 0) + (output?.missingCount ?? 0);
      let text = summaryLine(issueCount === 0 ? "syntax valid" : `${issueCount} syntax issue(s)`, {
        theme,
        style: issueCount === 0 ? "success" : "warning",
      });
      if (expanded && textContent) text += `\n${textContent}`;
      return new Text(clampLinesToWidth(text.split("\n"), width).join("\n"), 0, 0);
    },
  });
}
