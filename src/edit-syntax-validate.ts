import type { Node as SyntaxNode, Parser as WasmParser, Tree } from "web-tree-sitter";
import { getWasmParser, type WasmLanguageId } from "./readseek/parser-loader.js";
import { reportParserError } from "./readseek/parser-errors.js";

export interface ValidateInput {
  filePath: string;
  before: string | undefined;
  after: string;
}

export interface ValidateResult {
  errorLines: string[];
  newErrorCount: number;
  newMissingCount: number;
}

interface NodeStats {
  errors: Array<{ startLine: number; endLine: number }>;
  missing: Array<{ startLine: number; endLine: number }>;
}

function countNodes(parser: WasmParser, source: string): NodeStats {
  const tree: Tree | null = parser.parse(source);
  const errors: Array<{ startLine: number; endLine: number }> = [];
  const missing: Array<{ startLine: number; endLine: number }> = [];
  if (!tree) return { errors, missing };
  try {
    const stack: SyntaxNode[] = [tree.rootNode];
    while (stack.length > 0) {
      const node = stack.pop()!;
      if (node.type === "ERROR") {
        errors.push({
          startLine: node.startPosition.row + 1,
          endLine: node.endPosition.row + 1,
        });
      }
      if (node.isMissing) {
        missing.push({
          startLine: node.startPosition.row + 1,
          endLine: node.endPosition.row + 1,
        });
      }
      for (let i = 0; i < node.namedChildCount; i++) {
        const c = node.namedChild(i);
        if (c) stack.push(c);
      }
      // Also descend into anonymous children to find MISSING tokens.
      for (let i = 0; i < node.childCount; i++) {
        const c = node.child(i);
        if (c && !c.isNamed) stack.push(c);
      }
    }
    return { errors, missing };
  } finally {
    tree.delete();
  }
}

function dedupeSortLines(
  ranges: Array<{ startLine: number; endLine: number }>,
): string[] {
  const seen = new Set<string>();
  const out: Array<{ key: string; start: number }> = [];
  for (const r of ranges) {
    const key = r.startLine === r.endLine
      ? String(r.startLine)
      : `${r.startLine}-${r.endLine}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push({ key, start: r.startLine });
    }
  }
  out.sort((a, b) => a.start - b.start);
  return out.map((o) => o.key);
}

const EXTENSION_TO_LANGUAGE: Record<string, WasmLanguageId> = {
	".rs": "rust",
	".c": "cpp",
	".cc": "cpp",
	".cpp": "cpp",
	".cxx": "cpp",
	".h": "c-header",
	".hpp": "c-header",
	".hxx": "c-header",
	".java": "java",
};

function detectWasmLang(filePath: string): WasmLanguageId | null {
	for (const [ext, langId] of Object.entries(EXTENSION_TO_LANGUAGE)) {
		if (filePath.toLowerCase().endsWith(ext)) return langId;
	}
	return null;
}

export async function validateSyntaxRegression(
	input: ValidateInput,
): Promise<ValidateResult | null> {
	const langId = detectWasmLang(input.filePath);
	if (!langId) return null;
	const parser = await getWasmParser(langId);
  if (!parser) return null;

  try {
    const beforeStats = input.before === undefined
      ? { errors: [], missing: [] }
      : countNodes(parser, input.before);
    const afterStats = countNodes(parser, input.after);

    // ±1 tolerance on ERROR count, no tolerance on MISSING.
    const newErrorCount = Math.max(
      0,
      afterStats.errors.length - beforeStats.errors.length - 1,
    );
    const newMissingCount = Math.max(
      0,
      afterStats.missing.length - beforeStats.missing.length,
    );

    if (newErrorCount === 0 && newMissingCount === 0) return null;

    const errorLines = dedupeSortLines([
      ...afterStats.errors,
      ...afterStats.missing,
    ]);
    return { errorLines, newErrorCount, newMissingCount };
	} catch (err) {
		reportParserError(`wasm:syntax-validate:${langId}:${err instanceof Error ? err.message : String(err)}`, err, {
			context: `tree-sitter syntax validation failed for ${langId}`,
		});
		return null;
	} finally {
		parser.delete();
  }
}
