// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::engine::flags::GitFlags;
use crate::engine::hash::LineHash;
use crate::engine::lang::{AnalysisEngine, Language};
use crate::engine::source::{SourceFile, SourceMap, Symbol};
use crate::engine::symbols;
use anyhow::{Context, Result, bail};
use crc::CRC_32_ISO_HDLC;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zerocopy::byteorder::{LittleEndian, U16, U32};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

const READSEEK_DIR: &str = ".readseek";
const MAPS_DIR: &str = "maps";
const DEF_INDEX_DIR: &str = "def-index";
const SHARD_COUNT: u32 = 256;
const MAGIC: [u8; 4] = *b"RSMP";
const SCHEMA_VERSION: u32 = 6;

const HEADER_SIZE: usize = size_of::<Header>();
const SYM_ENTRY_SIZE: usize = size_of::<SymEntry>();
const BLAKE3_RAW_LEN: usize = 32;
const ENGINE_TAG_NONE: u8 = 0xff;
const CHECKSUM_OFFSET: usize = offset_of!(Header, checksum);

const _: () = assert!(
    crate::engine::hash::HASHLINE_MODULUS <= 0x10000,
    "HASHLINE_MODULUS must fit in a u16 for binary format storage"
);

#[derive(FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
struct Header {
    magic: [u8; 4],
    version: U32<LittleEndian>,
    sym_count: U32<LittleEndian>,
    strtab_sz: U32<LittleEndian>,
    file_hash: [u8; BLAKE3_RAW_LEN],
    checksum: U32<LittleEndian>,
    lang_tag: U16<LittleEndian>,
    engine_tag: u8,
    _reserved: [u8; 9],
}

#[derive(FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
struct SymEntry {
    kind_off: U32<LittleEndian>,
    name_off: U32<LittleEndian>,
    qname_off: U32<LittleEndian>,
    start_line: U32<LittleEndian>,
    end_line: U32<LittleEndian>,
    start_byte: U32<LittleEndian>,
    end_byte: U32<LittleEndian>,
    name_byte: U32<LittleEndian>,
    kind_len: U16<LittleEndian>,
    name_len: U16<LittleEndian>,
    qname_len: U16<LittleEndian>,
    start_hash: U16<LittleEndian>,
    end_hash: U16<LittleEndian>,
}

const _: () = assert!(size_of::<Header>() == 64, "Header must be exactly 64 bytes");
const _: () = assert!(
    size_of::<SymEntry>() == 42,
    "SymEntry must be exactly 42 bytes",
);

#[derive(Debug, serde::Serialize)]
pub(crate) struct UpdateStats {
    pub(crate) created: usize,
    pub(crate) removed: usize,
    pub(crate) unchanged: usize,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct DefIndexEntry {
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) file_hash: String,
    pub(crate) path: PathBuf,
}

struct UpdatePathResult {
    file_hash: String,
    created: bool,
    entries: Vec<DefIndexEntry>,
}

static DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Pin the `.readseek` directory, bypassing ancestor discovery (`--readseek-dir`).
pub(crate) fn set_dir_override(path: PathBuf) {
    DIR_OVERRIDE.set(path).ok();
}

fn dir_override() -> Option<&'static Path> {
    DIR_OVERRIDE.get().map(PathBuf::as_path)
}

pub(crate) fn find_readseek_dir(base: &Path) -> Option<PathBuf> {
    if let Some(dir) = dir_override() {
        return dir.is_dir().then(|| dir.to_path_buf());
    }
    let base = base.canonicalize().ok()?;
    base.ancestors()
        .find(|ancestor| {
            let candidate = ancestor.join(READSEEK_DIR);
            candidate.is_dir()
        })
        .map(|ancestor| ancestor.join(READSEEK_DIR))
}

fn readseek_dir_or_err(base: &Path) -> Result<PathBuf> {
    find_readseek_dir(base)
        .with_context(|| format!("no {READSEEK_DIR} directory found; run 'readseek init' first"))
}

#[derive(Clone, Debug)]
pub(crate) struct InitResult {
    pub(crate) path: PathBuf,
    pub(crate) reinitialized: bool,
}

