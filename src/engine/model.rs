// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Qwen3-VL model cache: lazily downloads and SHA-256-verifies the GGUF model
//! and multimodal projection into the user cache directory (`dirs::cache_dir`) on
//! first use. A progress bar is shown while downloading when stdout is a TTY.

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::IsTerminal as _;
use std::path::PathBuf;

const BASE: &str = "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main";

/// `(remote path, local file name, byte size, sha256)`.
const FILES: &[(&str, &str, u64, &str)] = &[
    (
        "Qwen3VL-2B-Instruct-Q4_K_M.gguf",
        "Qwen3VL-2B-Instruct-Q4_K_M.gguf",
        1_107_409_952,
        "089d75c52f4b7ffc56ba998ffc50aae89fcafc755f9e7208aacca281dca6c2ae",
    ),
    (
        "mmproj-Qwen3VL-2B-Instruct-F16.gguf",
        "mmproj-Qwen3VL-2B-Instruct-F16.gguf",
        819_394_848,
        "c3d5afbef5287953acd57b4043d2269456e5761a4eaccb3b71b062996970aea5",
    ),
];

/// Returns the path to cached model file `name`, downloading and verifying it on
/// first use. Subsequent calls reuse the cached file when its size matches,
/// avoiding a full re-hash of multi-gigabyte models on every run.
pub(crate) fn file(name: &str) -> Result<PathBuf> {
    let (remote, local, size, sha) = FILES
        .iter()
        .find(|(_, local, _, _)| *local == name)
        .copied()
        .ok_or_else(|| anyhow!("unknown model file `{name}`"))?;
    let dir = cache_dir()?;
    let target = dir.join(local);
    if fs::metadata(&target).is_ok_and(|meta| meta.len() == size) {
        return Ok(target);
    }
    download(remote, local, &target)?;
    let got = sha256_file(&target);
    if got != sha {
        let _ = fs::remove_file(&target);
        return Err(anyhow!(
            "checksum mismatch for {local}: expected {sha}, got {got}"
        ));
    }
    Ok(target)
}

/// Root cache directory for the Qwen3-VL model files.
fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .context("no user cache directory is available on this platform")?
        .join("readseek")
        .join("qwen3-vl-2b");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

/// Downloads `remote` into `target` via a `.part` file, then atomically renames.
/// A progress bar is shown when stdout is a TTY.
fn download(remote: &str, local: &str, target: &PathBuf) -> Result<()> {
    let url = format!("{BASE}/{remote}");
    let part = target.with_extension("part");
    let response = ureq::get(&url)
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
