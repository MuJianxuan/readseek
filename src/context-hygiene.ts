/** Metadata used to mask stale read/search results after this extension mutates a file. */

export const CONTEXT_HYGIENE_SCHEMA_VERSION = 1 as const;
export const DEFAULT_CONTEXT_HYGIENE_MAX_EVENTS = 1000;

export type ContextHygieneClassification = "read-context" | "search-context" | "mutation";

export type ContextHygieneResourceKind = "file" | "symbol";

export interface ContextHygieneFileResource {
  kind: "file";
  key: string;
  path: string;
}

export interface ContextHygieneSymbolResource {
  kind: "symbol";
  key: string;
  path: string;
  symbolName: string;
  symbolKind?: string;
}

export type ContextHygieneResource = ContextHygieneFileResource | ContextHygieneSymbolResource;

export interface ContextHygieneReadRehydrateInput {
  path: string;
  offset?: number | string;
  limit?: number | string;
  symbol?: string;
  map?: true;
  bundle?: "local";
}

export interface ContextHygieneGrepRehydrateInput {
  pattern: string;
  path?: string;
  glob?: string;
  literal?: true;
  ignoreCase?: true;
  context?: number | string;
  summary?: true;
  scope?: "symbol";
  scopeContext?: number | string;
}

export interface ContextHygieneSearchRehydrateInput {
  pattern: string;
  lang?: string;
  path?: string;
  cached?: true;
  others?: true;
  ignored?: true;
}

export interface ContextHygieneReadRehydrateDescriptor {
  tool: "read";
  input: ContextHygieneReadRehydrateInput;
}

export interface ContextHygieneGrepRehydrateDescriptor {
  tool: "grep";
  input: ContextHygieneGrepRehydrateInput;
}

export interface ContextHygieneSearchRehydrateDescriptor {
  tool: "search";
  input: ContextHygieneSearchRehydrateInput;
}

export type ContextHygieneRehydrateDescriptor =
  | ContextHygieneReadRehydrateDescriptor
  | ContextHygieneGrepRehydrateDescriptor
  | ContextHygieneSearchRehydrateDescriptor;

export type ContextHygieneStaleInvalidationReason = "mutation-after-read";

export interface ContextHygieneStaleRecord {
  status: "stale";
  originalTool: string;
  originalEventId?: number;
  originalResultId?: string;
  staleResourceKeys: string[];
  invalidatingMutationEventId: number;
  invalidatingMutationResultId?: string;
  reason: ContextHygieneStaleInvalidationReason;
  rehydrate?: ContextHygieneRehydrateDescriptor;
}

export interface BuildStaleContextRecordInput {
  originalTool: string;
  originalEventId?: number;
  originalResultId?: string;
  staleResourceKeys: readonly string[];
  invalidatingMutationEventId: number;
  invalidatingMutationResultId?: string;
  reason?: ContextHygieneStaleInvalidationReason;
  rehydrate?: ContextHygieneRehydrateDescriptor;
}

export function cloneContextHygieneRehydrateDescriptor(
  descriptor: ContextHygieneRehydrateDescriptor,
): ContextHygieneRehydrateDescriptor {
  switch (descriptor.tool) {
    case "read":
      return { tool: "read", input: { ...descriptor.input } };
    case "grep":
      return { tool: "grep", input: { ...descriptor.input } };
    case "search":
      return { tool: "search", input: { ...descriptor.input } };
  }
}

export function buildStaleContextRecord(input: BuildStaleContextRecordInput): ContextHygieneStaleRecord {
  const record: ContextHygieneStaleRecord = {
    status: "stale",
    originalTool: input.originalTool,
    staleResourceKeys: sortResourceKeys(new Set(input.staleResourceKeys)),
    invalidatingMutationEventId: input.invalidatingMutationEventId,
    reason: input.reason ?? "mutation-after-read",
  };
  if (input.originalEventId !== undefined) record.originalEventId = input.originalEventId;
  if (input.originalResultId) record.originalResultId = input.originalResultId;
  if (input.invalidatingMutationResultId) record.invalidatingMutationResultId = input.invalidatingMutationResultId;
  if (input.rehydrate) record.rehydrate = cloneContextHygieneRehydrateDescriptor(input.rehydrate);
  return record;
}

export function renderStaleReadPlaceholder(): string {
  return "[Stale read result — this earlier read was superseded by a later file change; nothing is wrong with read. Run read again for current content.]";
}

export function renderStaleGrepPlaceholder(): string {
  return "[Stale grep result — this earlier grep was superseded by a later file change; nothing is wrong with grep. Run grep again for current matches.]";
}

