# opencode-readseek

`opencode-readseek` exposes ReadSeek's hash-anchored text tools and structural
code navigation as OpenCode tools.

## Installation

Add the plugin to `opencode.json`:

```json
{
  "plugin": [["opencode-readseek", { "imageMode": "auto" }]]
}
```

OpenCode installs the package and its supported-platform `@jarkkojs/readseek`
binary dependency with Bun at startup.

## Tools

- `readseek_read`: read text with `LINE:HASH` anchors; image/PDF handling is explicit and PDFs default to one page.
- `readseek_edit`: apply line, range, and insertion edits with fresh `LINE:HASH` anchors.
- `readseek_write`: create or replace a complete text file.
- `readseek_grep`: plain-text or regular-expression search with anchored results.
- `readseek_map`: generate a structural symbol map.
- `readseek_search`: AST-pattern search.
- `readseek_def`, `readseek_refs`, `readseek_hover`: symbol navigation.
- `readseek_rename`: apply a verified rename by default; `apply: false` returns a dry-run plan.
- `readseek_check`: parse diagnostics.
- `readseek_view`: view an indexed PDF overview or narrow it by page, node, kind, or depth.

The plugin requests OpenCode read, grep, external-directory, and edit permissions
as needed. File changes invalidate remembered anchors. Compaction context records
paths with fresh anchors and summaries of pending dry-run rename plans. Text reads
return at most 2,000 lines by default.

The plugin also instructs OpenCode to prefer ReadSeek's anchored read, edit,
write, and rename tools over built-in file mutation tools while leaving the
built-ins available as fallbacks.

## Configuration

`imageMode` defaults to `"auto"`: it exposes `none`, `all`, `ocr`, `caption`, and
`objects`. `"on"` omits `none`; `"off"` skips image/PDF files. Omitting
`image` also skips visual files.

## Licensing

This package uses the
[Apache-2.0 license](https://github.com/jarkkojs/readseek/blob/main/LICENSE-APACHE-2.0).
`@jarkkojs/readseek` declares `Apache-2.0 AND LGPL-2.1-or-later`.
