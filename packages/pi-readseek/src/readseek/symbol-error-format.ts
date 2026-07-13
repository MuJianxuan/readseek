import type { FileMap } from "./types.js";
import type { SymbolMatch } from "./symbol-lookup.js";

export function formatAmbiguous(query: string, candidates: SymbolMatch[]): string {
	const lines = candidates.map((c) => `- ${c.name} (${c.kind}) — lines ${c.startLine}-${c.endLine}`);
	const hints = candidates.map((c) => `${query}@${c.startLine}`).join(" or ");
	return [
		`Symbol '${query}' is ambiguous.`,
		"Matches:",
		...lines,
		`Use ${hints} to select by start line.`,
	].join("\n");
}

export function formatNotFound(query: string, map: FileMap): string {
	const available = map.symbols.slice(0, 20).map((s) => s.name).join(", ");
	return `[Warning: symbol '${query}' not found. Available symbols: ${available}]`;
}
