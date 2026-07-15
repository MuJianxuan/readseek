# opencode-readseek

`opencode-readseek` exposes ReadSeek's hash-anchored reads and structural code
navigation as OpenCode tools. It intentionally does not replace OpenCode's
built-in `read`, `edit`, or `write` tools.

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

- `readseek_read`: read text with `LINE:HASH` anchors and explicitly selected image/PDF handling.
- `readseek_map`: generate a structural symbol map.
- `readseek_search`: AST-pattern search.
- `readseek_def`, `readseek_refs`, `readseek_hover`: symbol navigation.
- `readseek_rename`: generate a rename plan without writing files.
- `readseek_check`: parse diagnostics.

The plugin discards remembered anchors after OpenCode reports `file.edited`,
records anchors from successful ReadSeek tool results, refuses any attempt to
apply a rename directly, and adds current anchors plus a pending rename plan to
the OpenCode compaction context.

`imageMode` controls which `readseek_read.image` values the model sees:
`"auto"` exposes `none`, `ocr`, `caption`, and `objects`; `"on"` exposes
`ocr`, `caption`, and `objects`; `"off"` skips image and PDF files. Omitting
`image` also skips visual files. The default is `"auto"`.

## Licensing

This package is Apache-2.0. `@jarkkojs/readseek` is licensed separately as
Apache-2.0 AND LGPL-2.1-or-later.
