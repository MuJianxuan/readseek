// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use crate::source::{SourceFile, SourceMap, Symbol, SymbolLookup};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

const DB_SCHEMA_VERSION: i64 = 5;

pub(crate) fn load_source_map(source: &SourceFile) -> Result<Option<SourceMap>> {
    let Some(mut connection) = connection()? else {
        return Ok(None);
    };
    let tx = connection.transaction()?;
    let Some((cache_id, symbol_count)) = entry(&tx, source)? else {
        return Ok(None);
    };
    let symbols = load_symbols(&tx, cache_id)?;
    if i64::try_from(symbols.len())? != symbol_count {
        tx.execute("DELETE FROM map_cache WHERE id = ?1", params![cache_id])?;
        tx.commit()?;
        return Ok(None);
    }
    let now = unix_time()?;
    tx.execute(
        "UPDATE map_cache SET last_used_at = ?1 WHERE id = ?2",
        params![now, cache_id],
    )?;
    tx.commit()?;

    Ok(Some(SourceMap { symbols }))
}

pub(crate) fn symbol_by_address(
    source: &SourceFile,
    address: &str,
) -> Result<Option<SymbolLookup>> {
    let Some(mut connection) = connection()? else {
        return Ok(None);
    };
    let tx = connection.transaction()?;
    let Some((cache_id, symbol_count)) = entry(&tx, source)? else {
        return Ok(None);
    };
    if !validate_entry(&tx, cache_id, symbol_count)? {
        tx.commit()?;
        return Ok(None);
    }

    let lookup = {
        let mut statement = tx.prepare(
            "SELECT kind, name, address, start_line, end_line, start_hash, end_hash \
             FROM map_symbols WHERE cache_id = ?1 AND (address = ?2 OR name = ?2) \
             ORDER BY rowid LIMIT 2",
        )?;
        let mut rows = statement.query_map(params![cache_id, address], symbol_from_row)?;
        match (rows.next().transpose()?, rows.next().transpose()?) {
            (None, _) => SymbolLookup::NotFound,
            (Some(symbol), None) => SymbolLookup::Found(symbol),
            (Some(_), Some(_)) => SymbolLookup::Ambiguous,
        }
    };
    update_last_used(&tx, cache_id)?;
    tx.commit()?;

    Ok(Some(lookup))
}

pub(crate) fn symbol_at_line(source: &SourceFile, line: usize) -> Result<Option<SymbolLookup>> {
    let Some(mut connection) = connection()? else {
        return Ok(None);
    };
    let tx = connection.transaction()?;
    let Some((cache_id, symbol_count)) = entry(&tx, source)? else {
        return Ok(None);
    };
    if !validate_entry(&tx, cache_id, symbol_count)? {
        tx.commit()?;
        return Ok(None);
    }

    let symbol = tx
        .query_row(
            "SELECT kind, name, address, start_line, end_line, start_hash, end_hash \
             FROM map_symbols WHERE cache_id = ?1 AND start_line <= ?2 AND ?2 <= end_line \
             ORDER BY end_line - start_line, rowid LIMIT 1",
            params![cache_id, i64::try_from(line)?],
            symbol_from_row,
        )
        .optional()?;
    update_last_used(&tx, cache_id)?;
    tx.commit()?;

    Ok(Some(match symbol {
        Some(symbol) => SymbolLookup::Found(symbol),
        None => SymbolLookup::NotFound,
    }))
}

