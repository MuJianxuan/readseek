// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Binding-accurate rename planning.
//!
//! `rename` resolves the lexical binding under a cursor (via the scope resolver)
//! and emits an edit plan: one edit per occurrence that binds to the same
//! declaration. The plan is data only; it carries the line hashline for each
//! edit so an applier can verify the file has not drifted before writing.
//!
//! With `--workspace` the rename expands across a directory or repository. The
//! cursor file stays binding-accurate; other files are matched by name (free
//! uses only, local shadows excluded) because readseek has no cross-file symbol
//! resolver. `--apply` then writes every file all-or-nothing.

use crate::binding::{self, OccurrenceKind};
use crate::cli::RenameCommand;
use crate::flags::GitFlags;
use crate::output::{RenameConflict, RenameEdit, RenameFileOutput, RenameOutput};
use crate::paths::{bytes_contain_identifier, command_paths, identifier_spans};
use crate::source::{SourceFile, source_from_text};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::fs;
use std::path::Path;

#[allow(clippy::too_many_lines)]
pub(crate) fn output(command: &RenameCommand) -> Result<RenameOutput> {
    let line = command.line;
    let column = command.column.unwrap_or(1);
    if line == 0 || column == 0 {
        bail!("line and column must be greater than zero");
    }
    if command.to.is_empty() {
        bail!("new name must not be empty");
    }
    if !is_plain_identifier(&command.to) {
        bail!("new name must be a plain identifier");
    }
    if !command.target.is_file() {
        bail!("rename requires a single regular file target");
    }

    let bytes =
        fs::read(&command.target).with_context(|| format!("read {}", command.target.display()))?;
    let text = String::from_utf8(bytes).context("file is not valid UTF-8")?;
    let source = source_from_text(&command.target, text, command.language, false, None)?;

    if !binding::supported(source.detection.language) {
        // No binding analysis for this language: report a no-op rather than a
        // failure so callers can warn instead of surfacing an error.
        return Ok(RenameOutput {
            file: source.path.clone(),
            language: source.detection.language,
            engine: source.detection.engine,
            file_hash: source.file_hash.clone(),
            old_name: String::new(),
            new_name: command.to.clone(),
            applied: false,
            unsupported: true,
            conflicts: Vec::new(),
            edits: Vec::new(),
            others: Vec::new(),
        });
    }

    let cursor_byte = source.cursor_byte(line, column)?;

    // The cursor file is binding-accurate when its symbol resolves to a local
    // declaration. Top-level symbols (functions, types) do not resolve, so in
    // workspace mode the cursor file falls back to the same name-based plan used
    // for the other files; without a workspace it stays an error as before.
    let (old_name, conflicts, edits) = if let Some((binding, raw_conflicts)) =
        binding::resolve_with_conflicts(&source, cursor_byte, Some(&command.to))
    {
        if binding.name == command.to {
            bail!("new name is identical to the current name");
        }
        let conflicts = raw_conflicts
            .into_iter()
            .map(|conflict| {
                let (line, column) = byte_to_line_column(&source, conflict.byte);
                RenameConflict {
                    line,
                    column,
                    reason: conflict.reason,
                }
            })
            .collect();
        let edits = binding
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.kind != OccurrenceKind::Shadowed)
            .map(|occurrence| {
                rename_edit(
                    &source,
                    occurrence.start_byte,
                    occurrence.end_byte,
                    occurrence.kind,
                )
            })
            .collect();
        (binding.name, conflicts, edits)
    } else {
        if command.workspace.is_none() {
            bail!(
                "no resolvable binding at {}:{line}:{column}",
                command.target.display()
            );
        }
        let name = binding::identifier_at(&source, cursor_byte).with_context(|| {
            format!(
                "no identifier at {}:{line}:{column}",
                command.target.display()
            )
        })?;
        if name == command.to {
            bail!("new name is identical to the current name");
        }
        let plan = build_other(&source, &name, &command.to).with_context(|| {
            format!(
                "`{name}` has no renamable occurrences in {}",
                command.target.display()
            )
        })?;
        (name, plan.conflicts, plan.edits)
    };

    let others = workspace_others(command, &old_name)?;

    let applied = if command.apply {
        apply_all(command, &old_name, &edits, &conflicts, &others)?;
        true
    } else {
        false
    };

    Ok(RenameOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        file_hash: source.file_hash.clone(),
        old_name,
        new_name: command.to.clone(),
        applied,
        unsupported: false,
        conflicts,
        edits,
        others,
    })
}

