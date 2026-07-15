import { describe, expect, it, vi } from "vitest";

const { readSeekIdentifyMock, readSeekRenameMock } = vi.hoisted(() => ({
  readSeekIdentifyMock: vi.fn(),
  readSeekRenameMock: vi.fn(),
}));

vi.mock("../src/readseek-client.js", () => ({
  classifyReadSeekFailure: vi.fn(),
  readSeekIdentify: readSeekIdentifyMock,
  readSeekRename: readSeekRenameMock,
}));

vi.mock("../src/register-tool.js", () => ({
  filePathParam: vi.fn(),
  registerReadSeekTool: vi.fn(),
}));

vi.mock("../src/tool-prompt-metadata.js", () => ({
  defineToolPromptMetadata: () => ({ description: "tool", promptGuidelines: [], promptSnippet: "tool" }),
}));

const { executeHover } = await import("../src/hover.js");
const { executeRename } = await import("../src/rename.js");

describe("cursor column validation", () => {
  it.each([0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1])("rejects hover column %s", async (column) => {
    const result = await executeHover({
      params: { path: "target.ts", line: 1, column },
      signal: undefined,
      cwd: process.cwd(),
    });

    expect(result.details.readSeekValue.error.code).toBe("invalid-parameter");
    expect(readSeekIdentifyMock).not.toHaveBeenCalled();
  });

  it.each([0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1])("rejects rename column %s", async (column) => {
    const result = await executeRename({
      params: { path: "target.ts", line: 1, column, to: "renamed" },
      signal: undefined,
      cwd: process.cwd(),
    });

    expect(result.details.readSeekValue.error.code).toBe("invalid-parameter");
    expect(readSeekRenameMock).not.toHaveBeenCalled();
  });
});

describe("rename anchor invalidation", () => {
  it("invalidates every file changed by an applied rename", async () => {
    readSeekRenameMock.mockResolvedValueOnce({
      file: "/workspace/one.ts",
      old_name: "value",
      new_name: "renamed",
      applied: true,
      edits: [{ line: 1 }],
      conflicts: [],
      others: [{ file: "two.ts", edits: [{ line: 2 }], conflicts: [] }],
    });
    const onFileMutated = vi.fn();

    const result = await executeRename({
      params: { path: "one.ts", line: 1, to: "renamed" },
      signal: undefined,
      cwd: "/workspace",
      onFileMutated,
    });

    expect(result.isError).not.toBe(true);
    expect(onFileMutated.mock.calls).toEqual([["/workspace/one.ts"], ["/workspace/two.ts"]]);
  });
});
