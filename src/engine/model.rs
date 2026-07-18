// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Qwen3-VL model cache. The quantized text model and multimodal projector are
//! downloaded and SHA-256-verified on first use, then reused by file size.

use std::fs;
use std::io::{BufReader, IsTerminal as _, Read as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};

const BASE: &str = "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main";
const CACHE_SUBDIR: &str = "models";
const LOCK_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
    let _lock = ModelLock::acquire(&target.with_extension("file-lock"))?;
    if let Err(error) = remove_stale_parts(&target) {
        log::warn!("stale model download cleanup skipped: {error:#}");
    }
    if valid_file(&target, size, sha256) {
        return Ok(target);
    }
    let _ = fs::remove_file(&target);

    download(remote, local, &target)?;
    Ok(target)
}

fn cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .context("no user cache directory is available on this platform")?
        .join("readseek");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

/// Download a verified model through a unique temporary file.
fn download(remote: &str, local: &str, target: &PathBuf) -> Result<()> {
    let url = format!("{BASE}/{remote}");
    let part = unique_part(target)?;
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

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .with_context(|| format!("create {}", part.display()))?;
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
    let (_, _, size, sha256) = FILES
        .iter()
        .find(|(_, name, _, _)| *name == local)
        .copied()
        .expect("download only receives known model files");
    if !valid_file(&part, size, sha256) {
        let actual = sha256_file(&part).unwrap_or_default();
        let _ = fs::remove_file(&part);
        return Err(anyhow!(
            "checksum mismatch for {local}: expected {sha256}, got {actual}"
        ));
    }
    fs::rename(&part, target)
        .with_context(|| format!("rename {} -> {}", part.display(), target.display()))?;
    Ok(())
}

fn sha256_file(path: &PathBuf) -> Result<String> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn valid_file(path: &PathBuf, expected_size: u64, expected_sha256: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.len() == expected_size)
        && sha256_file(path).is_ok_and(|actual| actual == expected_sha256)
}

fn unique_part(target: &Path) -> Result<PathBuf> {
    static NEXT_PART: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_PART.fetch_add(1, Ordering::Relaxed);
    let part = target.with_extension(format!("{}.{}.part", std::process::id(), sequence));
    if part.exists() {
        return Err(anyhow!(
            "temporary model file already exists: {}",
            part.display()
        ));
    }
    Ok(part)
}

fn remove_stale_parts(target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .with_context(|| format!("model path has no parent: {}", target.display()))?;
    let stem = target
        .file_stem()
        .with_context(|| format!("model path has no file name: {}", target.display()))?
        .to_string_lossy();
    let prefix = format!("{stem}.");
    for entry in fs::read_dir(parent).with_context(|| format!("read {}", parent.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".part") {
            fs::remove_file(entry.path()).with_context(|| {
                format!("remove stale model download {}", entry.path().display())
            })?;
        }
    }
    Ok(())
}

struct ModelLock {
    _file: fs::File,
}

impl ModelLock {
    fn acquire(path: &Path) -> Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("open model cache lock {}", path.display()))?;
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(fs::TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(anyhow!(
                            "timed out waiting for model cache lock {}",
                            path.display()
                        ));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(fs::TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("lock {}", path.display()));
                }
            }
        }
    }
}
