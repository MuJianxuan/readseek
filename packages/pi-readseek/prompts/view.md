View the structure or selected content of an indexed PDF. Start with the default overview, then narrow by page or node instead of reading the whole document.

## Parameters

- `path` — PDF document to view.
- `node` — optional node ID to use as the view root.
- `page` — optional one-based source page.
- `kind` — optional node kind filter.
- `depth` — optional maximum depth below selected roots.
- `outline` — return outline nodes only.

## Output

Returns a bounded text projection with stable node IDs and source page references.
