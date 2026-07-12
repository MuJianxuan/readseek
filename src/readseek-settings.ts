import { readFileSync, statSync, type Stats } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export type ReadSeekImageAnalysisMode = "force" | "off" | "auto";
export type ReadSeekSyntaxValidationMode = "warn" | "block" | "off";
interface ReadSeekGrepSettings {
  maxLines?: number;
  maxBytes?: number;
}

const READSEEK_TOOL_REPLACEMENTS = {
  read: "readSeek_read",
  edit: "readSeek_edit",
  grep: "readSeek_grep",
  write: "readSeek_write",
} as const;

type ReadSeekReplacementTool = keyof typeof READSEEK_TOOL_REPLACEMENTS;

interface ReadSeekJsonSettings {
  replaceTools?: ReadSeekReplacementTool[];
  imageMode?: ReadSeekImageAnalysisMode;
  syntaxValidation?: ReadSeekSyntaxValidationMode;
  timeoutMs?: number;
  grep?: ReadSeekGrepSettings;
}

export interface ReadSeekSettingsWarning {
  source: string;
  message: string;
  path?: string;
}

export interface ReadSeekSettingsResult {
  settings: ReadSeekJsonSettings;
  warnings: ReadSeekSettingsWarning[];
}

const READSEEK_KEYS = ["replaceTools", "imageMode", "syntaxValidation", "timeoutMs", "grep"];
const READSEEK_GREP_KEYS = ["maxLines", "maxBytes"];

function globalSettingsPath(): string {
  return join(homedir(), ".pi/agent/settings.json");
}

