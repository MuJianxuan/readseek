import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Key, matchesKey, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { findReadseekDir, initReadseekDir } from "./readseek-repo.js";

type ReadseekAction = "init" | "deinit" | null;

function stripFromGitignore(dir: string): void {
	const gitignorePath = join(dir, ".gitignore");
	if (!existsSync(gitignorePath)) return;
	const lines = readFileSync(gitignorePath, "utf-8").split("\n");
	const filtered = lines.filter((line) => line.trim() !== "/.readseek");
	writeFileSync(gitignorePath, filtered.join("\n"), "utf-8");
}

async function deinit(ctx: ExtensionContext): Promise<void> {
	const dir = findReadseekDir(ctx.cwd);
	if (!dir) {
		ctx.ui.notify("No .readseek directory found", "info");
		return;
	}
	const projectDir = dirname(dir);
	rmSync(dir, { recursive: true, force: true });
	stripFromGitignore(projectDir);
	ctx.ui.notify("Removed .readseek/", "info");
}

async function init(ctx: ExtensionContext): Promise<void> {
	const projectDir = ctx.cwd;
	if (findReadseekDir(projectDir)) {
		ctx.ui.notify(".readseek/ already exists", "info");
		return;
	}
	initReadseekDir(projectDir);
	ctx.ui.notify("Initialized .readseek/", "info");
}

export function registerReadseekCommand(pi: ExtensionAPI): void {
	const maybePi = pi as ExtensionAPI & {
		registerCommand?: ExtensionAPI["registerCommand"];
	};

	maybePi.registerCommand?.("readseek", {
		description: "Manage .readseek/ map cache",
		handler: async (_args, ctx) => {
			if (!ctx.hasUI) return;

			const action = await new Promise<ReadseekAction>((resolve) => {
				const initialized = findReadseekDir(ctx.cwd) !== null;

				void ctx.ui.custom<ReadseekAction>(
					(tui, theme, _kb, done) => {
						return {
							render(width: number): string[] {
								const innerW = Math.max(1, width - 4);
								const border = theme.fg("border", "│");
								const dim = (s: string) => theme.fg("dim", s);
								const accent = (s: string) => theme.fg("accent", s);
								const borderFg = (s: string) => theme.fg("border", s);

								function row(content: string): string {
									const line = truncateToWidth(content, innerW);
									const pad = Math.max(0, innerW - visibleWidth(line));
									return `${border} ${line}${" ".repeat(pad)} ${border}`;
								}

								const lines: string[] = [];

								const label = " Readseek ";
								const topFill = borderFg(
									"─".repeat(Math.max(0, width - 4 - visibleWidth(label))),
								);
								lines.push(
									`${borderFg("╭─")}${accent(label)}${topFill}${borderFg("─╮")}`,
								);

								lines.push(row(""));

								const readseekDir = findReadseekDir(ctx.cwd);
								const statusDot = initialized
									? theme.fg("success", "●")
									: theme.fg("warning", "●");
								const statusLabel = initialized ? "Initialized" : "Not initialized";
								const statusColor = initialized
									? theme.fg("success", statusLabel)
									: theme.fg("warning", statusLabel);
								const pathText = readseekDir
									? dim(relative(ctx.cwd, readseekDir) || readseekDir)
									: dim(".readseek");
								lines.push(
									row(`${statusDot} ${statusColor} ${dim("·")} ${pathText}`),
								);

								lines.push(row(""));

								if (initialized) {
									lines.push(
										row(`${accent("▶")} ${dim("[d]")} Deinit  ${dim("remove .readseek/")}`),
									);
								} else {
									lines.push(
										row(
											`${accent("▶")} ${dim("[i]")} Init  ${dim("create .readseek/ map cache")}`,
										),
									);
								}

								lines.push(row(""));
								lines.push(row(dim("esc close")));

								lines.push(
									`${borderFg("╰")}${borderFg("─".repeat(Math.max(0, width - 2)))}${borderFg("╯")}`,
								);

								return lines;
							},

							handleInput(data: string): void {
								if (matchesKey(data, Key.escape) || matchesKey(data, Key.ctrl("c"))) {
									done(null);
									return;
								}

								if (!initialized && (data === "i" || data === "I")) {
									done("init");
									return;
								}
								if (initialized && (data === "d" || data === "D")) {
									done("deinit");
									return;
								}
							},

							invalidate(): void {},
						};
					},
					{
						overlay: true,
						overlayOptions: {
							anchor: "center",
							width: 60,
							margin: 2,
						},
					},
				).then((result) => {
					resolve(result ?? null);
				});
			});

			if (action === "init") await init(ctx);
			else if (action === "deinit") await deinit(ctx);
		},
	});
}
