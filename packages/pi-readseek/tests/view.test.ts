import { beforeEach, describe, expect, it, vi } from "vitest";

const { readSeekViewMock } = vi.hoisted(() => ({
	readSeekViewMock: vi.fn(),
}));

vi.mock("@earendil-works/pi-coding-agent", async () => ({
	...(await import("./support/pi-coding-agent-mock.js")).createPiCodingAgentBaseMock(),
}));

vi.mock("../src/readseek-client.js", () => ({
	classifyReadSeekFailure: (error: unknown) => ({
		code: "view-error",
		message: error instanceof Error ? error.message : String(error),
	}),
	readSeekView: readSeekViewMock,
}));

const { executeView } = await import("../src/view.js");

describe("executeView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("forwards document selectors", async () => {
		readSeekViewMock.mockResolvedValue("- page-1 [page] Page 1\n");

		const result = await executeView({
			params: {
				path: "paper.pdf",
				node: "page-1",
				page: "2",
				kind: "heading",
				depth: "3",
				outline: true,
			},
			signal: undefined,
			cwd: "/repo",
		});

		expect(readSeekViewMock).toHaveBeenCalledWith("/repo/paper.pdf", {
			node: "page-1",
			page: 2,
			kind: "heading",
			depth: 3,
			outline: true,
			signal: undefined,
		});
		expect(result.content).toEqual([{ type: "text", text: "- page-1 [page] Page 1\n" }]);
		expect(result.details.readSeekValue.tool).toBe("view");
	});

	it.each([
		[{ path: "paper.pdf", page: 0 }, "invalid-page"],
		[{ path: "paper.pdf", depth: -1 }, "invalid-depth"],
		[{ path: "paper.pdf", node: "   " }, "invalid-node"],
	])("rejects invalid selectors", async (params, code) => {
		const result = await executeView({ params, signal: undefined, cwd: "/repo" });

		expect(result.isError).toBe(true);
		expect(result.details.readSeekValue.error.code).toBe(code);
		expect(readSeekViewMock).not.toHaveBeenCalled();
	});
});
