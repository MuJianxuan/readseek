# Research: speeding up ReadSeek vision inference

Scope: the current Qwen3-VL path in `src/engine/vision.rs`, model handling in
`src/engine/model.rs`, vision caching, and release packaging. The goal is lower
latency without knowingly reducing caption, grounding, or OCR quality.

## Implementation status

The implemented optimization path includes:

- `fast`, `balanced` (default), and `accurate` visual-token profiles;
- task-specific output ceilings and JSON-schema grammar-constrained decoding;
- a 2048 logical batch, 512 micro-batch, supported thread/batch environment
  overrides, and an opt-in `openmp` Cargo feature;
- the official Q8_0 projector and a metadata-bound verification marker after the
  initial full checksum;
- profile-specific result-cache entries; and
- `--vision-diagnostics` plus `--vision-benchmark N` JSON reports with warmup,
  p50/p95, stage timings, throughput, backend, token counts, and peak RSS.

GPU-enabled release artifacts were explicitly dropped from this implementation
scope. Existing GPU Cargo features remain available for source builds.

Before treating a faster profile as quality-equivalent, run every profile against
a fixed local corpus containing screenshots, photographs, diagrams, dense scans,
small text, crowded scenes, and representative PDF-extracted images. Record caption
rubric results, OCR CER/WER, object label recall, and box IoU alongside benchmark
p50/p95 and peak RSS. Keep `balanced` as the default unless the corpus shows no
material regression; use `accurate` as the comparison path for detail-sensitive
failures. Large model-backed fixtures are intentionally not part of the normal test
suite.

## Executive summary

The largest practical opportunities are:

1. **Make the visual-token budget task/quality dependent.** A one-image CPU
   experiment reduced generation time from 58.8 s at the current 2048-token cap
   to 17.4 s at 512, a 3.4x speedup. This changes output detail, so it needs a
   quality suite before changing the default.
2. **Actually ship GPU-enabled binaries.** Cargo exposes Metal, Vulkan, ROCm,
   and OpenCL features, and the runtime is already configured to offload, but
   `.github/workflows/release.yml` builds every release without those features.
   Enabling Metal for the macOS artifact is the simplest first step.
3. **Evaluate the official Q8_0 multimodal projector.** It is 445 MB versus the
   current 819 MB F16 projector, a 45.7% reduction in weight size and bandwidth.
4. **Tune prompt batching and CPU threads empirically.** ReadSeek forces a 512
   batch and uses all logical CPUs. Upstream llama.cpp defaults to a 2048 logical
   batch and warns that too many threads can reduce token-generation speed.
5. **Bound tail latency.** Every task currently permits 4096 generated tokens.
   Caption and object modes should have much smaller task-specific limits; OCR
   can retain the larger budget.

A benchmark and quality corpus should come first. The repository currently has
no inference benchmark suite.

## Current pipeline

`src/engine/vision.rs` uses:

- Qwen3-VL-2B-Instruct Q4_K_M language weights: 1,107,409,952 bytes.
- F16 multimodal projector: 819,394,848 bytes.
- 8192-token context, 512-token batch, 4096-token output ceiling, and a
  2048-token image ceiling.
- Greedy, one-token-at-a-time decoding with early return after a complete JSON
  object.
- All available logical CPUs for prompt and token evaluation.
- A reusable model/context within one process and a content-addressed result
  cache across processes.

Important existing optimizations:

- Caption, objects, and OCR run in one model pass when requested together.
- PDF RGB images bypass PNG encoding where possible.
- The language model is already Q4_K_M and memory-mapped.
- llama.cpp 0.1.151 defaults Flash Attention to `AUTO`.
- Its model default uses `n_gpu_layers = -1`, which resolves to all layers when
  a GPU backend exists. ReadSeek also sets `MtmdContextParams::use_gpu` from
  `backend.supports_gpu_offload()`.

The main missing GPU piece is therefore release packaging, not inference code.

## Measurements

The test image was `packages/opencode-readseek/screenshot.png` (1374x1054),
caption mode, CPU-only, on a 6-core/12-thread Ryzen 5 PRO 4650U. Timing probes
were temporary and were removed after measurement. The llama.cpp native code
was built with CMake `Release`; only Rust orchestration used the development
profile. Compare phase timings rather than end-to-end development-build startup.

