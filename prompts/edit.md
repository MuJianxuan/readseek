Surgically edit existing text files. Prefer hash-verified anchors from fresh `read`, `grep`, `search`, or `write` output; copy `LINE:HASH` anchors exactly.

`edit` requires the target file to have been anchored earlier in the current session. If you get `file-not-read`, run `read`, `grep`, `search`, or `write` first.

## Variants

| Variant | Use | Anchors |
|---|---|---|
| `set_line` | Replace or delete one line | 1 |
| `replace_lines` | Replace or delete one contiguous range | 2 |
| `insert_after` | Insert after an existing line | 1 |
| `replace_symbol` | Replace one function/class/method/interface/type/enum/etc. | 0 (`symbol`) |
| `replace` | Exact string replacement escape hatch; one match by default, all with `all: true` | 0 |

Set `new_text` (or `replace_lines.new_text`) to `""` to delete anchored line(s). For an intentionally blank line, use `"\n"` or whitespace content, not `""`.

Prefer `set_line`, `replace_lines`, and `insert_after`: they verify that the file still matches the anchored content. Use `replace` only when anchors are impractical, such as repeated text across many unrelated lines.

## Input shape

```json
{
  "path": "src/foo.ts",
  "edits": [
    { "set_line": { "anchor": "42:ab1", "new_text": "const x = 2;" } },
    { "replace_lines": { "start_anchor": "50:c3d", "end_anchor": "55:e4f", "new_text": "const y = 3;\nreturn y;" } },
    { "insert_after": { "anchor": "60:f5a", "new_text": "// TODO\n" } },
    { "replace_symbol": { "symbol": "add", "new_body": "export function add(a, b) {\n  return a + b;\n}" } },
    { "replace": { "old_text": "value", "new_text": "result", "all": true } }
  ]
}
```

Use only the needed variant(s); the example shows all shapes for reference. Each `edits[]` entry must contain exactly one variant key. `new_text` / `new_body` is plain file content — no hash prefixes or diff markers.

## Exact and fuzzy replacement

`replace` is exact-only by default. Missing `old_text` fails with `text-not-found`.

Wrap string replacements as `{ "replace": { "old_text": "...", "new_text": "..." } }`; a bare top-level `{ old_text, new_text }` inside `edits[]` is rejected with guidance.

`fuzzy: true` is a narrow fallback after exact matching fails. It normalizes whitespace and confusable Unicode such as smart hyphens; it is **not** approximate, Levenshtein, or semantic matching. Fuzzy successes return a warning.

## `replace_symbol`

Use `replace_symbol` when you want to replace one whole mapped symbol without line anchors. Query symbols like `read({ symbol })`: `Name`, `Class.method`, or `Name@<line>`.

Rules:

- Use an exact name, dotted path, or `@<line>`. If `read({ symbol })` returned a fuzzy match, confirm the exact symbol first.
- Supported for TypeScript, JavaScript, Rust, and Java. For other languages, use anchored edits.
- `new_body` must not be empty or whitespace-only.
- Write `new_body` without extra leading indentation; `edit` re-indents it to match the original symbol.
- If `new_body` appears to declare a different symbol name, the edit still applies but returns a `name-mismatch` warning.
- Do not combine `replace_symbol` with anchored edits that touch the same lines. Duplicate or overlapping `replace_symbol` ranges are rejected.

## Stale anchors

If anchors no longer match, `edit` fails with `hash-mismatch` and shows nearby current lines. Lines marked `>>>` include updated anchors:

```text
>>> 41:b34|  const renamed = 3;
```

Copy the updated `LINE:HASH` and retry. If the target moved farther away, re-run `read`, `grep`, `search`, or `write` for fresh anchors.

If `edit` auto-relocates an anchor, read the warning and verify that the edit landed in the intended place.

## Validation and warnings

- All edits are checked before writing; if a hard validation fails, nothing is written.
- Anchored edits are applied bottom-up so line numbers stay stable.
- `no-op` means the requested edit matched the current file already or produced identical content.
- Whitespace-only warnings mean formatting changed but behavior probably did not.
- A `replace`-only success may remind you to prefer anchored edits next time.

Syntax validation runs before writing when supported:

- Supported: Rust, C++, C headers, Java.
- Default `warn`: write succeeds, but warnings include `syntax-regression: lines X-Y`.
- `block`: aborts without writing.
- `off`: skips validation.
- `READSEEK_SYNTAX_VALIDATE` can set the default mode.

Existing syntax errors are tolerated; warnings are for newly introduced parser errors.

## Optional post-edit verification

`postEditVerify: true` opts into read-back verification for this call. It is off by default. When enabled, `edit` writes normally, then reads the file back and compares persisted content to the intended content, including BOM and original line endings. This is not syntax validation.

## Diff data contract

Successful `edit` results include:

- `details.diff` and `details.readseekValue.diff`: compact human-readable hashline diff strings.
- `details.patch`: standard unified diff with file and hunk headers.
- `details.diffData` and `details.readseekValue.diffData`: stable structured diff data.

`diffData` shape:

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
