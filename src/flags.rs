// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GitFlags {
    pub(crate) cached: bool,
    pub(crate) others: bool,
    pub(crate) ignored: bool,
}

impl GitFlags {
    pub(crate) fn validate(self) -> Result<()> {
        if self.ignored && !self.others {
            bail!("--ignored requires --others");
        }
        Ok(())
    }

    pub(crate) fn has_any(self) -> bool {
        self.cached || self.others || self.ignored
    }
}
