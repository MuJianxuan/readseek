import { describe, expect, it } from "vitest";

import { applyHashlineEdits, computeLineHash, ensureHashInit, parseLineRef } from "../src/hashline.js";

describe("parseLineRef", () => {
	it("rejects unsafe anchor line numbers", () => {
		expect(() => parseLineRef("9007199254740992:abc")).toThrow(/safe integer/);
	});

	it("strips the >> match marker from search/refs anchors", () => {
		expect(parseLineRef(">>78:f0e")).toMatchObject({ line: 78, hash: "f0e" });
	});

	it("strips the >>> mismatch-hint marker and keeps content", () => {
		expect(parseLineRef(">>> 41:b34|  const renamed = 3;")).toMatchObject({
			line: 41,
			hash: "b34",
			content: "  const renamed = 3;",
		});
	});

	it("strips leading indentation from context lines", () => {
		expect(parseLineRef("    78:f0e")).toMatchObject({ line: 78, hash: "f0e" });
	});

	it("still accepts a bare anchor", () => {
		expect(parseLineRef("78:f0e")).toMatchObject({ line: 78, hash: "f0e" });
	});

	it("ignores a trailing newline after the anchor", () => {
		expect(parseLineRef("76:4c9|#[derive(Clone, Copy)]\n")).toMatchObject({
			line: 76,
			hash: "4c9",
			content: "#[derive(Clone, Copy)]",
		});
	});

	it("uses only the first line of a multi-line paste", () => {
		expect(parseLineRef("76:4c9|#[derive(Clone, Copy)]\n77:abc|pub struct Foo")).toMatchObject({
			line: 76,
			hash: "4c9",
			content: "#[derive(Clone, Copy)]",
		});
	});
});

describe("applyHashlineEdits new_text prefix stripping", () => {
	it("strips >> / >>> / indentation gutters copied into new_text", async () => {
		await ensureHashInit();
		const content = "alpha\nbeta\ngamma";
		const anchor = `2:${computeLineHash("beta")}|beta`;
		const result = applyHashlineEdits(content, [
			{ set_line: { anchor, new_text: ">>10:abc|first\n>>> 11:def|second\n    12:f0e|third" } },
		]);
		expect(result.content).toBe("alpha\nfirst\nsecond\nthird\ngamma");
	});

	it("leaves plain replacement text untouched", async () => {
		await ensureHashInit();
		const content = "alpha\nbeta\ngamma";
		const anchor = `2:${computeLineHash("beta")}|beta`;
		const result = applyHashlineEdits(content, [
			{ set_line: { anchor, new_text: "first()\nsecond()" } },
		]);
		expect(result.content).toBe("alpha\nfirst()\nsecond()\ngamma");
	});
});
