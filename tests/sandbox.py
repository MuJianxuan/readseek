#!/usr/bin/env python3

# SPDX-License-Identifier: LGPL-2.1-or-later
# Copyright (c) 2026 Jarkko Sakkinen

"""Integration tests for readseek."""

import json
import os
import subprocess
import sys
import tempfile


def main():
    readseek_bin = os.environ.get("READSEEK_BIN", "")
    if not readseek_bin:
        print("FAIL READSEEK_BIN not set")
        sys.exit(1)

    pass_count = 0
    fail_count = 0
    sandbox_home = None
    cache_home = None

    def passed(name):
        nonlocal pass_count
        pass_count += 1
        print(f"PASS {name}")

    def failed(name, reason):
        nonlocal fail_count
        fail_count += 1
        print(f"FAIL {name} -- {reason}")

    def run(args):
        env = os.environ.copy()
        if sandbox_home is not None:
            env["HOME"] = sandbox_home
            env.pop("XDG_CACHE_HOME", None)
        return subprocess.run(
            [readseek_bin] + args,
            capture_output=True,
            env=env,
            text=True,
        )

    def readseek_json(name, args):
        result = run(args)
        if result.returncode != 0:
            failed(name, f"expected status 0, got {result.returncode} stderr={result.stderr[:200]}")
            return None
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError as error:
            failed(name, f"invalid json: {error} stdout={result.stdout[:400]}")
            return None

    def expect_failure(name, args):
        result = run(args)
        if result.returncode != 0:
            passed(name)
        else:
            failed(name, f"expected failure but got success. stdout={result.stdout[:200]}")

    def write_file(directory, name, contents):
        path = os.path.join(directory, name)
        mode = "wb" if isinstance(contents, bytes) else "w"
        with open(path, mode) as file:
            file.write(contents)
        return path

    def assert_equal(name, actual, expected):
        if actual != expected:
            failed(name, f"expected {expected!r}, got {actual!r}")
            return False
        return True

    def assert_true(name, value, reason):
        if not value:
            failed(name, reason)
            return False
        return True

    def assert_symbol(name, symbols, kind, symbol_name):
        if any(symbol.get("kind") == kind and symbol.get("name") == symbol_name for symbol in symbols):
            return True
        failed(name, f"missing {kind} symbol {symbol_name}: {symbols!r}")
        return False

    def expect_mapped_symbol(name, file_name, contents, language, kind, symbol_name):
        path = write_file(tmpdir, file_name, contents)
        data = readseek_json(name, ["map", path])
        if not data:
            return

        symbols = data.get("symbols", [])
        if all(
            [
                assert_equal(name, data.get("language"), language),
                assert_symbol(name, symbols, kind, symbol_name),
            ]
        ):
            passed(name)

    def expect_supported_file(name, file_name, contents, language):
        path = write_file(tmpdir, file_name, contents)
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), language),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

    with tempfile.TemporaryDirectory() as tmpdir:
        sandbox_home = os.path.join(tmpdir, "home")
        os.mkdir(sandbox_home)
        if sys.platform == "darwin":
            cache_home = os.path.join(sandbox_home, "Library", "Caches")
        else:
            cache_home = os.path.join(sandbox_home, ".cache")
        name = "file: rust file"
        path = write_file(tmpdir, "sample.rs", "fn main() {}\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "rust"),
                assert_equal(name, data.get("supported"), True),
                assert_equal(name, data.get("binary"), False),
            ]
        ):
            passed(name)

        name = "read: requested range"
        path = write_file(tmpdir, "sample.py", "one\ntwo\nthree\n")
        data = readseek_json(name, ["read", path, "--start", "2", "--end", "3"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "python"),
                assert_equal(name, data.get("line_count"), 3),
                assert_equal(name, data.get("start_line"), 2),
                assert_equal(name, data.get("end_line"), 3),
                assert_equal(name, len(data.get("hashlines", [])), 2),
                assert_equal(name, data["hashlines"][0].get("line"), 2),
                assert_equal(name, data["hashlines"][0].get("text"), "two"),
                assert_equal(name, len(data["hashlines"][0].get("hash", "")), 3),
            ]
        ):
            passed(name)

        name = "read: offset and limit"
        path = write_file(tmpdir, "offset.txt", "one\ntwo\nthree\nfour\n")
        data = readseek_json(name, ["read", path, "--offset", "2", "--limit", "2"])
        if data and all(
            [
                assert_equal(name, data.get("line_count"), 4),
                assert_equal(name, data.get("start_line"), 2),
                assert_equal(name, data.get("end_line"), 3),
                assert_equal(name, len(data.get("hashlines", [])), 2),
                assert_equal(name, data["hashlines"][0].get("line"), 2),
                assert_equal(name, data["hashlines"][0].get("text"), "two"),
                assert_equal(name, data["hashlines"][1].get("line"), 3),
                assert_equal(name, data["hashlines"][1].get("text"), "three"),
            ]
        ):
            passed(name)

        name = "read: limit clamps to end"
        path = write_file(tmpdir, "clamp.txt", "one\ntwo\nthree\n")
        data = readseek_json(name, ["read", path, "--offset", "2", "--limit", "99"])
        if data and all(
            [
                assert_equal(name, data.get("line_count"), 3),
                assert_equal(name, data.get("start_line"), 2),
                assert_equal(name, data.get("end_line"), 3),
                assert_equal(name, len(data.get("hashlines", [])), 2),
            ]
        ):
            passed(name)

        name = "read: plain text normalizes"
        path = write_file(tmpdir, "normalized.txt", "\ufeffone\r\ntwo\rthree\n")
        data = readseek_json(name, ["read", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "unknown"),
                assert_equal(name, data.get("line_count"), 3),
                assert_equal(name, data["hashlines"][0].get("text"), "one"),
                assert_equal(name, data["hashlines"][1].get("text"), "two"),
                assert_equal(name, data["hashlines"][2].get("text"), "three"),
            ]
        ):
            passed(name)

        name = "read: preserves interior blank lines"
        path = write_file(tmpdir, "blank-lines.txt", "one\n\nthree\n")
        data = readseek_json(name, ["read", path])
        if data and all(
            [
                assert_equal(name, data.get("line_count"), 3),
                assert_equal(name, len(data.get("hashlines", [])), 3),
                assert_equal(name, data["hashlines"][1].get("line"), 2),
                assert_equal(name, data["hashlines"][1].get("text"), ""),
            ]
        ):
            passed(name)

        name = "read: empty file has zero lines"
        path = write_file(tmpdir, "empty.txt", "")
        data = readseek_json(name, ["read", path])
        if data and all(
            [
                assert_equal(name, data.get("line_count"), 0),
                assert_equal(name, data.get("start_line"), 1),
                assert_equal(name, data.get("end_line"), 0),
                assert_equal(name, data.get("hashlines"), []),
            ]
        ):
            passed(name)

        name = "read: target line"
        path = write_file(tmpdir, "target-line.txt", "one\ntwo\nthree\n")
        data = readseek_json(name, ["read", f"{path}:2", "--limit", "1"])
        if data and all(
            [
                assert_equal(name, data.get("start_line"), 2),
                assert_equal(name, data.get("end_line"), 2),
                assert_equal(name, data["hashlines"][0].get("text"), "two"),
            ]
        ):
            passed(name)

        name = "read: target hash"
        path = write_file(tmpdir, "target-hash.txt", "one\ntwo\nthree\n")
        whole = readseek_json(name, ["read", path])
        if whole:
            line_hash = whole["hashlines"][1]["hash"]
            data = readseek_json(name, ["read", f"{path}:{line_hash}", "--limit", "1"])
            if data and all(
                [
                    assert_equal(name, data.get("start_line"), 2),
                    assert_equal(name, data.get("end_line"), 2),
                    assert_equal(name, data["hashlines"][0].get("text"), "two"),
                ]
            ):
                passed(name)

        name = "map: javascript symbols"
        path = write_file(
            tmpdir,
            "sample.js",
            "class Greeter {\n  greet() { return 'hello'; }\n}\nconst make = () => new Greeter();\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "javascript"),
                    assert_symbol(name, symbols, "class", "Greeter"),
                    assert_symbol(name, symbols, "method", "greet"),
                    assert_symbol(name, symbols, "function", "make"),
                ]
            ):
                passed(name)

        name = "map: jsx symbols"
        path = write_file(
            tmpdir,
            "sample.jsx",
            "export function App() {\n  return <main>Hello</main>;\n}\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "jsx"),
                    assert_symbol(name, symbols, "function", "App"),
                ]
            ):
                passed(name)

        name = "map: typescript symbols"
        path = write_file(
            tmpdir,
            "sample.ts",
            "interface Named { name: string; }\nclass Greeter implements Named {\n  name = 'reader';\n  greet() { return 'hello'; }\n}\nconst make = (): Greeter => new Greeter();\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "typescript"),
                    assert_symbol(name, symbols, "interface", "Named"),
                    assert_symbol(name, symbols, "class", "Greeter"),
                    assert_symbol(name, symbols, "method", "greet"),
                    assert_symbol(name, symbols, "function", "make"),
                    assert_true(
                        name,
                        os.path.exists(os.path.join(cache_home, "readseek", "cache.sqlite3")),
                        "cache database missing",
                    ),
                ]
            ):
                passed(name)

        name = "map: tsx symbols"
        path = write_file(
            tmpdir,
            "sample.tsx",
            "type Props = { name: string };\nexport function App(props: Props) {\n  return <main>{props.name}</main>;\n}\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "tsx"),
                    assert_symbol(name, symbols, "type", "Props"),
                    assert_symbol(name, symbols, "function", "App"),
                ]
            ):
                passed(name)

        name = "search: cpp pattern directory"
        search_dir = os.path.join(tmpdir, "search")
        os.mkdir(search_dir)
        path = write_file(
            search_dir,
            "sample.cpp",
            "int greet() {\n  return 1;\n}\n",
        )
        write_file(search_dir, "notes.txt", "int ignored() { return 0; }\n")
        data = readseek_json(
            name,
            [
                "search",
                search_dir,
                "int $NAME() { return $VALUE; }",
            ],
        )
        if data:
            results = data.get("results", [])
            matches = results[0].get("matches", []) if results else []
            captures = matches[0].get("captures", []) if matches else []
            capture_names = [capture.get("name") for capture in captures]
            if all(
                [
                    assert_equal(name, len(results), 1),
                    assert_equal(name, results[0].get("file"), path),
                    assert_equal(name, results[0].get("language"), "cpp"),
                    assert_equal(name, len(results[0].get("file_hash", "")), 64),
                    assert_equal(name, len(matches), 1),
                    assert_equal(name, matches[0].get("start_line"), 1),
                    assert_equal(name, matches[0].get("end_line"), 3),
                    assert_equal(name, len(matches[0].get("hashlines", [])), 3),
                    assert_true(name, "NAME" in capture_names, "NAME capture missing"),
                    assert_true(name, "VALUE" in capture_names, "VALUE capture missing"),
                ]
            ):
                passed(name)

        expect_mapped_symbol(
            "map: makefile target",
            "Makefile",
            "build: main.c\n\tcc -o app main.c\n",
            "make",
            "target",
            "build",
        )

        expect_mapped_symbol(
            "map: justfile recipe",
            "Justfile",
            "build:\n    cargo build\n",
            "just",
            "recipe",
            "build",
        )

        expect_supported_file(
            "file: meson build",
            "meson.build",
            "project('sample', 'c')\n",
            "meson",
        )

        supported_files = [
            (
                "html",
                "index.html",
                "<!doctype html><html><body>Hello</body></html>\n",
            ),
            (
                "xml",
                "sample.xml",
                "<?xml version=\"1.0\"?><note><body>Hello</body></note>\n",
            ),
            ("json", "sample.json", "{\"name\": \"sample\"}\n"),
            ("yaml", "sample.yaml", "name: sample\n"),
            ("puppet", "site.pp", "class profile::sample {}\n"),
        ]
        for language, file_name, contents in supported_files:
            expect_supported_file(
                f"file: {language}",
                file_name,
                contents,
                language,
            )

        name = "symbol: cached mapped address"
        path = os.path.join(tmpdir, "sample.ts")
        data = readseek_json(name, ["symbol", path, "Greeter.greet"])
        if data and all(
            [
                assert_equal(name, data["symbol"].get("kind"), "method"),
                assert_equal(name, data["symbol"].get("name"), "greet"),
                assert_equal(name, data["symbol"].get("address"), "Greeter.greet"),
            ]
        ):
            passed(name)

        name = "symbol: target suffix"
        path = write_file(
            tmpdir,
            "symbol.ts",
            "class Greeter {\n  greet() {\n    return 'hello';\n  }\n}\n",
        )
        data = readseek_json(name, ["symbol", f"{path}:Greeter.greet"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "typescript"),
                assert_equal(name, data["symbol"].get("kind"), "method"),
                assert_equal(name, data["symbol"].get("name"), "greet"),
                assert_equal(name, data["symbol"].get("address"), "Greeter.greet"),
                assert_equal(name, len(data.get("hashlines", [])), 3),
                assert_equal(name, data["hashlines"][0].get("line"), 2),
            ]
        ):
            passed(name)

        name = "symbol: target line"
        data = readseek_json(name, ["symbol", f"{path}:3"])
        if data and all(
            [
                assert_equal(name, data["symbol"].get("kind"), "method"),
                assert_equal(name, data["symbol"].get("name"), "greet"),
                assert_equal(name, data["hashlines"][0].get("line"), 2),
            ]
        ):
            passed(name)

        name = "cli: no arguments"
        result = run([])
        if all(
            [
                assert_equal(name, result.returncode, 2),
                assert_true(name, "Usage: readseek" in result.stderr, "usage missing from stderr"),
            ]
        ):
            passed(name)

        name = "map: cpp symbols"
        path = write_file(
            tmpdir,
            "sample.cpp",
            "namespace demo {\nclass Thing {\npublic:\n  int value();\n};\n}\nint add(int a, int b) { return a + b; }\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_symbol(name, symbols, "namespace", "demo"),
                    assert_symbol(name, symbols, "class", "Thing"),
                    assert_symbol(name, symbols, "function", "add"),
                ]
            ):
                passed(name)

        name = "file: language override"
        path = write_file(tmpdir, "script", "#!/usr/bin/env node\nfunction main() {}\n")
        data = readseek_json(name, ["file", path, "--language", "typescript"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "typescript"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: language alias"
        path = write_file(tmpdir, "header", "int add(int a, int b);\n")
        data = readseek_json(name, ["file", path, "--language", "c-plus-plus"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "cpp"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: special filename"
        path = write_file(tmpdir, "go.mod", "module example.com/readseek\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "go"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: tex language"
        path = write_file(tmpdir, "paper.tex", "\\section{Intro}\nText.\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "latex"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: typst language"
        path = write_file(tmpdir, "paper.typ", "= Intro\nText.\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "typst"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: assembly language"
        path = write_file(tmpdir, "boot.S", "_start:\n    mov %rsp, %rbp\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "assembly"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: riscv language"
        path = write_file(tmpdir, "boot.riscv", "_start:\n    addi sp, sp, -16\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "riscv"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: gdscript language"
        path = write_file(tmpdir, "player.gd", "func _ready():\n    pass\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "gdscript"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: sql language"
        path = write_file(tmpdir, "query.sql", "select * from users;\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "sql"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: toml language"
        path = write_file(tmpdir, "config.toml", "name = \"demo\"\n")
        data = readseek_json(name, ["file", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "toml"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: binary rejected"
        path = write_file(tmpdir, "blob.bin", bytes([0, 1, 2, 3]))
        expect_failure(name, ["file", path])

        name = "read: binary uses lossy text"
        path = write_file(tmpdir, "blob-read.bin", bytes([0, 65, 255, 10]))
        data = readseek_json(name, ["read", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "unknown"),
                assert_equal(name, data.get("line_count"), 1),
                assert_equal(name, data["hashlines"][0].get("text"), "\x00A�"),
            ]
        ):
            passed(name)

        name = "map: unknown text has no symbols"
        path = write_file(tmpdir, "notes.txt", "alpha\nbeta\n")
        data = readseek_json(name, ["map", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "unknown"),
                assert_equal(name, data.get("symbols"), []),
            ]
        ):
            passed(name)

        name = "map: markdown headings"
        path = write_file(
            tmpdir,
            "notes.md",
            "# Title\n\nSome text.\n\n## Details ##\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "markdown"),
                    assert_symbol(name, symbols, "heading", "Title"),
                    assert_symbol(name, symbols, "heading", "Details"),
                ]
            ):
                passed(name)

        name = "map: kconfig symbols"
        path = write_file(
            tmpdir,
            "Kconfig",
            "menu \"Drivers\"\n\nconfig USB_SUPPORT\n    bool \"USB support\"\n\nmenuconfig NET\n    bool \"Networking support\"\n\nendmenu\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "kconfig"),
                    assert_symbol(name, symbols, "menu", "Drivers"),
                    assert_symbol(name, symbols, "config", "USB_SUPPORT"),
                    assert_symbol(name, symbols, "menuconfig", "NET"),
                ]
            ):
                passed(name)

        name = "map: swift symbols"
        path = write_file(
            tmpdir,
            "sample.swift",
            "actor BankAccount {\n    var balance: Double = 0\n    func deposit(_ amount: Double) {\n        balance += amount\n    }\n}\n",
        )
        data = readseek_json(name, ["map", path])
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("language"), "swift"),
                    assert_symbol(name, symbols, "class", "actor BankAccount"),
                    assert_symbol(name, symbols, "function", "func deposit(_ amount: Double)"),
                ]
            ):
                passed(name)

    print(f"SUMMARY pass={pass_count} fail={fail_count}")
    if fail_count != 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
