export type DiffEntry =
  | { kind: "context"; oldLine: number; newLine: number; text: string }
  | { kind: "add"; newLine: number; text: string }
  | { kind: "remove"; oldLine: number; text: string }
  | { kind: "meta"; text: string };

export type DiffSpan =
  | { kind: "equal"; text: string }
  | { kind: "add"; text: string }
  | { kind: "remove"; text: string };

export type InlineDiff = {
  removeLineIndex: number;
  addLineIndex: number;
  removeSpans: DiffSpan[];
  addSpans: DiffSpan[];
};

export type DiffBlockRange = {
  kind: "add" | "remove";
  startLine: number;
  endLine: number;
};

export type DiffData = {
  version: 1;
  entries: DiffEntry[];
  stats: { added: number; removed: number; context: number };
  language?: string;
  blockRanges?: DiffBlockRange[];
  inlineDiffs?: InlineDiff[];
};

export type BuildDiffDataInput = {
  path: string;
  oldContent: string;
  newContent: string;
  diff: string;
  blockRanges?: DiffBlockRange[];
};

export const MAX_INLINE_DIFF_LINE_LENGTH = 4096;
export const MAX_INLINE_DIFF_TOKENS = 512;
export const MAX_INLINE_DIFF_CELLS = 200_000;
export const MAX_INLINE_DIFF_PAIRS = 200;

const INLINE_SIMILARITY_THRESHOLD = 0.35;
const INLINE_TOKEN_PATTERN = /([A-Za-z_$][\w$]*|\d+|\s+|[^A-Za-z_$\w\s]+)/gu;

const LANGUAGE_BY_EXTENSION = new Map<string, string>([
  [".ts", "typescript"],
  [".tsx", "typescript"],
  [".js", "javascript"],
  [".jsx", "javascript"],
  [".py", "python"],
  [".rs", "rust"],
  [".java", "java"],
]);

function inferLanguage(path: string): string | undefined {
  const extensionMatch = path.match(/\.[^.\/\\]+$/);
  if (!extensionMatch) return undefined;
  return LANGUAGE_BY_EXTENSION.get(extensionMatch[0].toLowerCase());
}

function parseFullDiffEntries(diff: string): DiffEntry[] {
  const entries: DiffEntry[] = [];
  let nextOldLine = 1;
  let nextNewLine = 1;

  for (const line of diff ? diff.split("\n") : []) {
    const removeMatch = line.match(/^-\s*(\d+) (.*)$/);
    if (removeMatch) {
      const oldLine = Number(removeMatch[1]);
      entries.push({ kind: "remove", oldLine, text: removeMatch[2] ?? "" });
      nextOldLine = oldLine + 1;
      continue;
    }

    const addMatch = line.match(/^\+\s*(\d+) (.*)$/);
    if (addMatch) {
      const newLine = Number(addMatch[1]);
      entries.push({ kind: "add", newLine, text: addMatch[2] ?? "" });
      nextNewLine = newLine + 1;
      continue;
    }

    const contextMatch = line.match(/^ \s*(\d+) (.*)$/);
    if (contextMatch) {
      const oldLine = Number(contextMatch[1]);
      const lineDelta = nextNewLine - nextOldLine;
      const newLine = oldLine + lineDelta;
      entries.push({ kind: "context", oldLine, newLine, text: contextMatch[2] ?? "" });
      nextOldLine = oldLine + 1;
      nextNewLine = newLine + 1;
      continue;
    }

    entries.push({ kind: "meta", text: line });
  }

  return entries;
}

function parseCompactDiffEntries(diff: string, oldContent: string, newContent: string): DiffEntry[] | undefined {
  const oldLines = oldContent.split("\n");
  const newLines = newContent.split("\n");

  const compactDelete = diff.match(/^(\d+):[0-9a-f]{3}\|(.*) → \[deleted\]$/);
  if (compactDelete) {
    const oldLine = Number(compactDelete[1]);
    const text = compactDelete[2] ?? "";
    if (oldLines[oldLine - 1] === text) return [{ kind: "remove", oldLine, text }];
  }

  const compactPrefix = diff.match(/^(\d+):[0-9a-f]{3}\|/);
  if (!compactPrefix) return undefined;

  const oldLine = Number(compactPrefix[1]);
  const oldTextStart = compactPrefix[0].length;
  const separatorPattern = / → (\d+):[0-9a-f]{3}\|/g;
  let separator: RegExpExecArray | null;
  while ((separator = separatorPattern.exec(diff)) !== null) {
    const newLine = Number(separator[1]);
    const oldText = diff.slice(oldTextStart, separator.index);
    const newText = diff.slice(separator.index + separator[0].length);
    if (oldLines[oldLine - 1] === oldText && newLines[newLine - 1] === newText) {
      return [
        { kind: "remove", oldLine, text: oldText },
        { kind: "add", newLine, text: newText },
      ];
    }
  }

  return undefined;
}

function buildStats(entries: DiffEntry[]): DiffData["stats"] {
  return entries.reduce(
    (stats, entry) => {
      if (entry.kind === "add") stats.added++;
      else if (entry.kind === "remove") stats.removed++;
      else if (entry.kind === "context") stats.context++;
      return stats;
    },
    { added: 0, removed: 0, context: 0 },
  );
}

function tokenizeInlineDiff(text: string): string[] {
  return text.match(INLINE_TOKEN_PATTERN) ?? (text ? [text] : []);
}

function longestCommonSubsequence(a: string[], b: string[]): Array<[number, number]> {
  const rows = a.length + 1;
  const cols = b.length + 1;
  const table: number[][] = Array.from({ length: rows }, () => Array<number>(cols).fill(0));

  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i]![j] = a[i] === b[j] ? table[i + 1]![j + 1]! + 1 : Math.max(table[i + 1]![j]!, table[i]![j + 1]!);
    }
  }

  const pairs: Array<[number, number]> = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      pairs.push([i, j]);
      i++;
      j++;
    } else if (table[i + 1]![j]! >= table[i]![j + 1]!) {
      i++;
    } else {
      j++;
    }
  }

  return pairs;
}

