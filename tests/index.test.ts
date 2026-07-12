import { beforeEach, describe, expect, it, vi } from "vitest";

const { excludeTools, settingsWarnings, availability } = vi.hoisted(() => ({
	excludeTools: { value: [] as string[] },
	settingsWarnings: { value: [] as Array<{ source: string; message: string }> },
	availability: { value: { available: true } as { available: true } | { available: false; reason: string } },
}));

vi.mock("../src/readseek-client.js", async (importOriginal) => ({
	...(await importOriginal<typeof import("../src/readseek-client.js")>()),
	readSeekBinaryAvailability: () => availability.value,
}));

vi.mock("../src/readseek-settings.js", () => ({
	resolveReadSeekJsonSettings: () => ({
		settings: { excludeTools: excludeTools.value },
		warnings: settingsWarnings.value,
	}),
	resolveReadSeekImageMode: () => "force",
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
	let sessionStart: ((event: unknown, ctx: unknown) => void) | undefined;
	const registeredTools: string[] = [];
	const notify = vi.fn();

	const pi = {
		registerTool: vi.fn((tool: { name: string }) => {
			registeredTools.push(tool.name);
		}),
		on: vi.fn((event: string, handler: (event: unknown, ctx: unknown) => void) => {
			if (event === "session_start") sessionStart = handler;
		}),
		getActiveTools: vi.fn(() => [...activeTools]),
		getAllTools: vi.fn(() => [...activeToolNames, ...registeredTools].map((name) => ({ name }))),
		setActiveTools: vi.fn((toolNames: string[]) => {
			activeTools = [...toolNames];
		}),
	};

	return {
		pi: pi as any,
		registeredTools,
		notify,
		runSessionStart: () => sessionStart?.({ reason: "startup" }, { hasUI: true, ui: { notify } }),
		activeTools: () => activeTools,
	};
}

describe("pi-readseek extension", () => {
	beforeEach(() => {
		excludeTools.value = [];
		settingsWarnings.value = [];
		availability.value = { available: true };
	});

	it("activates readseek tools without removing active built-ins", () => {
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

	it("leaves the active tools alone when readseek ships no binary for the platform", () => {
		availability.value = { available: false, reason: "@jarkkojs/readseek ships no binary for linux-arm64" };
		excludeTools.value = ["read"];
		const ctx = createPi(["read", "bash"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(ctx.notify).toHaveBeenCalledWith(
			"readseek tools are inactive: @jarkkojs/readseek ships no binary for linux-arm64",
			"warning",
		);
		expect(ctx.pi.setActiveTools).not.toHaveBeenCalled();
		expect(ctx.activeTools()).toEqual(["read", "bash"]);
	});

	it("warns about settings problems at session start", () => {
		settingsWarnings.value = [
			{ source: "/home/user/.pi/agent/readseek/settings.json", message: "Invalid readseek setting at readseek.imageMode" },
		];
		const ctx = createPi(["read"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(ctx.notify).toHaveBeenCalledWith(
			"Invalid readseek setting at readseek.imageMode (/home/user/.pi/agent/readseek/settings.json)",
			"warning",
		);
	});

	it("warns about tool names that excludeTools cannot match", () => {
		excludeTools.value = ["readseek_hover", "read"];
		const ctx = createPi(["read", "bash"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(ctx.notify).toHaveBeenCalledTimes(1);
		expect(ctx.notify).toHaveBeenCalledWith('Unknown tool "readseek_hover" in readseek.excludeTools', "warning");
		expect(ctx.activeTools()).toEqual(["bash", ...READSEEK_TOOLS]);
	});
});
