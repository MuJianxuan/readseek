# readseek

`readseek` is a structural source reader for scripts, editors, and coding agents.
It emits pretty-printed JSON with stable `LINE:HASH` anchors, structural symbol
maps, parse diagnostics, AST search matches, references, and rename plans.

Current documented CLI API: **0.4.x**.

## Install

```sh
npm install -g @jarkkojs/readseek
```

Or build the native binary from source:

```sh
cargo build --release
```

## Common commands

```sh
readseek detect src/main.rs
readseek read src/main.rs --start 10 --end 20
readseek map src/main.rs
readseek check src/main.rs
readseek symbol src/main.rs --name run
readseek identify src/main.rs:42 --column 8
readseek def src run --language rust --format plain
readseek refs src main --language rust --format plain
readseek search src 'fn $NAME() { $$$BODY }' --language rust
readseek rename src/main.rs --line 42 --column 8 --to renamed
```

Use `--stdin <path>` with `detect`, `read`, `map`, `check`, `symbol`, and
`identify` to analyze unsaved editor buffers while still providing a path for
language detection:

```sh
printf '%s\n' 'fn main() {}' | readseek identify --stdin scratch.rs --line 1 --column 4
```

Use `def --from-identify` to pipe `identify` JSON into definition lookup:

```sh
readseek identify src/main.rs:42 --column 8 | readseek def --from-identify src --format plain
```

## 0.4 CLI notes

- `read --start` is the range-start option. The old `--offset` spelling is not
  part of the 0.4 API.
- `def --from-identify` reads `identify` JSON from standard input. The old
  `def --stdin` spelling is not part of the 0.4 API.
- Cursor columns are one-based byte columns. `identify`, `refs --scope`, and
  `rename` reject line/column values outside the selected line instead of
  silently clamping them.
- `--readseek-dir <dir>` pins the `.readseek` cache directory and must appear
  before the subcommand.

## Cache

`readseek init [path]` creates a `.readseek/` directory containing map cache files
under `maps/` and definition-index shards under `def-index/`. Commands discover
that directory by walking up from the target path, or use the directory passed by
`--readseek-dir`.

## Documentation

The manual page is the authoritative CLI reference:

```sh
man man/man1/readseek.1
```

Pass `--help` to any command for command-specific usage.

## Licensing

The JavaScript npm wrapper is licensed under `Apache-2.0`. The Rust source and
native binaries are licensed under `LGPL-2.1-or-later`. Corresponding source for
each published native binary is available from the GitHub repository tag that
matches the package version.
