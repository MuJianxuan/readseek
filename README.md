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

Use a `stdin:` target prefix with `detect`, `read`, `map`, `check`, `symbol`,
and `identify` to analyze unsaved editor buffers while still providing a path
for language detection and a cursor address:

```sh
printf '%s\n' 'fn main() {}' | readseek identify stdin:scratch.rs:1 --column 4
```

## Images

`detect` reports format, dimensions, and animation status for images. Add a vision
flag to analyze image contents with the BLIP, YOLOv8-nano, and TrOCR models:

```sh
readseek detect photo.jpg --caption        # detailed natural-language caption
readseek detect photo.jpg --objects        # object labels + bounding boxes
readseek detect photo.jpg --ocr            # extracted text
```

The flags can be combined; each model loads once per invocation. The model files
(~258 MB BLIP GGUF + ~6 MB YOLOv8-nano + ~1.24 GB TrOCR) are downloaded lazily
into the user cache directory on first vision use and reused on subsequent runs;
a progress bar is shown while downloading when stdout is an interactive TTY.
Inference is CPU-only; object detection and OCR take seconds and captioning up to
a couple of minutes per image.

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

readseek downloads the BLIP caption model (a quantized GGUF from
`lmz/candle-blip`, with the tokenizer from `Salesforce/blip-image-captioning-large`),
the YOLOv8-nano object-detection model (`lmz/candle-yolo-v8`), and the TrOCR
printed-text model (`microsoft/trocr-base-printed`) into the `models/` subdirectory
of the user cache on first use. BLIP is licensed under `BSD-3-Clause`; YOLOv8-nano
is derived from Ultralytics YOLOv8 (`AGPL-3.0`); TrOCR is released by Microsoft.
