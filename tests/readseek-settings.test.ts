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
	resolveReadSeekJsonSettings,
	resolveReadSeekImageMode,
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
		await mkdir(path.join(tempHome, ".pi", "agent"), { recursive: true });
		await writeFile(path.join(tempHome, ".pi", "agent", "settings.json"), JSON.stringify(settings));
	}

	async function writeProject(settings: unknown) {
		await mkdir(path.join(tempCwd, ".pi"), { recursive: true });
		await writeFile(path.join(tempCwd, ".pi", "settings.json"), JSON.stringify(settings));
	}

	it("defaults imageMode to force", () => {
		expect(resolveReadSeekImageMode()).toBe("force");
	});

	it("reads imageMode from global settings", async () => {
		await writeGlobal({ readseek: { imageMode: "auto" } });
		expect(resolveReadSeekImageMode()).toBe("auto");
	});

	it("accepts on as an imageMode alias for force", async () => {
		await writeGlobal({ readseek: { imageMode: "on" } });
		expect(resolveReadSeekImageMode()).toBe("force");
	});

	it("lets project settings override global", async () => {
		await writeGlobal({ readseek: { imageMode: "auto" } });
		await writeProject({ readseek: { imageMode: "off" } });
		expect(resolveReadSeekImageMode()).toBe("off");
	});

	it("warns on invalid imageMode and falls back to force", async () => {
		await writeGlobal({ readseek: { imageMode: "maybe" } });
		const { settings, warnings } = resolveReadSeekJsonSettings();
		expect(settings.imageMode).toBeUndefined();
		expect(warnings).toHaveLength(1);
		expect(warnings[0]?.path).toBe("readseek.imageMode");
		expect(resolveReadSeekImageMode()).toBe("force");
	});

	it("merges nested grep settings", async () => {
		await writeGlobal({ readseek: { grep: { maxLines: 50, maxBytes: 1000 } } });
		await writeProject({ readseek: { grep: { maxLines: 25 } } });
		expect(resolveReadSeekJsonSettings().settings.grep).toEqual({ maxLines: 25, maxBytes: 1000 });
	});

	it("reads excludeTools and syntaxValidation", async () => {
		await writeGlobal({ readseek: { excludeTools: ["read", "edit"], syntaxValidation: "block" } });
		expect(resolveReadSeekJsonSettings().settings.excludeTools).toEqual(["read", "edit"]);
		expect(resolveReadSeekSyntaxValidation()).toBe("block");
	});

	it("keeps the valid excludeTools entries and warns about the rest", async () => {
		await writeGlobal({ readseek: { excludeTools: ["read", ""] } });
		const { settings, warnings } = resolveReadSeekJsonSettings();
		expect(warnings).toHaveLength(1);
		expect(warnings[0]?.path).toBe("readseek.excludeTools[1]");
		expect(settings.excludeTools).toEqual(["read"]);
	});

	it("warns on unknown keys in the readseek section", async () => {
		await writeGlobal({ readseek: { imagemode: "off", grep: { maxlines: 10 } } });
		const { warnings } = resolveReadSeekJsonSettings();
		expect(warnings.map((warning) => warning.path)).toEqual(["readseek.imagemode", "readseek.grep.maxlines"]);
	});

	it("warns when settings are not nested under a readseek section", async () => {
		await writeGlobal({ read: { imageMode: "off" } });
		const { settings, warnings } = resolveReadSeekJsonSettings();
		expect(warnings).toHaveLength(1);
		expect(warnings[0]?.path).toBe("readseek");
		expect(settings.imageMode).toBeUndefined();
	});

	it("picks up settings changes and deletions despite caching", async () => {
		await writeGlobal({ readseek: { imageMode: "auto" } });
		expect(resolveReadSeekImageMode()).toBe("auto");
		expect(resolveReadSeekImageMode()).toBe("auto");

		await writeGlobal({ readseek: { imageMode: "off" } });
		expect(resolveReadSeekImageMode()).toBe("off");

		await rm(path.join(tempHome, ".pi", "agent", "settings.json"));
		expect(resolveReadSeekImageMode()).toBe("force");
	});
});
