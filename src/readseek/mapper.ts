import { stat } from "node:fs/promises";

import { readseekMap, readseekMapContent } from "../readseek-client.js";
import { throwIfAborted } from "../runtime.js";
import type { FileMap, MapOptions } from "./types.js";

export async function generateMap(
	filePath: string,
	options: MapOptions = {},
): Promise<FileMap | null> {
	throwIfAborted(options.signal);
	const fileStat = await stat(filePath);
	throwIfAborted(options.signal);
	const map = await readseekMap(filePath, fileStat.size, { signal: options.signal });
	throwIfAborted(options.signal);
	return map;
}

export async function generateMapFromContent(
	filePath: string,
	content: string,
	options: MapOptions = {},
): Promise<FileMap | null> {
	throwIfAborted(options.signal);
	const map = await readseekMapContent(filePath, content, { signal: options.signal });
	throwIfAborted(options.signal);
	return map;
}
