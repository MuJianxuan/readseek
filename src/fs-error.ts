export type FsErrorCode = "path-is-directory" | "permission-denied" | "file-not-found";

export interface FsErrorInfo {
  code: FsErrorCode;
  message: string;
  hint?: string;
}

/**
 * Translate a Node `fs` errno into the shared readseek error taxonomy with a
 * canonical message. Returns null for errno values without a dedicated code so
 * callers fall back to their own `fs-error` handling.
 */
export function classifyFsError(err: { code?: unknown }, path: string): FsErrorInfo | null {
  switch (err?.code) {
    case "EISDIR":
      return {
        code: "path-is-directory",
        message: `Path is a directory: ${path}`,
        hint: `Use ls(${JSON.stringify(path)}) to inspect directories.`,
      };
    case "EACCES":
    case "EPERM":
      return { code: "permission-denied", message: `Permission denied: ${path}` };
    case "ENOENT":
      return { code: "file-not-found", message: `File not found: ${path}` };
    default:
      return null;
  }
}
