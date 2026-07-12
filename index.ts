import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerReadTool } from "./src/read.js";
import { registerEditTool } from "./src/edit.js";
import { registerGrepTool } from "./src/grep.js";
import { registerSgTool } from "./src/sg.js";
import { registerRefsTool } from "./src/refs.js";
import { registerRenameTool } from "./src/rename.js";
import { registerHoverTool } from "./src/hover.js";
import { registerWriteTool } from "./src/write.js";
import { registerDefTool } from "./src/def.js";
import { SessionAnchors } from "./src/session-anchors.js";
import { readSeekBinaryAvailability } from "./src/readseek-client.js";
import { resolveReadSeekJsonSettings, type ReadSeekSettingsWarning } from "./src/readseek-settings.js";

const READSEEK_TOOL_NAMES = [
	"readSeek_read",
	"readSeek_edit",
	"readSeek_grep",
	"readSeek_search",
	"readSeek_refs",
	"readSeek_rename",
	"readSeek_hover",
	"readSeek_write",
	"readSeek_def",
];

function formatSettingsWarning(warning: ReadSeekSettingsWarning): string {
	return `${warning.message} (${warning.source})`;
}

export default function piReadSeekExtension(pi: ExtensionAPI): void {
	const sessionAnchors = new SessionAnchors();
	const markAnchored = (absolutePath: string) => sessionAnchors.markAnchored(absolutePath);
	const hasFreshAnchors = (absolutePath: string) => sessionAnchors.hasFreshAnchors(absolutePath);

	registerReadTool(pi, { onSuccessfulRead: markAnchored });
	registerEditTool(pi, { wasReadInSession: hasFreshAnchors });
	registerGrepTool(pi, { onFileAnchored: markAnchored });
	registerSgTool(pi, { onFileAnchored: markAnchored });
	registerRefsTool(pi, { onFileAnchored: markAnchored });
	registerRenameTool(pi);
	registerHoverTool(pi);
	registerDefTool(pi, { onFileAnchored: markAnchored });
	registerWriteTool(pi, { onFileAnchored: markAnchored });

	pi.on("session_start", (_event, ctx) => {
		const { settings, warnings } = resolveReadSeekJsonSettings();
		const replaceTools = new Set<string>(settings.replaceTools ?? []);
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

		const activeTools = [...pi.getActiveTools(), ...READSEEK_TOOL_NAMES]
			.filter((name) => !replaceTools.has(name));
		pi.setActiveTools([...new Set(activeTools)]);
	});
}
