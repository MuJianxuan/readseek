AST-aware structural code search. Use when text search is too broad or brittle and you need code shape, such as calls, imports, declarations, or JSX. Returns matches grouped by file with edit-ready hashline anchors.

## Parameters

- `pattern` — ast-grep-style pattern to match.
- `lang` — language hint such as `typescript`, `tsx`, `javascript`, `jsx`, `rust`, `python`, `dockerfile`, `lua`, `nix`, `perl`, or `zig`; set it when syntax is ambiguous.
- `path` — file or directory, default cwd.
- `cached` — in a Git repository, search tracked/indexed files.
- `others` — in a Git repository, search untracked files.
- `ignored` — with `others`, include ignored untracked files.

## Pattern syntax

- `$NAME` matches one AST node.
- `$_` matches any one node.
- `$$$ARGS` matches zero or more nodes; use `$$$` for variable-length args, body statements, object fields, JSX children, etc.
- Reusing the same metavariable name requires each occurrence to match the same source text.

## Examples

- `console.log($$$ARGS)` — calls.
- `import $NAME from '$SOURCE'` — default imports.
- `export function $NAME($$$PARAMS) { $$$BODY }` — exported functions.
- `$OBJ.$METHOD($$$ARGS)` — method calls.
- `<$TAG $$$ATTRS>$$$CHILDREN</$TAG>` — JSX/TSX elements.

## Tips

Patterns are parsed as code, not text: formatting is mostly ignored, but syntax must be valid for `lang`. Include semicolons in languages that require them. Use `grep` for plain text and `search` for structure.

When searching a directory inside a Git repository, readseek 0.2.x defaults to tracked/indexed files plus untracked non-ignored files. Use `cached`, `others`, and `ignored` to narrow or expand that Git selection.
