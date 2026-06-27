import { formatAnchoredFileBlocks, type ReadSeekLine, type ReadSeekRange } from "./readseek-value.js";

export interface SgOutputFile {
  displayPath: string;
  path: string;
  ranges: ReadSeekRange[];
  lines: ReadSeekLine[];
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
      ranges: ReadSeekRange[];
      lines: ReadSeekLine[];
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

  return {
    text: formatAnchoredFileBlocks(input.files),
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
