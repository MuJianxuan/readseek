use crate::cli::DefCommand;
use crate::flags::GitFlags;
use crate::output::is_identifier_byte;
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn command_paths(target: &Path, flags: GitFlags) -> Result<Vec<PathBuf>> {
    let metadata = fs::metadata(target).with_context(|| format!("stat {}", target.display()))?;
    if metadata.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if !metadata.is_dir() {
        bail!(
            "search target is not a file or directory: {}",
            target.display()
        );
    }

    if let Some(paths) = git_search_paths(target, flags)? {
        return Ok(paths);
    }

    if flags.has_any() {
        log::debug!(
            "ignoring Git file selection flags outside repository: {}",
            target.display()
        );
    }

    let mut paths = Vec::new();
    collect_search_paths(target, &mut paths)?;
    Ok(paths)
}

pub(crate) fn def_candidate_paths(command: &DefCommand, search_name: &str) -> Result<Vec<PathBuf>> {
    let flags = GitFlags {
        cached: command.cached,
        others: command.others,
        ignored: command.ignored,
    };
    if let Some(paths) = git_candidate_paths(&command.target, flags, search_name)? {
        return Ok(paths);
    }

    command_paths(&command.target, flags)
}

struct GitScope {
    repository: git2::Repository,
    workdir: PathBuf,
    output_root: PathBuf,
    scope: PathBuf,
}

fn resolve_git_scope(target: &Path, flags: GitFlags) -> Result<Option<GitScope>> {
    let original_target = target;
    let Ok(repository) = git2::Repository::discover(target) else {
        return Ok(None);
    };

    flags.validate()?;

    let workdir = repository
        .workdir()
        .context("Git repository has no work tree")?;
    let target = target
        .canonicalize()
        .with_context(|| format!("canonicalize {}", target.display()))?;
    let workdir = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize {}", workdir.display()))?;
    let scope = target
        .strip_prefix(&workdir)
        .with_context(|| format!("{} is outside Git work tree", target.display()))?;
    let output_root = output_root_for_scope(original_target, scope)?;

    Ok(Some(GitScope {
        repository,
        workdir,
        output_root,
        scope: scope.to_path_buf(),
    }))
}

fn git_candidate_paths(
    target: &Path,
    flags: GitFlags,
    search_name: &str,
) -> Result<Option<Vec<PathBuf>>> {
    let Some(scope) = resolve_git_scope(target, flags)? else {
        return Ok(None);
    };
    let default_selection = !flags.has_any();
    let cached = flags.cached || default_selection;
    let others = flags.others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        let index = scope.repository.index().context("read Git index")?;
        for entry in index.iter() {
            let relative = git_path(&entry.path)?;
            if !path_is_in_scope(&relative, &scope.scope) {
                continue;
            }

            if search_name.is_empty() {
                paths.insert(scope.output_root.join(&relative));
                continue;
            }
            let Ok(content) = fs::read(scope.workdir.join(&relative)) else {
                continue;
            };
            if bytes_contain_identifier(&content, search_name.as_bytes()) {
                paths.insert(scope.output_root.join(relative));
            }
        }
    }
    if others {
        let mut other_paths = BTreeSet::new();
        collect_other_paths(
            &scope.repository,
            &scope.workdir,
            &scope.output_root,
            &scope.scope,
            flags.ignored,
            &mut other_paths,
        )?;

        if search_name.is_empty() {
            paths.extend(other_paths);
        } else {
            for path in other_paths {
                let Ok(content) = fs::read(&path) else {
                    continue;
                };
                if bytes_contain_identifier(&content, search_name.as_bytes()) {
                    paths.insert(path);
                }
            }
        }
    }

    Ok(Some(paths.into_iter().collect()))
}

pub(crate) fn bytes_contain_identifier(text: &[u8], identifier: &[u8]) -> bool {
    if identifier.is_empty() {
        return true;
    }

    identifier_spans(text, identifier).next().is_some()
}

/// Return byte offsets where `identifier` appears as a whole identifier.
pub(crate) fn identifier_spans<'a>(
    text: &'a [u8],
    identifier: &'a [u8],
) -> impl Iterator<Item = usize> + 'a {
    memchr::memmem::find_iter(text, identifier).filter(|&byte_index| {
        let before = byte_index.checked_sub(1).map(|i| text[i]);
        let after = text.get(byte_index + identifier.len()).copied();
        !identifier.is_empty()
            && !before.is_some_and(is_identifier_byte)
            && !after.is_some_and(is_identifier_byte)
    })
}

fn git_search_paths(target: &Path, flags: GitFlags) -> Result<Option<Vec<PathBuf>>> {
    let Some(scope) = resolve_git_scope(target, flags)? else {
        return Ok(None);
    };
    let default_selection = !flags.has_any();
    let cached = flags.cached || default_selection;
    let others = flags.others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        collect_cached_paths(
            &scope.repository,
            &scope.output_root,
            &scope.scope,
            &mut paths,
        )?;
    }
    if others {
        collect_other_paths(
            &scope.repository,
            &scope.workdir,
            &scope.output_root,
            &scope.scope,
            flags.ignored,
            &mut paths,
        )?;
    }

    Ok(Some(paths.into_iter().collect()))
}

fn output_root_for_scope(target: &Path, scope: &Path) -> Result<PathBuf> {
    let mut output_root = target.to_path_buf();
    for _ in scope.components() {
        if !output_root.pop() {
            bail!("{} is outside Git work tree", target.display());
        }
    }
    Ok(output_root)
}

fn collect_cached_paths(
    repository: &git2::Repository,
    output_root: &Path,
    scope: &Path,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let index = repository.index().context("read Git index")?;
    for entry in index.iter() {
        let relative = git_path(&entry.path)?;
        if path_is_in_scope(&relative, scope) {
            paths.insert(output_root.join(relative));
        }
    }

    Ok(())
}

fn collect_other_paths(
    repository: &git2::Repository,
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    ignored: bool,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    if ignored {
        options.include_ignored(true).recurse_ignored_dirs(true);
    }

    for entry in repository.statuses(Some(&mut options))?.iter() {
        let status = entry.status();
        let include = status.contains(git2::Status::WT_NEW)
            || (ignored && status.contains(git2::Status::IGNORED));
        if !include {
            continue;
        }

        let Some(relative) = entry.path().map(PathBuf::from) else {
            continue;
        };
        insert_scoped_file(workdir, output_root, scope, &relative, paths);
    }

    Ok(())
}

fn insert_scoped_file(
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    relative: &Path,
    paths: &mut BTreeSet<PathBuf>,
) {
    if !path_is_in_scope(relative, scope) {
        return;
    }

    let path = workdir.join(relative);
    if path.is_file() {
        paths.insert(output_root.join(relative));
    }
}

fn git_path(path: &[u8]) -> Result<PathBuf> {
    let path = std::str::from_utf8(path).context("Git index path is not UTF-8")?;
    Ok(PathBuf::from(path))
}

fn path_is_in_scope(path: &Path, scope: &Path) -> bool {
    scope.as_os_str().is_empty() || path.starts_with(scope)
}

fn collect_search_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("read directory entry from {}", directory.display()))?;
    entries.sort_by_key(std::fs::DirEntry::path);

    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read file type for {}", path.display()))?;
        let is_readseek = file_type.is_dir() && entry.file_name() == ".readseek";
        if is_readseek {
            continue;
        }
        if file_type.is_dir() {
            collect_search_paths(&path, paths)?;
        } else if file_type.is_file() {
            paths.push(path);
        }
    }

    Ok(())
}
