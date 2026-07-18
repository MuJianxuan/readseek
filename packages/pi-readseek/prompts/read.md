Read anchored text by range, map, or symbol, and process images or PDFs with an explicit mode.

## Choose the right read

- Normal read: a small file or an `offset` / `limit` range.
- `map: true`: append a structural map.
- `symbol: "Name"`: read one mapped symbol.
- `bundle: "local"`: include direct same-file support for a symbol.

## Parameters

- `path` — file path.
- `offset` / `limit` — positive lines; `offset` is 1-indexed.
- `map` — full-file map; incompatible with `symbol` or `bundle`.
- `symbol` — `Name`, `Class.method`, or `Name@<line>`; incompatible with `offset` / `limit`.
- `bundle` — only `"local"`; requires `symbol` and excludes `map`.
- `image` — an exposed image/PDF mode; unavailable when `imageMode` is `"off"`.

Default cap: {{DEFAULT_MAX_LINES}} lines or {{DEFAULT_MAX_BYTES}}. Omitting `image` skips images and PDFs; when `imageMode` is `"off"`, visual files are always skipped.

Truncated full-file reads append a map when available. Use its ranges for follow-up reads.

## Symbol examples

| Query | Reads |
|---|---|
| `{ "symbol": "processEvent" }` | function or unqualified symbol |
| `{ "symbol": "EventEmitter" }` | class/interface/type/enum/etc. |
| `{ "symbol": "EventEmitter.emit" }` | child method/member |
| `{ "symbol": "Foo.bar@42" }` | overload/definition near line 42 |
| `{ "symbol": "handleRequest", "bundle": "local" }` | symbol plus direct same-file support |

## Symbol resolution

`@<line>` is only a trailing suffix, as in `Foo.bar@42`; `foo@bar` is an ordinary name. Resolution prefers the containing range, then the nearest symbol at or after the line, then one above it.

Result behavior:

- **Found:** the symbol range.
- **Ambiguous:** candidates and `name@<startLine>` retry hints.
- **Fuzzy:** a warned best match; verify before editing.
- **Not found/unmappable:** normal read with a warning and suggestions when available.

Hash anchors from normal, symbol, and bundled reads are valid for `readSeek_edit` until the file changes.
