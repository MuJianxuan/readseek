import { stat as fsStat } from "node:fs/promises";
import type { Stats } from "node:fs";

import { buildToolErrorResult, type ToolErrorResult } from "./readseek-value.js";
import { formatFsError } from "./fs-error.js";

export type StatSearchPathResult =
  | { ok: true; stats: Stats }
  | { ok: false; error: ToolErrorResult };

/**
 * Stat a tool's search path, mapping access failures into the shared readseek
 * error taxonomy. Used by refs and search, which reject a missing or
 * unreadable path with identical codes and messages.
 */
export async function statSearchPathOrError(
  tool: string,
  rawPath: string | undefined,
  searchPath: string,
): Promise<StatSearchPathResult> {
  try {
    return { ok: true, stats: await fsStat(searchPath) };
  } catch (err: any) {
    const display = rawPath ?? ".";
    const path = rawPath ?? searchPath;
    const { code, message } = formatFsError(err, "stat-error");
    return { ok: false, error: buildToolErrorResult(tool, code, message, { path }) };
  }
}
