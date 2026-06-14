// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::lang::{AnalysisEngine, Language};
use crate::source::{SourceFile, SourceMap, Symbol};
use crate::symbols;
use anyhow::{Context, Result, bail};
use crc::CRC_32_ISO_HDLC;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use zerocopy::byteorder::{LittleEndian, U16, U32};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

const READSEEK_DIR: &str = ".readseek";
const MAPS_DIR: &str = "maps";
const MAGIC: [u8; 4] = *b"RSMP";
const SCHEMA_VERSION: u32 = 2;

const HEADER_SIZE: usize = 64;
const SYM_ENTRY_SIZE: usize = 32;
const BLAKE3_RAW_LEN: usize = 32;
const ENGINE_TAG_NONE: u8 = 0xff;

const _: () = assert!(
    crate::hash::HASHLINE_MODULUS <= 0x10000,
    "HASHLINE_MODULUS must fit in a u16 for binary format storage"
);

#[derive(FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
struct Header {
    magic: [u8; 4],
    version: U32<LittleEndian>,
    flags: U32<LittleEndian>,
    lang_tag: U32<LittleEndian>,
    engine_tag: u8,
    _pad0: [u8; 3],
    sym_count: U32<LittleEndian>,
    strtab_sz: U32<LittleEndian>,
    file_hash: [u8; BLAKE3_RAW_LEN],
    checksum: U32<LittleEndian>,
}

#[derive(FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, align(8))]
struct SymEntry {
    kind_off: U32<LittleEndian>,
    name_off: U32<LittleEndian>,
    qname_off: U32<LittleEndian>,
    start_line: U32<LittleEndian>,
    end_line: U32<LittleEndian>,
    kind_len: U16<LittleEndian>,
    name_len: U16<LittleEndian>,
    qname_len: U16<LittleEndian>,
    start_hash: U16<LittleEndian>,
    end_hash: U16<LittleEndian>,
    _tail_pad: [u8; 2],
}

#[derive(Debug)]
pub(crate) struct UpdateStats {
    pub(crate) created: usize,
    pub(crate) removed: usize,
    pub(crate) unchanged: usize,
}

