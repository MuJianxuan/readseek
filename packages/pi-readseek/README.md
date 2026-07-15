# pi-readseek

`pi-readseek` is a pi extension for readseek-backed file reading, hash-anchored
editing, anchored grep, structural maps, symbol lookup, and structural search.
It exposes readseek tools under the `readSeek_` prefix. Built-in pi tools stay
active unless excluded in settings.

## Installation

```bash
pi install npm:pi-readseek
```

The structural search and map features require the `@jarkkojs/readseek` native
binary. The extension auto-installs the correct platform package, or you can
install it manually:

```bash
# Auto-installed by the extension on supported platforms.
# Manual install (if needed):
npm install --save-dev @jarkkojs/readseek
```

## Tools

- **readSeek_read:** reads text with `LINE:HASH` anchors; when image modes are
  enabled, images and PDFs can be returned as base64 or analyzed locally.
- **readSeek_edit:** edits existing text files using fresh `LINE:HASH` anchors.
- **readSeek_grep:** searches text and returns edit-ready anchors.
- **readSeek_search:** searches code by structural AST pattern.
- **readSeek_refs:** finds identifier references with enclosing symbols.
- **readSeek_rename:** plans or applies binding-aware renames.
- **readSeek_hover:** identifies the cursor token and enclosing symbol.
- **readSeek_def:** finds structural symbol definitions.
- **readSeek_check:** checks a source file for parser errors and missing syntax.
- **readSeek_write:** creates or overwrites whole files and returns anchors.

## Settings

`pi-readseek` reads optional JSON settings from:

- `~/.pi/agent/settings.json` — Global
- `.pi/settings.json` — Project

Project settings override global settings. The `readseek` section lives inside pi's
shared `settings.json`, alongside other extensions' sections. All settings are
optional (defaults shown):

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

- **replacedTools:** built-in tool names to replace with their `readSeek_*` equivalents.
  Valid values are `"read"`, `"edit"`, `"write"`, and `"grep"`. For a
  readseek-only file surface, use `["read", "edit", "write", "grep"]`.
- **imageMode:** `"auto"` exposes `none`, `ocr`, `caption`, and `objects`;
  `"on"` omits `none`; `"off"` removes the `image` parameter and always skips
  image/PDF files. Omitting `image` also skips visual files.
- **syntaxValidation:** pre-write syntax-regression check in `readSeek_edit`:
  `"warn"` writes with a warning, `"block"` aborts without writing, `"off"`
  skips the check.
- **timeoutMs:** readseek invocation timeout in milliseconds.
- **grep.maxLines** / **grep.maxBytes:** visible `readSeek_grep` output
  budget; values above the defaults are clamped.

## Licensing

`pi-readseek` is licensed under `Apache-2.0`. See [LICENSE](LICENSE) for more
information.

The upstream `@jarkkojs/readseek` packages are licensed separately as
`Apache-2.0 AND LGPL-2.1-or-later`.
