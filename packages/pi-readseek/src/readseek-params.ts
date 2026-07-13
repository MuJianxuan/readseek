import { Type } from "@sinclair/typebox";

import { buildToolErrorResult, type ToolErrorResult } from "./readseek-value.js";

/** Optional search path shared by the readseek search tools. */
export function searchPathParam() {
  return Type.Optional(Type.String({ description: "Search path" }));
}

/** Optional language hint shared by the readseek search tools. */
export function langParam() {
  return Type.Optional(Type.String({ description: "Language hint" }));
}

/**
 * TypeBox fragment for the Git-scope parameters shared by every readseek
 * search tool. Returns fresh schema instances so each tool spreads its own.
 */
export function readSeekGitSearchParams() {
  return {
    cached: Type.Optional(Type.Boolean({ description: "In a Git repository, search tracked/indexed files" })),
    others: Type.Optional(Type.Boolean({ description: "In a Git repository, search untracked files" })),
    ignored: Type.Optional(Type.Boolean({ description: "With others=true, include ignored untracked files" })),
  };
}

/**
 * Enforce that `ignored` is only meaningful alongside `others`. Returns an
 * error envelope tagged with {@link tool} when the rule is violated, else null.
 */
export function validateIgnoredRequiresOthers(
  tool: string,
  params: { others?: boolean; ignored?: boolean },
): ToolErrorResult | null {
  if (params.ignored && !params.others) {
    return buildToolErrorResult(tool, "invalid-parameter", `${tool} parameter 'ignored' requires 'others'`);
  }
  return null;
}
