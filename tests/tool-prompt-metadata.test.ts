import { describe, expect, it } from "vitest";

import { defineToolPromptMetadata } from "../src/tool-prompt-metadata.js";

describe("defineToolPromptMetadata", () => {
  const replaceableTools = [
    ["read.md", "readSeek_read", "read", "it provides LINE:HASH anchors for safe edits."],
    ["edit.md", "readSeek_edit", "edit", "it verifies fresh LINE:HASH anchors."],
    ["grep.md", "readSeek_grep", "grep", "it returns edit-ready anchors."],
    ["write.md", "readSeek_write", "write", "it returns LINE:HASH anchors."],
  ] as const;

  it.each(replaceableTools)("prefers %s to %s when both are active", (fileName, readSeekTool, builtInTool, benefit) => {
    const metadata = defineToolPromptMetadata({
      promptUrl: new URL(`../prompts/${fileName}`, import.meta.url),
      promptSnippet: "test",
    });

    expect(metadata.promptGuidelines[0]).toBe(`Prefer ${readSeekTool} over ${builtInTool} when both are available; ${benefit}`);
  });

  it.each(replaceableTools)("uses the registered %s alias", (fileName, readSeekTool, builtInTool, benefit) => {
    const metadata = defineToolPromptMetadata({
      promptUrl: new URL(`../prompts/${fileName}`, import.meta.url),
      promptSnippet: "test",
      registeredName: builtInTool,
    });

    expect(metadata.promptGuidelines).toEqual(expect.arrayContaining([`Use ${builtInTool}; ${benefit}`]));
    expect(metadata.promptGuidelines.join("\n")).not.toContain(readSeekTool);
  });
});