pub(crate) fn init(dir: &Path) -> Result<InitResult> {
    let canonical = dir.canonicalize().context("resolve init path")?;
    let (readseek_dir, canonical_readseek) = match dir_override() {
        Some(dir) => (dir.to_path_buf(), dir.to_path_buf()),
        None => (dir.join(READSEEK_DIR), canonical.join(READSEEK_DIR)),
    };
    let reinitialized = canonical_readseek.exists();
    let maps_dir = canonical_readseek.join(MAPS_DIR);
    fs::create_dir_all(&maps_dir).with_context(|| format!("create {}", maps_dir.display()))?;
    let def_index_dir = canonical_readseek.join(DEF_INDEX_DIR);
    fs::create_dir_all(&def_index_dir)
        .with_context(|| format!("create {}", def_index_dir.display()))?;

    Ok(InitResult {
        path: readseek_dir,
        reinitialized,
    })
}

fn hex_hash_to_raw(hex_str: &str) -> Result<[u8; BLAKE3_RAW_LEN]> {
    let mut raw = [0u8; BLAKE3_RAW_LEN];
    hex::decode_to_slice(hex_str, &mut raw)
        .with_context(|| format!("invalid hex hash: {hex_str}"))?;
    Ok(raw)
}

fn map_path(readseek_dir: &Path, hash_hex: &str) -> PathBuf {
    readseek_dir
        .join(MAPS_DIR)
        .join(&hash_hex[..2])
        .join(&hash_hex[2..])
}

fn shard_bucket(name: &str) -> u32 {
    xxhash_rust::xxh32::xxh32(name.as_bytes(), 0) % SHARD_COUNT
}

fn def_index_shard_path(readseek_dir: &Path, name: &str) -> PathBuf {
    readseek_dir
        .join(DEF_INDEX_DIR)
        .join(format!("{}.json", shard_bucket(name)))
}

pub(crate) fn load_map(
    readseek_dir: &Path,
    file_hash: &str,
) -> Result<Option<(SourceMap, Language, Option<AnalysisEngine>)>> {
    let path = map_path(readseek_dir, file_hash);
    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;

    if data.len() < HEADER_SIZE {
        log::warn!("truncated map file: {}", path.display());
        return Ok(None);
    }

    let header = Header::ref_from_bytes(&data[..HEADER_SIZE])
        .map_err(|e| anyhow::anyhow!("parse header of {}: {e}", path.display()))?;

    if header.magic != MAGIC {
        log::warn!("invalid magic in {}", path.display());
        return Ok(None);
    }
    if header.version.get() != SCHEMA_VERSION {
        log::warn!(
            "unsupported schema version {} in {}",
            header.version.get(),
            path.display()
        );
        return Ok(None);
    }

    let expected_hash = hex_hash_to_raw(file_hash)?;
    if header.file_hash != expected_hash {
        log::warn!("hash mismatch in {}", path.display());
        return Ok(None);
    }

    let crc32 = crc::Crc::<u32>::new(&CRC_32_ISO_HDLC);
    let computed = crc32.checksum(&data[HEADER_SIZE..]);
    if header.checksum.get() != computed {
        log::warn!("checksum mismatch in {}", path.display());
        return Ok(None);
    }

    let language = Language::from_repr(header.lang_tag.get())
        .with_context(|| format!("unknown language tag {}", header.lang_tag.get()))?;
    let engine = if header.engine_tag == ENGINE_TAG_NONE {
        None
    } else {
        Some(
            AnalysisEngine::from_repr(header.engine_tag)
                .with_context(|| format!("unknown analysis engine tag {}", header.engine_tag))?,
        )
    };

    let sym_count = header.sym_count.get() as usize;
    if sym_count == 0 {
        return Ok(Some((
            SourceMap {
                symbols: Vec::new(),
            },
            language,
            engine,
        )));
    }

    let strtab_sz = header.strtab_sz.get() as usize;

    let sym_total = sym_count
        .checked_mul(SYM_ENTRY_SIZE)
        .context("sym_count overflow")?;
    let expected = HEADER_SIZE
        .checked_add(sym_total)
        .and_then(|v| v.checked_add(strtab_sz))
        .context("map size overflow")?;
    if data.len() != expected {
        bail!(
            "invalid map: buffer is {} bytes, header claims {}",
            data.len(),
            expected
        );
    }

    let strtab_start = HEADER_SIZE
        .checked_add(sym_total)
        .context("map symbol table size overflow")?;
    let strtab_end = strtab_start
        .checked_add(strtab_sz)
        .context("map string table size overflow")?;

    if data.len() < strtab_end {
        log::warn!("truncated data in {}", path.display());
        return Ok(None);
    }

    let sym_bytes = &data[HEADER_SIZE..strtab_start];
    let strtab = &data[strtab_start..strtab_end];

    let symbols = (0..sym_count)
        .map(|i| parse_sym_entry(sym_bytes, strtab, i, &path))
        .collect::<Result<Vec<_>>>()?;

    Ok(Some((SourceMap { symbols }, language, engine)))
}