/// Plan name-based edits for every other file when `--workspace` is set.
///
/// The cursor file is excluded; remaining files are resolved per file via
/// [`binding::cross_file_matches`] (free uses only) and fall back to a plain
/// name scan for languages without binding support.
fn workspace_others(command: &RenameCommand, old_name: &str) -> Result<Vec<RenameFileOutput>> {
    let Some(workspace) = command.workspace.as_ref() else {
        return Ok(Vec::new());
    };
    let flags = GitFlags {
        cached: command.cached,
        others: command.others,
        ignored: command.ignored,
    };
    let paths = command_paths(workspace, flags)?;
    let origin = command.target.canonicalize().ok();

    let mut others: Vec<RenameFileOutput> = paths
        .par_iter()
        .filter(|path| !is_origin(path, &command.target, origin.as_deref()))
        .filter_map(|path| {
            let bytes = fs::read(path).ok()?;
            if !bytes_contain_identifier(&bytes, old_name.as_bytes()) {
                return None;
            }
            let text = String::from_utf8(bytes).ok()?;
            let source = source_from_text(path, text, command.language, false, None).ok()?;
            build_other(&source, old_name, &command.to)
        })
        .collect();
    others.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(others)
}

/// Build the edit plan for a single non-cursor file, or `None` when nothing in
/// it matches the old name.
fn build_other(source: &SourceFile, old_name: &str, new_name: &str) -> Option<RenameFileOutput> {
    let (occurrences, conflict_bytes) =
        match binding::cross_file_matches(source, old_name, new_name) {
            Some(matches) => (matches.occurrences, matches.conflicts),
            None => (name_scan(source, old_name), Vec::new()),
        };
    if occurrences.is_empty() {
        return None;
    }

    let edits = occurrences
        .into_iter()
        .map(|(start_byte, end_byte)| {
            rename_edit(source, start_byte, end_byte, OccurrenceKind::Reference)
        })
        .collect();
    let conflicts = conflict_bytes
        .into_iter()
        .map(|byte| {
            let (line, column) = byte_to_line_column(source, byte);
            RenameConflict {
                line,
                column,
                reason: format!("`{new_name}` already resolves to a binding here"),
            }
        })
        .collect();

    Some(RenameFileOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        file_hash: source.file_hash.clone(),
        conflicts,
        edits,
    })
}

/// Word-boundary name match for languages without binding support.
fn name_scan(source: &SourceFile, name: &str) -> Vec<(usize, usize)> {
    let text = source.text.as_bytes();
    let needle = name.as_bytes();
    let mut spans = Vec::new();
    for index in identifier_spans(text, needle) {
        spans.push((index, index + needle.len()));
    }
    spans
}

/// Whether `path` denotes the cursor file and so must be skipped during expansion.
fn is_origin(path: &Path, target: &Path, origin_canon: Option<&Path>) -> bool {
    if path == target {
        return true;
    }
    match (path.canonicalize(), origin_canon) {
        (Ok(candidate), Some(origin)) => candidate == origin,
        _ => false,
    }
}

fn rename_edit(
    source: &SourceFile,
    start_byte: usize,
    end_byte: usize,
    kind: OccurrenceKind,
) -> RenameEdit {
    let line_idx = source
        .line_starts
        .partition_point(|&start| start <= start_byte)
        .saturating_sub(1);
    let source_line = &source.lines[line_idx];
    let line_start = source.line_starts[line_idx];
    RenameEdit {
        line: source_line.number,
        start_column: start_byte - line_start + 1,
        end_column: end_byte - line_start + 1,
        start_byte,
        end_byte,
        occurrence: kind,
        line_hash: source_line.hash(),
        text: source_line.text.clone(),
    }
}

