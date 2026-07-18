// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Persistent content-addressed document indexes.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::engine::document::{Asset, Document, DocumentFormat, Node, NodeKind, SourceAnchor};

const SCHEMA_VERSION: i64 = 1;

pub(crate) fn load(readseek_dir: &Path, id: &str) -> Result<Option<Document>> {
    let path = index_path(readseek_dir, id);
    if !path.is_file() {
        return Ok(None);
    }

    let connection = Connection::open(&path)
        .with_context(|| format!("open document index {}", path.display()))?;
    require_schema(&connection, &path)?;

    let document = connection
        .query_row(
            "SELECT id, format, source, title, pages FROM document WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .with_context(|| format!("read document index {}", path.display()))?;
    let Some((id, format, source, title, pages)) = document else {
        return Ok(None);
    };
    let pages = usize::try_from(pages).context("invalid page count in document index")?;

    let mut statement = connection.prepare(
        "SELECT id, parent_id, kind, title, text, page, destination
         FROM nodes ORDER BY position",
    )?;
    let rows = statement.query_map([], |row| {
        let page = row.get::<_, Option<i64>>(5)?;
        let destination = row.get::<_, Option<String>>(6)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            page,
            destination,
        ))
    })?;
    let mut nodes = Vec::new();
    for row in rows {
        let (id, parent_id, kind, title, text, page, destination) = row?;
        let page = page
            .map(usize::try_from)
            .transpose()
            .context("invalid page number in document index")?;
        let source_anchor =
            (page.is_some() || destination.is_some()).then_some(SourceAnchor { page, destination });
        nodes.push(Node {
            id,
            parent_id,
            kind: NodeKind::parse(&kind)?,
            title,
            text,
            source_anchor,
        });
    }

    let mut statement = connection
        .prepare("SELECT id, mime, path, page, destination FROM assets ORDER BY position")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut assets = Vec::new();
    for row in rows {
        let (id, mime, path, page, destination) = row?;
        let page = page
            .map(usize::try_from)
            .transpose()
            .context("invalid asset page number in document index")?;
        let source_anchor =
            (page.is_some() || destination.is_some()).then_some(SourceAnchor { page, destination });
        assets.push(Asset {
            id,
            mime,
            path: PathBuf::from(path),
            source_anchor,
        });
    }

    Ok(Some(Document {
        id,
        format: DocumentFormat::parse(&format)?,
        source: PathBuf::from(source),
        title,
        pages,
        nodes,
        assets,
    }))
}

pub(crate) fn store(readseek_dir: &Path, document: &Document) -> Result<()> {
    let path = index_path(readseek_dir, &document.id);
    let parent = path.parent().context("document index path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let mut connection = Connection::open(&path)
        .with_context(|| format!("open document index {}", path.display()))?;
    initialize_schema(&connection, &path)?;
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM assets", [])?;
    transaction.execute("DELETE FROM nodes", [])?;
    transaction.execute("DELETE FROM document", [])?;
    let pages = i64::try_from(document.pages).context("document page count is too large")?;
    transaction.execute(
        "INSERT INTO document (id, format, source, title, pages)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            document.id,
            document.format.as_str(),
            document.source.to_string_lossy(),
            document.title,
            pages,
        ],
    )?;
    for (position, node) in document.nodes.iter().enumerate() {
        let position = i64::try_from(position).context("too many document nodes")?;
        let (page, destination) = node.source_anchor.as_ref().map_or((None, None), |anchor| {
            (anchor.page, anchor.destination.as_deref())
        });
        let page = page
            .map(i64::try_from)
            .transpose()
            .context("document node page number is too large")?;
        transaction.execute(
            "INSERT INTO nodes
             (id, parent_id, position, kind, title, text, page, destination)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                node.id,
                node.parent_id,
                position,
                node.kind.as_str(),
                node.title,
                node.text,
                page,
                destination,
            ],
        )?;
    }
    for (position, asset) in document.assets.iter().enumerate() {
        let position = i64::try_from(position).context("too many document assets")?;
        let (page, destination) = asset.source_anchor.as_ref().map_or((None, None), |anchor| {
            (anchor.page, anchor.destination.as_deref())
        });
        let page = page
            .map(i64::try_from)
            .transpose()
            .context("document asset page number is too large")?;
        transaction.execute(
            "INSERT INTO assets (id, position, mime, path, page, destination)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                asset.id,
                position,
                asset.mime,
                asset.path.to_string_lossy(),
                page,
                destination,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn index_path(readseek_dir: &Path, id: &str) -> PathBuf {
    readseek_dir.join("documents").join(id).join("index.sqlite")
}

fn require_schema(connection: &Connection, path: &Path) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported document index schema {version} in {}",
            path.display()
        );
    }
    Ok(())
}

fn initialize_schema(connection: &Connection, path: &Path) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 0 {
        return require_schema(connection, path);
    }
    connection.execute_batch(
        "BEGIN;
         CREATE TABLE document (
             id TEXT PRIMARY KEY,
             format TEXT NOT NULL,
             source TEXT NOT NULL,
             title TEXT NOT NULL,
             pages INTEGER NOT NULL
         );
         CREATE TABLE nodes (
             id TEXT PRIMARY KEY,
             parent_id TEXT,
             position INTEGER NOT NULL,
             kind TEXT NOT NULL,
             title TEXT,
             text TEXT,
             page INTEGER,
             destination TEXT
         );
         CREATE INDEX nodes_parent_position ON nodes(parent_id, position);
         CREATE TABLE assets (
             id TEXT PRIMARY KEY,
             position INTEGER NOT NULL,
             mime TEXT NOT NULL,
             path TEXT NOT NULL,
             page INTEGER,
             destination TEXT
         );
         PRAGMA user_version = 1;
         COMMIT;",
    )?;
    Ok(())
}
