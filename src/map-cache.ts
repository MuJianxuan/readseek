import { stat } from "node:fs/promises";
import type { FileMap } from "./readseek/types.js";
import { generateMap } from "./readseek/mapper.js";

interface CacheEntry {
	mtimeMs: number;
	map: FileMap | null;
}

export const MAP_CACHE_MAX_SIZE = 500;

interface MapCacheGlobalState {
	cache: Map<string, CacheEntry>;
	inflight: Map<string, Promise<FileMap | null>>;
	maxSize: number;
}

const MAP_CACHE_STATE_KEY = Symbol.for("pi-readseek.mapCacheState.v1");

function getMapCacheState(): MapCacheGlobalState {
	const globalObject = globalThis as any;
	globalObject[MAP_CACHE_STATE_KEY] ??= {
		cache: new Map<string, CacheEntry>(),
		inflight: new Map<string, Promise<FileMap | null>>(),
		maxSize: MAP_CACHE_MAX_SIZE,
	} satisfies MapCacheGlobalState;
	return globalObject[MAP_CACHE_STATE_KEY] as MapCacheGlobalState;
}

function rememberInMemory(absPath: string, entry: CacheEntry): void {
	const state = getMapCacheState();
	if (state.cache.has(absPath)) state.cache.delete(absPath);
	state.cache.set(absPath, entry);
	if (state.cache.size > state.maxSize) {
		const oldestKey = state.cache.keys().next().value;
		if (oldestKey !== undefined) state.cache.delete(oldestKey);
	}
}

/**
 * Get or generate a structural file map, with mtime-based in-memory caching.
 * Readseek already caches map results in `.readseek/maps/` with its own binary
 * format.  The in-memory layer here avoids redundant process spawns within a
 * single session.
 *
 * Returns null on any failure — never throws.
 */
export async function getOrGenerateMap(absPath: string): Promise<FileMap | null> {
	try {
		const fileStat = await stat(absPath);
		const { mtimeMs } = fileStat;
		const state = getMapCacheState();
		const cached = state.cache.get(absPath);
		if (cached && cached.mtimeMs === mtimeMs) {
			state.cache.delete(absPath);
			state.cache.set(absPath, cached);
			return cached.map;
		}
		const inflight = state.inflight.get(absPath);
		if (inflight) return inflight;

		const generation = (async () => {
			const map = await generateMap(absPath);
			rememberInMemory(absPath, { mtimeMs, map });
			return map;
		})()
			.catch(() => null)
			.finally(() => {
				state.inflight.delete(absPath);
			});
		state.inflight.set(absPath, generation);
		return generation;
	} catch {
		return null;
	}
}
