Find structural definitions by qualified or unqualified name across a file or
directory.

## Parameters

- `path` — file or directory, default `.`.
- `name` — qualified or unqualified symbol name.
- `lang` — language override.
- `cached`, `others`, `ignored` — Git file selection; `ignored` requires `others`.

## When to use

- After `readSeek_hover`, use its qualified symbol name.
- When asked where a function, class, or type is defined.
