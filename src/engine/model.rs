// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Qwen3-VL model cache. The quantized text model and multimodal projector are
//! downloaded and SHA-256-verified on first use, then reused by file size.

use std::fs;
use std::io::IsTerminal as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};

const BASE: &str = "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main";
const CACHE_SUBDIR: &str = "models";

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

/// Return a cached model file, downloading and verifying it when absent.
pub(crate) fn file(name: &str) -> Result<PathBuf> {
    let (remote, local, size, sha256) = FILES
        .iter()
        .find(|(_, local, _, _)| *local == name)
        .copied()
        .ok_or_else(|| anyhow!("unknown model file `{name}`"))?;
    let dir = cache_dir()?.join(CACHE_SUBDIR);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let target = dir.join(local);
    if fs::metadata(&target).is_ok_and(|metadata| metadata.len() == size) {
        return Ok(target);
    }

    download(remote, local, &target)?;
    let actual = sha256_file(&target);
    if actual != sha256 {
        let _ = fs::remove_file(&target);
        return Err(anyhow!(
            "checksum mismatch for {local}: expected {sha256}, got {actual}"
        ));
    }
    Ok(target)
}

fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .context("no user cache directory is available on this platform")?
        .join("readseek");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

/// Download through a temporary file and atomically install the result.
fn download(remote: &str, local: &str, target: &PathBuf) -> Result<()> {
    let url = format!("{BASE}/{remote}");
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

    if std::io::stdout().is_terminal() {
        let progress = MultiProgress::new();
        let bar = match total {
            Some(length) => ProgressBar::new(length),
            None => ProgressBar::new_spinner(),
        };
        bar.set_style(
            ProgressStyle::with_template(
                "{prefix:<36} {bar:30} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> "),
        );
        bar.set_prefix(local.to_owned());
        let bar = progress.add(bar);
        let mut wrapped = bar.wrap_read(&mut reader);
        std::io::copy(&mut wrapped, &mut file).with_context(|| format!("read {url}"))?;
        bar.finish_with_message(format!("{local} done"));
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
