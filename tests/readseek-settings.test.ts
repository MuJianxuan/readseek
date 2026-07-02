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

const { resolveReadSeekJsonSettings, resolveReadSeekOcrMode } = await import("../src/readseek-settings.js");

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

	it("defaults ocrMode to on", () => {
		expect(resolveReadSeekOcrMode()).toBe("on");
	});

	it("reads ocrMode from global settings", async () => {
		await writeGlobal({ read: { ocrMode: "auto" } });
		expect(resolveReadSeekOcrMode()).toBe("auto");
	});

	it("lets project settings override global", async () => {
		await writeGlobal({ read: { ocrMode: "auto" } });
		await writeProject({ read: { ocrMode: "off" } });
		expect(resolveReadSeekOcrMode()).toBe("off");
	});

	it("warns on invalid ocrMode and falls back to on", async () => {
		await writeGlobal({ read: { ocrMode: "maybe" } });
		const { settings, warnings } = resolveReadSeekJsonSettings();
		expect(settings.read?.ocrMode).toBeUndefined();
		expect(warnings).toHaveLength(1);
		expect(warnings[0]?.path).toBe("read.ocrMode");
		expect(resolveReadSeekOcrMode()).toBe("on");
	});

	it("lets env var override settings", async () => {
		await writeGlobal({ read: { ocrMode: "off" } });
		expect(resolveReadSeekOcrMode({ READSEEK_READ_OCR_MODE: "auto" })).toBe("auto");
	});
});
