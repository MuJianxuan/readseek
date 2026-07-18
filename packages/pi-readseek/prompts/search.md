Search syntax-aware code shapes with AST patterns and return edit-ready anchors. Use for calls, imports, declarations, JSX, object fields, or control flow; use `readSeek_grep` for plain text.

## Parameters

- `pattern` — ast-grep-style pattern to match.
- `lang` — language hint for ambiguous, extensionless, generated, or JSX-like code.
- `path` — file or directory, default cwd.
- `cached` — in a Git repository, search tracked/indexed files.
- `others` — in a Git repository, search untracked files.
- `ignored` — with `others`, include ignored untracked files.

## Pattern syntax

- `$NAME` matches one AST node.
- `$_` matches any one AST node when you do not need to reuse it.
- `$$$ARGS` matches zero or more sibling nodes. Use it for function args, body statements, object fields, JSX children, etc.
- Reusing a metavariable name requires every occurrence to match the same source text.

Patterns are code, not text: formatting is mostly ignored, but syntax and required
punctuation must be valid for the selected language.

## Examples

- `console.log($$$ARGS)` — calls.
- `import $NAME from '$SOURCE'` — default imports.
- `export function $NAME($$$PARAMS) { $$$BODY }` — exported functions.
- `$OBJ.$METHOD($$$ARGS)` — method calls.
- `<$TAG $$$ATTRS>$$$CHILDREN</$TAG>` — JSX/TSX elements.
- `if ($COND) { $$$BODY }` — control-flow blocks.

## Git selection

In Git repositories, directory search includes tracked/indexed and untracked,
non-ignored files by default. `cached` or `others` restricts the search to that
group; `ignored` requires `others`.
