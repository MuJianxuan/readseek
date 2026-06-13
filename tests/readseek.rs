// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use duct::Expression;
use std::path::{Path, PathBuf};

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

fn sandbox_command(repo_root: &Path) -> Expression {
    let script = repo_root.join("tests").join("sandbox.py");
    if cfg!(windows) {
        duct::cmd!("python", script)
    } else {
        duct::cmd!("/usr/bin/env", "python3", script)
    }
}
