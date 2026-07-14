import { describe, expect, it } from "vitest";

import { summaryLine, wrapReadHashlinesForWidth, wrapReadHashlinesForWidthCached } from "../src/tui-render-utils.js";

describe("summaryLine", () => {
	it("applies semantic color only to the existing marker", () => {
		const theme = {
			fg: (style: string, text: string) => `<${style}>${text}</${style}>`,
			bold: (text: string) => text,
		};

		expect(summaryLine("3 matches", { theme, style: "success" })).toBe("<success>↳</success> 3 matches");
	});
});

describe("wrapReadHashlinesForWidthCached", () => {
	it("matches the uncached wrap across repeated calls", () => {
		const key = {};
		const text = `1:abcd|${"x".repeat(200)}\nplain line`;
		const expected = wrapReadHashlinesForWidth(text, 80);
		expect(wrapReadHashlinesForWidthCached(key, text, 80)).toBe(expected);
		expect(wrapReadHashlinesForWidthCached(key, text, 80)).toBe(expected);
	});

	it("recomputes when width or text changes", () => {
		const key = {};
		const text = `1:abcd|${"x".repeat(200)}`;
		expect(wrapReadHashlinesForWidthCached(key, text, 80)).toBe(wrapReadHashlinesForWidth(text, 80));
		expect(wrapReadHashlinesForWidthCached(key, text, 40)).toBe(wrapReadHashlinesForWidth(text, 40));
		const changed = `${text}\n2:ef01|changed`;
		expect(wrapReadHashlinesForWidthCached(key, changed, 40)).toBe(wrapReadHashlinesForWidth(changed, 40));
	});
});
