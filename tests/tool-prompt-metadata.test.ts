import { describe, expect, it } from "vitest";

import { defineToolPromptMetadata } from "../src/tool-prompt-metadata.js";

describe("defineToolPromptMetadata", () => {
  it.each([
    ["read.md", "readSeek_read", "read", "it provides LINE:HASH anchors for readSeek_edit."],
    ["edit.md", "readSeek_edit", "edit", "it verifies fresh LINE:HASH anchors."],
    ["grep.md", "readSeek_grep", "grep", "it returns edit-ready anchors."],
    ["write.md", "readSeek_write", "write", "it returns LINE:HASH anchors."],
  ])("prefers %s to %s when both are active", (fileName, readSeekTool, builtInTool, benefit) => {
    const metadata = defineToolPromptMetadata({
      promptUrl: new URL(`../prompts/${fileName}`, import.meta.url),
      promptSnippet: "test",
    });

    expect(metadata.promptGuidelines[0]).toBe(`Prefer ${readSeekTool} over ${builtInTool} when both are available; ${benefit}`);
  });
});
