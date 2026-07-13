# Research: speeding up readseek's vision inference

Scope: the CPU vision pipeline in `src/engine/vision.rs` + `yolo.rs` + `vision_cache.rs`,
backed by `candle-core` / `candle-nn` / `candle-transformers` 0.11. The three task
models are quantized BLIP caption (q4_K GGUF, autoregressive, "up to a couple of
minutes"), YOLOv8-nano detection (F32 safetensors), and TrOCR printed OCR (F32
safetensors, per-line ViT+BART decode). Sources: arXiv, GitHits (candle-core 0.11.0
source and `ggml-org/llama.cpp`).

## 0. Where time actually goes (bottleneck map)

The README's own numbers are the most important signal: **object detection and OCR
take seconds; captioning takes up to a couple of minutes.** Combined with the code:

- BLIP and TrOCR are **autoregressive decoders**: one full transformer-decoder
  forward per output token, over 256 (BLIP) / 512-per-line (TrOCR) tokens. This is
  the dominant cost — not the image, not the encoder, not preprocessing.
- The ViT image encoder runs **once** per image (BLIP/TrOCR at 384×384 → 576 patches).
- YOLO is a single CNN forward at 640px.
- The three tasks already run concurrently via `std::thread::scope`, and a
  content-addressed (BLAKE3) result cache means repeat runs are free.
- Weights are already **zero-copy**: BLIP via `quantized_var_builder::from_gguf`
  (mmap'd GGUF); YOLO/TrOCR via `VarBuilder::from_mmaped_safetensors` (mmap'd
  safetensors). Activations are candle CPU tensors.

Conclusion up front: of the three ideas proposed, **#1 and #2 are mostly already
handled inside the kernels readseek already calls**, and their application-level
leverage in readseek is small. **#3 is partially true and has real, model-specific
nuance.** The largest *readseek-specific* wins are elsewhere (quantizing the F32
models, speculative decode, token merging) and are detailed in §4.

---

## 1. Idea 1 — Zero-copy, cache-line-aligned memory layout

### What is already true in readseek's stack

- **Weights are zero-copy.** mmap'd GGUF/safetensors means model weights never get
  copied into the process heap; the OS page cache + virtual memory give them to
  candle directly. This part of the idea is already realized.
- **Activations are plain `Vec<T>`**, not cache-line aligned. candle's CPU storage is
  the enum `CpuStorage { F32(Vec<f32>), F16(Vec<f16>), … }`
  (`candle-core@0.11.0` `src/cpu_backend/mod.rs:22`). A `Vec<f32>` is 4-byte
  aligned, not 64-byte (cache-line) aligned, and there is no aligned sub-allocator
  (grepping `crates:candle-core@0.11.0` for `alloc::Layout` returns nothing).
