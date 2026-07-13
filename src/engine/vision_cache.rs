// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Content-addressed cache for image vision-analysis results under
//! `.readseek/vision/`. Entries are keyed by the BLAKE3 hash of the image bytes
//! and hold the per-task results (caption/objects/OCR) independently, so
//! a later request for a new task reuses tasks computed by earlier runs. A
//! schema version guards against serving results produced by an incompatible
//! cache format or a different vision model; bump it whenever either changes.

use crate::engine::hash;
use crate::engine::vision::DetectedObject;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const VISION_CACHE_DIR: &str = "vision";
const CACHE_SCHEMA_VERSION: u32 = 5;
/// Length of a BLAKE3 hash rendered as lowercase hex.
const HASH_HEX_LEN: usize = 64;

/// Image file extensions recognized for eviction; mirrors the formats probed by
/// [`crate::engine::image`].
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "tif", "avif", "heic", "heif", "ico",
];

/// On-disk cache entry holding every vision task run against one image. A `None`
/// task field means the task has not been run yet; `Some` (even an empty string
/// or empty vec) means it ran and the result is final.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CacheEntry {
    schema_version: u32,
    pub(crate) caption: Option<String>,
    pub(crate) objects: Option<Vec<DetectedObject>>,
    pub(crate) ocr: Option<String>,
}

impl CacheEntry {
    /// A fresh entry with no tasks completed, stamped with the current schema.
    pub(crate) fn new_empty() -> Self {
        Self {
            schema_version: CACHE_SCHEMA_VERSION,
            caption: None,
            objects: None,
            ocr: None,
        }
    }

    /// Whether this entry was produced by the current cache format.
    fn is_valid(&self) -> bool {
        self.schema_version == CACHE_SCHEMA_VERSION
    }
}

/// `.readseek/vision/<hash[..2]>/<hash[2..]>.json`.
fn entry_path(readseek_dir: &Path, hash_hex: &str) -> PathBuf {
    readseek_dir
        .join(VISION_CACHE_DIR)
        .join(&hash_hex[..2])
        .join(format!("{}.json", &hash_hex[2..]))
}

/// Load and validate the cache entry for `hash_hex`. Returns `None` on a missing
/// file, I/O or parse failure, or a schema/model mismatch; all failures are
/// non-fatal so the caller falls through to fresh analysis.
pub(crate) fn load(readseek_dir: &Path, hash_hex: &str) -> Option<CacheEntry> {
    let path = entry_path(readseek_dir, hash_hex);
    if !path.exists() {
        return None;
    }
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(error) => {
            log::warn!("vision cache read {}: {error}", path.display());
            return None;
        }
    };
    let entry: CacheEntry = match serde_json::from_slice(&data) {
        Ok(entry) => entry,
        Err(error) => {
            log::warn!("vision cache parse {}: {error}", path.display());
            return None;
        }
    };
    if !entry.is_valid() {
        log::warn!(
            "vision cache schema/model mismatch in {}, treating as miss",
            path.display()
        );
        return None;
    }
    Some(entry)
}

/// Persist `entry` atomically under `.readseek/vision/`, creating the shard
/// directory if needed. Failures are logged and swallowed so the `detect`
/// command is unaffected by a cache write error.
pub(crate) fn store(readseek_dir: &Path, hash_hex: &str, entry: &CacheEntry) {
    let path = entry_path(readseek_dir, hash_hex);
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        log::warn!("vision cache mkdir {}: {error}", parent.display());
        return;
    }
    let data = match serde_json::to_vec(entry) {
        Ok(data) => data,
        Err(error) => {
            log::warn!("vision cache serialize: {error}");
            return;
        }
    };
    if let Err(error) = crate::engine::repo::write_atomic(&path, &data) {
        log::warn!("vision cache write {}: {error}", path.display());
    }
}

/// Whether `path` has a recognized image extension (case-insensitive).
fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// BLAKE3 hash of an image file, or `None` if the path is not an image. The
/// extension prefilter avoids reading non-image files; [`crate::engine::image`]
/// then validates the bytes.
pub(crate) fn image_hash(path: &Path) -> Option<String> {
    if !has_image_extension(path) {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    crate::engine::image::probe(&bytes)?;
    Some(hash::hash_bytes(&bytes))
}

/// Prune vision entries whose hash is not in `active`. Mirrors
/// `repo::remove_stale_maps` over the `vision/` subdirectory and its `.json`
/// files, reconstructing the hash from the shard prefix and file stem. Tolerates
/// a missing `vision/` directory. Returns the number of entries removed.
pub(crate) fn remove_stale(readseek_dir: &Path, active: &HashSet<String>) -> Result<usize> {
    let mut removed = 0;
    let vision_root = readseek_dir.join(VISION_CACHE_DIR);
    if !vision_root.is_dir() {
        return Ok(0);
    }
    for shard in fs::read_dir(&vision_root)
        .map_err(|error| anyhow::anyhow!("read {}: {error}", vision_root.display()))?
    {
        let shard = shard?;
        if !shard.file_type()?.is_dir() {
            continue;
        }
        let prefix = shard.file_name().to_string_lossy().into_owned();
        for file_entry in fs::read_dir(shard.path())? {
            let file_entry = file_entry?;
            let path = file_entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let hash_hex = format!("{prefix}{stem}");
            if hash_hex.len() == HASH_HEX_LEN
                && hex::decode(&hash_hex).is_ok()
                && !active.contains(&hash_hex)
            {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
    }

    Ok(removed)
}
