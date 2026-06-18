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

    def passed(name):
        nonlocal pass_count
        pass_count += 1
        print(f"PASS {name}")

    def failed(name, reason):
        nonlocal fail_count
        fail_count += 1
        print(f"FAIL {name} -- {reason}")

    def run(args, stdin=None):
        env = os.environ.copy()
        if sandbox_home is not None:
            env["HOME"] = sandbox_home
            env.pop("XDG_CACHE_HOME", None)
        return subprocess.run(
            [readseek_bin] + args,
            capture_output=True,
            env=env,
            input=stdin,
            text=True,
            encoding="utf-8",
        )

    def readseek_json(name, args, stdin=None):
        result = run(args, stdin=stdin)
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
        if isinstance(contents, bytes):
            with open(path, "wb") as file:
                file.write(contents)
        else:
            with open(path, "w", encoding="utf-8", newline="") as file:
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

    def git(directory, args):
        result = subprocess.run(
            ["git", "-C", directory] + args,
            capture_output=True,
            env=os.environ.copy(),
            text=True,
            encoding="utf-8",
        )
        if result.returncode != 0:
            raise RuntimeError(f"git {' '.join(args)} failed: {result.stderr[:200]}")

    def result_files(data):
        return sorted(result.get("file") for result in data.get("results", []))

    def map_file_count(directory):
        maps_dir = os.path.join(directory, ".readseek", "maps")
        return sum(len(files) for _, _, files in os.walk(maps_dir))

    def assert_symbol(name, symbols, kind, symbol_name):
        if any(symbol.get("kind") == kind and symbol.get("name") == symbol_name for symbol in symbols):
            return True
        failed(name, f"missing {kind} symbol {symbol_name}: {symbols!r}")
        return False

    def find_symbol(symbols, kind, symbol_name):
        return next(
            (
                symbol
                for symbol in symbols
                if symbol.get("kind") == kind and symbol.get("name") == symbol_name
            ),
            {},
        )

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
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), language),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

    def expect_mapped_language(name, file_name, contents, language):
        path = write_file(tmpdir, file_name, contents)
        data = readseek_json(name, ["map", path])
        if data and assert_equal(name, data.get("language"), language):
            passed(name)

    with tempfile.TemporaryDirectory() as tmpdir:
        sandbox_home = os.path.join(tmpdir, "home")
        os.mkdir(sandbox_home)
        name = "init: .readseek directory"
        result_init = run(["init", tmpdir])
        expected_init = f"Initialized empty readseek repository in {os.path.join(tmpdir, '.readseek')}/\n"
        if result_init.returncode != 0:
            failed(name, f"status {result_init.returncode} stderr={result_init.stderr[:200]}")
        elif assert_equal(name, result_init.stdout, expected_init):
            passed(name)

        write_file(tmpdir, "blob.bin", b"\0\1")

        name = "init: reinitialize .readseek directory"
        result_init = run(["init", tmpdir])
        expected_init = f"Reinitialized existing readseek repository in {os.path.join(tmpdir, '.readseek')}/\n"
        if result_init.returncode != 0:
            failed(name, f"status {result_init.returncode} stderr={result_init.stderr[:200]}")
        elif assert_equal(name, result_init.stdout, expected_init):
            passed(name)
        name = "file: rust file"
        path = write_file(tmpdir, "sample.rs", "fn main() {}\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "rust"),
                assert_equal(name, data.get("supported"), True),
                assert_equal(name, data.get("binary"), False),
            ]
        ):
            passed(name)

        name = "detect: does not update project cache"
        if assert_equal(name, map_file_count(tmpdir), 0):
            passed(name)

        name = "read: requested range"
        path = write_file(tmpdir, "sample.py", "one\ntwo\nthree\n")
        data = readseek_json(name, ["read", path, "--offset", "2", "--end", "3"])
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
            greeter = find_symbol(symbols, "class", "Greeter")
            if all(
                [
                    assert_equal(name, data.get("language"), "javascript"),
                    assert_symbol(name, symbols, "class", "Greeter"),
                    assert_symbol(name, symbols, "method", "greet"),
                    assert_symbol(name, symbols, "function", "make"),
                    assert_equal(name, greeter.get("start_byte"), 0),
                    assert_equal(name, greeter.get("name_byte"), 6),
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
                        os.path.isdir(os.path.join(tmpdir, ".readseek", "maps")),
                        "cache maps directory missing",
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

        name = "search: repeated metavariable"
        path = write_file(
            search_dir,
            "repeated.cpp",
            "int same(int value) { return value + value; }\n"
            "int different(int value) { return value + other; }\n",
        )
        data = readseek_json(
            name,
            [
                "search",
                path,
                "int $NAME(int $X) { return $X + $X; }",
            ],
        )
        if data:
            results = data.get("results", [])
            matches = results[0].get("matches", []) if results else []
            if all(
                [
                    assert_equal(name, len(results), 1),
                    assert_equal(name, len(matches), 1),
                    assert_equal(name, matches[0].get("start_line"), 1),
                ]
            ):
                passed(name)

        name = "search: unicode pattern"
        path = write_file(
            search_dir,
            "unicode.rs",
            "fn main() { let greeting = \"café\"; }\n",
        )
        data = readseek_json(name, ["search", path, "let greeting = \"café\";"])
        if data:
            results = data.get("results", [])
            matches = results[0].get("matches", []) if results else []
            if all(
                [
                    assert_equal(name, len(results), 1),
                    assert_equal(name, len(matches), 1),
                    assert_equal(name, matches[0].get("start_line"), 1),
                ]
            ):
                passed(name)

        git_dir = os.path.join(tmpdir, "git-search")
        os.makedirs(git_dir)
        subprocess.run(["git", "init"], check=True, capture_output=True, cwd=git_dir, text=True)
        tracked = write_file(git_dir, "tracked.rs", "fn marker() { let git = \"tracked\"; }\n")
        untracked = write_file(git_dir, "untracked.rs", "fn marker() { let git = \"untracked\"; }\n")
        ignored = write_file(git_dir, "ignored.rs", "fn marker() { let git = \"ignored\"; }\n")
        write_file(git_dir, ".gitignore", "ignored.rs\n")
        git(git_dir, ["add", "tracked.rs", ".gitignore"])

        git_cases = [
            ("search git: default", [], [tracked, untracked]),
            ("search git: cached", ["--cached"], [tracked]),
            ("search git: cached short", ["-c"], [tracked]),
            ("search git: others", ["--others"], [untracked]),
            ("search git: others short", ["-o"], [untracked]),
            (
                "search git: others ignored short",
                ["-o", "-i"],
                [ignored, untracked],
            ),
            (
                "search git: others ignored",
                ["--others", "--ignored"],
                [ignored, untracked],
            ),
        ]
        for case_name, options, expected in git_cases:
            data = readseek_json(
                case_name,
                ["search"] + options + [git_dir, "fn marker() { let git = $VALUE; }"],
            )
            if data and assert_equal(case_name, result_files(data), sorted(expected)):
                passed(case_name)

        name = "search git: explicit ignored file"
        data = readseek_json(name, ["search", ignored, "fn marker() { let git = $VALUE; }"])
        if data and assert_equal(name, result_files(data), [ignored]):
            passed(name)

        name = "search non-git: git flags fall back"
        non_git_dir = os.path.join(tmpdir, "non-git-search")
        os.makedirs(non_git_dir)
        non_git_file = write_file(
            non_git_dir,
            "plain.rs",
            "fn marker() { let git = \"plain\"; }\n",
        )
        data = readseek_json(
            name,
            ["search", "-c", "-o", "-i", non_git_dir, "fn marker() { let git = $VALUE; }"],
        )
        if data and assert_equal(name, result_files(data), [non_git_file]):
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

        expect_mapped_symbol(
            "map: vimscript function",
            "plugin.vim",
            "function! s:greet(name) abort\n  echo a:name\nendfunction\n",
            "vimscript",
            "function",
            "s:greet",
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
            ("dockerfile", "Dockerfile", "FROM alpine:3.20\nRUN echo hello\n"),
            ("nix", "flake.nix", "{ description = \"sample\"; }\n"),
            ("lua", "sample.lua", "function greet()\n  return 'hello'\nend\n"),
            ("perl", "sample.pl", "sub greet { return \"hello\"; }\n"),
            ("zig", "sample.zig", "pub fn main() void {}\n"),
            ("vimscript", "plugin.vim", "function! Greet() abort\nendfunction\n"),
        ]
        for language, file_name, contents in supported_files:
            expect_supported_file(
                f"file: {language}",
                file_name,
                contents,
                language,
            )

        mapped_languages = [
            ("dockerfile", "Containerfile", "FROM alpine:3.20\nRUN echo hello\n"),
            ("nix", "parser.nix", "{ packages.default = null; }\n"),
            ("lua", "parser.lua", "function greet()\n  return 'hello'\nend\n"),
            ("perl", "parser.pl", "sub greet { return \"hello\"; }\n"),
            ("zig", "parser.zig", "pub fn main() void {}\n"),
            ("vimscript", "parser.vim", "function! Greet() abort\nendfunction\n"),
        ]
        for language, file_name, contents in mapped_languages:
            expect_mapped_language(
                f"map: {language} parser",
                file_name,
                contents,
                language,
            )

        name = "symbol: cached mapped qualified name"
        path = os.path.join(tmpdir, "sample.ts")
        data = readseek_json(name, ["symbol", path, "--name", "Greeter.greet"])
        if data and all(
            [
                assert_equal(name, data["symbol"].get("kind"), "method"),
                assert_equal(name, data["symbol"].get("name"), "greet"),
                assert_equal(name, data["symbol"].get("qualified_name"), "Greeter.greet"),
            ]
        ):
            passed(name)

        name = "symbol: target suffix"
        path = write_file(
            tmpdir,
            "symbol.ts",
            "class Greeter {\n  greet() {\n    return 'hello';\n  }\n}\n",
        )
        data = readseek_json(name, ["symbol", path, "--name", "Greeter.greet"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "typescript"),
                assert_equal(name, data["symbol"].get("kind"), "method"),
                assert_equal(name, data["symbol"].get("name"), "greet"),
                assert_equal(name, data["symbol"].get("qualified_name"), "Greeter.greet"),
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

        name = "file: stdin path"
        data = readseek_json(
            name,
            ["detect", "--stdin", "buffer.ts"],
            stdin="class BufferGreeter {}\n",
        )
        if data and all(
            [
                assert_equal(name, data.get("file"), "buffer.ts"),
                assert_equal(name, data.get("language"), "typescript"),
                assert_equal(name, data.get("binary"), False),
            ]
        ):
            passed(name)

        name = "read: stdin path"
        data = readseek_json(
            name,
            ["read", "--stdin", "buffer.ts", "--offset", "2", "--end", "2"],
            stdin="one\ntwo\nthree\n",
        )
        if data and all(
            [
                assert_equal(name, data.get("file"), "buffer.ts"),
                assert_equal(name, data.get("start_line"), 2),
                assert_equal(name, data.get("end_line"), 2),
                assert_equal(name, data["hashlines"][0].get("text"), "two"),
            ]
        ):
            passed(name)

        buffer_source = "class BufferGreeter {\n  greet() { return 'hello'; }\n}\n"

        name = "map: stdin path"
        data = readseek_json(
            name,
            ["map", "--stdin", "buffer.ts"],
            stdin=buffer_source,
        )
        if data:
            symbols = data.get("symbols", [])
            if all(
                [
                    assert_equal(name, data.get("file"), "buffer.ts"),
                    assert_equal(name, data.get("language"), "typescript"),
                    assert_symbol(name, symbols, "class", "BufferGreeter"),
                    assert_symbol(name, symbols, "method", "greet"),
                ]
            ):
                passed(name)

        name = "check: clean source has no diagnostics"
        data = readseek_json(
            name,
            ["check", "--stdin", "clean.rs"],
            stdin="fn main() {}\n",
        )
        if data and all(
            [
                assert_equal(name, data.get("error_count"), 0),
                assert_equal(name, data.get("missing_count"), 0),
                assert_equal(name, data.get("diagnostics"), []),
            ]
        ):
            passed(name)

        name = "check: parse error is reported"
        data = readseek_json(
            name,
            ["check", "--stdin", "broken.rs"],
            stdin="fn main() {\n    let x =\n}\n",
        )
        if data:
            diagnostics = data.get("diagnostics", [])
            if all(
                [
                    assert_equal(name, data.get("error_count"), 1),
                    assert_equal(name, data.get("missing_count"), 0),
                    assert_equal(name, len(diagnostics), 1),
                    assert_equal(name, diagnostics[0].get("kind"), "error"),
                    assert_equal(name, diagnostics[0].get("start_line"), 2),
                ]
            ):
                passed(name)

        name = "check: missing token is reported"
        data = readseek_json(
            name,
            ["check", "--stdin", "missing.c"],
            stdin="int main() {\n  int x = 1\n  return x;\n}\n",
        )
        if data:
            diagnostics = data.get("diagnostics", [])
            if all(
                [
                    assert_equal(name, data.get("error_count"), 0),
                    assert_equal(name, data.get("missing_count"), 1),
                    assert_equal(name, len(diagnostics), 1),
                    assert_equal(name, diagnostics[0].get("kind"), "missing"),
                    assert_equal(name, diagnostics[0].get("start_line"), 2),
                ]
            ):
                passed(name)

        name = "check: unsupported file has no diagnostics"
        data = readseek_json(
            name,
            ["check", "--stdin", "notes.txt"],
            stdin="just some text\n",
        )
        if data and all(
            [
                assert_equal(name, data.get("language"), "unknown"),
                assert_equal(name, data.get("diagnostics"), []),
            ]
        ):
            passed(name)

        name = "symbol: stdin target line"
        data = readseek_json(
            name,
            ["symbol", "--stdin", "buffer.ts", "--line", "2"],
            stdin=buffer_source,
        )
        if data and all(
            [
                assert_equal(name, data.get("file"), "buffer.ts"),
                assert_equal(name, data["symbol"].get("qualified_name"), "BufferGreeter.greet"),
                assert_equal(name, data["hashlines"][0].get("line"), 2),
            ]
        ):
            passed(name)

        name = "identify: cursor identifier and symbol"
        data = readseek_json(
            name,
            ["identify", "--stdin", "buffer.ts", "--line", "2", "--column", "3"],
            stdin=buffer_source,
        )
        if data and all(
            [
                assert_equal(name, data.get("file"), "buffer.ts"),
                assert_equal(name, data.get("line"), 2),
                assert_equal(name, data.get("line_hash"), data["hashlines"][0].get("hash")),
                assert_equal(name, data["identifier"].get("text"), "greet"),
                assert_equal(name, data["identifier"].get("start_column"), 3),
                assert_equal(name, data["identifier"].get("end_column"), 8),
                assert_equal(name, data["identifier"].get("start_byte"), 24),
                assert_equal(name, data["identifier"].get("end_byte"), 29),
                assert_equal(name, data["symbol"].get("qualified_name"), "BufferGreeter.greet"),
            ]
        ):
            passed(name)

        name = "identify: unicode identifier span"
        data = readseek_json(
            name,
            ["identify", "--stdin", "unicode.rs", "--line", "1", "--column", "8"],
            stdin="fn café() {}\n",
        )
        if data and all(
            [
                assert_equal(name, data["identifier"].get("text"), "café"),
                assert_equal(name, data["identifier"].get("start_column"), 4),
                assert_equal(name, data["identifier"].get("end_column"), 9),
            ]
        ):
            passed(name)

        name = "definition: project symbol lookup"
        definitions_dir = os.path.join(tmpdir, "definitions")
        os.mkdir(definitions_dir)
        definition_path = write_file(definitions_dir, "defs.rs", "fn target() {}\nfn other() {}\n")
        data = readseek_json(name, ["def", definitions_dir, "target"])
        if data:
            definitions = data.get("definitions", [])
            if all(
                [
                    assert_equal(name, len(definitions), 1),
                    assert_equal(name, definitions[0].get("file"), definition_path),
                    assert_equal(name, definitions[0]["symbol"].get("kind"), "function"),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "target"),
                    assert_equal(name, definitions[0]["symbol"].get("start_line"), 1),
                ]
            ):
                passed(name)

        name = "definition: C typedef lookup"
        typedef_path = write_file(
            definitions_dir,
            "defs.h",
            "typedef unsigned int __u32;\ntypedef __u32 u32;\n__extension__ typedef unsigned long long __u64;\n",
        )
        data = readseek_json(name, ["def", definitions_dir, "u32"])
        if data:
            definitions = data.get("definitions", [])
            if assert_equal(name, len(definitions), 1) and all(
                [
                    assert_equal(name, definitions[0].get("file"), typedef_path),
                    assert_equal(name, definitions[0]["symbol"].get("kind"), "type"),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "u32"),
                    assert_equal(name, definitions[0]["symbol"].get("start_line"), 2),
                ]
            ):
                passed(name)

        name = "definition: C extension typedef lookup"
        data = readseek_json(name, ["def", definitions_dir, "__u64"])
        if data:
            definitions = data.get("definitions", [])
            if assert_equal(name, len(definitions), 1) and all(
                [
                    assert_equal(name, definitions[0].get("file"), typedef_path),
                    assert_equal(name, definitions[0]["symbol"].get("kind"), "type"),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "__u64"),
                    assert_equal(name, definitions[0]["symbol"].get("start_line"), 3),
                ]
            ):
                passed(name)

        name = "definition: C function declaration lookup"
        declarations_path = write_file(
            definitions_dir,
            "decls.h",
            "int foo(void);\nextern int global_count;\nstatic const struct ops driver_ops = {};\nvoid caller(void) { int local_value; }\n",
        )
        data = readseek_json(name, ["def", definitions_dir, "foo"])
        if data:
            definitions = data.get("definitions", [])
            if assert_equal(name, len(definitions), 1) and all(
                [
                    assert_equal(name, definitions[0].get("file"), declarations_path),
                    assert_equal(name, definitions[0]["symbol"].get("kind"), "function"),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "foo"),
                    assert_equal(name, definitions[0]["symbol"].get("start_line"), 1),
                ]
            ):
                passed(name)

        name = "definition: C extern declaration lookup"
        data = readseek_json(name, ["def", definitions_dir, "global_count"])
        if data:
            definitions = data.get("definitions", [])
            if assert_equal(name, len(definitions), 1) and all(
                [
                    assert_equal(name, definitions[0].get("file"), declarations_path),
                    assert_equal(name, definitions[0]["symbol"].get("kind"), "variable"),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "global_count"),
                    assert_equal(name, definitions[0]["symbol"].get("start_line"), 2),
                ]
            ):
                passed(name)

        name = "definition: C static object declaration lookup"
        data = readseek_json(name, ["def", definitions_dir, "driver_ops"])
        if data:
            definitions = data.get("definitions", [])
            if assert_equal(name, len(definitions), 1) and all(
                [
                    assert_equal(name, definitions[0].get("file"), declarations_path),
                    assert_equal(name, definitions[0]["symbol"].get("kind"), "variable"),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "driver_ops"),
                    assert_equal(name, definitions[0]["symbol"].get("start_line"), 3),
                ]
            ):
                passed(name)

        name = "definition: C local declaration ignored"
        data = readseek_json(name, ["def", definitions_dir, "local_value"])
        if data:
            definitions = data.get("definitions", [])
            if assert_equal(name, definitions, []):
                passed(name)

        name = "definition: substring candidate ignored"
        write_file(definitions_dir, "substring-only.c", "int target_suffix(void);\n")
        data = readseek_json(name, ["def", definitions_dir, "target"])
        if data:
            definitions = data.get("definitions", [])
            if all(
                [
                    assert_equal(name, len(definitions), 1),
                    assert_equal(name, definitions[0].get("file"), definition_path),
                ]
            ):
                passed(name)

        name = "definition: C macro does not hide function"
        macro_and_function_path = write_file(
            definitions_dir,
            "macro-and-function.c",
            "#define dual_name 1\n#undef dual_name\nint dual_name(void) { return 0; }\n",
        )
        data = readseek_json(name, ["def", definitions_dir, "dual_name"])
        if data:
            definitions = data.get("definitions", [])
            kinds_by_line = sorted(
                (definition["symbol"].get("kind"), definition["symbol"].get("start_line"))
                for definition in definitions
            )
            if all(
                [
                    assert_equal(name, len(definitions), 2),
                    assert_true(
                        name,
                        all(definition.get("file") == macro_and_function_path for definition in definitions),
                        f"expected definitions from {macro_and_function_path!r}: {definitions!r}",
                    ),
                    assert_equal(name, kinds_by_line, [("function", 3), ("macro", 1)]),
                ]
            ):
                passed(name)

        name = "definition: compact locations"
        data = readseek_json(name, ["def", "--format", "plain", definitions_dir, "target"])
        if data:
            locations = data.get("locations", [])
            if all(
                [
                    assert_equal(name, data.get("definitions"), None),
                    assert_equal(name, len(locations), 1),
                    assert_equal(name, locations[0].get("file"), definition_path),
                    assert_equal(name, locations[0].get("line"), 1),
                    assert_equal(name, locations[0].get("column"), 1),
                    assert_equal(name, locations[0].get("text"), "fn target() {}"),
                    assert_equal(name, locations[0].get("kind"), "function"),
                    assert_equal(name, locations[0].get("name"), "target"),
                    assert_equal(name, locations[0].get("qualified_name"), "target"),
                ]
            ):
                passed(name)

        name = "definition: identify context prefers identifier"
        qualified_definition_path = write_file(
            definitions_dir,
            "qualified.ts",
            "class First {\n  greet() { return 1; }\n}\nclass Second {\n  greet() { return 2; }\n}\n",
        )
        data = readseek_json(
            name,
            ["def", "--stdin", definitions_dir],
            stdin=json.dumps(
                {
                    "identifier": {"text": "greet"},
                    "symbol": {"qualified_name": "Second.greet"},
                }
            ),
        )
        if data:
            definitions = data.get("definitions", [])
            if all(
                [
                    assert_equal(name, len(definitions), 2),
                    assert_equal(name, definitions[0]["symbol"].get("qualified_name"), "First.greet"),
                    assert_equal(name, definitions[1]["symbol"].get("qualified_name"), "Second.greet"),
                ]
            ):
                passed(name)

        name = "definition: identify context falls back to identifier"
        data = readseek_json(
            name,
            ["def", "--stdin", definitions_dir],
            stdin=json.dumps({"identifier": {"text": "target"}, "symbol": None}),
        )
        if data:
            definitions = data.get("definitions", [])
            if all(
                [
                    assert_equal(name, len(definitions), 1),
                    assert_equal(name, definitions[0].get("file"), definition_path),
                    assert_equal(name, definitions[0]["symbol"].get("name"), "target"),
                ]
            ):
                passed(name)

        name = "references: project identifier lookup"
        references_dir = os.path.join(tmpdir, "refs")
        os.mkdir(references_dir)
        reference_path = write_file(
            references_dir,
            "refs.rs",
            "fn target() {}\nfn caller() {\n  target();\n  let target_value = 1;\n}\n",
        )
        write_file(references_dir, "substring.rs", "fn target_suffix() {}\nlet target_value = 1;\n")
        data = readseek_json(name, ["refs", references_dir, "target"])
        if data:
            references = data.get("references", [])
            if all(
                [
                    assert_equal(name, len(references), 2),
                    assert_equal(name, references[0].get("file"), reference_path),
                    assert_equal(name, references[0].get("line"), 1),
                    assert_equal(name, references[0].get("column"), 4),
                    assert_equal(name, references[0].get("text"), "fn target() {}"),
                    assert_equal(name, references[1].get("file"), reference_path),
                    assert_equal(name, references[1].get("line"), 3),
                    assert_equal(name, references[1].get("column"), 3),
                    assert_equal(
                        name,
                        references[1].get("symbol", {}).get("qualified_name"),
                        "caller",
                    ),
                ]
            ):
                passed(name)

        name = "references: scope resolves shadowed binding"
        scope_dir = os.path.join(tmpdir, "scope")
        os.mkdir(scope_dir)
        scope_path = write_file(
            scope_dir,
            "scope.rs",
            "fn outer() {\n    let x = 1;\n    {\n        let x = 2;\n        use_it(x);\n    }\n    use_it(x);\n}\n",
        )

        def occurrences(data):
            return sorted(
                (reference.get("line"), reference.get("column"), reference.get("occurrence"))
                for reference in data.get("references", [])
            )

        data = readseek_json(
            name, ["refs", scope_path, "x", "--scope", "--line", "2", "--column", "9"]
        )
        if data and assert_equal(
            name, occurrences(data), [(2, 9, "definition"), (7, 12, "reference")]
        ):
            passed(name)

        name = "references: scope resolves inner binding"
        data = readseek_json(
            name, ["refs", scope_path, "x", "--scope", "--line", "4", "--column", "13"]
        )
        if data and assert_equal(
            name, occurrences(data), [(4, 13, "definition"), (5, 16, "reference")]
        ):
            passed(name)

        name = "references: scope resolves function parameter"
        param_path = write_file(
            scope_dir,
            "param.rs",
            "fn add(value: i32) -> i32 {\n    value + 1\n}\nfn other(value: i32) -> i32 { value }\n",
        )
        data = readseek_json(
            name, ["refs", param_path, "value", "--scope", "--line", "1", "--column", "8"]
        )
        if data and assert_equal(
            name, occurrences(data), [(1, 8, "definition"), (2, 5, "reference")]
        ):
            passed(name)

        name = "references: scope rejects name mismatch"
        expect_failure(
            name, ["refs", param_path, "missing", "--scope", "--line", "1", "--column", "8"]
        )

        name = "references: line without scope fails"
        expect_failure(name, ["refs", param_path, "value", "--line", "1"])

        name = "rename: plan for inner binding"
        data = readseek_json(
            name, ["rename", scope_path, "--line", "4", "--column", "13", "--to", "renamed"]
        )
        if data:
            edits = sorted(
                (edit.get("line"), edit.get("start_column"), edit.get("occurrence"))
                for edit in data.get("edits", [])
            )
            if all(
                [
                    assert_equal(name, data.get("old_name"), "x"),
                    assert_equal(name, data.get("new_name"), "renamed"),
                    assert_equal(name, data.get("applied"), False),
                    assert_equal(name, data.get("conflicts"), []),
                    assert_equal(name, edits, [(4, 13, "definition"), (5, 16, "reference")]),
                    assert_true(
                        name,
                        all(len(edit.get("line_hash", "")) == 3 for edit in data["edits"]),
                        "edit missing line hash",
                    ),
                ]
            ):
                passed(name)

        name = "rename: reports binding conflict"
        conflict_path = write_file(
            scope_dir,
            "conflict.rs",
            "fn f() {\n    let a = 1;\n    let b = 2;\n    use_it(a + b);\n}\n",
        )
        data = readseek_json(
            name, ["rename", conflict_path, "--line", "2", "--column", "9", "--to", "b"]
        )
        if data:
            conflicts = data.get("conflicts", [])
            if all(
                [
                    assert_equal(name, len(conflicts), 1),
                    assert_equal(name, conflicts[0].get("line"), 4),
                ]
            ):
                passed(name)

        name = "rename: rejects identical name"
        expect_failure(
            name, ["rename", conflict_path, "--line", "2", "--column", "9", "--to", "a"]
        )

        name = "rename: rejects invalid identifier"
        expect_failure(
            name, ["rename", conflict_path, "--line", "2", "--column", "9", "--to", "2bad"]
        )

        name = "rename: applies edits to file"
        apply_path = write_file(
            scope_dir,
            "apply.rs",
            "fn outer() {\n    let x = 1;\n    {\n        let x = 2;\n        use_it(x);\n    }\n    use_it(x);\n}\n",
        )
        data = readseek_json(
            name, ["rename", apply_path, "--line", "4", "--column", "13", "--to", "renamed", "--apply"]
        )
        if data and assert_equal(name, data.get("applied"), True):
            with open(apply_path, encoding="utf-8") as file:
                contents = file.read()
            expected = (
                "fn outer() {\n    let x = 1;\n    {\n        let renamed = 2;\n"
                "        use_it(renamed);\n    }\n    use_it(x);\n}\n"
            )
            if assert_equal(name, contents, expected):
                passed(name)

        name = "references: scope resolves C block shadowing"
        c_scope_path = write_file(
            scope_dir,
            "scope.c",
            "int add(int value, int other) {\n    int result = value + other;\n    {\n        int result = value * 2;\n        use_it(result);\n    }\n    return result;\n}\n",
        )
        data = readseek_json(
            name,
            ["refs", c_scope_path, "result", "--scope", "--line", "2", "--column", "9", "--language", "c"],
        )
        if data and assert_equal(
            name, occurrences(data), [(2, 9, "definition"), (7, 12, "reference")]
        ):
            passed(name)

        name = "rename: applies C parameter rename"
        c_apply_path = write_file(
            scope_dir,
            "apply.c",
            "int add(int value, int other) {\n    return value + other;\n}\n",
        )
        data = readseek_json(
            name,
            ["rename", c_apply_path, "--line", "1", "--column", "13", "--to", "v", "--apply", "--language", "c"],
        )
        if data and assert_equal(name, data.get("applied"), True):
            with open(c_apply_path, encoding="utf-8") as file:
                contents = file.read()
            expected = "int add(int v, int other) {\n    return v + other;\n}\n"
            if assert_equal(name, contents, expected):
                passed(name)

        name = "rename: apply refuses on conflict"
        apply_conflict_path = write_file(
            scope_dir,
            "apply-conflict.rs",
            "fn f() {\n    let a = 1;\n    let b = 2;\n    use_it(a + b);\n}\n",
        )
        before = open(apply_conflict_path, encoding="utf-8").read()
        result = run(
            ["rename", apply_conflict_path, "--line", "2", "--column", "9", "--to", "b", "--apply"]
        )
        after = open(apply_conflict_path, encoding="utf-8").read()
        if all(
            [
                assert_true(name, result.returncode != 0, "expected failure"),
                assert_equal(name, after, before),
            ]
        ):
            passed(name)

        name = "rename: unsupported language is a no-op"
        unsupported_path = write_file(
            scope_dir,
            "unsupported.vim",
            "vim9script\n\nexport def Foo()\n  var value = 1\n  echo value\nenddef\n",
        )
        before = open(unsupported_path, encoding="utf-8").read()
        data = readseek_json(
            name,
            ["rename", unsupported_path, "--line", "4", "--column", "7", "--to", "renamed", "--apply"],
        )
        after = open(unsupported_path, encoding="utf-8").read()
        if data and all(
            [
                assert_equal(name, data.get("unsupported"), True),
                assert_equal(name, data.get("applied"), False),
                assert_equal(name, data.get("edits"), []),
                assert_equal(name, after, before),
            ]
        ):
            passed(name)

        name = "rename: single-file omits others"
        data = readseek_json(
            name, ["rename", scope_path, "--line", "4", "--column", "13", "--to", "renamed2"]
        )
        if data and assert_true(
            name, "others" not in data, "single-file output must omit others"
        ):
            passed(name)

        xfile_dir = os.path.join(tmpdir, "xfile")
        os.mkdir(xfile_dir)
        write_file(xfile_dir, "lib.rs", "pub fn target() -> u32 { 1 }\n")
        write_file(
            xfile_dir,
            "use1.rs",
            "use crate::target;\nfn run() -> u32 { target() + target() }\n",
        )
        write_file(xfile_dir, "shadow.rs", "fn run() -> u32 { let target = 5; target }\n")
        write_file(xfile_dir, "notes.md", "target appears here as prose target.\n")
        xfile_lib = os.path.join(xfile_dir, "lib.rs")

        name = "rename: cross-file plan excludes local shadow"
        data = readseek_json(
            name,
            ["rename", xfile_lib, "--line", "1", "--column", "8", "--to", "renamed", "--workspace", xfile_dir],
        )
        if data:
            others = {os.path.basename(o["file"]): len(o["edits"]) for o in data.get("others", [])}
            if all(
                [
                    assert_equal(name, data.get("old_name"), "target"),
                    assert_equal(name, len(data.get("edits", [])), 1),
                    assert_equal(name, others, {"notes.md": 2, "use1.rs": 3}),
                    assert_true(name, "shadow.rs" not in others, "local shadow must be excluded"),
                ]
            ):
                passed(name)

        name = "rename: cross-file applies every file"
        data = readseek_json(
            name,
            ["rename", xfile_lib, "--line", "1", "--column", "8", "--to", "renamed", "--workspace", xfile_dir, "--apply"],
        )
        if data and assert_equal(name, data.get("applied"), True):
            def xfile_read(filename):
                with open(os.path.join(xfile_dir, filename), encoding="utf-8") as file:
                    return file.read()
            if all(
                [
                    assert_equal(name, xfile_read("lib.rs"), "pub fn renamed() -> u32 { 1 }\n"),
                    assert_equal(
                        name,
                        xfile_read("use1.rs"),
                        "use crate::renamed;\nfn run() -> u32 { renamed() + renamed() }\n",
                    ),
                    assert_equal(
                        name,
                        xfile_read("shadow.rs"),
                        "fn run() -> u32 { let target = 5; target }\n",
                    ),
                    assert_equal(
                        name, xfile_read("notes.md"), "renamed appears here as prose renamed.\n"
                    ),
                ]
            ):
                passed(name)

        name = "rename: cross-file refuses on conflict and writes nothing"
        conflict_dir = os.path.join(tmpdir, "xfile-conflict")
        os.mkdir(conflict_dir)
        write_file(conflict_dir, "lib.rs", "pub fn target() -> u32 { 1 }\n")
        write_file(
            conflict_dir,
            "capture.rs",
            "fn run() -> u32 { let renamed = 9; target(); renamed }\n",
        )
        conflict_lib = os.path.join(conflict_dir, "lib.rs")
        conflict_capture = os.path.join(conflict_dir, "capture.rs")
        before_lib = open(conflict_lib, encoding="utf-8").read()
        before_capture = open(conflict_capture, encoding="utf-8").read()
        result = run(
            ["rename", conflict_lib, "--line", "1", "--column", "8", "--to", "renamed", "--workspace", conflict_dir, "--apply"]
        )
        if all(
            [
                assert_true(name, result.returncode != 0, "expected failure"),
                assert_equal(name, open(conflict_lib, encoding="utf-8").read(), before_lib),
                assert_equal(name, open(conflict_capture, encoding="utf-8").read(), before_capture),
            ]
        ):
            passed(name)

        name = "references: C ignores comments and literals"
        c_references_dir = os.path.join(tmpdir, "c-references")
        os.mkdir(c_references_dir)
        c_reference_path = write_file(
            c_references_dir,
            "refs.c",
            "void target(void);\nvoid caller(void) {\n  target();\n  /* target */\n  const char *name = \"target\";\n  char initial = 't';\n}\n",
        )
        data = readseek_json(name, ["refs", c_references_dir, "target", "--language", "c"])
        if data:
            references = [
                reference
                for reference in data.get("references", [])
                if reference.get("file") == c_reference_path
            ]
            if all(
                [
                    assert_equal(name, len(references), 2),
                    assert_equal(name, references[0].get("line"), 1),
                    assert_equal(name, references[0].get("column"), 6),
                    assert_equal(name, references[1].get("line"), 3),
                    assert_equal(name, references[1].get("column"), 3),
                ]
            ):
                passed(name)

        name = "references: compact locations"
        data = readseek_json(name, ["refs", "--format", "plain", references_dir, "target"])
        if data:
            locations = data.get("locations", [])
            if all(
                [
                    assert_equal(name, data.get("references"), None),
                    assert_equal(name, len(locations), 2),
                    assert_equal(name, locations[0].get("file"), reference_path),
                    assert_equal(name, locations[0].get("line"), 1),
                    assert_equal(name, locations[0].get("column"), 4),
                    assert_equal(name, locations[0].get("text"), "fn target() {}"),
                    assert_equal(name, locations[0].get("kind"), "function"),
                    assert_equal(name, locations[0].get("name"), "target"),
                    assert_equal(name, locations[1].get("file"), reference_path),
                    assert_equal(name, locations[1].get("line"), 3),
                    assert_equal(name, locations[1].get("column"), 3),
                    assert_equal(name, locations[1].get("qualified_name"), "caller"),
                ]
            ):
                passed(name)

        if os.name != "nt":
            name = "symbol: colon filename with qualified name argument"
            path = write_file(
                tmpdir,
                "colon:symbol.ts",
                "class Greeter {\n  greet() {\n    return 'hello';\n  }\n}\n",
            )
            data = readseek_json(name, ["symbol", path, "--name", "Greeter.greet"])
            if data and all(
                [
                    assert_equal(name, data.get("file"), path),
                    assert_equal(name, data["symbol"].get("kind"), "method"),
                    assert_equal(name, data["symbol"].get("qualified_name"), "Greeter.greet"),
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
        data = readseek_json(name, ["detect", path, "--language", "typescript"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "typescript"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: language alias"
        path = write_file(tmpdir, "header", "int add(int a, int b);\n")
        data = readseek_json(name, ["detect", path, "--language", "c-plus-plus"])
        if data and all(
            [
                assert_equal(name, data.get("language"), "cpp"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: special filename"
        path = write_file(tmpdir, "go.mod", "module example.com/readseek\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "go"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: tex language"
        path = write_file(tmpdir, "paper.tex", "\\section{Intro}\nText.\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "latex"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: typst language"
        path = write_file(tmpdir, "paper.typ", "= Intro\nText.\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "typst"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: assembly language"
        path = write_file(tmpdir, "boot.S", "_start:\n    mov %rsp, %rbp\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "assembly"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: riscv language"
        path = write_file(tmpdir, "boot.riscv", "_start:\n    addi sp, sp, -16\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "riscv"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: gdscript language"
        path = write_file(tmpdir, "player.gd", "func _ready():\n    pass\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "gdscript"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: sql language"
        path = write_file(tmpdir, "query.sql", "select * from users;\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "sql"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: toml language"
        path = write_file(tmpdir, "config.toml", "name = \"demo\"\n")
        data = readseek_json(name, ["detect", path])
        if data and all(
            [
                assert_equal(name, data.get("language"), "toml"),
                assert_equal(name, data.get("supported"), True),
            ]
        ):
            passed(name)

        name = "file: binary rejected"
        path = write_file(tmpdir, "blob.bin", bytes([0, 1, 2, 3]))
        expect_failure(name, ["detect", path])

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
            title = find_symbol(symbols, "heading", "Title")
            if all(
                [
                    assert_equal(name, data.get("language"), "markdown"),
                    assert_symbol(name, symbols, "heading", "Title"),
                    assert_symbol(name, symbols, "heading", "Details"),
                    assert_equal(name, title.get("start_line"), 1),
                    assert_equal(name, title.get("end_line"), 1),
                    assert_equal(name, title.get("end_hash"), title.get("start_hash")),
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
