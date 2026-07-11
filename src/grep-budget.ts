import { DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES } from "@earendil-works/pi-coding-agent";
import { resolveReadSeekJsonSettings } from "./readseek-settings.js";

export interface GrepOutputBudget {
	maxLines: number;
	maxBytes: number;
}

/**
 * Effective grep-output ceilings used as clamp upper bounds and as the
 * fallback defaults when the settings are unset.
 *
 * The bytes ceiling is the already-tightened 50 KiB used by `buildGrepOutput`
 * today, NOT the unclamped `DEFAULT_MAX_BYTES`.
 */
export const GREP_OUTPUT_DEFAULT_MAX_LINES = DEFAULT_MAX_LINES;
export const GREP_OUTPUT_DEFAULT_MAX_BYTES = Math.min(DEFAULT_MAX_BYTES, 50 * 1024);

function resolveDimension(jsonValue: number | undefined, ceiling: number): number {
	if (jsonValue !== undefined) return Math.min(jsonValue, ceiling);
	return ceiling;
}

/**
 * Resolve the effective grep visible-output budget from the readseek
 * settings. Below-default values are used as-is; above-default values clamp
 * to the current defaults.
 */
export function resolveGrepOutputBudget(): GrepOutputBudget {
	const settings = resolveReadSeekJsonSettings().settings.grep;
	return {
		maxLines: resolveDimension(settings?.maxLines, GREP_OUTPUT_DEFAULT_MAX_LINES),
		maxBytes: resolveDimension(settings?.maxBytes, GREP_OUTPUT_DEFAULT_MAX_BYTES),
	};
}
