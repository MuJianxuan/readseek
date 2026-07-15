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

- `readseek_read`: read text with `LINE:HASH` anchors; image/PDF handling is explicit.
- `readseek_edit`: apply line, range, and insertion edits with fresh `LINE:HASH` anchors.
- `readseek_write`: create or replace a complete text file.
- `readseek_grep`: plain-text or regular-expression search with anchored results.
- `readseek_map`: generate a structural symbol map.
- `readseek_search`: AST-pattern search.
- `readseek_def`, `readseek_refs`, `readseek_hover`: symbol navigation.
- `readseek_rename`: atomically apply a verified rename by default; `apply: false` returns a dry-run plan.
- `readseek_check`: parse diagnostics.

The plugin asks for OpenCode read, grep, external-directory, and edit permissions
as appropriate. It discards remembered anchors after file changes, records only
results that contain actual hashlines, and adds current anchors plus pending
dry-run rename plans to the OpenCode compaction context. Text reads return at
most 2000 lines by default.

`imageMode` defaults to `"auto"`: it exposes `none`, `ocr`, `caption`, and
`objects`. `"on"` omits `none`; `"off"` skips image/PDF files. Omitting
`image` also skips visual files.

## Licensing

This package is Apache-2.0. `@jarkkojs/readseek` is licensed separately as
Apache-2.0 AND LGPL-2.1-or-later.