export function renderStaleSearchPlaceholder(): string {
  return "[Stale search result — this earlier search was superseded by a later file change; nothing is wrong with search. Run search again for current matches.]";
}

export function renderStaleContextPlaceholder(record: ContextHygieneStaleRecord): string {
  switch (record.originalTool) {
    case "read":
      return renderStaleReadPlaceholder();
    case "grep":
      return renderStaleGrepPlaceholder();
    case "search":
      return renderStaleSearchPlaceholder();
    default:
      return "[Stale tool context: resource content changed after this result. Re-run the original tool to refresh.]";
  }
}

export interface BuildReadRehydrateDescriptorInput {
  path: string;
  offset?: number | string;
  limit?: number | string;
  symbol?: string;
  map?: boolean;
  bundle?: "local";
}

export function buildReadRehydrateDescriptor(
  input: BuildReadRehydrateDescriptorInput,
): ContextHygieneReadRehydrateDescriptor {
  const descriptorInput: ContextHygieneReadRehydrateInput = { path: input.path };
  if (input.offset !== undefined) descriptorInput.offset = input.offset;
  if (input.limit !== undefined) descriptorInput.limit = input.limit;
  if (input.symbol !== undefined) descriptorInput.symbol = input.symbol;
  if (input.map === true) descriptorInput.map = true;
  if (input.bundle !== undefined) descriptorInput.bundle = input.bundle;
  return { tool: "read", input: descriptorInput };
}

export interface BuildGrepRehydrateDescriptorInput {
  pattern: string;
  path?: string;
  glob?: string;
  literal?: boolean;
  ignoreCase?: boolean;
  context?: number | string;
  summary?: boolean;
  scope?: "symbol";
  scopeContext?: number | string;
}

export function buildGrepRehydrateDescriptor(
  input: BuildGrepRehydrateDescriptorInput,
): ContextHygieneGrepRehydrateDescriptor {
  const descriptorInput: ContextHygieneGrepRehydrateInput = { pattern: input.pattern };
  if (input.path !== undefined) descriptorInput.path = input.path;
  if (input.glob !== undefined) descriptorInput.glob = input.glob;
  if (input.literal === true) descriptorInput.literal = true;
  if (input.ignoreCase === true) descriptorInput.ignoreCase = true;
  if (input.context !== undefined) descriptorInput.context = input.context;
  if (input.summary === true) descriptorInput.summary = true;
  if (input.scope !== undefined) descriptorInput.scope = input.scope;
  if (input.scopeContext !== undefined) descriptorInput.scopeContext = input.scopeContext;
  return { tool: "grep", input: descriptorInput };
}

export interface BuildSearchRehydrateDescriptorInput {
  pattern: string;
  lang?: string;
  path?: string;
  cached?: boolean;
  others?: boolean;
  ignored?: boolean;
}

export function buildSearchRehydrateDescriptor(
  input: BuildSearchRehydrateDescriptorInput,
): ContextHygieneSearchRehydrateDescriptor {
  const descriptorInput: ContextHygieneSearchRehydrateInput = { pattern: input.pattern };
  if (input.lang !== undefined) descriptorInput.lang = input.lang;
  if (input.path !== undefined) descriptorInput.path = input.path;
  if (input.cached === true) descriptorInput.cached = true;
  if (input.others === true) descriptorInput.others = true;
  if (input.ignored === true) descriptorInput.ignored = true;
  return { tool: "search", input: descriptorInput };
}

export interface ContextHygieneMetadata {
  schemaVersion: typeof CONTEXT_HYGIENE_SCHEMA_VERSION;
  tool: string;
  classification: ContextHygieneClassification;
  resources: ContextHygieneResource[];
  rehydrate?: ContextHygieneRehydrateDescriptor;
}

export interface BuildContextHygieneMetadataInput {
  tool: string;
  classification: ContextHygieneClassification;
  resources?: readonly (ContextHygieneResource | null | undefined)[];
  rehydrate?: ContextHygieneRehydrateDescriptor | null;
}

export function normalizePathForContextHygiene(path: string): string {
  if (path === "") return "";

  const slashPath = path.replace(/\\+/g, "/");
  const isAbsolute = slashPath.startsWith("/");
  const parts: string[] = [];

  for (const part of slashPath.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length > 0 && parts[parts.length - 1] !== "..") {
        parts.pop();
      } else if (!isAbsolute) {
        parts.push(part);
      }
      continue;
    }
    parts.push(part);
  }

  const normalized = `${isAbsolute ? "/" : ""}${parts.join("/")}`;
  return normalized || (isAbsolute ? "/" : ".");
}

