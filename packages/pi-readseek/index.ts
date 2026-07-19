import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerReadTool } from "./src/read.js";
import { registerEditTool } from "./src/edit.js";
import { registerGrepTool } from "./src/grep.js";
import { registerSearchTool } from "./src/search.js";
import { registerRefsTool } from "./src/refs.js";
import { registerRenameTool } from "./src/rename.js";
import { registerHoverTool } from "./src/hover.js";
import { registerWriteTool } from "./src/write.js";
import { registerDefTool } from "./src/def.js";
import { registerCheckTool } from "./src/check.js";
import { registerViewTool } from "./src/view.js";
import { SessionAnchors } from "./src/session-anchors.js";
import { readSeekBinaryAvailability } from "./src/readseek-client.js";
import { resolveReadSeekJsonSettings, type ReadSeekSettingsWarning } from "./src/readseek-settings.js";

/** Built-in tool names that pi-readseek can replace with a readSeek-backed implementation. */
const REPLACEABLE_TOOLS = {
	read: "readSeek_read",
	edit: "readSeek_edit",
	write: "readSeek_write",
	grep: "readSeek_grep",
} as const;

type ReplacedBuiltIn = keyof typeof REPLACEABLE_TOOLS;

/**
 * Canonical list of readSeek tools in registration/activation order. Each
 * replaceable entry carries the built-in name it swaps in when replaced; null
 * entries are readSeek-only and never replaced.
 */
const READSEEK_TOOL_ENTRIES: ReadonlyArray<{ builtIn: ReplacedBuiltIn | null; readSeekName: string }> = [
	{ builtIn: "read", readSeekName: "readSeek_read" },
	{ builtIn: "edit", readSeekName: "readSeek_edit" },
	{ builtIn: "grep", readSeekName: "readSeek_grep" },
	{ builtIn: null, readSeekName: "readSeek_search" },
	{ builtIn: null, readSeekName: "readSeek_refs" },
	{ builtIn: null, readSeekName: "readSeek_rename" },
	{ builtIn: null, readSeekName: "readSeek_hover" },
	{ builtIn: "write", readSeekName: "readSeek_write" },
	{ builtIn: null, readSeekName: "readSeek_def" },
	{ builtIn: null, readSeekName: "readSeek_check" },
	{ builtIn: null, readSeekName: "readSeek_view" },
];

function formatSettingsWarning(warning: ReadSeekSettingsWarning): string {
	return `${warning.message} (${warning.source})`;
}

export default function piReadSeekExtension(pi: ExtensionAPI): void {
	const sessionAnchors = new SessionAnchors();
	const markAnchored = (absolutePath: string) => sessionAnchors.markAnchored(absolutePath);
	const hasFreshAnchors = (absolutePath: string) => sessionAnchors.hasFreshAnchors(absolutePath);
	const forgetAnchors = (absolutePath: string) => sessionAnchors.forget(absolutePath);

	// Resolve replacedTools at load time so the readSeek implementation is
	// registered under the built-in name (e.g. "edit") when replaced. Extension
	// tool registrations override built-ins of the same name in pi's tool
	// definition registry, so registering under "edit" makes calls to `edit`
	// dispatch to the readSeek implementation.
	const { settings } = resolveReadSeekJsonSettings();
	const replacedBuiltIns = new Set<ReplacedBuiltIn>(
		(settings.replacedTools ?? []) as ReplacedBuiltIn[],
	);
	// Only swap registration under the built-in name when the readSeek binary
	// is available; otherwise keep readSeek_* names so pi's built-ins stay
	// intact and usable.
	const binaryAvailable = readSeekBinaryAvailability().available;
	const swap = (builtIn: ReplacedBuiltIn, readSeekName: string) =>
		binaryAvailable && replacedBuiltIns.has(builtIn) ? builtIn : readSeekName;

	const readName = swap("read", "readSeek_read");
	const editName = swap("edit", "readSeek_edit");
	const writeName = swap("write", "readSeek_write");
	const grepName = swap("grep", "readSeek_grep");
	const toolAliases = {
		readSeek_read: readName,
		readSeek_edit: editName,
		readSeek_grep: grepName,
		readSeek_write: writeName,
	};

	registerReadTool(pi, { onSuccessfulRead: markAnchored, name: readName });
	registerEditTool(pi, { wasReadInSession: hasFreshAnchors, onFileMutated: forgetAnchors, name: editName, toolAliases });
	registerGrepTool(pi, { onFileAnchored: markAnchored, name: grepName });
	registerSearchTool(pi, { onFileAnchored: markAnchored });
	registerRefsTool(pi, { onFileAnchored: markAnchored });
	registerRenameTool(pi, { onFileMutated: forgetAnchors });
	registerHoverTool(pi);
	registerDefTool(pi, { onFileAnchored: markAnchored });
	registerCheckTool(pi);
	registerViewTool(pi);
	registerWriteTool(pi, { onFileAnchored: markAnchored, name: writeName });

	pi.on("session_start", (_event, ctx) => {
		sessionAnchors.clear();
		const { warnings } = resolveReadSeekJsonSettings();
		const problems = warnings.map(formatSettingsWarning);

		const availability = readSeekBinaryAvailability();
		if (!availability.available) {
			problems.push(`readseek tools are inactive: ${availability.reason}`);
		}

		for (const problem of problems) {
			if (ctx.hasUI) ctx.ui.notify(problem, "warning");
			else console.warn(problem);
		}

		if (!availability.available) return;

		// readSeek tools were registered at load under names chosen from the
		// load-time replacedTools. Replaced built-ins are registered under the
		// built-in name, so the built-in stays active (now readSeek-backed);
		// their readSeek_* variant was never registered and is excluded.
		const activeReadSeekNames = READSEEK_TOOL_ENTRIES.map((entry) =>
			entry.builtIn && replacedBuiltIns.has(entry.builtIn) ? entry.builtIn : entry.readSeekName,
		);
		const inactiveReadSeekNames = new Set(
			READSEEK_TOOL_ENTRIES.filter(
				(entry): entry is { builtIn: ReplacedBuiltIn; readSeekName: string } =>
					entry.builtIn !== null && replacedBuiltIns.has(entry.builtIn),
			).map((entry) => entry.readSeekName),
		);

		const activeTools = [...pi.getActiveTools(), ...activeReadSeekNames].filter(
			(name) => !inactiveReadSeekNames.has(name),
		);
		pi.setActiveTools([...new Set(activeTools)]);
	});

}