function projectSettingsPath(): string {
  return join(process.cwd(), ".pi/settings.json");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function invalid(source: string, path: string): ReadSeekSettingsWarning {
  return { source, path, message: `Invalid readseek setting at ${path}` };
}

function warnUnknownKeys(
  raw: Record<string, unknown>,
  known: string[],
  prefix: string,
  source: string,
  warnings: ReadSeekSettingsWarning[],
): void {
  for (const key of Object.keys(raw)) {
    if (known.includes(key)) continue;
    const path = `${prefix}.${key}`;
    warnings.push({ source, path, message: `Unknown readseek setting at ${path}` });
  }
}

function readPositive(
  raw: Record<string, unknown>,
  key: string,
  path: string,
  source: string,
  warnings: ReadSeekSettingsWarning[],
): number | undefined {
  if (!(key in raw)) return undefined;
  const val = raw[key];
  if (typeof val === "number" && Number.isSafeInteger(val) && val > 0) return val;
  warnings.push(invalid(source, path));
  return undefined;
}

function readEnum<T extends string>(
  raw: Record<string, unknown>,
  key: string,
  values: readonly T[],
  path: string,
  source: string,
  warnings: ReadSeekSettingsWarning[],
): T | undefined {
  if (!(key in raw)) return undefined;
  const val = raw[key];
  if (typeof val === "string" && (values as readonly string[]).includes(val)) return val as T;
  warnings.push(invalid(source, path));
  return undefined;
}

function readImageMode(
  raw: Record<string, unknown>,
  source: string,
  warnings: ReadSeekSettingsWarning[],
): ReadSeekImageAnalysisMode | undefined {
  if (!("imageMode" in raw)) return undefined;
  const val = raw.imageMode;
  if (val === "on") return "force";
  if (val === "force" || val === "off" || val === "auto") return val;
  warnings.push(invalid(source, "readseek.imageMode"));
  return undefined;
}

function readReplaceTools(
  raw: Record<string, unknown>,
  source: string,
  warnings: ReadSeekSettingsWarning[],
): ReadSeekReplacementTool[] | undefined {
  if (!("replaceTools" in raw)) return undefined;
  const value = raw.replaceTools;
  if (!Array.isArray(value)) {
    warnings.push(invalid(source, "readseek.replaceTools"));
    return undefined;
  }

  const tools: ReadSeekReplacementTool[] = [];
  value.forEach((tool, index) => {
    if (typeof tool === "string" && Object.hasOwn(READSEEK_TOOL_REPLACEMENTS, tool)) {
      tools.push(tool as ReadSeekReplacementTool);
    } else {
      warnings.push(invalid(source, `readseek.replaceTools[${index}]`));
    }
  });
  return tools;
}

function validateSettings(raw: unknown, source: string): ReadSeekSettingsResult {
  const settings: ReadSeekJsonSettings = {};
  const warnings: ReadSeekSettingsWarning[] = [];
  if (!isRecord(raw)) {
    warnings.push({ source, message: "Invalid readseek settings: expected a JSON object" });
    return { settings, warnings };
  }
  if (!("readseek" in raw)) {
    if (Object.keys(raw).length > 0) {
      warnings.push({ source, path: "readseek", message: "Missing readseek section: every setting belongs under \"readseek\"" });
    }
    return { settings, warnings };
  }
  if (!isRecord(raw.readseek)) {
    warnings.push(invalid(source, "readseek"));
    return { settings, warnings };
  }

  const section = raw.readseek;
  warnUnknownKeys(section, READSEEK_KEYS, "readseek", source, warnings);

  const replaceTools = readReplaceTools(section, source, warnings);
  if (replaceTools !== undefined) settings.replaceTools = replaceTools;

  const imageMode = readImageMode(section, source, warnings);
  if (imageMode !== undefined) settings.imageMode = imageMode;

  const syntaxValidation = readEnum(
    section,
    "syntaxValidation",
    ["warn", "block", "off"] as const,
    "readseek.syntaxValidation",
    source,
    warnings,
  );
  if (syntaxValidation !== undefined) settings.syntaxValidation = syntaxValidation;

  const timeoutMs = readPositive(section, "timeoutMs", "readseek.timeoutMs", source, warnings);
  if (timeoutMs !== undefined) settings.timeoutMs = timeoutMs;

  if ("grep" in section) {
    if (isRecord(section.grep)) {
      warnUnknownKeys(section.grep, READSEEK_GREP_KEYS, "readseek.grep", source, warnings);
      const grep: ReadSeekGrepSettings = {};
      const maxLines = readPositive(section.grep, "maxLines", "readseek.grep.maxLines", source, warnings);
      if (maxLines !== undefined) grep.maxLines = maxLines;
      const maxBytes = readPositive(section.grep, "maxBytes", "readseek.grep.maxBytes", source, warnings);
      if (maxBytes !== undefined) grep.maxBytes = maxBytes;
      if (Object.keys(grep).length > 0) settings.grep = grep;
    } else {
      warnings.push(invalid(source, "readseek.grep"));
    }
  }

  return { settings, warnings };
}

interface SettingsFileCacheEntry {
  mtimeMs: number;
  size: number;
  result: ReadSeekSettingsResult;
}

const settingsFileCache = new Map<string, SettingsFileCacheEntry>();

function statFileOrNull(path: string): Stats | null {
  try {
    return statSync(path);
  } catch {
    return null;
  }
}

function readSettingsFile(path: string): ReadSeekSettingsResult {
  const stats = statFileOrNull(path);
  if (!stats) {
    settingsFileCache.delete(path);
    return { settings: {}, warnings: [] };
  }

  const cached = settingsFileCache.get(path);
  if (cached && cached.mtimeMs === stats.mtimeMs && cached.size === stats.size) return cached.result;

  let result: ReadSeekSettingsResult;
  try {
    const text = readFileSync(path, "utf8");
    result = validateSettings(JSON.parse(text) as unknown, path);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    result = { settings: {}, warnings: [{ source: path, message: `Invalid JSON: ${message}` }] };
  }
  settingsFileCache.set(path, { mtimeMs: stats.mtimeMs, size: stats.size, result });
  return result;
}

function mergeSettings(base: ReadSeekJsonSettings, override: ReadSeekJsonSettings): ReadSeekJsonSettings {
  const settings: ReadSeekJsonSettings = { ...base, ...override };
  const grep = { ...(base.grep ?? {}), ...(override.grep ?? {}) };
  if (Object.keys(grep).length > 0) settings.grep = grep;
  return settings;
}

export function resolveReadSeekJsonSettings(): ReadSeekSettingsResult {
  const globalResult = readSettingsFile(globalSettingsPath());
  const projectResult = readSettingsFile(projectSettingsPath());
  return {
    settings: mergeSettings(globalResult.settings, projectResult.settings),
    warnings: [...globalResult.warnings, ...projectResult.warnings],
  };
}

export function resolveReadSeekImageMode(): ReadSeekImageAnalysisMode {
  return resolveReadSeekJsonSettings().settings.imageMode ?? "force";
}

export function resolveReadSeekSyntaxValidation(): ReadSeekSyntaxValidationMode | undefined {
  return resolveReadSeekJsonSettings().settings.syntaxValidation;
}

export function resolveReadSeekTimeoutMs(): number | undefined {
  return resolveReadSeekJsonSettings().settings.timeoutMs;
}
