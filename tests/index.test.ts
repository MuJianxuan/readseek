import { beforeEach, describe, expect, it, vi } from "vitest";

const { replacedTools, settingsWarnings, availability } = vi.hoisted(() => ({
	replacedTools: { value: [] as string[] },
	settingsWarnings: { value: [] as Array<{ source: string; message: string }> },
	availability: { value: { available: true } as { available: true } | { available: false; reason: string } },
}));

vi.mock("../src/readseek-client.js", async (importOriginal) => ({
	...(await importOriginal<typeof import("../src/readseek-client.js")>()),
	readSeekBinaryAvailability: () => availability.value,
}));

vi.mock("../src/readseek-settings.js", () => ({
	resolveReadSeekJsonSettings: () => ({
		settings: { replacedTools: replacedTools.value },
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
	const toolDefinitions = new Map<string, { promptGuidelines?: string[] }>();
	const notify = vi.fn();

	const pi = {
		registerTool: vi.fn((tool: { name: string; promptGuidelines?: string[] }) => {
			registeredTools.push(tool.name);
			toolDefinitions.set(tool.name, tool);
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
		toolDefinitions,
		notify,
		runSessionStart: () => sessionStart?.({ reason: "startup" }, { hasUI: true, ui: { notify } }),
		activeTools: () => activeTools,
	};
}

describe("pi-readseek extension", () => {
	beforeEach(() => {
		replacedTools.value = [];
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

	it("replaces configured built-in tools by registering readseek under the built-in name", () => {
		replacedTools.value = ["read", "edit", "write", "grep"];
		const ctx = createPi(["read", "bash", "edit", "write", "grep"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		// Replaced readSeek tools are registered under the built-in name; the
		// readSeek_* variants are not registered at all.
		expect(new Set(ctx.registeredTools)).toEqual(new Set([
			"read", "edit", "grep", "write",
			"readSeek_search",
			"readSeek_refs",
			"readSeek_rename",
			"readSeek_hover",
			"readSeek_def",
		]));
		// The built-in name stays active (now readSeek-backed); the readSeek_*
		// variants are dropped.
		expect(ctx.activeTools()).toEqual([
			"read",
			"bash",
			"edit",
			"write",
			"grep",
			"readSeek_search",
			"readSeek_refs",
			"readSeek_rename",
			"readSeek_hover",
			"readSeek_def",
		]);
		expect(ctx.toolDefinitions.get("read")?.promptGuidelines?.[0]).toBe("Use read; it provides LINE:HASH anchors for safe edits.");
		expect(ctx.toolDefinitions.get("edit")?.promptGuidelines?.[0]).toBe("Use edit; it verifies fresh LINE:HASH anchors.");
		expect(ctx.toolDefinitions.get("grep")?.promptGuidelines?.[0]).toBe("Use grep; it returns edit-ready anchors.");
		expect(ctx.toolDefinitions.get("write")?.promptGuidelines?.[0]).toBe("Use write; it returns LINE:HASH anchors.");
	});

	it("leaves the active tools alone when readseek ships no binary for the platform", () => {
		availability.value = { available: false, reason: "@jarkkojs/readseek ships no binary for linux-arm64" };
		replacedTools.value = ["read"];
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

	it("does not override a built-in with readSeek when the binary is unavailable", () => {
		availability.value = { available: false, reason: "@jarkkojs/readseek ships no binary for linux-arm64" };
		replacedTools.value = ["edit"];
		const ctx = createPi(["edit", "bash"]);

		piReadSeekExtension(ctx.pi);

		// readSeek_edit is registered as readSeek_edit (not "edit"), so pi's
		// built-in edit definition is not overridden.
		expect(ctx.registeredTools).toContain("readSeek_edit");
		expect(ctx.registeredTools).not.toContain("edit");
	});

	it("warns about settings problems at session start", () => {
		settingsWarnings.value = [
			{ source: "/home/user/.pi/agent/settings.json", message: "Invalid readseek setting at readseek.imageMode" },
		];
		const ctx = createPi(["read"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(ctx.notify).toHaveBeenCalledWith(
			"Invalid readseek setting at readseek.imageMode (/home/user/.pi/agent/settings.json)",
			"warning",
		);
	});

});
