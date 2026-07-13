// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Vision model cache: lazily downloads and SHA-256-verifies the BLIP
//! caption model (GGUF + tokenizer), the YOLOv8-nano object-detection weights,
//! and the `TrOCR` printed-text recognition model (weights + config + tokenizer)
//! into the `models/` subdirectory of the user cache (`dirs::cache_dir`) on
//! first use. A progress bar is shown while downloading when stdout is a TTY.

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::IsTerminal as _;
use std::path::PathBuf;

const HF_BASE: &str = "https://huggingface.co";
const CACHE_SUBDIR: &str = "models";

/// `(repo, remote path, local file name, cache subdir, byte size, sha256)`.
/// `(repo, revision, remote path, local file name, byte size, sha256)`.
const FILES: &[(&str, &str, &str, &str, u64, &str)] = &[
    (
        "lmz/candle-blip",
        "main",
        "blip-image-captioning-large-q4k.gguf",
        "blip-image-captioning-large-q4k.gguf",
        270_847_360,
        "c7f7a3e19a562c0cfef02d023562705050fa555a79296f5d44d5047167571533",
    ),
    (
        "Salesforce/blip-image-captioning-large",
        "main",
        "tokenizer.json",
        "blip-tokenizer.json",
        711_396,
        "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    ),
    (
        "lmz/candle-yolo-v8",
        "main",
        "yolov8n.safetensors",
        "yolov8n.safetensors",
        6_369_332,
        "5788ff529e26961281ebeb26facecaea38ec9a79a3ad2282995ab899eb905626",
    ),
    (
        "microsoft/trocr-base-printed",
        "refs/pr/7",
        "model.safetensors",
        "trocr-base-printed.safetensors",
        1_333_384_464,
        "1cf4a6eedab26afaaf505f1c7f73d9634944924dbd1ed049d569db98039cd596",
    ),
    (
        "microsoft/trocr-base-printed",
        "refs/pr/7",
        "config.json",
        "trocr-config.json",
        4_126,
        "5bda1deab455661feb3d91906656e5600e2ca520d5c00a2a03836614b850c93e",
    ),
    (
        "ToluClassics/candle-trocr-tokenizer",
        "main",
        "tokenizer.json",
        "trocr-tokenizer.json",
        2_108_614,
        "2f1a555a1ee93656b4e6f67aa75d492a843c225e5ef754bae24c36bd85851cd7",
    ),
];

/// Returns the path to cached model file `name`, downloading and verifying it
/// on first use. Subsequent calls reuse the cached file when its size matches,
/// avoiding a full re-hash of multi-gigabyte models on every run.
pub(crate) fn file(name: &str) -> Result<PathBuf> {
    let (repo, revision, remote, local, size, sha) = FILES
        .iter()
        .find(|(_, _, _, local, _, _)| *local == name)
        .copied()
        .ok_or_else(|| anyhow!("unknown model file `{name}`"))?;
    let dir = cache_dir()?.join(CACHE_SUBDIR);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let target = dir.join(local);
    if fs::metadata(&target).is_ok_and(|meta| meta.len() == size) {
        return Ok(target);
    }
    download(repo, revision, remote, local, &target)?;
    let got = sha256_file(&target);
    if got != sha {
        let _ = fs::remove_file(&target);
        return Err(anyhow!(
            "checksum mismatch for {local}: expected {sha}, got {got}"
        ));
    }
    Ok(target)
}

/// Root cache directory for the vision model files.
fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .context("no user cache directory is available on this platform")?
        .join("readseek");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

/// Path under the readseek model cache directory for a locally produced file
/// that is not downloaded from a registry — e.g. the runtime-quantized `TrOCR`
/// `q4_K` GGUF, derived from the SHA-256-verified F32 safetensors and reused
/// across runs so the one-time quantization cost is paid only once.
pub(crate) fn local_cache_path(name: &str) -> Result<PathBuf> {
    let dir = cache_dir()?.join(CACHE_SUBDIR);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir.join(name))
}

/// Downloads `remote` from `repo` into `target` via a `.part` file, then
/// atomically renames. A progress bar is shown when stdout is a TTY.
fn download(repo: &str, revision: &str, remote: &str, local: &str, target: &PathBuf) -> Result<()> {
    let url = format!(
        "{HF_BASE}/{repo}/resolve/{}/{}",
        revision.replace('/', "%2F"),
        remote,
    );
    let part = target.with_extension("part");
    let agent = ureq::Agent::with_parts(
        ureq::config::Config::default(),
        ureq::unversioned::transport::DefaultConnector::default(),
        crate::engine::resolver::DohResolver,
    );
    let response = agent
        .get(&url)
        .call()
        .with_context(|| format!("download {url}"))?;
    let total = response.body().content_length();

    let mut file = fs::File::create(&part).with_context(|| format!("create {}", part.display()))?;
    let mut body = response.into_body();
    let mut reader = body.as_reader();

    let tty = std::io::stdout().is_terminal();
    if tty {
        let mp = MultiProgress::new();
        let pb = match total {
            Some(len) => ProgressBar::new(len),
            None => ProgressBar::new_spinner(),
        };
        pb.set_style(
            ProgressStyle::with_template(
                "{prefix:<24} {bar:30} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> "),
        );
        pb.set_prefix(local.to_string());
        let pb = mp.add(pb);
        let mut wrapped = pb.wrap_read(&mut reader);
        std::io::copy(&mut wrapped, &mut file).with_context(|| format!("read {url}"))?;
        pb.finish_with_message(format!("{local} done"));
    } else {
        std::io::copy(&mut reader, &mut file).with_context(|| format!("read {url}"))?;
    }
    file.sync_all()
        .with_context(|| format!("sync {}", part.display()))?;
    drop(file);
    fs::rename(&part, target)
        .with_context(|| format!("rename {} -> {}", part.display(), target.display()))?;
    Ok(())
}

fn sha256_file(path: &PathBuf) -> String {
    match fs::read(path) {
        Ok(bytes) => hex::encode(Sha256::digest(&bytes)),
        Err(_) => String::new(),
    }
}
