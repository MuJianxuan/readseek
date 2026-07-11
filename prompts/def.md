---
tools: def
---

Find structural symbol definitions. Calls `readseek def` which searches
for the definition of a named symbol across a file or directory.

## Parameters

- `path` (optional): File or directory to search (default: ".").
- `name`: Qualified or unqualified symbol name.
- `lang` (optional): Language override.
- `cached` (optional): Search tracked/indexed files in a Git repository.
- `others` (optional): Search untracked files.
- `ignored` (optional): Include ignored untracked files.

## When to use

- After a `readSeek_hover` call, use the qualified symbol name to jump to its definition.
- When the user asks "where is X defined?".
- To find a function/class/type definition by its qualified name.