export function buildFileResource(path: string): ContextHygieneFileResource {
  const normalizedPath = normalizePathForContextHygiene(path);
  return {
    kind: "file",
    key: `file:${normalizedPath}`,
    path: normalizedPath,
  };
}

export function buildSymbolResource(
  path: string,
  symbolName: string,
  symbolKind?: string,
): ContextHygieneSymbolResource {
  const normalizedPath = normalizePathForContextHygiene(path);
  const normalizedKind = symbolKind?.trim();
  const keyPayload = JSON.stringify([normalizedPath, normalizedKind ?? "", symbolName]);
  const resource: ContextHygieneSymbolResource = {
    kind: "symbol",
    key: `symbol:${keyPayload}`,
    path: normalizedPath,
    symbolName,
  };
  if (normalizedKind) resource.symbolKind = normalizedKind;
  return resource;
}

export function buildContextHygieneMetadata(
  input: BuildContextHygieneMetadataInput,
): ContextHygieneMetadata {
  const resources: ContextHygieneResource[] = [];
  const seenResourceKeys = new Set<string>();

  for (const resource of input.resources ?? []) {
    if (!resource || seenResourceKeys.has(resource.key)) continue;
    seenResourceKeys.add(resource.key);
    resources.push({ ...resource } as ContextHygieneResource);
  }

  const metadata: ContextHygieneMetadata = {
    schemaVersion: CONTEXT_HYGIENE_SCHEMA_VERSION,
    tool: input.tool,
    classification: input.classification,
    resources,
  };
  if (input.rehydrate) metadata.rehydrate = cloneContextHygieneRehydrateDescriptor(input.rehydrate);
  return metadata;
}

export interface ContextHygieneRecordOptions {
  resultId?: string;
}

export interface ContextHygieneEvent {
  id: number;
  resultId?: string;
  tool: string;
  classification: ContextHygieneClassification;
  resources: ContextHygieneResource[];
  rehydrate?: ContextHygieneRehydrateDescriptor;
}

export interface ContextHygieneReuseReportEntry {
  resourceKey: string;
  count: number;
  eventIds: number[];
  resultIds: string[];
}

export interface ContextHygieneMutationAfterReadReportEntry {
  resourceKey: string;
  readEventIds: number[];
  mutationEventId: number;
}

export interface ContextHygieneStaleCandidateReportEntry {
  resourceKey: string;
  staleEventIds: number[];
  mutationEventId: number;
  reason: ContextHygieneStaleInvalidationReason;
  staleResults: ContextHygieneStaleRecord[];
}

export interface ContextHygieneReport {
  eventCount: number;
  resourceCount: number;
  readReuse: ContextHygieneReuseReportEntry[];
  mutationAfterRead: ContextHygieneMutationAfterReadReportEntry[];
  staleCandidates: ContextHygieneStaleCandidateReportEntry[];
  churn: {
    byClassification: Record<ContextHygieneClassification, number>;
    byTool: Record<string, number>;
    uniqueResourcesSeen: number;
  };
}

export interface ContextHygieneTracker {
  record(metadata: ContextHygieneMetadata, options?: ContextHygieneRecordOptions): ContextHygieneEvent;
  generateReport(): ContextHygieneReport;
}

export interface CreateContextHygieneTrackerOptions {
  maxEvents?: number;
}

function resultIdsForEvents(events: ContextHygieneEvent[]): string[] {
  return events.map((event) => event.resultId).filter((resultId): resultId is string => Boolean(resultId));
}

function cloneContextHygieneEvent(event: ContextHygieneEvent): ContextHygieneEvent {
  const cloned: ContextHygieneEvent = {
    ...event,
    resources: event.resources.map((resource) => ({ ...resource } as ContextHygieneResource)),
  };
  if (event.rehydrate) cloned.rehydrate = cloneContextHygieneRehydrateDescriptor(event.rehydrate);
  return cloned;
}

