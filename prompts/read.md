Read files through readseek. Text output uses `LINE:HASH|content` anchors that can be copied directly into `edit`; supported images return attachments instead of anchors. Default cap: {{DEFAULT_MAX_LINES}} lines or {{DEFAULT_MAX_BYTES}}.

## Choose the right read

- Normal read: inspect a whole small file or a targeted `offset` / `limit` range.
- `map: true`: append a structural map for navigation before reading more code.
- `symbol: "Name"`: read one function, class, method, interface, type, enum, or similar symbol.
- `bundle: "local"`: with `symbol`, include direct same-file local support when readseek can identify it.

## Parameters

- `path` — file path.
- `offset` / `limit` — positive line numbers; `offset` is 1-indexed.
- `map` — append the full-file structural map; cannot combine with `symbol` or `bundle`.
- `symbol` — symbol query; supports `Class.method`, package-relative Java names, and `Name@<line>` disambiguation; cannot combine with `offset` / `limit`.
- `bundle` — only `"local"`; requires `symbol` and cannot combine with `map`.

When a full-file read is truncated, readseek appends a structural map automatically when available. Use map line ranges for follow-up `read({ offset, limit })` calls.

## Symbol examples

| Query | Reads |
|---|---|
| `{ "symbol": "processEvent" }` | function or top-level symbol |
| `{ "symbol": "EventEmitter" }` | class/interface/type/enum/etc. |
| `{ "symbol": "EventEmitter.emit" }` | child method/member |
| `{ "symbol": "Foo.bar@42" }` | overload/definition near line 42 |
| `{ "symbol": "handleRequest", "bundle": "local" }` | symbol plus direct same-file support |

## Symbol resolution

`@<line>` only applies as a trailing suffix like `Foo.bar@42`; names such as `foo@bar` are ordinary queries. Resolution order: containing range → nearest symbol starting at/after the requested line → nearest symbol above it.

Result behavior:

- **Found**: returns only the symbol range with `[Symbol: name (kind), lines X-Y of Z]`.
- **Ambiguous**: lists candidates and retry hints such as `name@<startLine>`.
- **Fuzzy**: returns the best camelCase/substring match with a warning; verify before editing from those anchors.
- **Not found** or **unmappable**: falls back to normal read with a warning and, when available, symbol suggestions.

Hash anchors from normal, symbol, and bundled reads are valid for `edit` until the file changes.
