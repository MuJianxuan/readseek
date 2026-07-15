Find symbol declarations by qualified or unqualified name. Use instead of text search when locating where a function, class, type, or other symbol is defined.

## Parameters

- `path` — file or directory, default `.`.
- `name` — qualified or unqualified symbol name.
- `lang` — language override.
- `cached`, `others`, `ignored` — Git file selection; `ignored` requires `others`.

After `readSeek_hover`, use its qualified symbol name.
