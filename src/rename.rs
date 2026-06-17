// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Binding-accurate rename planning.
//!
//! `rename` resolves the lexical binding under a cursor (via the scope resolver)
//! and emits an edit plan: one edit per occurrence that binds to the same
//! declaration. The plan is data only; it carries the line hashline for each
//! edit so an applier can verify the file has not drifted before writing.

use crate::binding::{self, OccurrenceKind};
use crate::cli::RenameCommand;
use crate::output::{RenameConflict, RenameEdit, RenameOutput};
use crate::source::source_from_text;
use anyhow::{Context, Result, bail};
use std::fs;

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

    let line_start = *source
        .line_starts
        .get(line - 1)
        .with_context(|| format!("line {line} not found in {}", command.target.display()))?;
    let cursor_byte = line_start + column - 1;

    let (binding, conflicts) =
        binding::resolve_with_conflicts(&source, cursor_byte, Some(&command.to)).with_context(
            || {
                format!(
                    "no resolvable binding at {}:{line}:{column}",
                    command.target.display()
                )
            },
        )?;

    if binding.name == command.to {
        bail!("new name is identical to the current name");
    }

    let conflicts: Vec<RenameConflict> = conflicts
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

    let edits: Vec<RenameEdit> = binding
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.kind != OccurrenceKind::Shadowed)
        .map(|occurrence| {
            let line_idx = source
                .line_starts
                .partition_point(|&start| start <= occurrence.start_byte)
                .saturating_sub(1);
            let source_line = &source.lines[line_idx];
            let line_start = source.line_starts[line_idx];
            RenameEdit {
                line: source_line.number,
                start_column: occurrence.start_byte - line_start + 1,
                end_column: occurrence.end_byte - line_start + 1,
                start_byte: occurrence.start_byte,
                end_byte: occurrence.end_byte,
                occurrence: occurrence.kind,
                line_hash: source_line.hash(),
                text: source_line.text.clone(),
            }
        })
        .collect();

    let applied = if command.apply {
        apply_edits(command, &source, &conflicts, &edits)?;
        true
    } else {
        false
    };

    Ok(RenameOutput {
        file: source.path.clone(),
        language: source.detection.language,
        engine: source.detection.engine,
        file_hash: source.file_hash.clone(),
        old_name: binding.name,
        new_name: command.to.clone(),
        applied,
        conflicts,
        edits,
    })
}

/// Write the planned edits, refusing on conflicts or hashline drift.
///
/// Each edit's `line_hash` is re-verified against the file on disk before any
/// bytes are written, so a file that changed since the plan was computed is
/// rejected rather than corrupted. Edits are applied back-to-front to keep byte
/// offsets valid as text is replaced.
fn apply_edits(
    command: &RenameCommand,
    source: &crate::source::SourceFile,
    conflicts: &[RenameConflict],
    edits: &[RenameEdit],
) -> Result<()> {
    if !conflicts.is_empty() {
        bail!(
            "refusing to apply: {} naming conflict(s); resolve them or rename to a free name",
            conflicts.len()
        );
    }
    if edits.is_empty() {
        return Ok(());
    }

    let current = fs::read_to_string(&command.target)
        .with_context(|| format!("re-read {}", command.target.display()))?;
    let current =
        crate::source::source_from_text(&command.target, current, command.language, false, None)?;

    for edit in edits {
        let line = current.line(edit.line).with_context(|| {
            format!(
                "line {} no longer exists in {}",
                edit.line,
                command.target.display()
            )
        })?;
        if line.hash() != edit.line_hash {
            bail!(
                "refusing to apply: {}:{} changed since the plan was computed",
                command.target.display(),
                edit.line
            );
        }
    }

    let old_name = source.text[edits[0].start_byte..edits[0].end_byte].to_owned();
    let text = rewrite(&current.text, edits, &old_name, &command.to)?;

    fs::write(&command.target, text)
        .with_context(|| format!("write {}", command.target.display()))?;
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
