import { resolveReadSeekSyntaxValidation } from "./readseek-settings.js";

export type SyntaxValidateMode = "warn" | "block" | "off";

export interface SyntaxValidateOptions {
  syntaxValidate?: SyntaxValidateMode;
}

const DEFAULT: SyntaxValidateMode = "warn";

export function resolveSyntaxValidateMode(
  opts: SyntaxValidateOptions,
): SyntaxValidateMode {
  return opts.syntaxValidate ?? resolveReadSeekSyntaxValidation() ?? DEFAULT;
}
