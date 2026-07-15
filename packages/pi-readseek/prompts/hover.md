Identify the token and enclosing symbol at a cursor. Use before rename or definition lookup, or to identify a line's enclosing symbol. The file is read from disk.

## Parameters

- `path`, `line` — required file path and one-based cursor line.
- `column` — optional one-based cursor byte column.
