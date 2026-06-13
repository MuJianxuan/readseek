Search file contents. Non-summary results include edit-ready `LINE:HASH` anchors, so you usually do not need a follow-up `read` before `edit`.

## Modes

- Default: matching lines only. Match rows look like `path:>>LINE:HASH|content`.
- `context: N`: include N lines before and after each match. Context rows use `path:  LINE:HASH|content`; nearby ranges are merged and deduped.
- `summary: true`: return per-file match counts only. Use this first for broad searches, then narrow with `path`, `glob`, or a stricter pattern.
- `scope: "symbol"`: group matches by enclosing symbol. By default returns full symbol blocks. `scopeContext: N` clips each match to ±N lines inside the symbol; `0` returns only match lines. Ignored with `summary: true`.

## Parameters

- `pattern` — regular expression by default; set `literal: true` for exact text or regex metacharacters.
- `path` — file or directory, default cwd.
- `glob` — file-name filter such as `*.ts` or `**/*.test.ts`.
- `ignoreCase` — case-insensitive search.
- `context` — surrounding lines for normal grep.
- `limit` — maximum matches, default 100.
- `summary` — counts only, no anchors.
- `scope` — only `"symbol"` is supported.
- `scopeContext` — non-negative context within symbol scope; requires `scope: "symbol"`.

## Use well

Use `grep` for text: identifiers, strings, config keys, error messages, comments, or docs. Use `literal: true` unless you want regex behavior.

For code shape — calls, imports, declarations, JSX, object literals, control flow — prefer `search`, which parses AST patterns.

If output says results were truncated at `limit` or by display budget, narrow before editing. Good narrowing order: `summary` → `path`/`glob` → stricter pattern → `scope: "symbol"` or `context`.
