// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use std::process::Command;
use std::time::Instant;

use serde_json::Value;

const DATA: &str = include_str!("vision.txt");

fn main() {
    let bin = env!("CARGO_BIN_EXE_readseek");
    let image = format!("{}/tests/sonicware.jpg", env!("CARGO_MANIFEST_DIR"));

    for (line_number, raw) in DATA.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let case = Case::parse(line_number + 1, line);
        let args: Vec<_> = case
            .args
            .split_whitespace()
            .map(|arg| arg.replace("%s", &image))
            .collect();
        let started = Instant::now();
        let output = Command::new(bin)
            .args(&args)
            .output()
            .unwrap_or_else(|error| panic!("{}: start readseek: {error}", case.name));
        let elapsed = started.elapsed();

        if !output.status.success() {
            panic!(
                "{}: readseek exited {}; stderr={}",
                case.name,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap_or_else(|error| panic!("{}: invalid readseek JSON: {error}", case.name));
        println!("{}: {:.3?}", case.name, elapsed);
    }
}

struct Case<'a> {
    name: &'a str,
    args: &'a str,
}

impl<'a> Case<'a> {
    fn parse(line_number: usize, line: &'a str) -> Self {
        let mut name = None;
        let mut args = None;

        for field in line.split(" | ") {
            let (key, value) = field
                .split_once('=')
                .unwrap_or_else(|| panic!("line {line_number}: field without '=': {field}"));
            match key {
                "name" => name = Some(value),
                "args" => args = Some(value),
                _ => panic!("line {line_number}: unknown field `{key}`"),
            }
        }

        Self {
            name: name.unwrap_or_else(|| panic!("line {line_number}: missing name")),
            args: args.unwrap_or_else(|| panic!("line {line_number}: missing args")),
        }
    }
}
