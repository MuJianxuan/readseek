// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

const DATA: &str = include_str!("data.txt");

fn print_failed() {
    println!("failed");
}

fn print_ok() {
    println!("ok");
}

fn run_test(
    name: &str,
    test_fn: impl FnOnce() -> Result<(), String> + std::panic::UnwindSafe,
) -> bool {
    print!("Test {name} ... ");
    std::io::stdout().flush().expect("flush stdout");

    match std::panic::catch_unwind(test_fn) {
        Ok(Ok(())) => {
            print_ok();
            true
        }
        Ok(Err(reason)) => {
            print_failed();
            eprintln!("{reason}");
            false
        }
        Err(_) => {
            print_failed();
            false
        }
    }
}

fn main() {
    let bin = env!("CARGO_BIN_EXE_readseek");
    let mut failed_count = 0u32;
    let mut test_count = 0u32;

    for (index, raw) in DATA.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let case = Case::parse(index + 1, line);
        if let Some(platform) = case.platform {
            if platform == "unix" && !cfg!(unix) {
                continue;
            }
            if platform == "windows" && !cfg!(windows) {
                continue;
            }
        }
        test_count += 1;
        if !run_test(&case.name, || case.run(bin)) {
            failed_count += 1;
        }
    }

    eprintln!("\n{test_count} tests run.");
    if failed_count > 0 {
        eprintln!("{failed_count} test(s) failed.");
        std::process::exit(1);
    }
    eprintln!("All tests passed.");
}

struct Symbol<'a> {
    kind: &'a str,
    name: &'a str,
    fields: Vec<(&'a str, &'a str)>,
}

struct Case<'a> {
    name: String,
    path: Option<&'a str>,
    content: Option<&'a str>,
    stdin: Option<&'a str>,
    args: &'a str,
    status: Option<i32>,
    stderr: Option<&'a str>,
    platform: Option<&'a str>,
    symbols: Vec<Symbol<'a>>,
    checks: Vec<(&'a str, &'a str)>,
}

impl<'a> Case<'a> {
    fn parse(line_no: usize, line: &'a str) -> Self {
        let mut case = Case {
            name: format!("line {line_no}"),
            path: None,
            content: None,
            stdin: None,
            args: "",
            status: None,
            stderr: None,
            platform: None,
            symbols: Vec::new(),
            checks: Vec::new(),
        };
        for field in line.split(" | ") {
            let (key, value) = field
                .split_once('=')
                .unwrap_or_else(|| panic!("{}: field without '=': {field}", case.name));
            match key {
                "name" => case.name = value.to_owned(),
                "path" => case.path = Some(value),
                "content" => case.content = Some(value),
                "stdin" => case.stdin = Some(value),
                "args" => case.args = value,
                "status" => {
                    case.status = Some(value.parse().expect("status must be an integer"));
                }
                "stderr" => case.stderr = Some(value),
                "platform" => case.platform = Some(value),
                "symbol" => case.symbols.push(parse_symbol(value)),
                _ => case.checks.push((key, value)),
            }
        }
        case
    }

    fn run(&self, bin: &str) -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!(
            "readseek-data-{}-{}",
            std::process::id(),
            self.name.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create case dir");

        let file_path = self.path.map(|path| {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(&target, unescape_bytes(self.content.unwrap_or("")))
                .expect("write file");
            target
        });

        let result = self.invoke(bin, file_path.as_ref());
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn invoke(&self, bin: &str, file_path: Option<&PathBuf>) -> Result<(), String> {
        let path_str = file_path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let args: Vec<String> = self
            .args
            .split_whitespace()
            .map(|token| token.replace("%s", &path_str))
            .collect();

        let mut command = Command::new(bin);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|e| format!("spawn: {e}"))?;
        if let Some(stdin) = self.stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&unescape_bytes(stdin))
                .map_err(|e| format!("write stdin: {e}"))?;
        }
        let output = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if let Some(expected) = self.status {
            if code != expected {
                return Err(format!(
                    "exit {code}, expected {expected}; stderr={}",
                    stderr.trim()
                ));
            }
            if let Some(needle) = self.stderr
                && !stderr.contains(needle)
            {
                return Err(format!("stderr missing `{needle}`: {}", stderr.trim()));
            }
            return Ok(());
        }

        if code != 0 {
            return Err(format!("exit {code}; stderr={}", stderr.trim()));
        }
        let json: Value =
            serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid json: {e}"))?;
        self.check(&json)
    }

    fn check(&self, json: &Value) -> Result<(), String> {
        for (path, expected) in &self.checks {
            check_path(json, path, expected, json)?;
        }
        for symbol in &self.symbols {
            let entry = json
                .get("symbols")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|item| {
                    item.get("kind").and_then(Value::as_str) == Some(symbol.kind)
                        && item.get("name").and_then(Value::as_str) == Some(symbol.name)
                })
                .ok_or_else(|| format!("missing symbol {} `{}`", symbol.kind, symbol.name))?;
            for (field, expected) in &symbol.fields {
                check_path(entry, field, expected, entry)?;
            }
        }
        Ok(())
    }
}