function compareStable(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function sortResourceKeys(keys: Iterable<string>): string[] {
  return [...keys].sort(compareStable);
}

function createEmptyClassificationCounts(): Record<ContextHygieneClassification, number> {
  return {
    mutation: 0,
    "read-context": 0,
    "search-context": 0,
  };
}

class DefaultContextHygieneTracker implements ContextHygieneTracker {
  private readonly events: ContextHygieneEvent[] = [];
  private readonly maxEvents: number;
  private nextEventId = 1;

  constructor(options: CreateContextHygieneTrackerOptions = {}) {
    this.maxEvents = Math.max(1, Math.floor(options.maxEvents ?? DEFAULT_CONTEXT_HYGIENE_MAX_EVENTS));
  }

  record(metadata: ContextHygieneMetadata, options: ContextHygieneRecordOptions = {}): ContextHygieneEvent {
    const event: ContextHygieneEvent = {
      id: this.nextEventId++,
      tool: metadata.tool,
      classification: metadata.classification,
      resources: metadata.resources.map((resource) => ({ ...resource } as ContextHygieneResource)),
    };
    if (options.resultId) event.resultId = options.resultId;
    if (metadata.rehydrate) event.rehydrate = cloneContextHygieneRehydrateDescriptor(metadata.rehydrate);
    this.events.push(event);
    if (this.events.length > this.maxEvents) this.events.splice(0, this.events.length - this.maxEvents);
    return cloneContextHygieneEvent(event);
  }

  generateReport(): ContextHygieneReport {
    const eventsByResource = new Map<string, ContextHygieneEvent[]>();
    const readEventsByResource = new Map<string, ContextHygieneEvent[]>();
    const mutationEventsByResource = new Map<string, ContextHygieneEvent[]>();
    const byClassification = createEmptyClassificationCounts();
    const byTool: Record<string, number> = {};

    for (const event of this.events) {
      byClassification[event.classification] += 1;
      byTool[event.tool] = (byTool[event.tool] ?? 0) + 1;

      for (const resource of event.resources) {
        const bucket = eventsByResource.get(resource.key) ?? [];
        bucket.push(event);
        eventsByResource.set(resource.key, bucket);

        if (event.classification === "read-context" || event.classification === "search-context") {
          const readBucket = readEventsByResource.get(resource.key) ?? [];
          readBucket.push(event);
          readEventsByResource.set(resource.key, readBucket);
        }
        if (event.classification === "mutation") {
          const mutationBucket = mutationEventsByResource.get(resource.key) ?? [];
          mutationBucket.push(event);
          mutationEventsByResource.set(resource.key, mutationBucket);
        }
      }
    }

    const readReuse = sortResourceKeys(readEventsByResource.keys()).flatMap((resourceKey) => {
      const events = readEventsByResource.get(resourceKey) ?? [];
      if (events.length < 2) return [];
      return [{ resourceKey, count: events.length, eventIds: events.map((event) => event.id), resultIds: resultIdsForEvents(events) }];
    });

    const mutationAfterRead: ContextHygieneMutationAfterReadReportEntry[] = [];
    const staleCandidates: ContextHygieneStaleCandidateReportEntry[] = [];

    for (const resourceKey of sortResourceKeys(mutationEventsByResource.keys())) {
      const reads = readEventsByResource.get(resourceKey) ?? [];
      const mutations = mutationEventsByResource.get(resourceKey) ?? [];
      for (const mutation of mutations) {
        const priorReads = reads.filter((read) => read.id < mutation.id);
        const priorReadIds = priorReads.map((read) => read.id);
        if (priorReadIds.length === 0) continue;
        mutationAfterRead.push({ resourceKey, readEventIds: priorReadIds, mutationEventId: mutation.id });
        staleCandidates.push({
          resourceKey,
          staleEventIds: priorReadIds,
          mutationEventId: mutation.id,
          reason: "mutation-after-read",
          staleResults: priorReads.map((read) => buildStaleContextRecord({
            originalTool: read.tool,
            originalEventId: read.id,
            originalResultId: read.resultId,
            staleResourceKeys: [resourceKey],
            invalidatingMutationEventId: mutation.id,
            invalidatingMutationResultId: mutation.resultId,
            reason: "mutation-after-read",
            rehydrate: read.rehydrate,
          })),
        });
      }
    }

    return {
      eventCount: this.events.length,
      resourceCount: eventsByResource.size,
      readReuse,
      mutationAfterRead,
      staleCandidates,
      churn: {
        byClassification,
        byTool: Object.fromEntries(Object.entries(byTool).sort(([left], [right]) => compareStable(left, right))),
        uniqueResourcesSeen: eventsByResource.size,
      },
    };
  }
}

export function createContextHygieneTracker(options: CreateContextHygieneTrackerOptions = {}): ContextHygieneTracker {
  return new DefaultContextHygieneTracker(options);
}

let globalContextHygieneTracker = createContextHygieneTracker();

export function resetContextHygieneTracker(options: CreateContextHygieneTrackerOptions = {}): ContextHygieneTracker {
  globalContextHygieneTracker = createContextHygieneTracker(options);
  return globalContextHygieneTracker;
}

export function getContextHygieneTracker(): ContextHygieneTracker {
  return globalContextHygieneTracker;
}
