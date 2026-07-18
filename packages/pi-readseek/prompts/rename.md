Rename the symbol at a cursor without changing resolvable same-named bindings. Use for symbol renames; use search and edit for broader refactors.

## Parameters

- `path`, `line`, `to` — required file, one-based cursor line, and plain new name.
- `column` — optional one-based cursor byte column.
- `workspace` — expand across the project; local shadows are excluded where binding
  support is available, and other files are otherwise matched by name.
- `apply` — default `true`; set `false` to return only the verified plan.
