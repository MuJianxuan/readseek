import type { ReadseekLine, ReadseekRange } from "./readseek-value.js";

export interface SgOutputFile {
  displayPath: string;
  path: string;
  ranges: ReadseekRange[];
  lines: ReadseekLine[];
  symbols?: Array<{ name: string; kind?: string }>;
}

export interface BuildSgOutputInput {
  pattern: string;
  files: SgOutputFile[];
}

export interface SgOutputResult {
  text: string;
  readseekValue: {
    tool: "search";
    files: Array<{
      path: string;
      ranges: ReadseekRange[];
      lines: ReadseekLine[];
    }>;
  };
}

export function buildSgOutput(input: BuildSgOutputInput): SgOutputResult {
  if (input.files.length === 0) {
    return {
      text: `No matches found for pattern: ${input.pattern}`,
      readseekValue: {
        tool: "search",
        files: [],
      },
    };
  }

  const blocks: string[] = [];
  for (const file of input.files) {
    blocks.push(`--- ${file.displayPath} ---`);
    for (const line of file.lines) {
      blocks.push(`>>${line.anchor}|${line.display}`);
    }
  }

  return {
    text: blocks.join("\n"),
    readseekValue: {
      tool: "search",
      files: input.files.map((file) => ({
        path: file.path,
        ranges: file.ranges.map((range) => ({ ...range })),
        lines: file.lines.map((line) => ({ ...line })),
      })),
    },
  };
}
