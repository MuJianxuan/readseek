Create or replace a complete file and return `LINE:HASH` anchors. Use for new or fully generated files; use anchored edits for small changes.

## Use / avoid

Existing files are overwritten without confirmation. Binary-looking content gets
no anchors.

## Parameters

- `path` — file path.
- `content` — complete file contents.
- `map` — append a best-effort structural map.

## Output

Text writes return `LINE:HASH|content`. Visible output is capped at 2000 lines or
50 KB; full anchors remain in `readSeekValue`. Results also include a compact
diff, unified patch, and structured `details.diffData`.