fn parse_sym_entry(sym_bytes: &[u8], strtab: &[u8], i: usize, path: &Path) -> Result<Symbol> {
    let start = i
        .checked_mul(SYM_ENTRY_SIZE)
        .context("symbol index overflow")?;
    let end = start
        .checked_add(SYM_ENTRY_SIZE)
        .context("symbol entry range overflow")?;
    let entry_bytes = sym_bytes
        .get(start..end)
        .with_context(|| format!("symbol entry {i} out of bounds in {}", path.display()))?;
    let entry = SymEntry::ref_from_bytes(entry_bytes)
        .map_err(|e| anyhow::anyhow!("parse sym entry {i} of {}: {e}", path.display()))?;
    let kind = read_str(strtab, entry.kind_off.get(), entry.kind_len.get())?;
    let name = read_str(strtab, entry.name_off.get(), entry.name_len.get())?;
    let qualified_name = read_str(strtab, entry.qname_off.get(), entry.qname_len.get())?;
    Ok(Symbol {
        kind: kind.to_owned(),
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        start_line: entry.start_line.get() as usize,
        end_line: entry.end_line.get() as usize,
        start_hash: LineHash::new(entry.start_hash.get())?,
        end_hash: LineHash::new(entry.end_hash.get())?,
        start_byte: entry.start_byte.get() as usize,
        end_byte: entry.end_byte.get() as usize,
        name_byte: entry.name_byte.get() as usize,
    })
}

fn read_str(strtab: &[u8], offset: u32, len: u16) -> Result<&str> {
    let start = usize::try_from(offset).context("string table offset overflow")?;
    let len = usize::from(len);
    let end = start
        .checked_add(len)
        .context("string table range overflow")?;
    let bytes = strtab.get(start..end).with_context(|| {
        format!(
            "string table out of bounds: offset={offset} len={len} strtab_len={}",
            strtab.len()
        )
    })?;
    std::str::from_utf8(bytes)
        .with_context(|| format!("invalid UTF-8 in string table at offset {offset}"))
}

pub(crate) fn store_map(
    readseek_dir: &Path,
    file_hash: &str,
    source: &SourceFile,
    source_map: &SourceMap,
) -> Result<()> {
    let language = source.detection.language;
    let engine_tag = source.detection.engine.map_or(ENGINE_TAG_NONE, u8::from);

    let raw_hash = hex_hash_to_raw(file_hash)?;
    let sym_count = u32::try_from(source_map.symbols.len())
        .with_context(|| format!("too many symbols: {}", source_map.symbols.len()))?;

    let mut strtab = Vec::new();
    let mut entries = Vec::with_capacity(source_map.symbols.len());

    for symbol in &source_map.symbols {
        let kind_off = u32::try_from(strtab.len())?;
        let kind_len = u16::try_from(symbol.kind.len())
            .with_context(|| format!("kind name too long: {}", symbol.kind.len()))?;
        strtab.extend_from_slice(symbol.kind.as_bytes());

        let name_off = u32::try_from(strtab.len())?;
        let name_len = u16::try_from(symbol.name.len())
            .with_context(|| format!("name too long: {}", symbol.name.len()))?;
        strtab.extend_from_slice(symbol.name.as_bytes());

        let qname_off = u32::try_from(strtab.len())?;
        let qname_len = u16::try_from(symbol.qualified_name.len())
            .with_context(|| format!("qualified name too long: {}", symbol.qualified_name.len()))?;
        strtab.extend_from_slice(symbol.qualified_name.as_bytes());

        entries.push(SymEntry {
            kind_off: U32::new(kind_off),
            name_off: U32::new(name_off),
            qname_off: U32::new(qname_off),
            start_line: U32::new(u32::try_from(symbol.start_line)?),
            end_line: U32::new(u32::try_from(symbol.end_line)?),
            start_byte: U32::new(u32::try_from(symbol.start_byte)?),
            end_byte: U32::new(u32::try_from(symbol.end_byte)?),
            name_byte: U32::new(u32::try_from(symbol.name_byte)?),
            kind_len: U16::new(kind_len),
            name_len: U16::new(name_len),
            qname_len: U16::new(qname_len),
            start_hash: U16::new(symbol.start_hash.as_u16()),
            end_hash: U16::new(symbol.end_hash.as_u16()),
        });
    }

    let strtab_sz = u32::try_from(strtab.len())?;

    let header = Header {
        magic: MAGIC,
        version: U32::new(SCHEMA_VERSION),
        lang_tag: U16::new(u16::from(language)),
        engine_tag,
        _reserved: [0u8; 9],
        sym_count: U32::new(sym_count),
        strtab_sz: U32::new(strtab_sz),
        file_hash: raw_hash,
        checksum: U32::new(0),
    };

    let sym_total = entries
        .len()
        .checked_mul(SYM_ENTRY_SIZE)
        .context("symbol table size overflow")?;
    let total_size = HEADER_SIZE
        .checked_add(sym_total)
        .and_then(|size| size.checked_add(strtab.len()))
        .context("map size overflow")?;
    let mut buf = Vec::with_capacity(total_size);
    buf.extend_from_slice(header.as_bytes());
    for entry in &entries {
        buf.extend_from_slice(entry.as_bytes());
    }
    buf.extend_from_slice(&strtab);
    let crc32 = crc::Crc::<u32>::new(&CRC_32_ISO_HDLC);
    let checksum = crc32.checksum(&buf[HEADER_SIZE..]);
    buf[CHECKSUM_OFFSET..CHECKSUM_OFFSET + size_of::<U32<LittleEndian>>()]
        .copy_from_slice(&checksum.to_le_bytes());

    let path = map_path(readseek_dir, file_hash);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    write_atomic(&path, &buf).with_context(|| format!("write {}", path.display()))?;

    Ok(())
}

