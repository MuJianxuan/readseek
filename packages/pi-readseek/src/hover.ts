import { readFile } from "node:fs/promises";

import type { ExtensionAPI, ToolRenderResultOptions } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { Text } from "@earendil-works/pi-tui";

import { defineToolPromptMetadata } from "./tool-prompt-metadata.js";
import { buildToolErrorResult } from "./readseek-value.js";
import { resolveToCwd } from "./path-utils.js";
import { formatFsError } from "./fs-error.js";
import { classifyReadSeekFailure, readSeekIdentify } from "./readseek-client.js";
import { filePathParam, registerReadSeekTool } from "./register-tool.js";

import { clampLineToWidth, clampLinesToWidth, linkToolPath, renderErrorResult, renderPendingResult, resolveRenderResultContext, summaryLine } from "./tui-render-utils.js";

const HOVER_PROMPT_METADATA = defineToolPromptMetadata({
	promptUrl: new URL("../prompts/hover.md", import.meta.url),
	promptSnippet: "Identify the token and enclosing symbol at a cursor",
});

const hoverSchema = Type.Object({
	path: filePathParam(),
	line: Type.Integer({ minimum: 1, description: "One-based cursor line" }),
	column: Type.Optional(Type.Integer({ minimum: 1, description: "One-based cursor byte column" })),
});

interface HoverParams {
	path: string;
	line: number;
	column?: number;
}

export interface ExecuteHoverOptions {
	params: unknown;
	signal: AbortSignal | undefined;
	cwd: string;
}

export async function executeHover(opts: ExecuteHoverOptions): Promise<any> {
	const { params, signal, cwd } = opts;
	const p = params as HoverParams;

	if (!Number.isSafeInteger(p.line) || p.line < 1) {
		return buildToolErrorResult("hover", "invalid-parameter", "hover parameter 'line' must be a positive integer");
	}
	if (p.column !== undefined && (!Number.isSafeInteger(p.column) || p.column < 1)) {
		return buildToolErrorResult("hover", "invalid-parameter", "hover parameter 'column' must be a positive integer");
	}

	const filePath = resolveToCwd(p.path, cwd);

	let content: string;
	try {
		content = await readFile(filePath, "utf-8");
	} catch (err: any) {
		const { code, message } = formatFsError(err, "hover-error");
		return buildToolErrorResult("hover", code, message, { path: p.path });
	}

	try {
		const output = await readSeekIdentify(filePath, content, {
			line: p.line,
			column: p.column,
			signal,
		});

		const lines: string[] = [];
		if (output.identifier) {
			lines.push(`identifier: ${output.identifier.text}`);
		}
		if (output.symbol) {
			lines.push(`symbol: ${output.symbol.name}`);
			lines.push(`kind: ${output.symbol.kind}`);
			lines.push(`qualified: ${output.symbol.qualified_name}`);
		}
		lines.push(`file: ${output.file}`);
		lines.push(`language: ${output.language}`);
		lines.push(`location: ${output.line}:${output.column}`);

		return {
			content: [{ type: "text", text: lines.join("\n") }],
			details: {
				readSeekValue: {
					tool: "hover",
					ok: true,
					path: filePath,
					output,
				},
			},
		};
	} catch (err: any) {
		const failure = classifyReadSeekFailure(err);
		return buildToolErrorResult("hover", failure.code, failure.message, failure.hint ? { hint: failure.hint } : {});
	}
}

export function registerHoverTool(pi: ExtensionAPI) {
	registerReadSeekTool(pi, {
		name: "readSeek_hover",
		label: "Hover",
		description: HOVER_PROMPT_METADATA.description,
		promptSnippet: HOVER_PROMPT_METADATA.promptSnippet,
		promptGuidelines: HOVER_PROMPT_METADATA.promptGuidelines,
		parameters: hoverSchema,
		async execute(_toolCallId, params, signal, _onUpdate, ctx) {
			return executeHover({ params, signal, cwd: ctx.cwd });
		},
		renderCall(args: any, theme: any, ...rest: any[]) {
			const context = rest[0] ?? {};
			const cwd = context.cwd ?? process.cwd();
			const displayPath = typeof args?.path === "string" ? args.path : "?";
			let text = theme.fg("toolTitle", theme.bold("hover"));
			text += ` ${linkToolPath(theme.fg("accent", displayPath), displayPath, cwd)}`;
			if (args?.line) text += theme.fg("dim", `:${args.line}`);
			return new Text(clampLineToWidth(text, context.width), 0, 0);
		},
		renderResult(result: any, options: ToolRenderResultOptions, theme: any, ...rest: any[]) {
			const { isPartial, isError, expanded, width } = resolveRenderResultContext(options, rest);

			if (isPartial) return renderPendingResult("pending hover", width, theme);

			const content = result.content?.[0];
			const textContent = content?.type === "text" ? content.text : "";

			if (isError || result.isError) {
				return renderErrorResult(textContent, { expanded, width, fallback: "hover failed", theme });
			}

			const output = (result.details as any)?.readSeekValue?.output;
			const label = output?.identifier?.text
				? `identified ${output.identifier.text}`
				: output?.symbol?.name
					? `symbol ${output.symbol.name}`
					: "identified cursor";
			let text = summaryLine(label, { hidden: !!textContent && !expanded, theme, style: "success" });
			if (expanded && textContent) text += `\n${textContent}`;
			return new Text(clampLinesToWidth(text.split("\n"), width).join("\n"), 0, 0);
		},
	});
}
