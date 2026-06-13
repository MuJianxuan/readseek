Search code with readseek AST patterns. Use it when text search is too broad or brittle and the query depends on syntax: calls, imports, declarations, JSX, object fields, control flow, and similar code shapes. Results are grouped by file with edit-ready hashline anchors.

## Parameters

- `pattern` — ast-grep-style pattern to match.
- `lang` — language hint; set it when syntax is ambiguous, extensionless, generated, or TSX/JSX-like.
- `path` — file or directory, default cwd.
- `cached` — in a Git repository, search tracked/indexed files.
- `others` — in a Git repository, search untracked files.
- `ignored` — with `others`, include ignored untracked files.

## Pattern syntax

- `$NAME` matches one AST node.
- `$_` matches any one AST node when you do not need to reuse it.
- `$$$ARGS` matches zero or more sibling nodes. Use it for function args, body statements, object fields, JSX children, etc.
- Reusing a metavariable name requires every occurrence to match the same source text.

Patterns are parsed as code, not text. Formatting is mostly ignored, but syntax must be valid for the selected language. Include punctuation that the language grammar requires.

## Examples

- `console.log($$$ARGS)` — calls.
- `import $NAME from '$SOURCE'` — default imports.
- `export function $NAME($$$PARAMS) { $$$BODY }` — exported functions.
- `$OBJ.$METHOD($$$ARGS)` — method calls.
- `<$TAG $$$ATTRS>$$$CHILDREN</$TAG>` — JSX/TSX elements.
- `if ($COND) { $$$BODY }` — control-flow blocks.

## Languages

Useful `lang` values include `typescript`, `tsx`, `javascript`, `jsx`, `rust`, `python`, `go`, `java`, `c`, `cpp`, `csharp`, `ruby`, `php`, `lua`, `bash`, `json`, `yaml`, `toml`, `markdown`, `dockerfile`, `nix`, and `zig`. readseek 0.2.3 also accepts languages such as `assembly`, `css`, `gdscript`, `html`, `just`, `kconfig`, `latex`, `make`, `meson`, `perl`, `puppet`, `riscv`, `sql`, `swift`, `typst`, `xml`, and `unknown`.

`unknown` forces text-only handling and is not useful for parser-backed search.

## Git selection

When searching a directory inside a Git repository, readseek defaults to tracked/indexed files plus untracked non-ignored files. Use `cached`, `others`, and `ignored` to narrow or expand that selection. `ignored` requires `others`.

Use `grep` for plain text and `search` for structure.
