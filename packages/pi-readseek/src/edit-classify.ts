import * as Diff from "diff";

export type EditClassification = "no-op" | "whitespace-only" | "semantic" | "mixed";

export interface EditClassifyResult {
  classification: EditClassification;
}

export function classifyEdit(oldContent: string, newContent: string): EditClassifyResult {
  if (oldContent === newContent) {
    return { classification: "no-op" };
  }

  const oldLines = oldContent.split("\n");
  const newLines = newContent.split("\n");

  // When line counts match, do pairwise comparison
  if (oldLines.length === newLines.length) {
    let hasWhitespaceChange = false;
    let hasSemanticChange = false;
    for (let i = 0; i < oldLines.length; i++) {
      if (oldLines[i] === newLines[i]) continue;
      if (oldLines[i].trim() === newLines[i].trim()) {
        hasWhitespaceChange = true;
      } else {
        hasSemanticChange = true;
      }
    }
    if (hasSemanticChange && hasWhitespaceChange) return { classification: "mixed" };
    if (hasWhitespaceChange) return { classification: "whitespace-only" };
    return { classification: "semantic" };
  }

  // When line counts differ, use diff library to find changes
  const parts = Diff.diffLines(oldContent, newContent);
  let hasWhitespaceChange = false;
  let hasSemanticChange = false;

  for (const part of parts) {
    if (!part.added && !part.removed) continue;

    const lines = part.value.split("\n");
    if (lines[lines.length - 1] === "") lines.pop();

    for (const line of lines) {
      if (line.trim() === "") {
        hasWhitespaceChange = true;
      } else {
        hasSemanticChange = true;
      }
    }
  }

  if (hasSemanticChange && hasWhitespaceChange) return { classification: "mixed" };
  if (hasWhitespaceChange) return { classification: "whitespace-only" };
  return { classification: "semantic" };
}

