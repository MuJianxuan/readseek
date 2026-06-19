import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export type ReadseekToolPolicy = "read-only" | "mutating";

export type ReadseekToolExposure = "safe-by-default" | "opt-in" | "not-safe-by-default";

/**
 * Per-tool fields of the pi tool config (`ptc`) that genuinely differ between
 * readseek tools. The remaining fields are constant or derived.
 */
export interface ReadseekToolConfig {
  policy: ReadseekToolPolicy;
  pythonName: string;
  defaultExposure: ReadseekToolExposure;
}

export interface ReadseekToolPtc extends ReadseekToolConfig {
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
export function registerReadseekTool<T extends ToolSpec>(
  pi: ExtensionAPI,
  config: ReadseekToolConfig,
  tool: T,
): T & { ptc: ReadseekToolPtc } {
  const ptc: ReadseekToolPtc = {
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
