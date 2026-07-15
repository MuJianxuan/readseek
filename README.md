# readseek

`readseek` is a structural source reader for scripts, editors, and coding agents.
It emits pretty-printed JSON with stable `LINE:HASH` anchors, structural symbol
maps, parse diagnostics, AST search matches, references, and rename plans.

## Install

Build the native binary from source:

```sh
cargo build --release
```

Or install the npm wrapper:

```sh
npm install -g @jarkkojs/readseek
```

Prebuilt binaries are available for macOS ARM64, Linux ARM64 and x64, and
Windows x64. The Linux binaries are statically linked with musl.

## Pi extension

The bundled [pi-readseek extension](packages/pi-readseek/README.md) exposes
ReadSeek's anchored file and structural-code tools in Pi:

```sh
pi install npm:pi-readseek
```

## Common commands

```sh
readseek detect src/main.rs
readseek read src/main.rs:10 --end 20
readseek map src/main.rs
readseek check src/main.rs
readseek symbol src/main.rs:run --name
readseek identify src/main.rs:42 --column 8
readseek def src run --language rust --format plain
readseek refs src main --language rust --format plain
readseek search src 'fn $NAME() { $$$BODY }' --language rust
readseek rename src/main.rs --line 42 --column 8 --to renamed
```

To write JSON output to a file instead of stdout, place the global option before
the command:

```sh
readseek --output result.json detect src/main.rs
```

Use a `stdin:` target prefix with `detect`, `read`, `map`, `check`, `symbol`,
and `identify` to analyze unsaved editor buffers while still providing a path
for language detection and a cursor address:

```sh
printf '%s\n' 'fn main() {}' | readseek identify stdin:scratch.rs:1 --column 4
```

## Images and PDFs

`detect` reports image metadata and PDF page counts. `read` returns bounded
base64 images by default; use `--image` for one local analysis mode:

```sh
readseek read photo.jpg                   # default: bounded base64 image
readseek read photo.jpg --image caption   # detailed natural-language caption
readseek read photo.jpg --image objects   # object labels + bounding boxes
readseek read photo.jpg --image ocr       # extracted text
```

PDF reads return page-tagged Markdown and page-associated embedded images. The
same mode applies to each embedded image. Line/hash suffixes, `--end`, `--limit`,
and `--language` do not apply to visual files.

Vision models download lazily into the user cache and run on the CPU. Captioning
and OCR can take substantial time.

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

`readseek` is licensed under `LGPL-2.1-or-later`. The JavaScript npm wrapper
is licensed under `Apache-2.0`.
