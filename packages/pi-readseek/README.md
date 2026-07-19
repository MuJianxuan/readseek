# pi-readseek

`pi-readseek` is a Pi extension for ReadSeek-backed file reading, hash-anchored
editing, anchored grep, structural maps, symbol navigation, and structural search.
Pi's built-in tools remain unchanged by default. `replacedTools` can register the
corresponding ReadSeek implementations under built-in names.

When the ReadSeek binary is available, the extension adds model guidance that
prefers its anchored read, edit, write, rename, and syntax-check workflow while
leaving Pi's built-ins available as fallbacks.

## Installation

```sh
pi install npm:pi-readseek
```

`pi-readseek` depends on `@jarkkojs/readseek`; installation includes the native
binary automatically on supported platforms.

## Tools

- `readSeek_read`: reads text with `LINE:HASH` anchors; when image modes are
  enabled, images and PDFs can be returned as base64 or analyzed locally.
  PDF reads return one selected page at a time by default.
- `readSeek_edit`: edits existing text files using fresh `LINE:HASH` anchors.
- `readSeek_grep`: searches text and returns edit-ready anchors.
- `readSeek_search`: searches code by structural AST pattern.
- `readSeek_refs`: finds identifier references with enclosing symbols.
- `readSeek_rename`: plans or applies binding-aware renames; workspace matches are
  name-based where binding support is unavailable.
- `readSeek_hover`: identifies the cursor token and enclosing symbol.
- `readSeek_def`: finds structural symbol definitions.
- `readSeek_check`: checks a source file for parser errors and missing syntax.
- `readSeek_view`: views an indexed PDF overview or narrows it by page, node, kind, or depth.
- `readSeek_write`: creates or overwrites whole files and returns anchors.

## Settings

`pi-readseek` reads optional JSON settings from:

- `~/.pi/agent/settings.json` — global settings.
- `.pi/settings.json` — project settings.

Project settings override global settings. The `readseek` section lives in Pi's
shared `settings.json`. All settings are optional; defaults are shown below.

```json
{
  "readseek": {
    "replacedTools": [],
    "imageMode": "auto",
    "syntaxValidation": "warn",
    "timeoutMs": 120000,
    "grep": {
      "maxLines": 2000,
      "maxBytes": 51200
    }
  }
}
```

- `replacedTools`: built-in tool names to replace with ReadSeek implementations.
  Valid values are `"read"`, `"edit"`, `"write"`, and `"grep"`. For a
  ReadSeek-only file surface, use `["read", "edit", "write", "grep"]`.
- `imageMode`: `"auto"` exposes `none`, `all`, `ocr`, `caption`, and `objects`;
  `"on"` omits `none`; `"off"` removes the `image` parameter and always skips
  image/PDF files. Omitting `image` also skips visual files.
- `syntaxValidation`: pre-write syntax-regression check in `readSeek_edit`:
  `"warn"` writes with a warning, `"block"` aborts without writing, and `"off"`
  skips the check.
- `timeoutMs`: ReadSeek invocation timeout in milliseconds.
- `grep.maxLines` / `grep.maxBytes`: visible `readSeek_grep` output budget; values
  above the defaults are clamped.

## Licensing

`pi-readseek` is licensed under `Apache-2.0`. See [LICENSE](LICENSE) for more
information.

The upstream `@jarkkojs/readseek` packages are licensed separately as
`Apache-2.0 AND LGPL-2.1-or-later`.