/// Write the planned edits across every file, refusing on conflicts or drift.
///
/// Every file is verified before any is written: each edit's `line_hash` is
/// re-checked against the file on disk, so a file that changed since the plan
/// was computed aborts the whole rename rather than leaving a partial result.
/// If a write fails mid-way, already-written files are restored.
fn apply_all(
    command: &RenameCommand,
    old_name: &str,
    edits: &[RenameEdit],
    conflicts: &[RenameConflict],
    others: &[RenameFileOutput],
) -> Result<()> {
    let conflict_count = conflicts.len() + others.iter().map(|o| o.conflicts.len()).sum::<usize>();
    if conflict_count > 0 {
        bail!(
            "refusing to apply: {conflict_count} naming conflict(s); resolve them or rename to a free name"
        );
    }

    let mut plans: Vec<(&Path, &[RenameEdit])> = vec![(command.target.as_path(), edits)];
    for other in others {
        plans.push((other.file.as_path(), &other.edits));
    }

    // Verify every file and compute its new text before touching any of them.
    let mut writes: Vec<(&Path, String, String)> = Vec::new();
    for (path, edits) in plans {
        if edits.is_empty() {
            continue;
        }
        let current =
            fs::read_to_string(path).with_context(|| format!("re-read {}", path.display()))?;
        let current = source_from_text(path, current, command.language, false, None)?;
        for edit in edits {
            let line = current.line(edit.line).with_context(|| {
                format!("line {} no longer exists in {}", edit.line, path.display())
            })?;
            if line.hash() != edit.line_hash {
                bail!(
                    "refusing to apply: {}:{} changed since the plan was computed",
                    path.display(),
                    edit.line
                );
            }
        }
        let new_text = rewrite(&current.text, edits, old_name, &command.to)?;
        writes.push((path, current.text, new_text));
    }

    // All files verified: write them, restoring on any failure.
    let mut written: Vec<(&Path, &str)> = Vec::new();
    for (path, original, new_text) in &writes {
        if let Err(error) = fs::write(path, new_text) {
            for (done_path, done_original) in &written {
                let _ = fs::write(done_path, done_original);
            }
            return Err(error).with_context(|| format!("write {}", path.display()));
        }
        written.push((path, original));
    }
    Ok(())
}

/// Replace each edit span with `new_name`, applying back-to-front so earlier
/// byte offsets stay valid. Refuses unless every span still holds `old_name`.
fn rewrite(source: &str, edits: &[RenameEdit], old_name: &str, new_name: &str) -> Result<String> {
    let mut text = source.to_owned();
    let mut ordered: Vec<&RenameEdit> = edits.iter().collect();
    ordered.sort_by_key(|edit| std::cmp::Reverse(edit.start_byte));
    for edit in ordered {
        if edit.end_byte > text.len()
            || !text.is_char_boundary(edit.start_byte)
            || !text.is_char_boundary(edit.end_byte)
        {
            bail!("refusing to apply: edit span is out of range");
        }
        if &text[edit.start_byte..edit.end_byte] != old_name {
            bail!("refusing to apply: a target span no longer holds `{old_name}`");
        }
        text.replace_range(edit.start_byte..edit.end_byte, new_name);
    }
    Ok(text)
}

fn byte_to_line_column(source: &crate::source::SourceFile, byte: usize) -> (usize, usize) {
    let line_idx = source
        .line_starts
        .partition_point(|&start| start <= byte)
        .saturating_sub(1);
    let line = source
        .lines
        .get(line_idx)
        .map_or(1, |source_line| source_line.number);
    (line, byte - source.line_starts[line_idx] + 1)
}

fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_') && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
}
