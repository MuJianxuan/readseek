// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use duct::{Expression, cmd};
use std::path::PathBuf;

#[test]
fn sandbox_script() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let readseek = env!("CARGO_BIN_EXE_readseek");

    sandbox_command(&repo_root)
        .dir(&repo_root)
        .env("READSEEK_BIN", readseek)
        .env("READSEEK_VERSION", env!("CARGO_PKG_VERSION"))
        .run()
        .expect("sandbox script failed");
}

fn sandbox_command(repo_root: &std::path::Path) -> Expression {
    cmd(
        "/usr/bin/env",
        [
            "python3".to_owned(),
            repo_root
                .join("tests")
                .join("sandbox.py")
                .to_string_lossy()
                .into_owned(),
        ],
    )
}
