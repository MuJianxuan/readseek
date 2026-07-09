// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Vision model cache: lazily downloads and SHA-256-verifies the Moondream
//! caption model (GGUF + tokenizer), the YOLOv8-nano object-detection weights,
//! and the ocrs text detection/recognition models into the user cache
//! directory (`dirs::cache_dir`) on first use. A progress bar is shown while
//! downloading when stdout is a TTY.

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::IsTerminal as _;
use std::path::PathBuf;

const HF_BASE: &str = "https://huggingface.co";

/// `(repo, remote path, local file name, cache subdir, byte size, sha256)`.
const FILES: &[(&str, &str, &str, &str, u64, &str)] = &[
    (
        "santiagomed/candle-moondream",
        "model-q4_0.gguf",
        "moondream-q4_0.gguf",
        "moondream",
        1_513_740_160,
        "cdde43dcf5f4249111ad36cadd8810ea88bf3aabb33de2bab0146e50a31d78c0",
    ),
    (
        "santiagomed/candle-moondream",
        "tokenizer.json",
        "tokenizer.json",
        "moondream",
        2_114_924,
        "337da36be7a71a6e88aa9148967a7bc8736f4b47c7de8e19ba92b89e80734cfc",
    ),
    (
        "lmz/candle-yolo-v8",
        "yolov8n.safetensors",
        "yolov8n.safetensors",
        "yolov8n",
        6_369_332,
        "5788ff529e26961281ebeb26facecaea38ec9a79a3ad2282995ab899eb905626",
    ),
    (
        "robertknight/ocrs",
        "text-detection-ssfbcj81.rten",
        "text-detection.rten",
        "ocrs",
        2_523_564,
        "614aafabf27c94d386f7aa036c967c2e47e4b9938fa11531ca8f5698c1ca4c36",
    ),
    (
        "robertknight/ocrs",
        "text-rec-checkpoint-s52qdbqt.rten",
        "text-recognition.rten",
        "ocrs",
        9_716_444,
        "606d9a0414c6b73c99df75b707c11c70d1c8b12e1d4f900922e185fc37bfca65",
    ),
];

/// Returns the path to cached model file `name`, downloading and verifying it
/// on first use. Subsequent calls reuse the cached file when its size matches,
/// avoiding a full re-hash of multi-gigabyte models on every run.
pub(crate) fn file(name: &str) -> Result<PathBuf> {
    let (repo, remote, local, subdir, size, sha) = FILES
        .iter()
        .find(|(_, _, local, _, _, _)| *local == name)
        .copied()
        .ok_or_else(|| anyhow!("unknown model file `{name}`"))?;
    let dir = cache_dir()?.join(subdir);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let target = dir.join(local);
    if fs::metadata(&target).is_ok_and(|meta| meta.len() == size) {
        return Ok(target);
    }
    download(repo, remote, local, &target)?;
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

/// Downloads `remote` from `repo` into `target` via a `.part` file, then
/// atomically renames. A progress bar is shown when stdout is a TTY.
fn download(repo: &str, remote: &str, local: &str, target: &PathBuf) -> Result<()> {
    let url = format!("{HF_BASE}/{repo}/resolve/main/{remote}");
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
