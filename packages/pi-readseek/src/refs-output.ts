import { formatAnchoredFileBlocks, type ReadSeekLine } from "./readseek-value.js";

export interface RefsOutputLine extends ReadSeekLine {
  enclosingSymbol?: string;
}

export interface RefsOutputFile {
  displayPath: string;
  path: string;
  lines: RefsOutputLine[];
}

interface BuildRefsOutputInput {
  name: string;
  files: RefsOutputFile[];
}

interface RefsOutputResult {
  text: string;
  readSeekValue: {
    tool: "refs";
    files: Array<{
      path: string;
      lines: RefsOutputLine[];
    }>;
  };
}

export function buildRefsOutput(input: BuildRefsOutputInput): RefsOutputResult {
  if (input.files.length === 0) {
    return {
      text: `No references found for: ${input.name}`,
      readSeekValue: { tool: "refs", files: [] },
    };
  }

  return {
    text: formatAnchoredFileBlocks(input.files, (line) => (line.enclosingSymbol ? ` (in ${line.enclosingSymbol})` : "")),
    readSeekValue: {
      tool: "refs",
      files: input.files.map((file) => ({
        path: file.path,
        lines: file.lines.map((line) => ({ ...line })),
      })),
    },
  };
}
