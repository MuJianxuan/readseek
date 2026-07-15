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

/** Required file-path parameter shared by the read, edit, and write tools. */
export function filePathParam() {
  return Type.String({ description: "Absolute or relative file path" });
}

/** Optional structural-map toggle shared by the read and write tools. */
export function mapParam() {
  return Type.Optional(Type.Boolean({ description: "Append structural map" }));
}

type ToolSpec = Parameters<ExtensionAPI["registerTool"]>[0];

/** Register a readseek-backed tool definition. */
export function registerReadSeekTool<T extends ToolSpec>(pi: ExtensionAPI, tool: T): T {
  pi.registerTool(tool);
  return tool;
}
