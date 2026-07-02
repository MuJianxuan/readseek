const SYSLOG_PATH_SUFFIX = /,\s+[a-z]+(\s+'.*')?$/;

/**
 * Derive the canonical OS strerror from a Node {@link err.message} by
 * stripping the trailing syscall and optional path suffix so the result
 * is portable and never duplicates information already carried in the
 * error-envelope `path` field.
 */
function strerror(err: { message?: unknown }): string {
	const raw = String(err?.message ?? String(err));
	return raw.replace(SYSLOG_PATH_SUFFIX, "");
}

/**
 * Build a readseek error from a Node `fs` errno exception.
 *
 * Every errno maps to its own canonical code automatically — no
 * hand-curated switch, no `"fs-error"` catch-all.
 *
 * @param err  The caught exception (carries `err.code` and `err.message`).
 * @param domain  Tool-level prefix for the message: `"read-error"`,
 *                `"edit-error"`, `"write-error"`, `"stat-error"`.
 */
export function formatFsError(
	err: { code?: unknown; message?: string },
	domain: string,
): { code: string; message: string } {
	const errno = typeof err.code === "string" ? err.code : "EIO";
	return {
		code: errno,
		message: `${domain}: ${errno}: ${strerror(err)}`,
	};
}
