import { fileURLToPath } from "node:url";

import { expect, it } from "vitest";

import { readSeekRead } from "../src/readseek-client.js";

const fixture = fileURLToPath(new URL("./fixtures/readseek-e2e.ts", import.meta.url));

it("runs the installed readseek binary", async () => {
	const result = await readSeekRead(fixture);

	expect(result.language).toBe("typescript");
	expect(result.line_count).toBe(3);
	expect(result.hashlines.map((line) => line.text)).toEqual([
		"export function greet(name: string): string {",
		"\treturn `hello ${name}`;",
		"}",
	]);
});