pub(crate) fn load_index(readseek_dir: &Path, name: &str) -> Result<Option<Vec<DefIndexEntry>>> {
    let path = def_index_shard_path(readseek_dir, name);
    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let index: std::collections::BTreeMap<String, Vec<DefIndexEntry>> =
        serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(index.get(name).cloned().unwrap_or_default()))
}

pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dir = path.parent().context("map path has no parent")?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let tmp = dir.join(format!(".tmp-{pid}-{ts:x}"));
    fs::write(&tmp, data).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

pub(crate) fn update(dir: &Path, flags: GitFlags) -> Result<UpdateStats> {
    let readseek_dir = readseek_dir_or_err(dir)?;
    // With an override the readseek dir is decoupled from the work tree, so the
    // project root is the caller-supplied work tree rather than the dir's parent.
    let project_root = match dir_override() {
        Some(_) => dir
            .canonicalize()
            .with_context(|| format!("resolve {}", dir.display()))?,
        None => readseek_dir
            .parent()
            .context(".readseek has no parent")?
            .to_path_buf(),
    };
    let paths = crate::engine::paths::command_paths(&project_root, flags)?;

    let results: Vec<_> = paths
        .par_iter()
        .map(|path| process_update_path(&readseek_dir, path))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    let mut active_hashes = HashSet::new();
    let mut index_entries = Vec::new();
    let mut stats = UpdateStats {
        created: 0,
        removed: 0,
        unchanged: 0,
    };

    for result in results {
        active_hashes.insert(result.file_hash);
        index_entries.extend(result.entries);
        if result.created {
            stats.created += 1;
        } else {
            stats.unchanged += 1;
        }
    }

    let shards = build_index_shards(index_entries);
    let written_prefixes = write_index_shards(&readseek_dir, &shards)?;
    remove_stale_index_shards(&readseek_dir, &written_prefixes)?;
    remove_legacy_index(&readseek_dir);
    stats.removed += remove_stale_maps(&readseek_dir, &active_hashes)?;

    let active_image_hashes: HashSet<String> = paths
        .par_iter()
        .filter_map(|path| crate::engine::vision_cache::image_hash(path))
        .collect();
    stats.removed +=
        crate::engine::vision_cache::remove_stale(&readseek_dir, &active_image_hashes)?;

    Ok(stats)
}

