Rename an identifier with binding accuracy. The cursor binding is resolved and
only occurrences of that declaration are changed.

## Parameters

- `path`, `line`, `to` — required file, one-based cursor line, and plain new name.
- `column` — optional one-based cursor byte column.
- `workspace` — expand across the project; local shadows in other files are excluded.
- `apply` — default `true`; set `false` to return only the verified plan.

## When to use

- Use when the user requests a symbol rename and the cursor is known.

## When not to use

- Use `readSeek_grep`/`readSeek_search` for broad replacement and
  `readSeek_edit` for other refactors.
