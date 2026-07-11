import { stat } from "node:fs/promises";

import type { FileMap } from "./readseek/types.js";
import { readSeekMap } from "./readseek-client.js";

/**
 * Fetch a structural file map from readseek, which maintains its own on-disk
 * map cache keyed by file hash. Returns null on any failure — never throws.
 */
export async function getOrGenerateMap(absPath: string): Promise<FileMap | null> {
	try {
		const { size } = await stat(absPath);
		return await readSeekMap(absPath, size);
	} catch {
		return null;
	}
}