| Image cap | Prompt tokens | Tokenize | Prompt/image eval | Decode | Generation total | Relative |
|---:|---:|---:|---:|---:|---:|---:|
| 2048 | 1457 | 0.41 s | 49.45 s | 8.91 s | 58.81 s | 1.0x |
| 1024 | 1046 | 0.28 s | 30.28 s | 7.26 s | 37.85 s | 1.55x |
| 512 | 513 | 0.14 s | 11.80 s | 5.45 s | 17.42 s | 3.38x |

All three produced a valid, usable caption, but they were not semantically
identical. One screenshot is insufficient evidence that 512 tokens preserves
OCR and grounding quality.

An optimized release startup probe measured about 2.65 s before generation:

- Cached model validation, including SHA-256 over both 1.9 GB files: 1.32 s.
- Language model load: 0.75 s.
- Context creation: 0.11 s.
- Projector initialization: 0.47 s.

This makes visual-token count the dominant CPU cost today. Startup becomes more
important after GPU acceleration.

## Prioritized experiments

### P0: add a reproducible latency and quality suite

Add an opt-in benchmark command or ignored integration test that records:

- model validation/load, bitmap/tokenization, multimodal prefill, and decode;
- visual prompt tokens, generated tokens, tokens/s, peak RSS, and backend;
- cold process, warm OS cache, result-cache hit, and multi-image PDF cases.

Use a fixed corpus covering screenshots, photographs, diagrams, dense documents,
small text, many objects, and large PDFs. Compare caption similarity manually or
with a frozen rubric, OCR CER/WER, and object label/box recall. Report p50 and p95,
not one run.

This is necessary because several knobs exchange visual detail for speed and
because CPU timings vary strongly with temperature and thread oversubscription.

### P1: introduce fast/balanced/accurate visual-token profiles

Qwen's official documentation exposes visual-token budgeting and demonstrates a
256-1280 token range. ReadSeek hard-codes 2048 for every task.

Test profiles such as:

| Profile | Caption | Objects | OCR / all |
|---|---:|---:|---:|
| fast | 512 | 768 | 1024 |
| balanced | 768 | 1024 | 1536 |
| accurate | 2048 | 2048 | 2048 |

The exact values must come from the quality corpus. A single global reduction to
1024 is simpler, but task-specific profiles should preserve OCR and grounding
better. Since image limits are attached to `MtmdContext`, choose the profile
before runtime initialization rather than loading multiple projectors at once.

Expected impact: **high**. The experiment showed 1.6-3.4x generation improvement
for captioning.

### P1: enable GPU backends in release artifacts

`Cargo.toml` exposes `metal`, `vulkan`, `rocm`, and `opencl`, but release jobs run
plain `cargo build --release`. Even macOS therefore ships a CPU-only binary.

Start with:

- macOS arm64: build the artifact with `--features metal`;
- add an inference smoke test that verifies a non-CPU backend is detected;
- expose backend/offload diagnostics instead of silencing every llama.cpp log.

For Linux and Windows, evaluate separate CPU and Vulkan artifacts or llama.cpp's
`dynamic-backends` feature. A single statically linked Linux artifact may not be
compatible with every GPU driver stack. CUDA can be a later specialized artifact.

Expected impact: **very high** on supported hardware. Upstream llama.cpp explicitly
recommends GPU layer offload for generation performance.

### P2: replace the F16 projector with the official Q8_0 projector

The official Qwen GGUF repository provides:

- `mmproj-Qwen3VL-2B-Instruct-F16.gguf`: 819,394,848 bytes;
- `mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf`: 445,053,216 bytes.

The Q8_0 file is 374 MB smaller. It should reduce model download, resident memory,
and projector bandwidth during multimodal evaluation. Benchmark it on CPU and
GPU and validate OCR/box quality before switching. If accepted, update model
hashes and bump `CACHE_SCHEMA_VERSION`.

Expected impact: **medium to high**, especially during image encoding/prefill.

### P2: tune batch, micro-batch, threads, and CPU build features

ReadSeek sets `n_batch = 512`, leaves `n_ubatch = 512`, and passes 512 to
`eval_chunks`. The measured 1457-token prompt therefore needs multiple batches.
Benchmark 512/1024/2048 logical and physical batches while tracking RSS.

