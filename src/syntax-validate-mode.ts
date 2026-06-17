export type SyntaxValidateMode = "warn" | "block" | "off";

export interface SyntaxValidateOptions {
  syntaxValidate?: SyntaxValidateMode;
}

const VALID = new Set<SyntaxValidateMode>(["warn", "block", "off"]);
const DEFAULT: SyntaxValidateMode = "warn";

function coerce(value: unknown): SyntaxValidateMode | undefined {
  if (typeof value !== "string") return undefined;
  return VALID.has(value as SyntaxValidateMode)
    ? (value as SyntaxValidateMode)
    : undefined;
}

export function resolveSyntaxValidateMode(
  opts: SyntaxValidateOptions,
): SyntaxValidateMode {
  const fromOpt = coerce(opts.syntaxValidate);
  if (fromOpt) return fromOpt;
  const fromEnv = coerce(process.env.READSEEK_SYNTAX_VALIDATE);
  if (fromEnv) return fromEnv;
  return DEFAULT;
}
