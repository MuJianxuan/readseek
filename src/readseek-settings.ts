import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface ReadseekJsonSettings {
  grep?: { maxLines?: number; maxBytes?: number };
  edit?: { diffDisplay?: "collapsed" | "expanded" };
}

export interface ReadseekSettingsWarning {
  source: string;
  message: string;
  path?: string;
}

export interface ReadseekSettingsResult {
  settings: ReadseekJsonSettings;
  warnings: ReadseekSettingsWarning[];
}


function defaultGlobalSettingsPath(): string {
  return join(homedir(), ".pi/agent/readseek/settings.json");
}

function defaultProjectSettingsPath(): string {
  return join(process.cwd(), ".pi/readseek/settings.json");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function invalid(source: string, path: string): ReadseekSettingsWarning {
  return { source, path, message: `Invalid readseek setting at ${path}` };
}

function readPositive(
  raw: Record<string, unknown>,
  key: string,
  path: string,
  source: string,
  warnings: ReadseekSettingsWarning[],
): number | undefined {
  if (!(key in raw)) return undefined;
  const val = raw[key];
  if (typeof val === "number" && Number.isSafeInteger(val) && val > 0) return val;
  warnings.push(invalid(source, path));
  return undefined;
}


function validateSettings(raw: unknown, source: string): ReadseekSettingsResult {
  const settings: ReadseekJsonSettings = {};
  const warnings: ReadseekSettingsWarning[] = [];
  if (!isRecord(raw)) return { settings, warnings };

  if (isRecord(raw.grep)) {
    const grep: NonNullable<ReadseekJsonSettings["grep"]> = {};
    const maxLines = readPositive(raw.grep, "maxLines", "grep.maxLines", source, warnings);
    if (maxLines !== undefined) grep.maxLines = maxLines;
    const maxBytes = readPositive(raw.grep, "maxBytes", "grep.maxBytes", source, warnings);
    if (maxBytes !== undefined) grep.maxBytes = maxBytes;
    if (Object.keys(grep).length > 0) settings.grep = grep;
  }


  if (isRecord(raw.edit)) {
    const edit: NonNullable<ReadseekJsonSettings["edit"]> = {};
    if ("diffDisplay" in raw.edit) {
      const value = raw.edit.diffDisplay;
      if (value === "collapsed" || value === "expanded") edit.diffDisplay = value;
      else warnings.push(invalid(source, "edit.diffDisplay"));
    }
    if (Object.keys(edit).length > 0) settings.edit = edit;
  }

  return { settings, warnings };
}

function readSettingsFile(path: string): ReadseekSettingsResult {
  if (!existsSync(path)) return { settings: {}, warnings: [] };

  try {
    const text = readFileSync(path, "utf8");
    return validateSettings(JSON.parse(text) as unknown, path);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { settings: {}, warnings: [{ source: path, message: `Invalid JSON: ${message}` }] };
  }
}

function mergeSettings(base: ReadseekJsonSettings, override: ReadseekJsonSettings): ReadseekJsonSettings {
  const merged: ReadseekJsonSettings = {};
  const grep = { ...(base.grep ?? {}), ...(override.grep ?? {}) };
  if (Object.keys(grep).length > 0) merged.grep = grep;
  const edit = { ...(base.edit ?? {}), ...(override.edit ?? {}) };
  if (Object.keys(edit).length > 0) merged.edit = edit;
  return merged;
}

export function resolveReadseekJsonSettings(): ReadseekSettingsResult {
  const globalResult = readSettingsFile(defaultGlobalSettingsPath());
  const projectResult = readSettingsFile(defaultProjectSettingsPath());
  return {
    settings: mergeSettings(globalResult.settings, projectResult.settings),
    warnings: [...globalResult.warnings, ...projectResult.warnings],
  };
}

export function resolveEditDiffDisplay(env: NodeJS.ProcessEnv = process.env): "collapsed" | "expanded" {
  const raw = env.READSEEK_EDIT_DIFF_DISPLAY;
  if (typeof raw === "string") {
    const normalized = raw.trim().toLowerCase();
    if (normalized === "expanded" || normalized === "collapsed") return normalized;
  }
  const json = resolveReadseekJsonSettings().settings.edit?.diffDisplay;
  if (json === "expanded" || json === "collapsed") return json;
  return "collapsed";
}
