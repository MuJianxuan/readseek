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
//! cursor file stays binding-accurate; for a lexical binding target, other files
//! retain only free uses when a binding table is available. Other targets use
//! name matching. `--apply` verifies all files before writing and attempts to
//! restore files already written if a later write fails.

use crate::engine::binding::{self, OccurrenceKind};
use crate::engine::flags::GitFlags;
use crate::engine::lang::Language;
use crate::engine::output::{RenameConflict, RenameEdit, RenameFileOutput, RenameOutput};
use crate::engine::paths::{command_paths, identifier_spans};
use crate::engine::source::{
    ContentCategory, SourceFile, read_source_containing, source_from_text,
};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Inputs for [`output`]: the cursor location, the new name, and expansion scope.
pub(crate) struct Request {
    pub(crate) target: PathBuf,
    pub(crate) line: usize,
    pub(crate) column: Option<usize>,
    pub(crate) to: String,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) apply: bool,
    pub(crate) language: Option<Language>,
    pub(crate) flags: GitFlags,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn output(request: &Request) -> Result<RenameOutput> {
    let line = request.line;
    let column = request.column.unwrap_or(1);
    if line == 0 || column == 0 {
        bail!("line and column must be greater than zero");
    }
    if request.to.is_empty() {
        bail!("new name must not be empty");
    }
    if !is_plain_identifier(&request.to) {
        bail!("new name must be a plain identifier");
    }
    if !request.target.is_file() {
        bail!("rename requires a single regular file target");
    }

    let bytes =
        fs::read(&request.target).with_context(|| format!("read {}", request.target.display()))?;
    let text = String::from_utf8(bytes).context("file is not valid UTF-8")?;
    let source = source_from_text(
        &request.target,
        text,
        request.language,
        ContentCategory::Text,
        None,
    );

    let cursor_byte = source.cursor_byte(line, column)?;

    // The cursor file is binding-accurate when its symbol resolves to a local
    // declaration. Symbols without a lexical binding (macros, top-level functions
    // and types) fall back to the same name-based plan used for the other files,
    // so they rename in a single file as well as across a workspace.
    let (old_name, conflicts, edits, is_binding) = if let Some((binding, raw_conflicts)) =
        binding::resolve_with_conflicts(&source, cursor_byte, Some(&request.to))
    {
        if binding.name == request.to {
            bail!("new name is identical to the current name");
        }
        let conflicts = raw_conflicts
            .into_iter()
            .map(|conflict| {
                let (line, column) = source.line_column(conflict.byte);
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
        (binding.name, conflicts, edits, true)
    } else {
        let name = binding::identifier_at(&source, cursor_byte).with_context(|| {
            format!(
                "no identifier at {}:{line}:{column}",
                request.target.display()
            )
        })?;
        if name == request.to {
            bail!("new name is identical to the current name");
        }
        let plan = build_other(&source, &name, &request.to, false).with_context(|| {
            format!(
                "`{name}` has no renamable occurrences in {}",
                request.target.display()
            )
        })?;
        (name, plan.conflicts, plan.edits, false)
    };

    let others = workspace_others(request, &old_name, is_binding)?;

    let applied = if request.apply {
        apply_all(
            request,
            &old_name,
            &source.file_hash,
            &edits,
            &conflicts,
            &others,
        )?;
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
        new_name: request.to.clone(),
        applied,
        conflicts,
        edits,
        others,
    })
}

/// The cursor file is excluded. Lexical binding targets use
/// [`binding::cross_file_matches`] (free uses only) where available; other
/// targets and unsupported languages use a plain name scan.
fn workspace_others(
    request: &Request,
    old_name: &str,
    target_is_binding: bool,
) -> Result<Vec<RenameFileOutput>> {
    let Some(workspace) = request.workspace.as_ref() else {
        return Ok(Vec::new());
    };
    let paths = command_paths(workspace, request.flags)?;
    let origin = request.target.canonicalize().ok();

    let mut others: Vec<RenameFileOutput> = paths
        .par_iter()
        .filter(|path| !is_origin(path, &request.target, origin.as_deref()))
        .filter_map(|path| {
            let source = read_source_containing(path, old_name, request.language)?;
            build_other(&source, old_name, &request.to, target_is_binding)
        })
        .collect();
    others.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(others)
}

/// Build the edit plan for a single non-cursor file, or `None` when nothing in
/// it matches the old name.
///
/// Binding-aware targets still use a byte-level name scan so preprocessor
/// bodies and parse-error subtrees are retained, then exclude same-file local
/// declarations and uses that resolve to them. Non-binding targets rename every
/// name match.
fn build_other(
    source: &SourceFile,
    old_name: &str,
    new_name: &str,
    target_is_binding: bool,
) -> Option<RenameFileOutput> {
    let mut occurrences = name_scan(source, old_name);
    if occurrences.is_empty() {
        return None;
    }

    let matches = binding::cross_file_matches(source, old_name, new_name);
    let (excluded, conflict_bytes) = if target_is_binding {
        match matches {
            Some(matches) => (matches.excluded, matches.conflicts),
            None => (Vec::new(), Vec::new()),
        }
    } else {
        let conflicts = matches.map(|matches| matches.conflicts).unwrap_or_default();
        (Vec::new(), conflicts)
    };
    if !excluded.is_empty() {
        occurrences.retain(|(start_byte, _)| excluded.binary_search(start_byte).is_err());
        if occurrences.is_empty() {
            return None;
        }
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
            let (line, column) = source.line_column(byte);
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
    identifier_spans(text, needle)
        .map(|index| (index, index + needle.len()))
        .collect()
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
    let line_idx = source.line_index(start_byte);
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
    request: &Request,
    old_name: &str,
    target_hash: &str,
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

    let plans: Vec<(&Path, &str, &[RenameEdit])> =
        std::iter::once((request.target.as_path(), target_hash, edits))
            .chain(others.iter().map(|other| {
                (
                    other.file.as_path(),
                    other.file_hash.as_str(),
                    other.edits.as_slice(),
                )
            }))
            .collect();
    let mut lock_paths: Vec<PathBuf> = plans
        .iter()
        .filter(|(_, _, edits)| !edits.is_empty())
        .map(|(path, _, _)| path.with_extension("readseek-rename.lock"))
        .collect();
    lock_paths.sort();
    lock_paths.dedup();
    let _locks: Vec<RenameLock> = lock_paths
        .into_iter()
        .map(RenameLock::acquire)
        .collect::<Result<_>>()?;

    // Verify every file and compute its new text before touching any of them.
    let mut writes: Vec<(&Path, String, String)> = Vec::new();
    for (path, expected_hash, edits) in plans {
        if edits.is_empty() {
            continue;
        }
        let raw_text =
            fs::read_to_string(path).with_context(|| format!("re-read {}", path.display()))?;
        let current = source_from_text(
            path,
            raw_text,
            request.language,
            ContentCategory::Text,
            None,
        );
        if current.file_hash != expected_hash {
            bail!(
                "refusing to apply: {} changed since the plan was computed",
                path.display()
            );
        }
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
        let new_text = rewrite(&current.text, edits, old_name, &request.to)?;
        writes.push((path, current.text, new_text));
    }

    // All files verified: write them, restoring on any failure.
    let mut written: Vec<(&Path, &str)> = Vec::new();
    for (path, original, new_text) in &writes {
        if let Err(error) = fs::write(path, new_text) {
            let _ = fs::write(path, original);
            for (done_path, done_original) in &written {
                let _ = fs::write(done_path, done_original);
            }
            return Err(error).with_context(|| format!("write {}", path.display()));
        }
        written.push((path, original));
    }
    Ok(())
}

struct RenameLock {
    path: PathBuf,
}

impl RenameLock {
    fn acquire(path: PathBuf) -> Result<Self> {
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        bail!("timed out waiting for rename lock {}", path.display());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("lock {}", path.display()));
                }
            }
        }
    }
}

