import { beforeEach, describe, expect, it, vi } from "vitest";

const { replacedTools, settingsWarnings, availability, imageMode } = vi.hoisted(() => ({
	replacedTools: { value: [] as string[] },
	settingsWarnings: { value: [] as Array<{ source: string; message: string }> },
	availability: { value: { available: true } as { available: true } | { available: false; reason: string } },
	imageMode: { value: "auto" as "on" | "auto" | "off" },
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
	resolveReadSeekImageMode: () => imageMode.value,
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
	const toolDefinitions = new Map<string, { description?: string; promptSnippet?: string; promptGuidelines?: string[] }>();
	const notify = vi.fn();

	const pi = {
		registerTool: vi.fn((tool: { name: string; description?: string; promptSnippet?: string; promptGuidelines?: string[] }) => {
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
		imageMode.value = "auto";
	});

	it("exposes image modes according to imageMode", () => {
		const auto = createPi([]);
		piReadSeekExtension(auto.pi);
		const autoRead = auto.toolDefinitions.get("readSeek_read") as any;
		expect(autoRead.promptGuidelines.at(-1)).toContain("none, ocr, caption, objects");
		expect(autoRead.parameters.properties.image.anyOf).toHaveLength(4);

		imageMode.value = "on";
		const on = createPi([]);
		piReadSeekExtension(on.pi);
		const onRead = on.toolDefinitions.get("readSeek_read") as any;
		expect(onRead.promptGuidelines.at(-1)).toContain("ocr, caption, objects");
		expect(onRead.promptGuidelines.at(-1)).not.toContain("none");

		imageMode.value = "off";
		const off = createPi([]);
		piReadSeekExtension(off.pi);
		const offRead = off.toolDefinitions.get("readSeek_read") as any;
		expect(offRead.promptGuidelines.at(-1)).toBe("Image and PDF reads are disabled.");
		expect(offRead.parameters.properties.image).toBeUndefined();
	});

	it("activates readseek tools without removing active built-ins", () => {
		const ctx = createPi(["read", "bash", "edit", "write"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(new Set(ctx.registeredTools)).toEqual(new Set(READSEEK_TOOLS));
		expect(ctx.activeTools()).toEqual(["read", "bash", "edit", "write", ...READSEEK_TOOLS]);
		expect(Object.fromEntries(
			READSEEK_TOOLS.map((name) => [name, ctx.toolDefinitions.get(name)?.promptSnippet]),
		)).toEqual({
			readSeek_read: "Read files or images with anchors, maps, symbols, and OCR",
			readSeek_edit: "Edit with fresh hash-verified anchors",
			readSeek_grep: "Search file text with edit-ready anchors",
			readSeek_search: "Search code by AST pattern with edit-ready anchors",
			readSeek_refs: "Find identifier references with enclosing symbols",
			readSeek_rename: "Rename a binding accurately from its cursor",
			readSeek_hover: "Identify a cursor token and enclosing symbol",
			readSeek_write: "Create or overwrite a file with edit anchors",
			readSeek_def: "Find structural definitions for a symbol",
		});
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
		expect(ctx.toolDefinitions.get("edit")?.description).toBe("Edit existing text files using fresh LINE:HASH anchors from read, grep, readSeek_search, or write.");
		expect(ctx.toolDefinitions.get("edit")?.promptSnippet).toBe("Edit with fresh hash-verified anchors");
	});

	it("leaves the active tools alone when readseek ships no binary for the platform", () => {
		availability.value = { available: false, reason: "@jarkkojs/readseek ships no binary for linux-riscv64" };
		replacedTools.value = ["read"];
		const ctx = createPi(["read", "bash"]);

		piReadSeekExtension(ctx.pi);
		ctx.runSessionStart();

		expect(ctx.notify).toHaveBeenCalledWith(
			"readseek tools are inactive: @jarkkojs/readseek ships no binary for linux-riscv64",
			"warning",
		);
		expect(ctx.pi.setActiveTools).not.toHaveBeenCalled();
		expect(ctx.activeTools()).toEqual(["read", "bash"]);
	});

	it("does not override a built-in with readSeek when the binary is unavailable", () => {
		availability.value = { available: false, reason: "@jarkkojs/readseek ships no binary for linux-riscv64" };
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
