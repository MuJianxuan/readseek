List one directory. Output is directories first (with `/`), then files, sorted alphabetically; dotfiles are included.

## Parameters

- `path` — directory to list, default cwd.
- `limit` — max entries, default 500; must be positive.
- `glob` — optional entry-name filter such as `*.ts`, `.env*`, or `test-*`.

## Usage

Use `ls` to inspect one known directory. Use `find` for recursive discovery, `grep` or `search` for contents, and `read` for file content. If output exceeds `limit` or 50 KB, narrow with `glob` or switch to `find`.