fn entry(tx: &Transaction<'_>, source: &SourceFile) -> Result<Option<(i64, i64)>> {
    tx.query_row(
        "SELECT id, symbol_count FROM map_cache \
         WHERE cache_version = ?1 AND file_hash = ?2 AND language = ?3 AND engine = ?4",
        params![
            DB_SCHEMA_VERSION,
            source.file_hash,
            source.detection.language.id(),
            source.detection.engine.id()
        ],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn validate_entry(tx: &Transaction<'_>, cache_id: i64, symbol_count: i64) -> Result<bool> {
    let actual_count = tx.query_row(
        "SELECT COUNT(*) FROM map_symbols WHERE cache_id = ?1",
        params![cache_id],
        |row| row.get::<_, i64>(0),
    )?;
    if actual_count == symbol_count {
        return Ok(true);
    }

    tx.execute("DELETE FROM map_cache WHERE id = ?1", params![cache_id])?;
    Ok(false)
}

fn update_last_used(tx: &Transaction<'_>, cache_id: i64) -> Result<()> {
    let now = unix_time()?;
    tx.execute(
        "UPDATE map_cache SET last_used_at = ?1 WHERE id = ?2",
        params![now, cache_id],
    )?;

    Ok(())
}

fn symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    Ok(Symbol {
        kind: row.get(0)?,
        name: row.get(1)?,
        address: row.get(2)?,
        start_line: usize::try_from(row.get::<_, i64>(3)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        end_line: usize::try_from(row.get::<_, i64>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        start_hash: row.get(5)?,
        end_hash: row.get(6)?,
    })
}

fn load_symbols(tx: &Transaction<'_>, cache_id: i64) -> Result<Vec<Symbol>> {
    let mut statement = tx.prepare(
        "SELECT kind, name, address, start_line, end_line, start_hash, end_hash \
         FROM map_symbols WHERE cache_id = ?1 ORDER BY rowid",
    )?;
    let rows = statement.query_map(params![cache_id], symbol_from_row)?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(crate) fn store_source_map(source: &SourceFile, source_map: &SourceMap) -> Result<()> {
    let Some(mut connection) = connection()? else {
        return Ok(());
    };
    let tx = connection.transaction()?;
    let now = unix_time()?;

    tx.execute(
        "DELETE FROM map_cache \
         WHERE cache_version = ?1 AND file_hash = ?2 AND language = ?3 AND engine = ?4",
        params![
            DB_SCHEMA_VERSION,
            source.file_hash,
            source.detection.language.id(),
            source.detection.engine.id()
        ],
    )?;
    tx.execute(
        "INSERT INTO map_cache \
         (cache_version, file_hash, language, engine, symbol_count, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            DB_SCHEMA_VERSION,
            source.file_hash,
            source.detection.language.id(),
            source.detection.engine.id(),
            i64::try_from(source_map.symbols.len())?,
            now,
            now,
        ],
    )?;
    let cache_id = tx.last_insert_rowid();

    {
        let mut insert_symbol = tx.prepare(
            "INSERT INTO map_symbols \
             (cache_id, kind, name, address, start_line, end_line, start_hash, end_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for symbol in &source_map.symbols {
            insert_symbol.execute(params![
                cache_id,
                symbol.kind,
                symbol.name,
                symbol.address,
                i64::try_from(symbol.start_line)?,
                i64::try_from(symbol.end_line)?,
                symbol.start_hash,
                symbol.end_hash,
            ])?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn connection() -> Result<Option<Connection>> {
    let Some(mut cache_dir) = cache_base_dir() else {
        return Ok(None);
    };
    cache_dir.push("readseek");
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create cache directory {}", cache_dir.display()))?;
    let database_path = cache_dir.join("cache.sqlite3");
    let connection = Connection::open(&database_path)
        .with_context(|| format!("open cache {}", database_path.display()))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    initialize_schema(&connection)?;

    Ok(Some(connection))
}

fn cache_base_dir() -> Option<PathBuf> {
    env::var_os("READSEEK_CACHE_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::cache_dir)
}

fn initialize_schema(connection: &Connection) -> Result<()> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if version > DB_SCHEMA_VERSION {
        bail!("cache schema version {version} is newer than supported version {DB_SCHEMA_VERSION}");
    }
    if version == DB_SCHEMA_VERSION {
        return Ok(());
    }

    connection.execute_batch(&format!(
        "DROP TABLE IF EXISTS map_symbols;
        DROP TABLE IF EXISTS map_cache;
        CREATE TABLE map_cache (
            id INTEGER PRIMARY KEY,
            cache_version INTEGER NOT NULL,
            file_hash TEXT NOT NULL,
            language TEXT NOT NULL,
            engine TEXT NOT NULL,
            symbol_count INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            UNIQUE(file_hash, language, engine, cache_version)
        );
        CREATE TABLE map_symbols (
            cache_id INTEGER NOT NULL REFERENCES map_cache(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            address TEXT NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            start_hash TEXT NOT NULL,
            end_hash TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS map_symbols_address_idx ON map_symbols(cache_id, address);
        CREATE INDEX IF NOT EXISTS map_symbols_name_idx ON map_symbols(cache_id, name);
        CREATE INDEX IF NOT EXISTS map_symbols_line_idx ON map_symbols(cache_id, start_line, end_line);
        PRAGMA user_version = {DB_SCHEMA_VERSION};"
    ))?;

    Ok(())
}

fn unix_time() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX epoch")?;

    i64::try_from(duration.as_secs()).context("current time exceeds supported range")
}
