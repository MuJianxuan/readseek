import { describe, expect, test } from "bun:test";

describe("package manifest", () => {
  test("exposes the OpenCode server entrypoint", async () => {
    const manifest = (await Bun.file(new URL("../package.json", import.meta.url)).json()) as {
      exports?: Record<string, unknown>;
    };

    expect(manifest.exports?.["./server"]).toBe("./index.ts");
  });
});
