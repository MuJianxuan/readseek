import { describe, expect, it, vi } from "vitest";

const { excludeTools } = vi.hoisted(() => ({
	excludeTools: { value: [] as string[] },
}));

vi.mock("../src/readseek-settings.js", () => ({
	resolveReadSeekExcludeTools: () => excludeTools.value,
	resolveReadSeekJsonSettings: () => ({ settings: {}, warnings: [] }),
	resolveReadSeekOcrMode: () => "force",
	resolveReadSeekSyntaxValidation: () => undefined,
	resolveReadSeekTimeoutMs: () => undefined,
}));

const { default: piReadSeekExtension } = await import("../index.js");

const READSEEK_TOOLS = [
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

function createPi(activeToolNames: string[]) {
	let activeTools = [...activeToolNames];
	let sessionStart: (() => void) | undefined;
	const registeredTools: string[] = [];

	const pi = {
		registerTool: vi.fn((tool: { name: string }) => {
			registeredTools.push(tool.name);
		}),
		on: vi.fn((event: string, handler: () => void) => {
			if (event === "session_start") sessionStart = handler;
		}),
		getActiveTools: vi.fn(() => [...activeTools]),
		setActiveTools: vi.fn((toolNames: string[]) => {
			activeTools = [...toolNames];
		}),
	};

	return {
		pi: pi as any,
		registeredTools,
		runSessionStart: () => sessionStart?.(),
		activeTools: () => activeTools,
	};
}

describe("pi-readseek extension", () => {
	it("activates readseek tools without removing active built-ins", () => {
		excludeTools.value = [];
		const ctx = createPi(["read", "bash", "edit", "write"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(new Set(ctx.registeredTools)).toEqual(new Set(READSEEK_TOOLS));
		expect(ctx.activeTools()).toEqual(["read", "bash", "edit", "write", ...READSEEK_TOOLS]);
	});

	it("excludes configured active tools after adding readseek tools", () => {
		excludeTools.value = ["read", "edit", "write", "grep", "readSeek_hover"];
		const ctx = createPi(["read", "bash", "edit", "write", "grep"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(ctx.activeTools()).toEqual([
			"bash",
			"readSeek_read",
			"readSeek_edit",
			"readSeek_grep",
			"readSeek_search",
			"readSeek_refs",
			"readSeek_rename",
			"readSeek_write",
			"readSeek_def",
		]);
	});
});
