# readseek

`readseek` is a structural source and PDF reader for scripts, editors, and coding agents.
It emits compact JSON with stable line/hash anchors, structural symbol maps, parse
diagnostics, AST search matches, references, and rename plans.

## Installation

Install the npm wrapper and a prebuilt binary:

```sh
npm install -g @jarkkojs/readseek
```

Install the native program from crates.io:

```sh
cargo install readseek
```

Prebuilt binaries are available for macOS ARM64; Linux ARM64 and x64; and Windows
x64. The Linux binaries are static glibc PIE executables.

To build and install from source:

```sh
make install
```

Source builds require CMake, Clang/libclang, and a C++ compiler because image
inference uses `llama-cpp-2`.

GPU acceleration can be enabled at build time with the `metal`, `opencl`, `rocm`,
or `vulkan` Cargo feature. CPU builds can enable `openmp` for OpenMP parallelism.

## Plugins

### Pi extension

The bundled [pi-readseek extension](packages/pi-readseek/README.md) exposes
ReadSeek's anchored file and structural-code tools in Pi:

```sh
pi install npm:pi-readseek
```

### OpenCode plugin

The [opencode-readseek plugin](packages/opencode-readseek/README.md) provides the
same anchored and structural tools in OpenCode.

Add the plugin to `opencode.json`:

```json
{
  "plugin": ["opencode-readseek"]
}
```

### Vim plugin

The bundled `readseek.vim` plugin provides structural navigation, search,
references, and rename operations in Vim9. The runtime directories are at the
repository root, so install it directly with a Vim plugin manager, for example
`Plug 'jarkkojs/readseek'`. It installs the matching prebuilt ReadSeek release
binary automatically.

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
base64 images by default; use `--image` for a local analysis mode:

```sh
readseek read photo.jpg                   # default: bounded base64 image
readseek read photo.jpg --image caption   # detailed natural-language caption
readseek read photo.jpg --image objects   # object labels + bounding boxes
readseek read photo.jpg --image ocr       # extracted text
readseek read photo.jpg --image all       # caption, objects, and OCR in one pass
```

Inference defaults to the `balanced` profile. `fast` uses fewer visual tokens for
lower latency, while `accurate` retains more visual detail for dense OCR and small
objects:

```sh
readseek read photo.jpg --image caption --vision-profile fast
readseek read scan.png --image ocr --vision-profile accurate
```

Use `--vision-diagnostics` to emit cache status, backend devices, token counts,
per-stage timings, throughput, and peak RSS as JSON on stderr. For repeatable warm
measurements, `--vision-benchmark N` runs one warmup followed by `N` measured
iterations (currently image files only):

```sh
readseek read fixture.png --image all --vision-benchmark 5 \
  >result.json 2>benchmark.json
```

CPU tuning overrides are available through `READSEEK_VISION_THREADS`,
`READSEEK_VISION_BATCH`, and `READSEEK_VISION_UBATCH`. Values must be positive,
and the micro-batch cannot exceed the logical batch. Compare overrides on a fixed
representative image corpus; larger batches can improve prefill while increasing
peak memory.

PDF reads return page-tagged Markdown and page-associated embedded images. Use
`--page` to select one page; the same image mode applies to each embedded image.
After initializing `.readseek/`, `view` creates or reuses a persistent structural
PDF index and returns an overview that can be narrowed by page, node, kind, or
depth. Line/hash suffixes, `--end`, `--limit`, and `--language` do not apply to
visual files.

The Qwen3-VL GGUF language model and Q8_0 multimodal projector download lazily
into the user cache, are checksum-verified once, and then use a metadata-bound
verification marker on later starts. Inference uses the backends enabled in the
build; CPU-only captioning can still take substantial time.

## Cache

`readseek init [path]` creates and populates `.readseek/maps/` and
`.readseek/def-index/`. Map-dependent commands update entries on demand and
discover `.readseek/` by walking up from the target path, or use the directory
passed by `--readseek-dir`. `view` creates PDF structure indexes and extracted
assets under `.readseek/documents/` on demand. Image analysis caches results
under the `.readseek/` found from the current working directory. Cache entries are profile-specific.

## Documentation

The manual page provides the full CLI reference:

```sh
man ./man/man1/readseek.1
```

Pass `--help` to any command for command-specific usage.

## Licensing

The native `readseek` crate is licensed under `LGPL-2.1-or-later`. The
`@jarkkojs/readseek` wrapper and platform packages declare
`Apache-2.0 AND LGPL-2.1-or-later`; the bundled plugins use Apache-2.0 or MIT as
noted in their package directories.

The downloaded `Qwen/Qwen3-VL-2B-Instruct-GGUF` model is licensed under
`Apache-2.0`.