impl Drop for RenameLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

/// Replace each edit span with `new_name` in one forward copy. Refuses unless
/// every span still holds `old_name`.
fn rewrite(source: &str, edits: &[RenameEdit], old_name: &str, new_name: &str) -> Result<String> {
    let line_starts = raw_line_starts(source);
    let mut ordered = Vec::with_capacity(edits.len());
    for edit in edits {
        let line_start = *line_starts
            .get(
                edit.line
                    .checked_sub(1)
                    .context("edit line is out of range")?,
            )
            .context("edit line is out of range")?;
        let start = line_start
            .checked_add(edit.start_column - 1)
            .context("edit span is out of range")?;
        let end = line_start
            .checked_add(edit.end_column - 1)
            .context("edit span is out of range")?;
        ordered.push((edit, start, end));
    }
    ordered.sort_by_key(|(_, start, _)| *start);
    let extra = new_name
        .len()
        .saturating_sub(old_name.len())
        .saturating_mul(ordered.len());
    let mut text = String::with_capacity(source.len().saturating_add(extra));
    let mut previous_end = 0;
    for (_, start, end) in ordered {
        if start < previous_end
            || end > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            bail!("refusing to apply: edit span is out of range");
        }
        if &source[start..end] != old_name {
            bail!("refusing to apply: a target span no longer holds `{old_name}`");
        }
        text.push_str(&source[previous_end..start]);
        text.push_str(new_name);
        previous_end = end;
    }
    text.push_str(&source[previous_end..]);
    Ok(text)
}

fn raw_line_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut offset = usize::from(text.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let mut starts = Vec::with_capacity(memchr::memchr_iter(b'\n', bytes).count() + 1);
    starts.push(offset);
    while offset < bytes.len() {
        match bytes[offset] {
            b'\r' if bytes.get(offset + 1) == Some(&b'\n') => {
                offset += 2;
                starts.push(offset);
            }
            b'\r' | b'\n' => {
                offset += 1;
                starts.push(offset);
            }
            _ => offset += 1,
        }
    }
    starts
}

fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_') && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
}
