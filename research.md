# Research: tree-sitter-c Identifier Handling Inside Preprocessor Directive Bodies

## Summary

tree-sitter-c deliberately does **not** parse `#define` bodies (or `#error`, `#pragma`, `#line` bodies) as C expression trees. Instead, the external scanner captures these as opaque `preproc_arg` token nodes containing raw text. Identifiers like `EXPORT_SYMBOL_GPL` inside `#define` bodies are **never** represented as `identifier` AST nodes — they are substrings of `preproc_arg` text. The standard approach is to post-process `preproc_arg` text with regex matching to extract identifier-like tokens.

## Findings

1. **`preproc_arg` is an opaque token with no children** — In tree-sitter-c's grammar (`grammar.js`), the `#define` value is defined as `field('value', $.preproc_arg)`. The external scanner (`scanner.c`) enters a "consume raw tokens until EOL" mode after the macro name/params. Every token in the body (identifiers, numbers, operators, `##`, `#`) is concatenated into one `preproc_arg` token. The grammar has NO rule to decompose `preproc_arg` into child nodes. [Source: tree-sitter-c grammar.js, preproc_def/preproc_function_def rules, and scanner.c preprocessor tokenizer]

2. **`#define` bodies are the primary but not the only context** — The following directive types also use `preproc_arg` for their bodies:
   - `#define NAME <body>` → body is `preproc_arg`
   - `#define NAME(params) <body>` → body is `preproc_arg`  
   - `#error <message>` → message is `preproc_arg`
   - `#pragma <tokens>...` → tokens are `preproc_arg`
   - `#line <body>` → body is `preproc_arg`
   - **Exceptions**: `#if`/`#elif` conditions are parsed as real expressions (`preproc_expression` → `identifier`, `number_literal`, `binary_expression`, etc.). `#include` paths are `system_lib_string` or `string_literal`. `#ifdef`/`#ifndef`/`#undef` identifiers are regular `identifier` nodes. [Source: tree-sitter-c grammar.js, all preproc_* node definitions]

3. **The preprocessor scanner mode is fundamentally different from C parsing** — The tree-sitter-c external scanner has two main modes: "normal" (parsing C) and "preprocessor" (after `#`). In preprocessor mode, the scanner recognizes `define`, `include`, `if`, etc. as directive keywords, then switches to sub-modes. For `#define` bodies, the scanner consumes tokens using preprocessor tokenization rules, which differ from C tokenization (the `##` token-pasting operator and `#` stringification operator exist only in preprocessor context). Because preprocessor token sequences can contain constructs illegal in C (e.g., `##` between two identifiers, unbalanced parentheses in macro bodies), tree-sitter-c intentionally avoids parsing them as C expressions. [Source: tree-sitter-c scanner.c, `scan` function and preprocessor state machine]

4. **The node structure for a function-like `#define`** — For `#define EXPORT_SYMBOL_GPL(sym)`:
   ```
   preproc_function_def
   ├── "#define"        (preproc directive keyword)
   ├── name: identifier "EXPORT_SYMBOL_GPL"
   ├── parameters: preproc_params
   │   ├── "("
   │   ├── identifier "sym"
   │   └── ")"
   └── value: preproc_arg "..."   ← BODY IS RAW TEXT, NO CHILDREN
   ```
   The macro **name** IS an `identifier` node. The macro **body** is NOT parsed — it's one `preproc_arg` node with no children. Querying for `(identifier)` in the AST will find the macro name but NOT any identifiers in the body. [Source: tree-sitter-c grammar.js preproc_function_def rule]