fn process_update_path(readseek_dir: &Path, path: &Path) -> Result<Option<UpdatePathResult>> {
    let Some(source) = crate::engine::source::load_indexable_source(path, None)? else {
        return Ok(None);
    };
    if !source.detection.supported {
        return Ok(None);
    }

    let cached = load_map(readseek_dir, &source.file_hash)?
        .filter(|(_, language, _)| *language == source.detection.language);
    let (source_map, created) = if let Some((source_map, _, _)) = cached {
        (source_map, false)
    } else {
        let source_map = symbols::parse_source_map(&source)?;
        store_map(readseek_dir, &source.file_hash, &source, &source_map)?;
        (source_map, true)
    };

    let entries = source_map
        .symbols
        .into_iter()
        .map(|symbol| DefIndexEntry {
            name: symbol.name,
            qualified_name: symbol.qualified_name,
            file_hash: source.file_hash.clone(),
            path: source.path.clone(),
        })
        .collect();

    Ok(Some(UpdatePathResult {
        file_hash: source.file_hash,
        created,
        entries,
    }))
}

fn build_index_shards(
    index_entries: Vec<DefIndexEntry>,
) -> BTreeMap<u32, BTreeMap<String, Vec<DefIndexEntry>>> {
    let mut shards: BTreeMap<u32, BTreeMap<String, Vec<DefIndexEntry>>> = BTreeMap::new();
    for entry in index_entries {
        let bucket = shard_bucket(&entry.name);
        shards
            .entry(bucket)
            .or_default()
            .entry(entry.name.clone())
            .or_default()
            .push(entry.clone());
        if entry.qualified_name != entry.name {
            let bucket = shard_bucket(&entry.qualified_name);
            shards
                .entry(bucket)
                .or_default()
                .entry(entry.qualified_name.clone())
                .or_default()
                .push(entry);
        }
    }
    for inner in shards.values_mut() {
        for entries in inner.values_mut() {
            entries.sort_unstable_by(|left, right| {
                left.qualified_name
                    .cmp(&right.qualified_name)
                    .then_with(|| left.path.cmp(&right.path))
            });
        }
    }

    shards
}

fn write_index_shards(
    readseek_dir: &Path,
    shards: &BTreeMap<u32, BTreeMap<String, Vec<DefIndexEntry>>>,
) -> Result<BTreeSet<u32>> {
    let shard_dir = readseek_dir.join(DEF_INDEX_DIR);
    fs::create_dir_all(&shard_dir).with_context(|| format!("create {}", shard_dir.display()))?;
    let mut written_buckets = BTreeSet::new();
    for (bucket, shard) in shards {
        let data = serde_json::to_vec(shard).context("serialize definition index shard")?;
        let path = shard_dir.join(format!("{bucket}.json"));
        write_atomic(&path, &data)?;
        written_buckets.insert(*bucket);
    }

    Ok(written_buckets)
}

fn remove_stale_index_shards(readseek_dir: &Path, written_buckets: &BTreeSet<u32>) -> Result<()> {
    let shard_dir = readseek_dir.join(DEF_INDEX_DIR);
    for entry in
        fs::read_dir(&shard_dir).with_context(|| format!("read {}", shard_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let retained = stem
                .parse::<u32>()
                .is_ok_and(|bucket| written_buckets.contains(&bucket));
            if !retained {
                fs::remove_file(&path)?;
            }
        }
    }

    Ok(())
}

fn remove_legacy_index(readseek_dir: &Path) {
    let legacy_path = readseek_dir.join("def-index.json");
    if legacy_path.exists() {
        fs::remove_file(&legacy_path).ok();
    }
}

fn remove_stale_maps(readseek_dir: &Path, active_hashes: &HashSet<String>) -> Result<usize> {
    let mut removed = 0;
    let maps_root = readseek_dir.join(MAPS_DIR);
    if maps_root.is_dir() {
        for entry in
            fs::read_dir(&maps_root).with_context(|| format!("read {}", maps_root.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                for file_entry in fs::read_dir(entry.path())? {
                    let file_entry = file_entry?;
                    let filename = file_entry.file_name();
                    let hash_fragment = filename.to_string_lossy();
                    let parent = file_entry
                        .path()
                        .parent()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
                    if let Some(prefix) = parent {
                        let hash_hex = format!("{prefix}{hash_fragment}");
                        if hash_hex.len() == BLAKE3_RAW_LEN * 2
                            && hex::decode(&hash_hex).is_ok()
                            && !active_hashes.contains(&hash_hex)
                        {
                            fs::remove_file(file_entry.path())?;
                            removed += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(removed)
}
