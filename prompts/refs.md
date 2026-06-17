Find binding-accurate references to an identifier with readseek. Use it when you need every place a name is used — before renaming, deleting, or changing a symbol — and plain text search would be too broad. Results are grouped by file with edit-ready hashline anchors and the enclosing symbol for each reference.

## Parameters

- `name` — identifier to find references for.
- `path` — file or directory, default cwd.
- `lang` — language hint; set it when syntax is ambiguous, extensionless, or generated.
- `scope` — restrict results to the binding under `line`/`column` in a single file. Requires `line`.
- `line` — one-based cursor line, used with `scope`.
- `column` — one-based cursor byte column, used with `scope`.
- `cached` — in a Git repository, search tracked/indexed files.
- `others` — in a Git repository, search untracked files.
- `ignored` — with `others`, include ignored untracked files.

## Scope

Without `scope`, references match by identifier name across the target. With `scope` plus `line` (and optionally `column`), results are limited to the specific binding under that cursor in a single file, so shadowed or unrelated same-named identifiers are excluded.

## Git selection

When searching a directory inside a Git repository, readseek defaults to tracked/indexed files plus untracked non-ignored files. Use `cached`, `others`, and `ignored` to narrow or expand that selection. `ignored` requires `others`.

Use `grep` for plain text, `search` for code shape, and `refs` for identifier usage.
