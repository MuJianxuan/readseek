// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! SmolVLM-500M model cache: lazily downloads and SHA-256-verifies the GGUF model
//! and multimodal projection into the user cache directory (`dirs::cache_dir`) on
//! first use. A progress bar is shown while downloading when stdout is a TTY.

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::IsTerminal as _;
use std::path::PathBuf;

const BASE: &str = "https://huggingface.co/ggml-org/SmolVLM-500M-Instruct-GGUF/resolve/main";

/// `(remote path, local file name, byte size, sha256)`.
const FILES: &[(&str, &str, u64, &str)] = &[
    (
        "SmolVLM-500M-Instruct-Q8_0.gguf",
        "SmolVLM-500M-Instruct-Q8_0.gguf",
        436_806_912,
        "9d4612de6a42214499e301494a3ecc2be0abdd9de44e663bda63f1152fad1bf4",
    ),
    (
        "mmproj-SmolVLM-500M-Instruct-f16.gguf",
        "mmproj-SmolVLM-500M-Instruct-f16.gguf",
        199_468_800,
        "4b2fca922e549e9b9608f4ba56f93565dd5220e935ba5570a2a864a0fcc988a6",
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

/// Root cache directory for the SmolVLM-500M model files.
fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .context("no user cache directory is available on this platform")?
        .join("readseek")
        .join("smolvlm-500m");
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
