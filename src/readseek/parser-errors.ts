const emitted = new Set<string>();

function causeMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

export function reportParserError(
  onceKey: string,
  err: unknown,
  options: { context?: string } = {},
): void {
  if (!process.env.PI_READSEEK_DEBUG) return;
  if (emitted.has(onceKey)) return;
  emitted.add(onceKey);
  const context = options.context ?? onceKey;
  console.error(`[readseek] ${context}: ${causeMessage(err)}`);
}

