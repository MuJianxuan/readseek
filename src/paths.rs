use crate::cli::DefinitionCommand;
use crate::flags::GitFlags;
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

pub(crate) fn definition_candidate_paths(
    command: &DefinitionCommand,
    search_name: &str,
) -> Result<Vec<PathBuf>> {
    let flags = GitFlags {
        cached: command.cached,
        others: command.others,
        ignored: command.ignored,
    };
    if let Some(paths) = git_definition_candidate_paths(&command.target, flags, search_name)? {
        return Ok(paths);
    }

    command_paths(&command.target, flags)
}

fn git_definition_candidate_paths(
    target: &Path,
    flags: GitFlags,
    search_name: &str,
) -> Result<Option<Vec<PathBuf>>> {
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
    let default_selection = !flags.has_any();
    let cached = flags.cached || default_selection;
    let others = flags.others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        collect_cached_definition_paths(&workdir, &output_root, scope, search_name, &mut paths)?;
    }
    if others {
        collect_other_definition_paths(
            &repository,
            &workdir,
            &output_root,
            scope,
            flags.ignored,
            search_name,
            &mut paths,
        )?;
    }

    Ok(Some(paths.into_iter().collect()))
}

fn collect_cached_definition_paths(
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    search_name: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let repository =
        git2::Repository::discover(workdir).context("discover git repository from workdir")?;
    let index = repository.index().context("read Git index")?;
    for entry in index.iter() {
        let relative = git_path(&entry.path)?;
        if !path_is_in_scope(&relative, scope) {
            continue;
        }

        if search_name.is_empty() {
            paths.insert(output_root.join(relative));
            continue;
        }
        let Ok(content) = fs::read(workdir.join(&relative)) else {
            continue;
        };
        if memchr::memmem::find(&content, search_name.as_bytes()).is_some() {
            paths.insert(output_root.join(relative));
        }
    }

    Ok(())
}

fn collect_other_definition_paths(
    repository: &git2::Repository,
    workdir: &Path,
    output_root: &Path,
    scope: &Path,
    ignored: bool,
    search_name: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let mut other_paths = BTreeSet::new();
    collect_other_paths(
        repository,
        workdir,
        output_root,
        scope,
        ignored,
        &mut other_paths,
    )?;

    for path in other_paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if text.contains(search_name) {
            paths.insert(path);
        }
    }

    Ok(())
}

fn git_search_paths(target: &Path, flags: GitFlags) -> Result<Option<Vec<PathBuf>>> {
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
    let default_selection = !flags.has_any();
    let cached = flags.cached || default_selection;
    let others = flags.others || default_selection;

    let mut paths = BTreeSet::new();
    if cached {
        collect_cached_paths(&repository, &output_root, scope, &mut paths)?;
    }
    if others {
        collect_other_paths(
            &repository,
            &workdir,
            &output_root,
            scope,
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
        if file_type.is_dir() {
            collect_search_paths(&path, paths)?;
        } else if file_type.is_file() {
            paths.push(path);
        }
    }

    Ok(())
}