fn parse_symbol(value: &str) -> Symbol<'_> {
    let mut parts = value.split(',');
    let kind = parts.next().expect("symbol kind");
    let name = parts.next().expect("symbol name");
    let fields = parts
        .map(|field| field.split_once('=').expect("symbol field needs '='"))
        .collect();
    Symbol { kind, name, fields }
}

fn check_path(base: &Value, path: &str, expected: &str, root: &Value) -> Result<(), String> {
    if let Some(prefix) = path.strip_suffix(".absent") {
        let actual = resolve(base, prefix).is_none();
        let expected = match expected {
            "true" => true,
            "false" => false,
            _ => return Err(format!("{path}: expected true or false, got {expected}")),
        };
        return if actual == expected {
            Ok(())
        } else {
            Err(format!("{prefix}.absent = {actual}, expected {expected}"))
        };
    }

    if let Some(prefix) = path.strip_suffix(".len") {
        let value = resolve(base, prefix).ok_or_else(|| format!("{path}: not found"))?;
        let len = match value {
            Value::Array(items) => items.len(),
            Value::String(text) => text.chars().count(),
            other => return Err(format!("{prefix}: not array or string ({other})")),
        };
        let want: usize = expected
            .parse()
            .map_err(|_| format!("{path}: bad length {expected}"))?;
        return if len == want {
            Ok(())
        } else {
            Err(format!("{path} = {len}, expected {want}"))
        };
    }

    let actual = resolve(base, path).ok_or_else(|| format!("{path}: not found"))?;
    if compare(actual, expected, base, root) {
        Ok(())
    } else {
        Err(format!("{path} = {actual}, expected {expected}"))
    }
}

fn resolve<'v>(value: &'v Value, path: &str) -> Option<&'v Value> {
    let mut current = value;
    for component in path.split('.') {
        current = match current {
            Value::Array(items) => items.get(component.parse::<usize>().ok()?)?,
            _ => current.get(component)?,
        };
    }
    Some(current)
}

fn compare(actual: &Value, expected: &str, base: &Value, root: &Value) -> bool {
    if let Some(reference) = expected.strip_prefix('@') {
        let from = match reference.strip_prefix("root.") {
            Some(rest) => resolve(root, rest),
            None => resolve(base, reference),
        };
        return from == Some(actual);
    }
    match expected {
        "true" => actual.as_bool() == Some(true),
        "false" => actual.as_bool() == Some(false),
        _ => {
            if let Ok(number) = expected.parse::<i64>() {
                actual.as_i64() == Some(number)
            } else {
                actual.as_str() == Some(unescape_str(expected).as_str())
            }
        }
    }
}

fn unescape_str(text: &str) -> String {
    String::from_utf8(unescape_bytes(text)).expect("escaped value is not UTF-8")
}

fn unescape_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next().expect("trailing backslash") {
            'n' => out.push(b'\n'),
            't' => out.push(b'\t'),
            'r' => out.push(b'\r'),
            '\\' => out.push(b'\\'),
            '|' => out.push(b'|'),
            'x' => {
                let hex: String = (0..2)
                    .map(|_| chars.next().expect("\\x needs two hex digits"))
                    .collect();
                out.push(u8::from_str_radix(&hex, 16).expect("invalid \\x escape"));
            }
            'u' => {
                let hex: String = (0..4)
                    .map(|_| chars.next().expect("\\u needs four hex digits"))
                    .collect();
                let code = u32::from_str_radix(&hex, 16).expect("invalid \\u escape");
                let ch = char::from_u32(code).expect("invalid \\u code point");
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            other => panic!("unknown escape \\{other}"),
        }
    }
    out
}
