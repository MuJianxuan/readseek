Edit existing text files safely with fresh `LINE:HASH` anchors; on `file-not-read`, read or search the file first.

Anchors come from `readSeek_read`, `readSeek_grep`, `readSeek_search`, or `readSeek_write`.

## Variants

| Variant | Use | Anchors |
|---|---|---|
| `set_line` | Replace or delete one line | 1 |
| `replace_lines` | Replace or delete one contiguous range | 2 |
| `insert_after` | Insert after an existing line | 1 |
| `replace_symbol` | Replace one mapped symbol | 0 (`symbol`) |
| `replace` | Exact string replacement; one match unless `all: true` | 0 |

Set `new_text` to `""` to delete lines; use `"\n"` for a blank line. Prefer
anchored variants. Use `replace` only when anchors are impractical.

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

Use only needed variants. Each `edits[]` entry has exactly one key. `new_text`
and `new_body` are plain file content, not diffs or hashlines.

## Exact and fuzzy replacement

`replace` is exact by default; missing text fails. `fuzzy: true` only normalizes
whitespace and confusable Unicode after exact matching fails. Verify warned fuzzy
matches before continuing.

## `replace_symbol`

Use `replace_symbol` for one whole mapped `Name`, `Class.method`, or
`Name@<line>` in TypeScript, JavaScript, Rust, or Java. `new_body` must be
non-empty and unindented. Confirm fuzzy symbol matches first; do not overlap it
with anchored edits.

## Stale anchors

On `hash-mismatch`, nearby lines marked `>>>` include fresh anchors:

```text
>>> 41:b34|  const renamed = 3;
```

Retry with those anchors, or read/search again. Verify any auto-relocation warning.

## Validation and warnings

All edits validate before writing; hard failures write nothing. Anchored edits run
bottom-up. `no-op` means no change. Syntax validation for Rust, C++, C headers,
and Java follows `readseek.syntaxValidation`: `warn` (default), `block`, or `off`.
It reports only newly introduced parser errors.

## Optional post-edit verification

`postEditVerify: true` reads back the written file and compares the persisted
content, including BOM and line endings. Results provide a compact hashline diff,
a unified patch, and structured `details.diffData`.
