// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use std::process::Command;

use serde_json::Value;

#[test]
fn apply_rejects_a_changed_rename_plan() {
    let bin = env!("CARGO_BIN_EXE_readseek");
    let directory =
        std::env::temp_dir().join(format!("readseek-rename-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create test directory");
    let file = directory.join("input.ts");
    std::fs::write(&file, "let value = 1;\nconsole.log(value);\n").expect("write source file");

    let plan = Command::new(bin)
        .args([
            "rename",
            file.to_str().expect("UTF-8 test path"),
            "--line",
            "1",
            "--column",
            "5",
            "--to",
            "renamed",
        ])
        .output()
        .expect("run rename plan");
    assert!(
        plan.status.success(),
        "plan failed: {}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let output: Value = serde_json::from_slice(&plan.stdout).expect("parse rename plan");
    let plan_hash = output["plan_hash"].as_str().expect("rename plan hash");

    let changed = "// changed after permission\nlet value = 1;\nconsole.log(value);\n";
    std::fs::write(&file, changed).expect("change source file");
    let apply = Command::new(bin)
        .args([
            "rename",
            file.to_str().expect("UTF-8 test path"),
            "--line",
            "2",
            "--column",
            "5",
            "--to",
            "renamed",
            "--plan-hash",
            plan_hash,
            "--apply",
        ])
        .output()
        .expect("run rename apply");

    assert!(!apply.status.success());
    assert!(
        String::from_utf8_lossy(&apply.stderr).contains("rename plan changed"),
        "unexpected error: {}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&file).expect("read source file"),
        changed
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}
