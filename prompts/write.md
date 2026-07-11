Create or overwrite a whole file and return `LINE:HASH` anchors for immediate follow-up `readSeek_edit` calls.

## Use / avoid

Use `readSeek_write` for new files, generated files, or intentional full-file replacement. For small changes or appends to an existing file, run `readSeek_read` or `readSeek_search` first and use `readSeek_edit` (`insert_after` for appends).

Existing files are overwritten without confirmation. Binary-looking content can be written, but hashlines are not generated, so there are no anchors for `readSeek_edit`.

## Parameters

- `path` — relative or absolute file path.
- `content` — complete file contents.
- `map` — optional; append a structural map when possible. Map generation is best-effort and does not make the write fail.

## Output

Successful text writes return `LINE:HASH|content`. Display hashlines escape control characters for safe rendering. Visible output is capped at 2000 lines or 50 KB, but full anchors remain available in `readSeekValue`.

## Diff data contract

Successful text `readSeek_write` results include additive final `details.diff`, `details.readSeekValue.diff`, `details.diffData`, and `details.readSeekValue.diffData` fields. String diff fields remain the backward-compatible human-readable fallback.

`diffData` is a stable versioned contract:

```ts
type DiffData = {
  version: 1;
  entries: Array<
    | { kind: "context"; oldLine: number; newLine: number; text: string }
    | { kind: "add"; newLine: number; text: string }
    | { kind: "remove"; oldLine: number; text: string }
    | { kind: "meta"; text: string }
  >;
  stats: { added: number; removed: number; context: number };
  language?: string;
  blockRanges?: Array<{ kind: "add" | "remove"; startLine: number; endLine: number }>;
  inlineDiffs?: Array<{
    removeLineIndex: number;
    addLineIndex: number;
    removeSpans: Array<{ kind: "equal" | "remove" | "add"; text: string }>;
    addSpans: Array<{ kind: "equal" | "remove" | "add"; text: string }>;
  }>;
};
```

For compact one-line hashline diffs, `details.diff` remains compact while `diffData.entries` uses expanded remove/add rows so renderers can show inline word changes without breaking hashline output.
