// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub(crate) struct Ignorer {
    patterns: Vec<IgnorePattern>,
}

#[derive(Clone, Debug)]
struct IgnorePattern {
    glob: glob::Pattern,
    negated: bool,
    dir_only: bool,
}

impl Ignorer {
    pub(crate) fn load(readseek_dir: &Path) -> Option<Self> {
        let ignore_path = readseek_dir.join("ignore");
        let contents = fs::read_to_string(&ignore_path).ok()?;
        Some(Self::parse(&contents))
    }

    fn parse(text: &str) -> Self {
        let mut patterns = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, rest) = line
                .strip_prefix('!')
                .map_or((false, line), |rest| (true, rest));
            let dir_only = rest.ends_with('/');
            let pattern_str = if dir_only {
                &rest[..rest.len() - 1]
            } else {
                rest
            };
            if pattern_str.is_empty() {
                continue;
            }
            let Ok(glob) = glob::Pattern::new(pattern_str) else {
                continue;
            };
            patterns.push(IgnorePattern {
                glob,
                negated,
                dir_only,
            });
        }
        Self { patterns }
    }

    pub(crate) fn is_ignored(&self, name: &OsStr, is_dir: bool) -> bool {
        let Some(name_str) = name.to_str() else {
            return false;
        };

        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.dir_only && !is_dir {
                continue;
            }
            if pattern.glob.matches(name_str) {
                ignored = !pattern.negated;
            }
        }
        ignored
    }
}