- **The hot matmul already tiles for cache internally.** With the `mkl`/`accelerate`
  features off (which is exactly readseek's current config on Linux), candle's CPU
  path is:
  `candle-core/src/cpu_backend/mod.rs:1331` `struct MatMul` →
  `impl Map2 for MatMul { fn f(...) { use gemm::{gemm, Parallelism}; … gemm(m, n, k, …,
  Parallelism::Rayon(num_threads)) } }`. The **`gemm` crate** is a hand-tiled,
  microkernel-based GEMM with AVX2/AVX512/NEON vectorization and explicit L1/L2
  cache blocking. The IO-aware, cache-tiled GEMM the idea gestures at is *already the
  code path readseek uses for every matmul in BLIP/TrOCR/YOLO.*
- **The quantized path already uses packed, cache/SIMD-shaped blocks.** readseek's
  BLIP runs q4_K. In ggml the q4_K super-block is 144 bytes
  (`QK_K=256`), and there are **repacked** layouts
  `block_q4_Kx8` / `block_q4_Kx16` that interleave 8/16 super-blocks' scales and
  quants contiguously with `static_assert`s pinning the byte sizes
  (`llama.cpp` `ggml/src/ggml-cpu/repack.h:43-65`). This is precisely "reorganize the
  memory layout so a load feeds a whole SIMD width and stays in a cache line" — done
  at the kernel layer, the right place for it.

### What the literature says

FlashAttention (Dao, 2205.14135; v2 2307.08691; v3 2407.08608) is the canonical
"IO-aware, tiling for the cache hierarchy" result: it avoids materializing the
N×N attention matrix and fuses softmax with block-wise matmuls to minimize
HBM↔SRAM traffic. The principle — *tiling the computation so each tile fits in the
fast cache level and reads/writes the slow level once* — is exactly the L1-cache
instinct behind this idea, and it belongs in the matmul/attention kernel, not the
application tensor.

### Verdict for readseek

- **Application-level cache-line alignment of activations: negligible leverage.**
  The expensive reads are the weight tiles inside `gemm`/ggml microkernels, which
  already block for L1/L2. Aligning the top-level `Vec<f32>` activation to 64 bytes
  would at best shave a single line-fetch per tensor per op — invisible next to a
  256-token decode loop. Touching candle internals is out of scope for readseek.
- **The one place alignment/posture already pays:** it's the *quantized* weight
  layout. readseek already gets it for BLIP. The actionable corollary is in §4:
  bring the F32 YOLO/TrOCR onto the same packed-quant kernels (idea #1 is literally
  realized by q4_K/q8_0 block layout).
- **Cheap hygiene win in readseek code:** the preprocessing builds the input tensor
  via `Tensor::from_vec(img.into_raw(), (H,W,3), …)?.permute((2,0,1))?` then
  `/255`, `broadcast_sub`, `broadcast_div`. `permute` produces a non-contiguous
  strided view; the subsequent elementwise ops may force a materialization each.
  Fusing the resize→permute→normalize into a single pass over the RGB buffer
  avoids 2–3 allocations per image. This matters for *latency hygiene*, not throughput.

---

## 2. Idea 2 — "Texture-mapping" 8×8 tile (Morton/Z-order) reorg of the image

### What is already true in readseek's stack

- **candle's CPU conv2d already tiles.** `candle-core/src/cpu_backend/conv2d.rs`:
  `DEFAULT_CONV2D_IMPL = Conv2dImpl::TiledIm2Col`, and the comment on
  `conv2d_tiled` is *"instead of materializing the full matrix, we process
  input/output in tiles, in parallel"* with an explicit **"Convert NCHW input to NHWC
  layout for tiled im2col"** step (`conv2d.rs:142`) and a rayon parallel loop. So the
  framework already performs the 2D-spatial reorganization (NCHW→NHWC + tiled
  im2col) that app-level image tiling would attempt to provide.
- **The ViT patch-embed consumes patches, not a raster.** BLIP/TrOCR vision encoders
  start with a `Conv2d(kernel=16, stride=16)` — effectively a patch unfolding into a
  (tokens × channels) matmul. That conv *is* the reorg into 16×16 tiles; a
  user-level 8×8 Morton reorg of the source image would be undone immediately by the
  im2col/patch-embed, which indexes pixels by (y,x) coordinates.
- **Image preprocessing is negligible.** Resize + a single f32 conversion of a
  ~0.4 MP image is sub-millisecond-to-low-millisecond; the model forward is seconds
  to minutes. A layout transform on the source image can only ever shave the
  negligible part.

### What the literature says

- Z-order/Morton ("texture swizzle") locality is real and used in cache-sensitive
  2D-stencil workloads — e.g. an NeRF edge coprocessor clusters rays by Z-order and
  reorders ray-packets by spatial proximity to cut cache misses
  (arXiv 2510.07667). It pays when *you own the 2D stencil kernel*.
- FlashAttention (2205.14135) is the modern form of the same idea applied to
  attention: **tile the Q/K/V blocks** (typically 64–128 elements) so each tile's
  working set fits in the on-chip cache. This is "8×8 tiling for cache hit rate,"
  applied where the bandwidth actually is.

### Verdict for readseek

- **App-level image tiling: do not pursue.** It would add a copy and a coordinate
  remap that candle's `TiledIm2Col`/patch-embed then re-derives anyway, while the
  real bandwidth is inside `gemm`/attention, which the app cannot reach.
- The principle is sound; the *implementation site* must be the kernel
  (`gemm` microkernel / FlashAttention-style attention tiling / ggml `block_q4_Kx8`
  repack). readseek already inherits the kernel versions of this.
- If readseek ever ships its *own* vision kernel (unlikely, given candle), then
  NHWC + 16×16 blocked access matching the patch size would be the right layout —
  i.e., exactly what candle already does.

---

## 3. Idea 3 — Compile-time max W/H from model capability, downscale before inference

### What is already true / the key nuance

- readseek **already downscales before inference** for every model:
  BLIP `resize_to_fill(384,384)`, TrOCR `resize_exact(384,384)`, YOLO resizes the
  longer side to 640 (32-divisible). So the "downscale before inference" step exists.
- **BLIP and TrOCR vision encoders are *fixed-resolution* ViTs.** patch=16 at 384 →
  a 24×24 = 576-token grid, and the positional embeddings are trained for that grid.
  You **cannot feed a smaller image to save time** without interpolating the
  positional embeddings (the DeiT/timm `resize_pos_embed` trick), and even then the
  model was trained at 384, so quality drifts. NaViT (arXiv 2307.06304, "Patch n'
  Pack") calls fixed-res resizing "ubiquitous and demonstrably suboptimal" and shows
  arbitrary-resolution ViTs are a *training-recipe* change, not a drop-in inference
  tweak for a pretrained fixed-grid model.
- **Crucially, the encoder isn't where BLIP spends its time anyway** — the
  autoregressive decoder (256 forwards) dominates. So shrinking the ViT input for
  BLIP/TrOCR would barely move caption latency.
- **YOLO is the opposite: anchor-free, freely resizable** (just needs 32-divisible
  strides). Lowering its 640px ceiling is a real, low-risk knob: FLOPs scale with
  H·W, so 480 or 384 roughly halves/quarters the detection forward.

### What the literature says

- NaViT / Patch n' Pack (2307.06304): native-resolution ViT via sequence packing —
  a *training* change; not applicable to readseek's pretrained weights, but it is
  the principled answer to "variable input resolution."
- ViT quantization (FQ-ViT 2111.13824, I-ViT 2207.01405, AdaLog 2407.12951): int8 /
  integer-only inference for ViTs — relevant because readseek's TrOCR encoder is a
  ViT and is currently F32 (see §4a).

### Verdict for readseek

- **Promote the magic numbers to named compile-time constants keyed to the model
  config** — good, cheap, currently literally inlined (`384`, `640`, `256`, `512`).
  This is hygiene/type-safety, not a speedup by itself.
- **Make YOLO input resolution a knob** (compile-time const with an env override):
  genuine, easy speed/accuracy dial for the one flexible model.
- **Decode-downscale large source images in one pass.** readseek currently does
  `image::load_from_memory` (decodes full-res) *then* `resize_to_fill`. For a 4032×3024
  phone photo this decodes ~12 MP just to throw it away. The `image` crate supports
  requesting a downscaled decode (JPEG DCT 1/2/1/4/1/8 scaling, and dimension limits)
  so the full-res buffer is never materialized. This is the genuinely useful
  instance of idea #3: cap the *decoded* size to the model's max (384 long-edge for
  ViTs) before any pixel work.
- **Do not** try to run BLIP/TrOCR below 384 to save encoder time; tiny gain,
  accuracy risk, and not the bottleneck.

---

## 4. The bigger, readseek-specific levers (research-backed)

### 4a. Quantize the F32 models to q8_0 / q4_K — highest leverage, lowest effort, and *it is idea #1 realized*

- BLIP is already q4_K and benefits from ggml's packed block layout
  (`block_q4_Kx8/x16`, §1). **TrOCR and YOLO are F32.**
- TrOCR = ViT encoder + BART decoder, both transformer matmuls → **trivially
  quantizable** onto the same `candle::quantized::k_quants::matmul` path BLIP already
  uses (we can see `pub fn matmul<T: GgmlType>` and `matmul_q4k_x8` in
  `candle-core/src/quantized/k_quants.rs`). Bandwidth drops ~4× (q4_K) / ~2× (q8_0),
  which is the binding constraint on CPU decode.
- TrOCR is the second-slowest task and the biggest weight (1.24 GB F32) — this is
  the single best ROI in the codebase.
- YOLO is small (6 MB) and conv-heavy; candle's quantized path is matmul-oriented
  and quantized `Conv2d` support is limited. Lower priority; F32 YOLO is already
  "seconds." F16 (not integer) would still halve its bandwidth and is simpler.
- Cost: convert safetensors→GGUF (candle ships a `quantize` tool), bump
  `CACHE_SCHEMA_VERSION` in `vision_cache.rs` (the schema guard already exists for
  exactly this). Accuracy: q8_0 is near-lossless; q4_K has measurable but usually
  acceptable error for these models.

### 4b. Speculative decoding for the autoregressive decoders — attacks the actual bottleneck

- Speculative decoding (Leviathan et al., arXiv 2211.17192, "Fast Inference from
  Transformers via Speculative Decoding") autoregresses a cheap *draft* model and
  verifies K tokens in one parallel pass of the target — lossless, 2–3× on
  memory-bound decode.
- **Directly validated for vision-language models:** "On Speculative Decoding for
  Multimodal LLMs" (2404.08856) shows a **2.37× speedup on LLaVA-7B using a 115M
  language-only draft model** — the draft skips the image branch entirely. The
  analogue for readseek: a small distilled BLIP/TrOCR decoder (or even an n-gram
  draft per the multilingual spec-decode work, arXiv 2605.30580) drafted against the
  q4_K target.
- **DEED (Dynamic Early Exit on Decoder, 2311.08623)** is purpose-built for
  encoder-decoder VL models (exactly BLIP/TrOCR's shape): trains per-layer exit
  heads so easy tokens exit early. LayerSkip (2404.16710) does the same as
  self-speculative decode for LLMs. These target the minutes-long caption path
  directly.
- Effort is higher than 4a (needs a draft model or exit-head training), but the
  ceiling is 2–3× on the slowest task.

### 4c. Token merging (ToMe) on the ViT encoders — training-free, ~2×, tiny acc loss

- "Token Merging: Your ViT But Faster" (Bolya, arXiv 2210.09461): a training-free
  bipartite merge of similar tokens each block — **2× throughput on ViT-L@512 with
  0.2–0.3% accuracy drop**, applied to off-the-shelf models. "ToMe for Stable
  Diffusion" (2303.17604) shows the same recipe ported to U-Net/transformer stacks
  with up to 60% token reduction.
- In readseek this shrinks the BLIP/TrOCR **encoder token count (576)** and the
  cross-attention cost the decoder pays against those embeddings every token step.
  The decoder autoregression is still the main cost, so pair it with 4b, but it is
  free quality-leaning speed on the encoder.
- Effort: insert a ToMe merge step between ViT blocks; needs hooking into
  `quantized_blip`/`trocr` forward, so non-trivial in candle but no retraining.

### 4d. BLAS + thread posture

- candle-core 0.11 has optional `accelerate` (macOS) and `mkl` (x86) features;
  readseek enables **neither** (only `metal` on macOS). When Metal is unavailable
  (the documented CPU fallback in `best_device()`), matmul falls to the `gemm` crate
  — decent and tiled, but `accelerate-src` is typically faster on Apple Silicon and
  would also help the CPU path. On Linux, `mkl`/an OpenBLAS-flavored GEMM is the
  equivalent knob (heavier to vendor).
- Thread count: `candle::utils::get_num_threads()` honors rayon's env (`RAYON_NUM_THREADS`)
  and on macOS detects **P-cores** (`hw.perflevel0.logicalcpu`, `utils.rs`). Two
  concerns:
  - The three tasks already spawn via `std::thread::scope` **and** each task's matmul
    pulls from a shared rayon pool → possible oversubscription on small-core
    machines. Forcing heavy tasks to P-cores / limiting per-task threads can help.
  - BLIP's per-token decode is short and latency-bound; many threads hurt more than
    help there. A tuned `RAYON_NUM_THREADS` (≈ P-core count) for the decode phase is
    worth measuring.

### 4e. Minor / hygiene

- Fuse the resize→permute→normalize preprocessing into one pass (§1) — tiny but free.
- TrOCR per-crop decode already early-stops on EOS; BLIP could add a repetition /
  low-confidence early stop before the 256 ceiling.
- YOLO `resize_exact` uses `CatmullRom` (expensive bicubic); `Triangle` (bilinear) is
  used for the ViTs and is cheaper and adequate.
- The content cache key is BLAKE3 of the *full image bytes* — if 4a changes the
  model, the `schema_version` bump invalidates cleanly (already designed for this).

---

## 5. Prioritized recommendations

| # | Lever | Maps to idea | Effort | Expected gain on the slow path | Evidence |
|---|-------|--------------|--------|--------------------------------|----------|
| 1 | Quantize TrOCR (q8_0/q4_K GGUF) onto candle's `k_quants::matmul` (#1 realized at kernel level) | #1 | Med | Large (TrOCR is 2nd-slowest, 1.24 GB F32) | `candle-core/src/quantized/k_quants.rs`; ggml `block_q4_Kx8` (`repack.h`) |
| 2 | Speculative decode for BLIP + TrOCR decode | — | High | 2–3× on caption/OCR | 2211.17192; 2404.08856 (2.37×); 2311.08623 (DEED) |
| 3 | ToMe on BLIP/TrOCR ViT encoders | — | Med | ~up to 2× encoder; pairs w/ #2 | 2210.09461; 2303.17604 |
| 4 | Decode-downscale huge images in one pass (cap to model long-edge pre-decode) | #3 | Low | Removes full-res decode of big photos | `image` crate decoder scaling |
| 5 | YOLO input resolution as a knob (lower than 640) | #3 | Low | ~scales with H·W | YOLOv8 is anchor-free/32-divisible |
| 6 | Enable `accelerate` on macOS CPU path; tune `RAYON_NUM_THREADS` / P-cores | #1 (adjacent) | Low | Modest, esp. Metal-unavailable macs | candle `MatMul` → `gemm`; `feature = "accelerate"` |
| 7 | Promote 384/640/256/512/16 to named consts from model config | #3 | Trivial | Hygiene only | `vision.rs` literals |
| — | App-level cache-line alignment of activations | #1 | — | **Not worth it** (already in `gemm`) | `CpuStorage = Vec<f32>`; `gemm` tiles |
| — | App-level 8×8 / Morton image tiling | #2 | — | **Not worth it** (conv/patch-embed re-derives layout) | `conv2d.rs TiledIm2Col`; FlashAttention 2205.14135 |

## 6. References

**arXiv**
- FlashAttention (IO-aware, tiling): 2205.14135; v2 2307.08691; v3 2407.08608.
- Token Merging (ToMe): 2210.09461; ToMe for Stable Diffusion 2303.17604; adjacent
  token-merging 2508.00367, 2306.16009.
- Native/flexible resolution ViT: NaViT / Patch n' Pack 2307.06304.
- Speculative decoding: Leviathan "Fast Inference via Speculative Decoding" 2211.17192;
  SpecDec 2203.16487; "On Speculative Decoding for MLLMs" 2404.08856 (2.37×);
  Multilingual spec-decode / n-gram drafts 2605.30580.
- Early exit / self-speculative: LayerSkip 2404.16710; DEED (enc-decoder VL) 2311.08623;
  ADEPT 2601.03700; BEExformer 2412.05225.
- ViT quantization: FQ-ViT 2111.13824; I-ViT 2207.01405; Patch-wise mixed-precision
  2305.06559; AdaLog 2407.12951.
- Z-order spatial locality (NeRF ray cache): 2510.07667.

**GitHits source (evidence)**
- `crates:candle-core@0.11.0`
  - `src/cpu_backend/mod.rs:22` — `CpuStorage` is `Vec<f32>`-based (no cache-line alignment).
  - `src/cpu_backend/mod.rs:1331` `struct MatMul`; `impl Map2 for MatMul` →
    `use gemm::{gemm, Parallelism}` with `Parallelism::Rayon(num_threads)` (the
    CPU GEMM that already tiles/SIMD-blocks for cache).
  - `src/cpu_backend/conv2d.rs:21` `DEFAULT_CONV2D_IMPL = TiledIm2Col`;
    `conv2d_tiled` converts NCHW→NHWC and tiles in parallel (rayon).
  - `src/quantized/k_quants.rs:2375` `pub fn matmul<T: GgmlType>`; `:2579 matmul_q4k_x8`
    (the packed-quant kernel path readseek's BLIP already uses).
  - `src/utils.rs:343` `get_num_threads()` honors rayon env; macOS P-core detection.
  - `Cargo.toml` — optional `mkl`/`accelerate` features (readseek enables neither
    off macOS).
- `github:ggml-org/llama.cpp`
  - `ggml/src/ggml-cpu/repack.h:43,51` — `block_q4_Kx8`/`block_q4_Kx16` repacked
    quantized layouts with `static_assert` byte sizes (cache+SIMD-shaped blocks:
    idea #1/#2 realized at the kernel layer).
  - `ggml/include/ggml.h:267` `#define GGML_PAD` (row/alignment padding).
