Find files or directories recursively by basename. Uses glob patterns by default, respects nested `.gitignore`, includes hidden entries, and returns relative paths.

## Parameters

- `pattern` — required. Glob by default; with `regex: true`, JavaScript regex against each basename.
- `path` — directory to search, default cwd.
- `type` — `"file"` default, `"dir"`, or `"any"`.
- `limit` — max returned entries after filtering/sorting, default 1000.
- `maxDepth` — non-negative directory depth limit.
- `sortBy` — `"name"` default, `"mtime"`, or `"size"`; use `reverse: true` for descending/newest/largest first.
- `modifiedSince` — keep entries modified strictly after an ISO date/time or relative age like `30m`, `1h`, `24h`, `7d`.
- `minSize` / `maxSize` — inclusive file-size filters; numbers are bytes, strings accept 1024-based `KB`, `MB`, `GB`, etc. Directories are not removed by size filters.

## Output and usage

Output is one relative path per line. Directories end with `/`. Filtering and sorting happen before `limit`, so newest/largest queries work as expected.

Use `find` for recursive name discovery, `ls` for one directory, `grep` or `search` for contents, and `read` for file content. Remember: `pattern` matches basenames, not full paths.
