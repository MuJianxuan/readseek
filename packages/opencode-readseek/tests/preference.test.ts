import { describe, expect, test } from "bun:test";

import { ReadSeekPlugin } from "../index.ts";

describe("ReadSeek tool preference", () => {
  test("adds editing guidance to the system prompt", async () => {
    const hooks = await ReadSeekPlugin({} as never);
    const output = { system: ["base prompt"] };

    await hooks["experimental.chat.system.transform"]?.({ model: {} as never }, output);

    expect(output.system[0]).toBe("base prompt");
    expect(output.system[1]).toContain("Prefer readseek_edit");
    expect(output.system[1]).toContain("Do not use built-in edit, write, or apply_patch");
  });

  test("marks ReadSeek mutation tools as preferred", async () => {
    const hooks = await ReadSeekPlugin({} as never);
    const output = { description: "Original description.", parameters: {} };

    await hooks["tool.definition"]?.({ toolID: "readseek_edit" }, output);

    expect(output.description).toBe(
      "Preferred tool for editing existing text files with verified LINE:HASH anchors. Original description.",
    );
  });

  test("does not modify unrelated tool descriptions", async () => {
    const hooks = await ReadSeekPlugin({} as never);
    const output = { description: "Original description.", parameters: {} };

    await hooks["tool.definition"]?.({ toolID: "bash" }, output);

    expect(output.description).toBe("Original description.");
  });
});
