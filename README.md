# readseek

`readseek` reads source files and PDFs for scripts, editors, and coding agents.
It returns compact JSON with stable line/hash anchors, symbol maps, parse diagnostics,
AST matches, references, and rename plans.

## Installation

Install the npm wrapper with a prebuilt binary:

```sh
npm install -g @jarkkojs/readseek
```

Or install the native program from crates.io:

```sh
cargo install readseek
```

Prebuilt binaries are available for macOS ARM64; Linux ARM64 and x64; and Windows
x64. The Linux binaries are statically linked with musl.

To build and install from source:

```sh
make install
```

## Plugins

### Pi extension

The bundled [pi-readseek extension](packages/pi-readseek/README.md) adds ReadSeek's
anchored file and structural-code tools to Pi:

```sh
pi install npm:pi-readseek
```

### OpenCode plugin

The [opencode-readseek plugin](packages/opencode-readseek/README.md) adds the same
tools to OpenCode. Add it to `opencode.json`:

```json
{
  "plugin": ["opencode-readseek"]
}
```

### Vim plugin

The bundled `readseek.vim` plugin provides anchored reads, structural navigation,
parse diagnostics, search, references, and renames. It requires Vim9 with `+job`,
`+channel`, `+popupwin`, and `+textprop`; Neovim is not supported.

Install it with a Vim plugin manager, such as `Plug 'jarkkojs/readseek'`, then run
`:ReadSeekInstall` to install the matching prebuilt binary. Downloads are explicit
by default.

## Common commands

```sh
readseek detect src/main.rs
readseek read src/main.rs:10 --end 20
readseek read report.pdf --page 3
readseek view report.pdf --page 3
readseek map src/main.rs
readseek check src/main.rs
readseek symbol src/main.rs:run --name
readseek identify src/main.rs:42 --column 8
readseek def src run --language rust --format plain
readseek refs src main --language rust --format plain
readseek search src 'fn $NAME() { $$$BODY }' --language rust
readseek rename src/main.rs --line 42 --column 8 --to renamed
```

Global options must precede the command. For example, write JSON to a file with:

```sh
readseek --output result.json detect src/main.rs
```

Prefix a target with `stdin:` to analyze an unsaved buffer while retaining a path
for language detection and cursor addressing. This works with `detect`, `read`,
`map`, `check`, `symbol`, and `identify`:

```sh
printf '%s\n' 'fn main() {}' | readseek identify stdin:scratch.rs:1 --column 4
```

## Images and PDFs

`detect` reports image metadata and PDF page counts. `read` returns bounded base64
images by default; select local analysis with `--image`:

```sh
readseek read photo.jpg                   # default: bounded base64 image
readseek read photo.jpg --image caption   # detailed natural-language caption
readseek read photo.jpg --image objects   # object labels + bounding boxes
readseek read photo.jpg --image ocr       # extracted text
readseek read photo.jpg --image all       # caption, objects, and OCR in one pass
```

Vision analysis uses the `balanced` profile by default. Choose `fast` for lower
latency or `accurate` for dense OCR and small objects:

```sh
readseek read photo.jpg --image caption --vision-profile fast
readseek read scan.png --image ocr --vision-profile accurate
```

`--vision-diagnostics` writes cache status, devices, token counts, timings,
throughput, and peak RSS as JSON to stderr. `--vision-benchmark N` runs one warmup
and `N` measured iterations for an image file:

```sh
readseek read fixture.png --image all --vision-benchmark 5 \
  >result.json 2>benchmark.json
```

Set `READSEEK_VISION_THREADS` to a positive integer to override Rayon's worker
count. More workers may improve throughput at the cost of CPU and memory.

PDF reads return page-tagged Markdown and embedded images. Use `--page` to select a
page; `--image` applies to each embedded image. After `readseek init`, `view` creates
or reuses a structural PDF index that can be narrowed by page, node, kind, or depth.
Line/hash suffixes, `--end`, `--limit`, and `--language` do not apply to visual files.

The Qwen3-VL-2B Q8_0 model and multimodal projector download lazily to the user
cache and are checksum-verified. Inference uses ReadSeek's built-in CPU engine and
can be slow.

## Cache

`readseek init [path]` creates `.readseek/maps/` and `.readseek/def-index/`.
Map-dependent commands update them on demand and find `.readseek/` by walking up
from the target; `--readseek-dir` selects one explicitly. PDF indexes and extracted
assets live in `.readseek/documents/`, while image analysis results are cached under
`.readseek/vision/`. Image cache entries are profile-specific.

## Documentation

For the complete CLI reference:

```sh
man ./man/man1/readseek.1
```

Pass `--help` to any command for command-specific usage.

## Licensing

The native `readseek` crate is licensed under `LGPL-2.1-or-later`. The npm wrapper
and platform packages declare `Apache-2.0 AND LGPL-2.1-or-later`; plugin licenses
are listed in their package directories. The downloaded Qwen model is licensed
under `Apache-2.0`.
