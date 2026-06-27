// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Qwen2.5-VL model cache: lazily downloads and SHA-256-verifies the GGUF model
//! and multimodal projection into the user cache directory (`dirs::cache_dir`) on
//! first use. A progress bar is shown while downloading when stdout is a TTY.

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::IsTerminal as _;
use std::path::PathBuf;

const BASE: &str = "https://huggingface.co/unsloth/Qwen2.5-VL-3B-Instruct-GGUF/resolve/main";

/// `(remote path, local file name, sha256)`.
const FILES: &[(&str, &str, &str)] = &[
    (
        "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
        "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
        "c47e8c1f6fb3e8cff6ec58909baff16dbeffb64a5bb3b746b96e05e6334c129f",
    ),
    (
        "mmproj-F16.gguf",
        "mmproj-F16.gguf",
        "4c1240f514de94c81b70709b0f9a80c7e3297598ea7c83f39dc00b18ee5be60c",
    ),
];

/// Returns the path to cached model file `name`, downloading and verifying it on
/// first use. Subsequent calls reuse the cached file when its checksum matches.
pub(crate) fn file(name: &str) -> Result<PathBuf> {
    let (remote, local, sha) = FILES
        .iter()
        .find(|(_, local, _)| *local == name)
        .copied()
        .ok_or_else(|| anyhow!("unknown model file `{name}`"))?;
    let dir = cache_dir()?;
    let target = dir.join(local);
    if target.exists() && sha256_file(&target) == sha {
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

/// Root cache directory for the Qwen2.5-VL model files.
fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .context("no user cache directory is available on this platform")?
        .join("readseek")
        .join("qwen2.5-vl-3b");
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
