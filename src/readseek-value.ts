import { computeLineHash, escapeControlCharsForDisplay } from "./hashline.js";
import type { DiffData } from "./diff-data.js";

export interface ReadSeekLine {
  line: number;
  hash: string;
  anchor: string;
  raw: string;
  display: string;
}

export interface ReadSeekWarningSymbol {
  name: string;
  kind: string;
  startLine: number;
  endLine: number;
  parentName?: string;
}
export interface ReadSeekWarning {
  code: string;
  message: string;
  tier?: "camelCase" | "substring";
  symbol?: ReadSeekWarningSymbol;
  otherCandidates?: ReadSeekWarningSymbol[];
}

export interface ReadSeekError {
  code: string;
  message: string;
  hint?: string;
  details?: unknown;
}

export interface ReadSeekRange {
  startLine: number;
  endLine: number;
  totalLines?: number;
}

export interface SemanticSummary {
  classification: "no-op" | "whitespace-only" | "semantic" | "mixed";
}
export interface ReadSeekEditResult {
  tool: "edit";
  ok: boolean;
  path: string;
  summary: string;
  diff: string;
  diffData?: DiffData;
  firstChangedLine: number | undefined;
  warnings: string[];
  noopEdits: unknown[];
  semanticSummary?: SemanticSummary;
}

/**
 * Build a {@link ReadSeekLine} from an already-known hash. Use this when the
 * hash is supplied by readseek (search, refs) rather than computed from `raw`;
 * {@link buildReadSeekLine} delegates here after hashing.
 */
export function buildReadSeekLineWithHash(line: number, hash: string, raw: string): ReadSeekLine {
  return {
    line,
    hash,
    anchor: `${line}:${hash}`,
    raw,
    display: escapeControlCharsForDisplay(raw),
  };
}

export function buildReadSeekLine(line: number, raw: string): ReadSeekLine {
  return buildReadSeekLineWithHash(line, computeLineHash(raw), raw);
}

export function buildReadSeekLines(startLine: number, rawLines: string[]): ReadSeekLine[] {
  return rawLines.map((raw, index) => buildReadSeekLine(startLine + index, raw));
}

function renderReadSeekLine(line: ReadSeekLine): string {
  return `${line.anchor}|${line.display}`;
}

export function renderReadSeekLines(lines: ReadSeekLine[]): string {
  return lines.map(renderReadSeekLine).join("\n");
}

/**
 * Render the anchored-file block format shared by the `search` and `refs`
 * tools: a `--- <displayPath> ---` header per file followed by one
 * `>><anchor>|<display>` line per match. {@link lineSuffix} appends tool
 * specific trailers (e.g. refs' enclosing-symbol annotation).
 */
export function formatAnchoredFileBlocks<T extends ReadSeekLine>(
  files: Array<{ displayPath: string; lines: T[] }>,
  lineSuffix?: (line: T) => string,
): string {
  const blocks: string[] = [];
  for (const file of files) {
    blocks.push(`--- ${file.displayPath} ---`);
    for (const line of file.lines) {
      blocks.push(`>>${line.anchor}|${line.display}${lineSuffix?.(line) ?? ""}`);
    }
  }
  return blocks.join("\n");
}

export function buildReadSeekWarning(
  code: string,
  message: string,
  metadata: Omit<ReadSeekWarning, "code" | "message"> = {},
): ReadSeekWarning {
  return { code, message, ...metadata };
}

export function buildReadSeekError(
  code: string,
  message: string,
  hint?: string,
  details?: unknown,
): ReadSeekError {
  return {
    code,
    message,
    ...(hint !== undefined ? { hint } : {}),
    ...(details !== undefined ? { details } : {}),
  };
}

export interface ToolErrorResult {
  content: [{ type: "text"; text: string }];
  isError: true;
  details: { readSeekValue: Record<string, unknown> };
}

/**
 * Build the standard failure envelope shared by every read-family tool: a text
 * content block plus a `readSeekValue` carrying `ok: false` and a
 * {@link buildReadSeekError} payload.
 *
 * `path` is included only when provided, and `extra` is merged into
 * `readSeekValue` so callers can attach tool-specific fields (e.g. write's
 * `lines`/`warnings`).
 */
export function buildToolErrorResult(
  tool: string,
  code: string,
  message: string,
  opts: { path?: string; hint?: string; details?: unknown; extra?: Record<string, unknown> } = {},
): ToolErrorResult {
  return {
    content: [{ type: "text", text: message }],
    isError: true,
    details: {
      readSeekValue: {
        tool,
        ...(opts.extra ?? {}),
        ok: false,
        ...(opts.path !== undefined ? { path: opts.path } : {}),
        error: buildReadSeekError(code, message, opts.hint, opts.details),
      },
    },
  };
}

export function buildReadSeekEditResult(input: {
  ok?: boolean;
  path: string;
  summary: string;
  diff: string;
  diffData?: DiffData;
  firstChangedLine: number | undefined;
  warnings: string[];
  noopEdits: unknown[];
  semanticSummary?: SemanticSummary;
}): ReadSeekEditResult {
  return {
    tool: "edit",
    ok: input.ok ?? true,
    path: input.path,
    summary: input.summary,
    diff: input.diff,
    ...(input.diffData ? { diffData: input.diffData } : {}),
    firstChangedLine: input.firstChangedLine,
    warnings: [...input.warnings],
    noopEdits: [...input.noopEdits],
    ...(input.semanticSummary ? { semanticSummary: input.semanticSummary } : {}),
  };
}