pub(crate) fn find_readseek_dir(base: &Path) -> Option<PathBuf> {
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

pub(crate) fn init(dir: &Path) -> Result<PathBuf> {
    let dir = dir.canonicalize().context("resolve init path")?;
    let readseek_dir = dir.join(READSEEK_DIR);
    let maps_dir = readseek_dir.join(MAPS_DIR);

    if readseek_dir.exists() {
        bail!("{} already exists in {}", READSEEK_DIR, dir.display());
    }

    fs::create_dir_all(&maps_dir).with_context(|| format!("create {}", maps_dir.display()))?;

    append_to_gitignore(&dir)?;

    Ok(readseek_dir)
}

fn append_to_gitignore(dir: &Path) -> Result<()> {
    let gitignore = dir.join(".gitignore");
    let entry = format!("/{READSEEK_DIR}\n");

    let needs_append = match fs::read_to_string(&gitignore) {
        Ok(contents) => !contents
            .lines()
            .any(|line| line.trim() == format!("/{READSEEK_DIR}")),
        Err(_) => true,
    };

    if needs_append {
        let mut contents = fs::read_to_string(&gitignore).unwrap_or_default();
        if !contents.ends_with('\n') && !contents.is_empty() {
            contents.push('\n');
        }
        contents.push_str(&entry);
        fs::write(&gitignore, contents)
            .with_context(|| format!("append to {}", gitignore.display()))?;
    }

    Ok(())
}

fn hex_hash_to_raw(hex_str: &str) -> Result<[u8; BLAKE3_RAW_LEN]> {
    let mut raw = [0u8; BLAKE3_RAW_LEN];
    hex::decode_to_slice(hex_str, &mut raw)
        .with_context(|| format!("invalid hex hash: {hex_str}"))?;
    Ok(raw)
}

fn hash_subdir(hash_hex: &str) -> &str {
    &hash_hex[..2]
}

fn hash_filename(hash_hex: &str) -> &str {
    &hash_hex[2..]
}

fn map_path(readseek_dir: &Path, hash_hex: &str) -> PathBuf {
    readseek_dir
        .join(MAPS_DIR)
        .join(hash_subdir(hash_hex))
        .join(hash_filename(hash_hex))
}

fn engine_from_tag(tag: u8) -> Result<Option<AnalysisEngine>> {
    if tag == ENGINE_TAG_NONE {
        return Ok(None);
    }
    Ok(Some(AnalysisEngine::try_from(tag)?))
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

    let language = Language::try_from(header.lang_tag.get())
        .with_context(|| format!("unknown language tag {}", header.lang_tag.get()))?;
    let engine = engine_from_tag(header.engine_tag)?;

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
    let syms_slice = &data[HEADER_SIZE..];
    let syms_end = sym_count * SYM_ENTRY_SIZE;
    let strtab_start = HEADER_SIZE + syms_end;
    let strtab_end = strtab_start + strtab_sz;

    if data.len() < strtab_end {
        log::warn!("truncated data in {}", path.display());
        return Ok(None);
    }

    let sym_bytes = &syms_slice[..syms_end];
    let strtab = &data[strtab_start..strtab_end];

    let mut symbols = Vec::with_capacity(sym_count);
    for i in 0..sym_count {
        let start = i * SYM_ENTRY_SIZE;
        let entry = SymEntry::ref_from_bytes(&sym_bytes[start..start + SYM_ENTRY_SIZE])
            .map_err(|e| anyhow::anyhow!("parse sym entry {i} of {}: {e}", path.display()))?;
        let kind = read_str(strtab, entry.kind_off.get(), entry.kind_len.get())?;
        let name = read_str(strtab, entry.name_off.get(), entry.name_len.get())?;
        let qualified_name = read_str(strtab, entry.qname_off.get(), entry.qname_len.get())?;

        symbols.push(Symbol {
            kind: kind.to_owned(),
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            start_line: entry.start_line.get() as usize,
            end_line: entry.end_line.get() as usize,
            start_hash: format!("{:03x}", entry.start_hash.get()),
            end_hash: format!("{:03x}", entry.end_hash.get()),
        });
    }

    Ok(Some((SourceMap { symbols }, language, engine)))
}

fn read_str(strtab: &[u8], offset: u32, len: u16) -> Result<&str> {
    let start = offset as usize;
    let end = start + len as usize;
    if end > strtab.len() {
        bail!(
            "string table out of bounds: offset={offset} len={len} strtab_len={}",
            strtab.len()
        );
    }
    std::str::from_utf8(&strtab[start..end])
        .with_context(|| format!("invalid UTF-8 in string table at offset {offset}"))
}

pub(crate) fn store_map(
    readseek_dir: &Path,
    file_hash: &str,
    source: &SourceFile,
    source_map: &SourceMap,
) -> Result<()> {
    let language = source.detection.language;
    let engine = source.detection.engine;
    let engine_tag = engine.map_or(ENGINE_TAG_NONE, u8::from);

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

        let start_hash = u16::from_str_radix(&symbol.start_hash, 16)
            .with_context(|| format!("invalid start hash for symbol '{}'", symbol.name))?;
        let end_hash = u16::from_str_radix(&symbol.end_hash, 16)
            .with_context(|| format!("invalid end hash for symbol '{}'", symbol.name))?;

        entries.push(SymEntry {
            kind_off: U32::new(kind_off),
            name_off: U32::new(name_off),
            qname_off: U32::new(qname_off),
            start_line: U32::new(u32::try_from(symbol.start_line)?),
            end_line: U32::new(u32::try_from(symbol.end_line)?),
            kind_len: U16::new(kind_len),
            name_len: U16::new(name_len),
            qname_len: U16::new(qname_len),
            start_hash: U16::new(start_hash),
            end_hash: U16::new(end_hash),
            _tail_pad: [0u8; 2],
        });
    }

    let strtab_sz = u32::try_from(strtab.len())?;

    let header = Header {
        magic: MAGIC,
        version: U32::new(SCHEMA_VERSION),
        flags: U32::new(0),
        lang_tag: U32::new(language as u32),
        engine_tag,
        _pad0: [0u8; 3],
        sym_count: U32::new(sym_count),
        strtab_sz: U32::new(strtab_sz),
        file_hash: raw_hash,
        checksum: U32::new(0),
    };

    let total_size = HEADER_SIZE + entries.len() * SYM_ENTRY_SIZE + strtab.len();
    let mut buf = Vec::with_capacity(total_size);
    buf.extend_from_slice(header.as_bytes());
    for entry in &entries {
        buf.extend_from_slice(entry.as_bytes());
    }
    buf.extend_from_slice(&strtab);
    let crc32 = crc::Crc::<u32>::new(&CRC_32_ISO_HDLC);
    let checksum = crc32.checksum(&buf[HEADER_SIZE..]);
    buf[HEADER_SIZE - size_of::<U32<LittleEndian>>()..HEADER_SIZE]
        .copy_from_slice(&checksum.to_le_bytes());

    let path = map_path(readseek_dir, file_hash);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    write_atomic(&path, &buf).with_context(|| format!("write {}", path.display()))?;

    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().context("map path has no parent")?;
    let tmp = tempfile_in(dir);
    fs::write(&tmp, data).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn tempfile_in(dir: &Path) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let name = format!(".tmp-{pid}-{ts:x}");
    dir.join(name)
}

pub(crate) fn update(dir: &Path, cached: bool, others: bool, ignored: bool) -> Result<UpdateStats> {
    let readseek_dir = readseek_dir_or_err(dir)?;
    let project_root = readseek_dir.parent().context(".readseek has no parent")?;

    let paths = crate::paths::command_paths(project_root, cached, others, ignored)?;

    let mut stats = UpdateStats {
        created: 0,
        removed: 0,
        unchanged: 0,
    };

    let results: Vec<(String, bool)> = paths
        .par_iter()
        .filter_map(|path| {
            let source =
                crate::source::load_source(path, None, crate::lang::BinaryMode::Reject).ok()?;
            if !source.detection.supported {
                return None;
            }
            let map_path = map_path(&readseek_dir, &source.file_hash);
            let created = if map_path.exists() {
                false
            } else {
                let source_map = symbols::parse_source_map(&source).ok()?;
                store_map(&readseek_dir, &source.file_hash, &source, &source_map).ok()?;
                true
            };
            Some((source.file_hash, created))
        })
        .collect();

    let mut active_hashes = std::collections::HashSet::new();

    for (hash, created) in results {
        active_hashes.insert(hash);
        if created {
            stats.created += 1;
        } else {
            stats.unchanged += 1;
        }
    }

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
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
                    if let Some(prefix) = parent {
                        let hash_hex = format!("{prefix}{hash_fragment}");
                        if hash_hex.len() == BLAKE3_RAW_LEN * 2
                            && hex::decode(&hash_hex).is_ok()
                            && !active_hashes.contains(&hash_hex)
                        {
                            fs::remove_file(file_entry.path())?;
                            stats.removed += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(stats)
}