The dependency disables default features, which also disables llama.cpp OpenMP.
Benchmark the `openmp` feature and, where packaging permits it, BLAS. Upstream says
BLAS may improve prompt processing for batches over 32 but does not improve
single-token generation.

Do not hard-code logical CPU count as optimal. Add a supported thread override and
sweep 1, 2, 4, physical cores, and logical CPUs per target. Upstream recommends
physical cores when logical-thread oversubscription slows generation.

Expected impact: **medium**, hardware dependent.

### P2: use task-specific output limits and structured sampling

`MAX_NEW_TOKENS = 4096` applies equally to caption, objects, OCR, and all mode.
Successful JSON returns early, but malformed output can consume the entire budget.
Use conservative per-task ceilings, for example:

- caption: 256-512 tokens and an explicit word limit in the prompt;
- objects: 512-1024 tokens and a maximum number of salient objects;
- OCR/all: retain the larger budget based on available context.

Evaluate llama.cpp's JSON-schema grammar sampler (available through the crate's
`common` feature) to guarantee valid structure and avoid pathological malformed
responses. Measure grammar overhead before enabling it.

Expected impact: **low for normal prefill-dominated captions, high for p95 tail
latency**.

### P3: reduce repeated startup and improve throughput

`model::valid_file()` hashes both model files on every new process, despite the
module comment saying cached files are reused by size. Keep full SHA-256 after a
download, then consider a verified sidecar keyed by size and modification time.
That removes about 1.3 s and 1.9 GB of reads from each uncached analysis process.

A persistent worker can retain the runtime across different image calls. It is a
lower priority on CPU because generation dominates, repeated identical images
already hit the result cache, and PDFs already reuse one runtime. It becomes more
valuable after GPU acceleration. Multi-image batching is another throughput
option, but should not increase single-image latency or duplicate the ~4 GB runtime.

### P3: task-specialized fast paths

For workloads dominated by one task, small dedicated models can beat a 2B VLM:
traditional OCR for clean documents, a nano detector for objects, or a smaller
caption model. Route only high-confidence/easy inputs to them and retain Qwen3-VL
as fallback. This is a larger product/quality tradeoff than the runtime tuning
above.

## Low-value or already-covered ideas

- **General image preprocessing optimization:** inference passes encoded bytes
  directly to `MtmdBitmap`; `image::preprocess()` is used for base64 output mode,
  not the Qwen inference path. PDF raw RGB already has a zero-encode fast path.
- **Forcing Flash Attention:** llama.cpp 0.1.151 already defaults it to `AUTO`.
  Verify backend diagnostics before overriding it.
- **Lower language-model precision first:** the language model is already Q4_K_M.
  The F16 projector is the obvious remaining quantization target.
- **A dependency upgrade:** `llama-cpp-2` 0.1.151 is already the latest crates.io
  release at the time of this research.
- **Parallel PDF inference on CPU:** the current runtime/context is mutable and
  serial. Per-thread runtimes would multiply memory and model initialization;
  batching or GPU throughput work should precede parallel contexts.

## Sources

- ReadSeek: `src/engine/vision.rs`, `src/engine/model.rs`,
  `src/engine/vision_cache.rs`, `src/engine/pdf.rs`, `Cargo.toml`, and
  `.github/workflows/release.yml`.
- Qwen3-VL visual-token controls and Flash Attention guidance:
  <https://github.com/QwenLM/Qwen3-VL/blob/main/README.md>
- Official Qwen GGUF file inventory and sizes:
  <https://huggingface.co/api/models/Qwen/Qwen3-VL-2B-Instruct-GGUF/tree/main?recursive=true&expand=true>
- llama.cpp build/backends and BLAS guidance:
  <https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md>
- llama.cpp token-generation thread/offload guidance:
  <https://github.com/ggml-org/llama.cpp/blob/master/docs/development/token_generation_performance_tips.md>
- llama.cpp multimodal architecture:
  <https://github.com/ggml-org/llama.cpp/blob/master/tools/mtmd/README.md>
- Exact local runtime source: `llama-cpp-2` / `llama-cpp-sys-2` 0.1.151,
  vendoring llama.cpp commit `560e06483b48349002d514508162d6d2c688c08f`.
