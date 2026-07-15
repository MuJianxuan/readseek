import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { executeEdit } from "../src/edit.js";
import { computeLineHash, ensureHashInit } from "../src/hashline.js";

describe("executeEdit anchor invalidation", () => {
  let cwd: string | undefined;

  afterEach(async () => {
    if (cwd) await rm(cwd, { recursive: true, force: true });
  });

  it("invalidates anchors after a successful write", async () => {
    cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-edit-"));
    const filePath = path.join(cwd, "target.ts");
    await writeFile(filePath, "const value = 1;\n", "utf-8");
    await ensureHashInit();
    const onFileMutated = vi.fn();

    const result = await executeEdit({
      params: {
        path: "target.ts",
        edits: [{ set_line: { anchor: `1:${computeLineHash("const value = 1;")}`, new_text: "const value = 2;" } }],
      },
      signal: undefined,
      cwd,
      wasReadInSession: () => true,
      onFileMutated,
      syntaxValidate: "off",
    });

    expect(result.isError).not.toBe(true);
    expect(await readFile(filePath, "utf-8")).toBe("const value = 2;\n");
    expect(onFileMutated).toHaveBeenCalledOnce();
    expect(onFileMutated).toHaveBeenCalledWith(filePath);
  });
});
