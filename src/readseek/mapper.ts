import { stat } from "node:fs/promises";

import { readseekMap, readseekMapContent } from "../readseek-client.js";
import { throwIfAborted } from "../runtime.js";
import type { FileMap, MapOptions } from "./types.js";

export const READSEEK_MAPPER_NAME = "readseek";
export const READSEEK_MAPPER_VERSION = 2;

export interface MapperIdentity {
  mapperName: string;
  mapperVersion: number;
}

export interface MapResultWithIdentity extends MapperIdentity {
  map: FileMap | null;
}

export const READSEEK_MAPPER_IDENTITY: MapperIdentity = {
  mapperName: READSEEK_MAPPER_NAME,
  mapperVersion: READSEEK_MAPPER_VERSION,
};
export async function generateMapWithIdentity(
  filePath: string,
  options: MapOptions = {},
): Promise<MapResultWithIdentity> {
  throwIfAborted(options.signal);
  const fileStat = await stat(filePath);
  throwIfAborted(options.signal);
  const map = await readseekMap(filePath, fileStat.size, { signal: options.signal });
  throwIfAborted(options.signal);
  return { map, ...READSEEK_MAPPER_IDENTITY };
}

export async function generateMap(
  filePath: string,
  options: MapOptions = {},
): Promise<FileMap | null> {
  return (await generateMapWithIdentity(filePath, options)).map;
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

