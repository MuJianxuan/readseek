import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { homeDir } = vi.hoisted(() => ({
	homeDir: { value: "" },
}));

vi.mock("node:os", async (importOriginal) => {
	const actual = await importOriginal<typeof import("node:os")>();
	return {
		...actual,
		homedir: () => homeDir.value,
	};
});

const {
	resolveReadSeekExcludeTools,
	resolveReadSeekJsonSettings,
	resolveReadSeekOcrMode,
	resolveReadSeekSyntaxValidation,
} = await import("../src/readseek-settings.js");

describe("readseek settings", () => {
	let tempHome: string;
	let tempCwd: string;
	let previousCwd: string;

	beforeEach(async () => {
		previousCwd = process.cwd();
		tempHome = await mkdtemp(path.join(tmpdir(), "pi-readseek-home-"));
		tempCwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-cwd-"));
		homeDir.value = tempHome;
		process.chdir(tempCwd);
	});

	afterEach(async () => {
		process.chdir(previousCwd);
		await rm(tempHome, { recursive: true, force: true });
		await rm(tempCwd, { recursive: true, force: true });
	});

	async function writeGlobal(settings: unknown) {
		const dir = path.join(tempHome, ".pi", "agent", "readseek");
		await mkdir(dir, { recursive: true });
		await writeFile(path.join(dir, "settings.json"), JSON.stringify(settings));
	}

	async function writeProject(settings: unknown) {
		const dir = path.join(tempCwd, ".pi", "readseek");
		await mkdir(dir, { recursive: true });
		await writeFile(path.join(dir, "settings.json"), JSON.stringify(settings));
	}

	it("defaults ocrMode to force", () => {
		expect(resolveReadSeekOcrMode()).toBe("force");
	});

	it("reads ocrMode from global settings", async () => {
		await writeGlobal({ readseek: { ocrMode: "auto" } });
		expect(resolveReadSeekOcrMode()).toBe("auto");
	});

	it("accepts on as an ocrMode alias for force", async () => {
		await writeGlobal({ readseek: { ocrMode: "on" } });
		expect(resolveReadSeekOcrMode()).toBe("force");
	});

	it("lets project settings override global", async () => {
		await writeGlobal({ readseek: { ocrMode: "auto" } });
		await writeProject({ readseek: { ocrMode: "off" } });
		expect(resolveReadSeekOcrMode()).toBe("off");
	});

	it("warns on invalid ocrMode and falls back to force", async () => {
		await writeGlobal({ readseek: { ocrMode: "maybe" } });
		const { settings, warnings } = resolveReadSeekJsonSettings();
		expect(settings.ocrMode).toBeUndefined();
		expect(warnings).toHaveLength(1);
		expect(warnings[0]?.path).toBe("readseek.ocrMode");
		expect(resolveReadSeekOcrMode()).toBe("force");
	});

	it("merges nested grep settings", async () => {
		await writeGlobal({ readseek: { grep: { maxLines: 50, maxBytes: 1000 } } });
		await writeProject({ readseek: { grep: { maxLines: 25 } } });
		expect(resolveReadSeekJsonSettings().settings.grep).toEqual({ maxLines: 25, maxBytes: 1000 });
	});

	it("reads excludeTools and syntaxValidation", async () => {
		await writeGlobal({ readseek: { excludeTools: ["read", "edit"], syntaxValidation: "block" } });
		expect(resolveReadSeekExcludeTools()).toEqual(["read", "edit"]);
		expect(resolveReadSeekSyntaxValidation()).toBe("block");
	});

	it("warns on invalid excludeTools", async () => {
		await writeGlobal({ readseek: { excludeTools: ["read", ""] } });
		const { warnings } = resolveReadSeekJsonSettings();
		expect(warnings).toHaveLength(1);
		expect(warnings[0]?.path).toBe("readseek.excludeTools");
		expect(resolveReadSeekExcludeTools()).toEqual([]);
	});

	it("picks up settings changes and deletions despite caching", async () => {
		await writeGlobal({ readseek: { ocrMode: "auto" } });
		expect(resolveReadSeekOcrMode()).toBe("auto");
		expect(resolveReadSeekOcrMode()).toBe("auto");

		await writeGlobal({ readseek: { ocrMode: "off" } });
		expect(resolveReadSeekOcrMode()).toBe("off");

		await rm(path.join(tempHome, ".pi", "agent", "readseek", "settings.json"));
		expect(resolveReadSeekOcrMode()).toBe("force");
	});
});
