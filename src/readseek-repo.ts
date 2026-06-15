import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

export function findReadseekDir(cwd: string): string | null {
	const abs = resolve(cwd);
	for (let dir = abs; ; dir = dirname(dir)) {
		const candidate = join(dir, ".readseek");
		if (existsSync(candidate)) return candidate;
		const parent = dirname(dir);
		if (parent === dir) break;
	}
	return null;
}

export function hasReadseekDir(cwd: string): boolean {
	return findReadseekDir(cwd) !== null;
}

export function initReadseekDir(cwd: string): string {
	const abs = resolve(cwd);
	const readseekDir = join(abs, ".readseek");
	const mapsDir = join(readseekDir, "maps");

	if (existsSync(readseekDir)) {
		throw new Error(`.readseek/ already exists in ${abs}`);
	}

	mkdirSync(mapsDir, { recursive: true });

	const gitignore = join(abs, ".gitignore");
	const entry = "/.readseek";
	const needsAppend = (() => {
		if (!existsSync(gitignore)) return true;
		const contents = readFileSync(gitignore, "utf-8");
		return !contents.split("\n").some((line) => line.trim() === entry);
	})();

	if (needsAppend) {
		let prefix = "";
		if (existsSync(gitignore)) {
			const contents = readFileSync(gitignore, "utf-8");
			if (contents.length > 0 && !contents.endsWith("\n")) {
				prefix = "\n";
			}
		}
		appendFileSync(gitignore, `${prefix}${entry}\n`, "utf-8");
	}

	return readseekDir;
}