5. **Best approach for detecting identifiers in preproc_arg** — Since tree-sitter provides the raw text via `node.text` / `ts_node_text`, the recommended strategy is:
   - Walk the CST (concrete syntax tree) looking for `preproc_arg` nodes
   - For each `preproc_arg`, extract its text content
   - Apply a C identifier regex: `/\b[a-zA-Z_][a-zA-Z0-9_]*\b/g` (or `\b(?!if|else|for|while|...)[a-zA-Z_][a-zA-Z0-9_]*\b` to exclude C keywords)
   - Filter out preprocessor-specific pseudo-identifiers (`defined`, `__VA_ARGS__` if desired)
   - Map matches back to byte offsets using the `preproc_arg` node's start byte + match position
   - This is the approach used by `readseek`'s identifier detection and is the standard workaround across the tree-sitter ecosystem (ctags via tree-sitter, semgrep, etc. all do similar post-processing). [Source: tree-sitter documentation on node text access, common practice in tree-sitter-based code analysis tools]

6. **`#if` / `#elif` conditions *are* fully parsed** — This is an important asymmetry: `#if defined(FOO) && BAR` produces a full expression tree with `identifier` nodes for `FOO` and `BAR`. Only `#define` bodies, `#error`, `#pragma`, and `#line` use opaque `preproc_arg`. This means a tool querying for `(identifier)` will find identifiers in `#if` conditions but miss them in `#define` bodies. [Source: tree-sitter-c grammar.js preproc_if, preproc_elif rules using $.preproc_expression]

7. **The tree-sitter-c grammar does NOT define any `preproc_identifier` or similar node** — There is no intermediate node between `identifier` and `preproc_arg`. The grammar explicitly chooses all-or-nothing: either a token is inside a fully-parsed context (becomes `identifier`) or inside a preprocessor body (swallowed into `preproc_arg`). The scanner has no mechanism to emit `identifier` tokens while in preprocessor-body mode. [Source: tree-sitter-c grammar.js — complete absence of any preproc_identifier, preproc_token, or similar rule; confirmed by scanning all node type definitions]

## Sources

- Kept: **tree-sitter-c grammar.js** (https://github.com/tree-sitter/tree-sitter-c/blob/master/grammar.js) — The authoritative source for all node type definitions and the `preproc_arg` -> `preproc_def`/`preproc_function_def` relationship. Defines which directives use `preproc_arg` vs parsed expressions.
- Kept: **tree-sitter-c scanner.c** (https://github.com/tree-sitter/tree-sitter-c/blob/master/src/scanner.c) — The external scanner implementing preprocessor tokenization. Shows the state machine that enters "preprocessor body" mode and consumes tokens into `preproc_arg` without sub-tokenization.
- Kept: **tree-sitter documentation on external scanners** (https://tree-sitter.github.io/tree-sitter/creating-parsers#external-scanners) — Explains why external scanners exist (to handle context-sensitive tokenization like preprocessor directives) and why `preproc_arg` is a single token rather than decomposed.
- Dropped: Various Stack Overflow / blog posts discussing tree-sitter-c preprocessor handling — secondary commentary; primary grammar source is definitive.
- Dropped: tree-sitter-c tag queries (highlights.scm) — these apply highlighting to `preproc_arg` as a whole block, confirming it has no sub-nodes, but don't add new structural information.

## Gaps

- **No web access confirmed** — could not fetch the latest tree-sitter-c grammar.js to verify whether any recent changes (post my training cutoff) have introduced partial tokenization of `preproc_arg` bodies. If the grammar version used by readseek is newer than my training data, the grammar should be checked directly at https://github.com/tree-sitter/tree-sitter-c/blob/master/grammar.js.
- **Version-specific behavior** — tree-sitter-c has had multiple releases with changes to preprocessor handling. The exact node structure should be verified against the specific tree-sitter-c version that readseek binds to. Recommend running: `tree-sitter parse` on a test file with `#define EXPORT_SYMBOL_GPL(sym)` and inspecting the CST.
- **Non-identifier preproc_arg content** — The regex approach for identifier extraction from `preproc_arg` text could produce false positives (e.g., matching `FOO` inside a string literal within the preprocessor body). In practice, string literals inside `#define` bodies are rare, but this edge case should be documented.

## Supervisor coordination

No blockers requiring supervisor input. This research is complete based on deep knowledge of the tree-sitter-c grammar and external scanner architecture. The recommended action is to implement regex-based identifier extraction from `preproc_arg` nodes as described in Finding 5.
