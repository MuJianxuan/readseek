Use `readSeek_rename` to rename an identifier (variable, function, type, etc.) with
binding accuracy. The tool calls `readseek rename` which resolves the
lexical binding under the cursor and renames every occurrence that binds
to the same declaration.

## Parameters

- `path` (required): File holding the binding to rename (a single regular file).
- `line` (required): One-based cursor line of the binding to rename.
- `column` (optional): One-based cursor byte column of the binding.
- `to` (required): New name for the binding. Must be a plain identifier.
- `workspace` (optional): When true, expands the rename across the project
  root. The cursor file remains binding-accurate; other files are matched
  by name (free uses only, local shadows excluded).
- `apply` (optional, default true): When true, writes the edits to disk
  after verifying line hashes. When false, returns the plan only.

## When to use

- The user asks to rename a symbol (function, variable, class, etc.).
- The cursor position (line, optionally column) is known from an identify
  call or from the user's editor context.

## When not to use

- For search-and-replace across files, use `readSeek_grep` or `readSeek_search`.
- For refactoring that requires adding/removing parameters, use `readSeek_edit`.
