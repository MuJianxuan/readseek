Identify the identifier and enclosing symbol at a cursor. Unsaved editor content
is included.

## Parameters

- `path`, `line` — required file path and one-based cursor line.
- `column` — optional one-based cursor byte column.

## When to use

- Before rename or go-to-definition, or to identify a line's enclosing symbol.
