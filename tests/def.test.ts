import { describe, expect, it, vi } from "vitest";

const { readSeekDefMock } = vi.hoisted(() => ({
	readSeekDefMock: vi.fn(),
}));

vi.mock("../src/readseek-client.js", () => ({
	classifyReadSeekFailure: (err: unknown) => ({
		code: "readseek-execution-error",
		message: String((err as { message?: unknown } | null)?.message || err),
	}),
	readSeekDef: readSeekDefMock,
}));

vi.mock("../src/register-tool.js", () => ({
	registerReadSeekTool: vi.fn(),
}));

vi.mock("../src/tool-prompt-metadata.js", () => ({
	defineToolPromptMetadata: () => ({
		description: "def",
		promptGuidelines: [],
		promptSnippet: "def",
	}),
}));

const { executeDef } = await import("../src/def.js");

describe("executeDef", () => {
	it("requires a symbol name", async () => {
		const result = await executeDef({
			params: { path: "." },
			signal: undefined,
			cwd: process.cwd(),
		});

		expect(result.isError).toBe(true);
		expect(result.details.readSeekValue.error).toEqual({
			code: "invalid-parameter",
			message: "readSeek_def requires 'name'",
		});
		expect(readSeekDefMock).not.toHaveBeenCalled();
	});
});
