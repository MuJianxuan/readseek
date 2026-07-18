Create or replace a complete file and return `LINE:HASH` anchors. Use for new or fully generated files; use anchored edits for small changes.

## Usage

Existing files are overwritten without confirmation. Binary-looking content is
rejected without writing.

## Parameters

- `path` — file path.
- `content` — complete file contents.
- `map` — append a best-effort structural map.

## Output

Text writes return `LINE:HASH|content`. Visible output is capped at
{{DEFAULT_MAX_LINES}} lines or {{DEFAULT_MAX_BYTES}}; full anchors remain in
`readSeekValue`. Results also include a compact
diff, unified patch, and structured `details.diffData`.
