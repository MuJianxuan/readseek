Find identifier references before renaming, deleting, or changing a symbol when
text search is too broad. Results include edit-ready anchors and enclosing symbols.

## Parameters

- `name` — identifier to find references for.
- `path` — file or directory, default cwd.
- `lang` — language hint for ambiguous, extensionless, or generated code.
- `scope` — restrict results to the binding under `line`/`column` in a single file. Requires `line`.
- `line` — one-based cursor line, used with `scope`.
- `column` — one-based cursor byte column, used with `scope`.
- `cached` — in a Git repository, search tracked/indexed files.
- `others` — in a Git repository, search untracked files.
- `ignored` — with `others`, include ignored untracked files.

## Scope

Without `scope`, references match by name. With `scope` and `line` (optionally
`column`), results are limited to the cursor binding in one file and exclude
shadows.

## Git selection

In Git repositories, directory search includes tracked/indexed and untracked
non-ignored files. `ignored` requires `others`.

Use `readSeek_grep` for plain text, `readSeek_search` for code shape, and `readSeek_refs` for identifier usage.
