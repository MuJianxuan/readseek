import { readFileSync } from "node:fs";

import { DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, formatSize } from "@earendil-works/pi-coding-agent";

const COMPACT_DESCRIPTIONS: Record<string, string> = {
  "read.md": "Read text files/images by path; text has LINE:HASH anchors, images return the attachment plus OCR text, a caption, and detected objects.",
  "edit.md": "Edit existing text files using fresh LINE:HASH anchors from readSeek_read, readSeek_grep, readSeek_search, or readSeek_write.",
  "grep.md": "Search file contents; non-summary results include LINE:HASH anchors for edits.",

  "write.md": "Create or overwrite a complete file and return anchors.",
  "sg.md": "Search code by AST pattern and return anchored matches.",
  "refs.md": "Find references to an identifier and return anchored usages with enclosing symbols.",
};

const REPLACEABLE_TOOL_GUIDELINES: Record<string, { readSeekName: string; builtInName: string; benefit: string }> = {
  "read.md": {
    readSeekName: "readSeek_read",
    builtInName: "read",
    benefit: "it provides LINE:HASH anchors for safe edits.",
  },
  "edit.md": {
    readSeekName: "readSeek_edit",
    builtInName: "edit",
    benefit: "it verifies fresh LINE:HASH anchors.",
  },
  "grep.md": {
    readSeekName: "readSeek_grep",
    builtInName: "grep",
    benefit: "it returns edit-ready anchors.",
  },
  "write.md": {
    readSeekName: "readSeek_write",
    builtInName: "write",
    benefit: "it returns LINE:HASH anchors.",
  },
};

const COMPACT_GUIDELINES: Record<string, string[]> = {
  "read.md": [
    "Use readSeek_read map or symbol mode before pulling large code files into context.",
    "Use readSeek_read for images; it returns the image attachment plus OCR text, a caption, and detected objects.",
  ],
  "edit.md": [
    "With readSeek_edit, prefer set_line, replace_lines, and insert_after; use replace only when anchors are impractical.",
  ],
  "grep.md": [
    "Use readSeek_grep summary mode for broad count/file discovery before narrowing.",
  ],
  "write.md": [
    "Use anchored edits rather than readSeek_write for small changes or appends to existing files.",
  ],
  "sg.md": [
    "Use readSeek_search for AST-shaped code patterns.",
    "Use readSeek_grep instead of readSeek_search for plain text.",
  ],
  "refs.md": [
    "Use readSeek_refs to find every usage of an identifier before renaming or deleting it.",
    "Use readSeek_refs with scope plus line/column to follow a specific binding instead of every same-named identifier.",
  ],
};

interface ToolPromptMetadata {
  description: string;
  promptSnippet: string;
  promptGuidelines: string[];
}

function loadPrompt(promptUrl: URL): string {
  return readFileSync(promptUrl, "utf-8")
    .replaceAll("{{DEFAULT_MAX_LINES}}", String(DEFAULT_MAX_LINES))
    .replaceAll("{{DEFAULT_MAX_BYTES}}", formatSize(DEFAULT_MAX_BYTES))
    .trim();
}

function firstPromptParagraph(prompt: string): string {
  return prompt.split(/\n\s*\n/, 1)[0]?.trim() ?? prompt;
}

function promptFileName(promptUrl: URL): string {
  return promptUrl.pathname.split("/").pop() ?? "";
}

function rewriteToolAliases(value: string, toolAliases: Readonly<Record<string, string>> | undefined): string {
  if (!toolAliases) return value;
  return Object.entries(toolAliases).reduce(
    (rewritten, [canonicalName, registeredName]) => rewritten.replaceAll(canonicalName, registeredName),
    value,
  );
}

export function defineToolPromptMetadata(options: {
  promptUrl: URL;
  promptSnippet: string;
  registeredName?: string;
  toolAliases?: Readonly<Record<string, string>>;
}): ToolPromptMetadata {
  const prompt = loadPrompt(options.promptUrl);
  const fileName = promptFileName(options.promptUrl);
  const compactDescription = COMPACT_DESCRIPTIONS[fileName];
  const replaceable = REPLACEABLE_TOOL_GUIDELINES[fileName];
  const registeredName = options.registeredName ?? replaceable?.readSeekName;
  const preferenceGuideline = replaceable && registeredName
    ? registeredName === replaceable.readSeekName
      ? `Prefer ${registeredName} over ${replaceable.builtInName} when both are available; ${replaceable.benefit}`
      : `Use ${registeredName}; ${replaceable.benefit}`
    : undefined;
  return {
    description: rewriteToolAliases(compactDescription ?? firstPromptParagraph(prompt), options.toolAliases),
    promptSnippet: rewriteToolAliases(options.promptSnippet, options.toolAliases),
    promptGuidelines: [
      ...(preferenceGuideline ? [preferenceGuideline] : []),
      ...(COMPACT_GUIDELINES[fileName] ?? []).map((guideline) =>
        rewriteToolAliases(
          registeredName && replaceable ? guideline.replaceAll(replaceable.readSeekName, registeredName) : guideline,
          options.toolAliases,
        ),
      ),
    ],
  };
}
