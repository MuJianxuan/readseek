import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerReadTool } from "./src/read.js";
import { registerEditTool } from "./src/edit.js";
import { registerGrepTool } from "./src/grep.js";
import { registerSgTool, isSgAvailable } from "./src/sg.js";
import { registerWriteTool } from "./src/write.js";
import { registerLsTool } from "./src/ls.js";
import { registerFindTool } from "./src/find.js";
import { registerReadseekCommand } from "./src/readseek-command.js";
import { applyContextHygieneStaleContext } from "./src/context-application.js";
import {
  createContextHygieneTracker,
  normalizePathForContextHygiene,
  type ContextHygieneEvent,
  type ContextHygieneMetadata,
  type ContextHygieneReport,
  type ContextHygieneResource,
  type ContextHygieneTracker,
} from "./src/context-hygiene.js";
import {
  consumeDoomLoopWarning,
  createDoomLoopState,
  formatDoomLoopMessage,
  recordToolCall,
} from "./src/doom-loop.js";

function isContextHygieneResource(value: unknown): value is ContextHygieneResource {
  if (!value || typeof value !== "object") return false;
  const resource = value as { kind?: unknown; key?: unknown };
  return (resource.kind === "file" || resource.kind === "symbol") && typeof resource.key === "string";
}

function isContextHygieneMetadata(value: unknown): value is ContextHygieneMetadata {
  if (!value || typeof value !== "object") return false;
  const metadata = value as Partial<ContextHygieneMetadata>;
  return (
    metadata.schemaVersion === 1 &&
    typeof metadata.tool === "string" &&
    (metadata.classification === "read-context" ||
      metadata.classification === "search-context" ||
      metadata.classification === "mutation") &&
    Array.isArray(metadata.resources) &&
    metadata.resources.every(isContextHygieneResource)
  );
}

function contextHygieneFromDetails(details: unknown): ContextHygieneMetadata | undefined {
  if (!details || typeof details !== "object") return undefined;
  const metadata = (details as { contextHygiene?: unknown }).contextHygiene;
  return isContextHygieneMetadata(metadata) ? metadata : undefined;
}

function recordContextHygiene(
  tracker: ContextHygieneTracker,
  metadata: ContextHygieneMetadata,
  toolCallId: unknown,
): ContextHygieneEvent {
  return tracker.record(metadata, {
    resultId: typeof toolCallId === "string" ? toolCallId : undefined,
  });
}

export default function piReadseekExtension(pi: ExtensionAPI): void {
  const readTurns = new Map<string, number>();
  const doomLoopState = createDoomLoopState();
  const contextHygieneTracker = createContextHygieneTracker();
  const readTurnKey = (absolutePath: string) => normalizePathForContextHygiene(absolutePath);
  const noteRead = (absolutePath: string) => {
    const report = contextHygieneTracker.generateReport();
    const eventId = report.eventCount + 1;
    readTurns.set(readTurnKey(absolutePath), eventId);
  };
  const wasReadInSession = (absolutePath: string) => readTurns.has(readTurnKey(absolutePath));

  registerReadTool(pi, { onSuccessfulRead: noteRead });
  registerEditTool(pi, { wasReadInSession });
  const sgAvailable = isSgAvailable();
  const searchGuideline = sgAvailable
    ? "Use grep summary for counts; use search for structural code patterns."
    : "Use grep summary for counts; install @jarkkojs/readseek to enable search.";

  registerGrepTool(pi, { searchGuideline, onFileAnchored: noteRead });
  registerSgTool(pi, { onFileAnchored: noteRead });
  registerWriteTool(pi, { onFileAnchored: noteRead });
  registerLsTool(pi);
  registerFindTool(pi);
  registerReadseekCommand(pi);

  pi.on("tool_call", (event: any) => {
    recordToolCall(
      doomLoopState,
      event.toolName,
      event.toolCallId,
      (event.input ?? {}) as Record<string, unknown>,
    );
  });

  const expireStaleReadTurns = (report: ContextHygieneReport) => {
    if (readTurns.size === 0) return;
    for (const candidate of report.staleCandidates) {
      if (!candidate.resourceKey.startsWith("file:")) continue;
      const resourcePath = readTurnKey(candidate.resourceKey.slice("file:".length));
      const recordedEventId = readTurns.get(resourcePath);
      if (recordedEventId === undefined) continue;
      if (recordedEventId < candidate.mutationEventId) {
        readTurns.delete(resourcePath);
      }
    }
  };

  pi.on("context", (event: any): any => {
    if (!Array.isArray(event.messages)) return undefined;
    const report = contextHygieneTracker.generateReport();
    const messages = applyContextHygieneStaleContext(event.messages, report);
    expireStaleReadTurns(report);
    return { messages };
  });

  pi.on("tool_result", (event: any) => {
    const contextHygiene = contextHygieneFromDetails(event.details);
    if (contextHygiene) recordContextHygiene(contextHygieneTracker, contextHygiene, event.toolCallId);

    const doomLoop = consumeDoomLoopWarning(doomLoopState, event.toolCallId);
    if (!doomLoop || !Array.isArray(event.content)) return undefined;

    const content = [...event.content];
    const prefix = `${formatDoomLoopMessage(doomLoop)}\n\n---\n`;
    const textIndex = content.findIndex((item) => {
      const maybeText = item as { type?: unknown; text?: unknown };
      return maybeText.type === "text" && typeof maybeText.text === "string";
    });
    if (textIndex >= 0) {
      const item = content[textIndex] as { type: "text"; text: string };
      content[textIndex] = { ...item, text: `${prefix}${item.text}` };
    } else {
      content.unshift({ type: "text" as const, text: prefix });
    }
    return {
      content,
      details: event.details,
      isError: event.isError,
    };
  });
}