function pushMergedSpan(spans: DiffSpan[], span: DiffSpan): void {
  if (!span.text) return;
  const previous = spans[spans.length - 1];
  if (previous?.kind === span.kind) {
    previous.text += span.text;
    return;
  }
  spans.push({ ...span });
}

function buildInlineSpans(removeText: string, addText: string): { removeSpans: DiffSpan[]; addSpans: DiffSpan[] } | undefined {
  if (removeText.length > MAX_INLINE_DIFF_LINE_LENGTH || addText.length > MAX_INLINE_DIFF_LINE_LENGTH) return undefined;

  const removeTokens = tokenizeInlineDiff(removeText);
  const addTokens = tokenizeInlineDiff(addText);
  if (!removeTokens.length || !addTokens.length) return undefined;
  if (removeTokens.length > MAX_INLINE_DIFF_TOKENS || addTokens.length > MAX_INLINE_DIFF_TOKENS) return undefined;
  if ((removeTokens.length + 1) * (addTokens.length + 1) > MAX_INLINE_DIFF_CELLS) return undefined;

  const pairs = longestCommonSubsequence(removeTokens, addTokens);
  const meaningfulRemoveTokenCount = removeTokens.filter((token) => token.trim().length > 0).length;
  const meaningfulAddTokenCount = addTokens.filter((token) => token.trim().length > 0).length;
  const meaningfulEqualTokenCount = pairs.filter(([removeIndex, addIndex]) => {
    const removeToken = removeTokens[removeIndex] ?? "";
    const addToken = addTokens[addIndex] ?? "";
    return removeToken === addToken && removeToken.trim().length > 0 && addToken.trim().length > 0;
  }).length;
  const similarity = meaningfulEqualTokenCount / Math.max(meaningfulRemoveTokenCount, meaningfulAddTokenCount);
  if (similarity < INLINE_SIMILARITY_THRESHOLD) return undefined;

  const removeSpans: DiffSpan[] = [];
  const addSpans: DiffSpan[] = [];
  let removeCursor = 0;
  let addCursor = 0;

  for (const [removeIndex, addIndex] of pairs) {
    if (removeCursor < removeIndex) {
      pushMergedSpan(removeSpans, { kind: "remove", text: removeTokens.slice(removeCursor, removeIndex).join("") });
    }
    if (addCursor < addIndex) {
      pushMergedSpan(addSpans, { kind: "add", text: addTokens.slice(addCursor, addIndex).join("") });
    }
    pushMergedSpan(removeSpans, { kind: "equal", text: removeTokens[removeIndex]! });
    pushMergedSpan(addSpans, { kind: "equal", text: addTokens[addIndex]! });
    removeCursor = removeIndex + 1;
    addCursor = addIndex + 1;
  }

  if (removeCursor < removeTokens.length) {
    pushMergedSpan(removeSpans, { kind: "remove", text: removeTokens.slice(removeCursor).join("") });
  }
  if (addCursor < addTokens.length) {
    pushMergedSpan(addSpans, { kind: "add", text: addTokens.slice(addCursor).join("") });
  }

  if (!removeSpans.some((span) => span.kind === "remove") || !addSpans.some((span) => span.kind === "add")) return undefined;
  return { removeSpans, addSpans };
}

function buildInlineDiffs(entries: DiffEntry[]): InlineDiff[] | undefined {
  const inlineDiffs: InlineDiff[] = [];
  let remainingPairs = MAX_INLINE_DIFF_PAIRS;

  for (let index = 0; index < entries.length;) {
    if (entries[index]?.kind !== "remove") {
      index++;
      continue;
    }

    const removeStart = index;
    while (entries[index]?.kind === "remove") index++;
    const addStart = index;
    while (entries[index]?.kind === "add") index++;

    const removeCount = addStart - removeStart;
    const addCount = index - addStart;
    if (removeCount === 0 || addCount === 0 || removeCount !== addCount) continue;

    for (let offset = 0; offset < removeCount; offset++) {
      if (remainingPairs <= 0) break;
      remainingPairs--;

      const removeIndex = removeStart + offset;
      const addIndex = addStart + offset;
      const removeEntry = entries[removeIndex];
      const addEntry = entries[addIndex];
      if (removeEntry?.kind !== "remove" || addEntry?.kind !== "add") continue;

      const spans = buildInlineSpans(removeEntry.text, addEntry.text);
      if (!spans) continue;

      inlineDiffs.push({
        removeLineIndex: removeIndex,
        addLineIndex: addIndex,
        removeSpans: spans.removeSpans,
        addSpans: spans.addSpans,
      });
    }
  }

  return inlineDiffs.length ? inlineDiffs : undefined;
}

function finalizeDiffData(path: string, entries: DiffEntry[], blockRanges: DiffBlockRange[] | undefined): DiffData {
  const language = inferLanguage(path);
  const inlineDiffs = buildInlineDiffs(entries);
  return {
    version: 1,
    entries,
    stats: buildStats(entries),
    ...(language ? { language } : {}),
    ...(blockRanges?.length ? { blockRanges: [...blockRanges] } : {}),
    ...(inlineDiffs ? { inlineDiffs } : {}),
  };
}

export function buildDiffData(input: BuildDiffDataInput): DiffData {
  const entries = parseCompactDiffEntries(input.diff, input.oldContent, input.newContent) ?? parseFullDiffEntries(input.diff);
  return finalizeDiffData(input.path, entries, input.blockRanges);
}
