import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";

/**
 * Optional tool parameter accepting an integer or its string form, since models
 * frequently pass integers as quoted strings. Both union halves share
 * {@link description}.
 */
export function optionalIntOrString(description: string) {
  return Type.Optional(Type.Union([Type.Number({ description }), Type.String({ description })]));
}

export type ReadSeekToolPolicy = "read-only" | "mutating";

export type ReadSeekToolExposure = "safe-by-default" | "opt-in" | "not-safe-by-default";

/**
 * Per-tool fields of the pi tool config (`ptc`) that genuinely differ between
 * readseek tools. The remaining fields are constant or derived.
 */
export interface ReadSeekToolConfig {
  policy: ReadSeekToolPolicy;
  pythonName: string;
  defaultExposure: ReadSeekToolExposure;
}

export interface ReadSeekToolPtc extends ReadSeekToolConfig {
  callable: true;
  enabled: true;
  readOnly: boolean;
}

type ToolSpec = Parameters<ExtensionAPI["registerTool"]>[0];

/**
 * Attach the standard `ptc` envelope to a tool definition and register it.
 *
 * `callable` and `enabled` are always true and `readOnly` is derived from
 * `policy`, so callers supply only the fields that vary between tools.
 */
export function registerReadSeekTool<T extends ToolSpec>(
  pi: ExtensionAPI,
  config: ReadSeekToolConfig,
  tool: T,
): T & { ptc: ReadSeekToolPtc } {
  const ptc: ReadSeekToolPtc = {
    callable: true,
    enabled: true,
    policy: config.policy,
    readOnly: config.policy === "read-only",
    pythonName: config.pythonName,
    defaultExposure: config.defaultExposure,
  };
  const registered = { ...tool, ptc };
  pi.registerTool(registered);
  return registered;
}
