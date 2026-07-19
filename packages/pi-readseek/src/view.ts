import type { ExtensionAPI, ToolRenderResultOptions } from "@earendil-works/pi-coding-agent";
import {
  DEFAULT_MAX_BYTES,
  DEFAULT_MAX_LINES,
  truncateHead,
} from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";

import { coerceObviousBase10Int } from "./coerce-obvious-int.js";
import { resolveToCwd } from "./path-utils.js";
import { classifyReadSeekFailure, readSeekView } from "./readseek-client.js";
import { buildToolErrorResult } from "./readseek-value.js";
import { filePathParam, optionalIntOrString, registerReadSeekTool } from "./register-tool.js";
import { defineToolPromptMetadata } from "./tool-prompt-metadata.js";
import {
  clampLineToWidth,
  clampLinesToWidth,
  linkToolPath,
  renderErrorResult,
  renderPendingResult,
  resolveRenderResultContext,
  summaryLine,
} from "./tui-render-utils.js";

const NODE_KINDS = [
  "artifact",
  "footer",
  "header",
  "heading",
  "marginal_label",
  "page",
  "page_number",
  "paragraph",
  "section",
  "structural_section",
] as const;

const VIEW_PROMPT_METADATA = defineToolPromptMetadata({
  promptUrl: new URL("../prompts/view.md", import.meta.url),
  promptSnippet: "View the structure or selected content of an indexed PDF",
});

interface ViewParams {
  path: string;
  node?: string;
  page?: number | string;
  kind?: (typeof NODE_KINDS)[number];
  depth?: number | string;
  outline?: boolean;
}

export async function executeView(opts: {
  params: unknown;
  signal: AbortSignal | undefined;
  cwd: string;
}): Promise<any> {
  const { params, signal, cwd } = opts;
  const input = params as ViewParams;
  const page = coerceObviousBase10Int(input.page, "page");
  if (!page.ok || (page.value !== undefined && page.value < 1)) {
    const message = page.ok
      ? `Invalid page: expected a positive integer, received ${page.value}.`
      : page.message;
    return buildToolErrorResult("view", "invalid-page", message, { path: input.path });
  }
  const depth = coerceObviousBase10Int(input.depth, "depth");
  if (!depth.ok || (depth.value !== undefined && depth.value < 0)) {
    const message = depth.ok
      ? `Invalid depth: expected a non-negative integer, received ${depth.value}.`
      : depth.message;
    return buildToolErrorResult("view", "invalid-depth", message, { path: input.path });
  }
  const node = input.node?.trim();
  if (input.node !== undefined && !node) {
    return buildToolErrorResult("view", "invalid-node", "Invalid node: expected a non-empty ID.", {
      path: input.path,
    });
  }

  const filePath = resolveToCwd(input.path, cwd);
  try {
    const output = await readSeekView(filePath, {
      node,
      page: page.value,
      kind: input.kind,
      depth: depth.value,
      outline: input.outline,
      signal,
    });
    const truncation = truncateHead(output, {
      maxLines: DEFAULT_MAX_LINES,
      maxBytes: DEFAULT_MAX_BYTES,
    });
    const notice = truncation.truncated
      ? "\n[… document view truncated; narrow it with page, node, kind, or depth]"
      : "";
    return {
      content: [{ type: "text", text: `${truncation.content}${notice}` }],
      details: {
        readSeekValue: {
          tool: "view",
          path: filePath,
          truncated: truncation.truncated,
        },
      },
    };
  } catch (error) {
    const failure = classifyReadSeekFailure(error);
    return buildToolErrorResult("view", failure.code, failure.message, failure.hint ? { hint: failure.hint } : {});
  }
}

export function registerViewTool(pi: ExtensionAPI) {
  return registerReadSeekTool(pi, {
    name: "readSeek_view",
    label: "View",
    description: VIEW_PROMPT_METADATA.description,
    promptSnippet: VIEW_PROMPT_METADATA.promptSnippet,
    promptGuidelines: VIEW_PROMPT_METADATA.promptGuidelines,
    parameters: Type.Object({
      path: filePathParam(),
      node: Type.Optional(Type.String({ description: "Node ID to use as the view root" })),
      page: optionalIntOrString("One-based source page"),
      kind: Type.Optional(Type.Union(NODE_KINDS.map((kind) => Type.Literal(kind)), {
        description: "Node kind filter",
      })),
      depth: optionalIntOrString("Maximum depth below the selected roots"),
      outline: Type.Optional(Type.Boolean({ description: "Return outline nodes only" })),
    }),
    async execute(_toolCallId, params, signal, _onUpdate, ctx) {
      return executeView({ params, signal, cwd: ctx.cwd });
    },
    renderCall(args: any, theme: any, ...rest: any[]) {
      const context = rest[0] ?? {};
      const cwd = context.cwd ?? process.cwd();
      const displayPath = typeof args?.path === "string" ? args.path : "?";
      const text = `${theme.fg("toolTitle", theme.bold("view"))} ${linkToolPath(theme.fg("accent", displayPath), displayPath, cwd)}`;
      return new Text(clampLineToWidth(text, context.width), 0, 0);
    },
    renderResult(result: any, options: ToolRenderResultOptions, theme: any, ...rest: any[]) {
      const { isPartial, isError, expanded, width } = resolveRenderResultContext(options, rest);
      if (isPartial) return renderPendingResult("pending view", width, theme);
      const textContent = result.content?.[0]?.type === "text" ? result.content[0].text : "";
      if (isError || result.isError) {
        return renderErrorResult(textContent, { expanded, width, fallback: "view failed", theme });
      }
      const lines = textContent.split("\n").filter(Boolean).length;
      let text = summaryLine(`loaded ${lines} document ${lines === 1 ? "line" : "lines"}`, {
        hidden: !!textContent && !expanded,
        theme,
        style: "success",
      });
      if (expanded && textContent) text += `\n${textContent}`;
      return new Text(clampLinesToWidth(text.split("\n"), width).join("\n"), 0, 0);
    },
  });
}
