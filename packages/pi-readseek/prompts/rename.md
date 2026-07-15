Rename the symbol at a cursor without changing same-named bindings. Use for symbol renames; use search and edit for broader refactors.

## Parameters

- `path`, `line`, `to` — required file, one-based cursor line, and plain new name.
- `column` — optional one-based cursor byte column.
- `workspace` — expand across the project; local shadows in other files are excluded.
- `apply` — default `true`; set `false` to return only the verified plan.
