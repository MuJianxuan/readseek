import type { ExtensionAPI, ToolRenderResultOptions } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import path from "node:path";
import { Text } from "@earendil-works/pi-tui";

import { defineToolPromptMetadata } from "./tool-prompt-metadata.js";
import { buildToolErrorResult } from "./readseek-value.js";
import { resolveToCwd } from "./path-utils.js";
import { classifyReadSeekFailure, readSeekRename, type RenameOutput } from "./readseek-client.js";
import { filePathParam, registerReadSeekTool } from "./register-tool.js";

import { clampLineToWidth, clampLinesToWidth, linkedPathLine, linkToolPath, renderErrorResult, renderPendingResult, resolveRenderResultContext, summaryLine } from "./tui-render-utils.js";

const RENAME_PROMPT_METADATA = defineToolPromptMetadata({
	promptUrl: new URL("../prompts/rename.md", import.meta.url),
	promptSnippet: "Rename a binding accurately from its cursor",
});

const renameSchema = Type.Object({
	path: filePathParam(),
	line: Type.Number({ description: "One-based cursor line of the binding to rename" }),
	column: Type.Optional(Type.Number({ description: "One-based cursor byte column of the binding to rename" })),
	to: Type.String({ description: "New name for the binding" }),
	workspace: Type.Optional(Type.Boolean({ description: "Expand rename across project root (name-based outside cursor file)" })),
	apply: Type.Optional(Type.Boolean({ description: "Write the planned edits to disk after verifying line hashes" })),
});

interface RenameParams {
	path: string;
	line: number;
	column?: number;
	to: string;
	workspace?: boolean;
	apply?: boolean;
}

export interface ExecuteRenameOptions {
	params: unknown;
	signal: AbortSignal | undefined;
	cwd: string;
}

export async function executeRename(opts: ExecuteRenameOptions): Promise<any> {
	const { params, signal, cwd } = opts;
	const p = params as RenameParams;

	if (!p.to.trim()) {
		return buildToolErrorResult("rename", "invalid-parameter", "rename parameter 'to' must not be empty");
	}
	if (!Number.isSafeInteger(p.line) || p.line < 1) {
		return buildToolErrorResult("rename", "invalid-parameter", "rename parameter 'line' must be a positive integer");
	}

	const filePath = resolveToCwd(p.path, cwd);

	try {
		const output = await readSeekRename(filePath, {
			to: p.to,
			line: p.line,
			column: p.column,
			workspace: p.workspace ? cwd : undefined,
			apply: p.apply ?? true,
			signal,
		});

		const files: string[] = [output.file];
		for (const other of output.others) {
			const abs = path.isAbsolute(other.file) ? other.file : path.resolve(cwd, other.file);
			if (!files.includes(abs)) files.push(abs);
		}

		const totalEdits = output.edits.length + output.others.reduce((sum, o) => sum + o.edits.length, 0);
		const totalConflicts = output.conflicts.length + output.others.reduce((sum, o) => sum + o.conflicts.length, 0);

		const parts: string[] = [];
		if (totalConflicts > 0) {
			parts.push(`${totalConflicts} naming conflict(s)`);
		}
		parts.push(`renamed ${output.old_name} to ${output.new_name} in ${totalEdits} location(s) across ${files.length} file(s)`);

		let text = parts.join("; ");
		if (output.applied) text += " (applied)";
		else text += " (dry-run)";

		// Surface first conflict reason for visibility
		const firstConflict = output.conflicts[0]
			?? output.others.find((o) => o.conflicts.length > 0)?.conflicts[0];
		if (firstConflict) {
			text += `\nFirst conflict at ${firstConflict.line}:${firstConflict.column}: ${firstConflict.reason}`;
		}

		return {
			content: [{ type: "text", text }],
			details: {
				readSeekValue: {
					tool: "rename",
					ok: true,
					path: filePath,
					output,
				},
			},
		};
	} catch (err: any) {
		const failure = classifyReadSeekFailure(err);
		return buildToolErrorResult("rename", failure.code, failure.message, failure.hint ? { hint: failure.hint } : {});
	}
}

export function registerRenameTool(pi: ExtensionAPI) {
	registerReadSeekTool(pi, {
		name: "readSeek_rename",
		label: "Rename",
		description: RENAME_PROMPT_METADATA.description,
		promptSnippet: RENAME_PROMPT_METADATA.promptSnippet,
		promptGuidelines: RENAME_PROMPT_METADATA.promptGuidelines,
		parameters: renameSchema,
		async execute(_toolCallId, params, signal, _onUpdate, ctx) {
			return executeRename({ params, signal, cwd: ctx.cwd });
		},
		renderCall(args: any, theme: any, ...rest: any[]) {
			const context = rest[0] ?? {};
			const cwd = context.cwd ?? process.cwd();
			const displayPath = typeof args?.path === "string" ? args.path : "?";
			let text = theme.fg("toolTitle", theme.bold("rename"));
			text += ` ${linkToolPath(theme.fg("accent", displayPath), displayPath, cwd)}`;
			if (args?.to) text += theme.fg("dim", ` → ${args.to}`);
			return new Text(clampLineToWidth(text, context.width), 0, 0);
		},
		renderResult(result: any, options: ToolRenderResultOptions, theme: any, ...rest: any[]) {
			const { isPartial, isError, expanded, cwd, width } = resolveRenderResultContext(options, rest);

			if (isPartial) return renderPendingResult("pending rename", width, theme);

			const content = result.content?.[0];
			const textContent = content?.type === "text" ? content.text : "";
			const readSeekValue = (result.details as any)?.readSeekValue;
			const output = readSeekValue?.output as RenameOutput | undefined;

			if (isError || result.isError) {
				return renderErrorResult(textContent, { expanded, width, fallback: "rename failed", theme });
			}

			const outputs = output ? [output, ...output.others] : [];
			const editCount = outputs.reduce((total, file) => total + file.edits.length, 0);
			const conflictCount = outputs.reduce((total, file) => total + file.conflicts.length, 0);
			const fileCount = outputs.length;
			const action = output?.applied
				? `renamed ${output.old_name} → ${output.new_name}`
				: `rename plan for ${output?.old_name ?? "?"} → ${output?.new_name ?? "?"} (dry-run)`;
			const details = output
				? ` • ${editCount} ${editCount === 1 ? "edit" : "edits"} in ${fileCount} ${fileCount === 1 ? "file" : "files"}`
				: "";
			const conflicts = conflictCount > 0 ? ` • ${conflictCount} ${conflictCount === 1 ? "conflict" : "conflicts"}` : "";
			let text = summaryLine(`${action}${details}${conflicts}`, {
				theme,
				style: conflictCount > 0 ? "warning" : "success",
			});

			if (expanded && output) {
				for (const file of outputs) {
					const display = path.relative(cwd, file.file) || file.file;
					text += `\n${linkedPathLine(theme, file.file, display, cwd, ` (${file.edits.length} edits)`, width)}`;
				}
			}

			const lines = clampLinesToWidth(text.split("\n"), width);
			return new Text(lines.join("\n"), 0, 0);
		},
	});
}
