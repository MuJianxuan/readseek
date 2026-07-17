import { beforeEach, describe, expect, it, vi } from "vitest";
import { Value } from "@sinclair/typebox/value";

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
	"readSeek_check",
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
		expect(autoRead.promptGuidelines.at(-1)).toContain("none, all, ocr, caption, objects");
		expect(autoRead.parameters.properties.image.anyOf).toHaveLength(5);

		imageMode.value = "on";
		const on = createPi([]);
		piReadSeekExtension(on.pi);
		const onRead = on.toolDefinitions.get("readSeek_read") as any;
		expect(onRead.promptGuidelines.at(-1)).toContain("all, ocr, caption, objects");
		expect(onRead.promptGuidelines.at(-1)).not.toContain("none");

		imageMode.value = "off";
		const off = createPi([]);
		piReadSeekExtension(off.pi);
		const offRead = off.toolDefinitions.get("readSeek_read") as any;
		expect(offRead.promptGuidelines.at(-1)).toContain("always skipped");
		expect(offRead.description).toContain("skipped");
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
			readSeek_read: "Read anchored text, symbols, maps, images, or PDFs",
			readSeek_edit: "Safely edit with fresh LINE:HASH anchors",
			readSeek_grep: "Search plain text or regex with edit-ready anchors",
			readSeek_search: "Search syntax-aware code shapes with AST patterns",
			readSeek_refs: "Find usages of an identifier or cursor binding",
			readSeek_rename: "Rename the symbol at a cursor without touching shadows",
			readSeek_hover: "Identify the token and enclosing symbol at a cursor",
			readSeek_write: "Create or replace a complete file with edit anchors",
			readSeek_def: "Find where a symbol is defined",
			readSeek_check: "Check a source file for parser errors and missing syntax",
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
			"readSeek_check",
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
			"readSeek_check",
		]);
		expect(ctx.toolDefinitions.get("read")?.promptGuidelines?.[0]).toBe("Use read; it provides LINE:HASH anchors for safe edits.");
		expect(ctx.toolDefinitions.get("edit")?.promptGuidelines?.[0]).toBe("Use edit; it verifies fresh LINE:HASH anchors.");
		expect(ctx.toolDefinitions.get("grep")?.promptGuidelines?.[0]).toBe("Use grep; it returns edit-ready anchors.");
		expect(ctx.toolDefinitions.get("write")?.promptGuidelines?.[0]).toBe("Use write; it returns LINE:HASH anchors.");
		expect(ctx.toolDefinitions.get("edit")?.description).toBe("Edit existing text files safely with fresh `LINE:HASH` anchors; on `file-not-read`, read or search the file first.");
		expect(ctx.toolDefinitions.get("edit")?.promptSnippet).toBe("Safely edit with fresh LINE:HASH anchors");
	});

	it("rejects extra edit variant keys while allowing nested replacement options", () => {
		const ctx = createPi([]);
		piReadSeekExtension(ctx.pi);
		const schema = (ctx.toolDefinitions.get("readSeek_edit") as any).parameters;

		expect(Value.Check(schema, {
			path: "target.ts",
			edits: [{ replace: { old_text: "old", new_text: "new", all: true, fuzzy: true } }],
		})).toBe(true);
		expect(Value.Check(schema, {
			path: "target.ts",
			edits: [{ set_line: { anchor: "1:abc", new_text: "new" }, replace: { old_text: "old", new_text: "new" } }],
		})).toBe(false);
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
