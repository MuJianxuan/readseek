import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { beforeEach, describe, expect, it, vi } from "vitest";

const { createReadToolExecuteMock, readSeekMapMock, readSeekReadMock, readSeekDetectMock, readSeekImageMock, imageMode } = vi.hoisted(() => ({
	createReadToolExecuteMock: vi.fn(),
	readSeekMapMock: vi.fn(),
	readSeekReadMock: vi.fn(),
	readSeekDetectMock: vi.fn(),
	readSeekImageMock: vi.fn(),
	imageMode: { value: "force" as "force" | "off" | "auto" },
}));

vi.mock("@earendil-works/pi-coding-agent", async () => ({
	...(await import("./support/pi-coding-agent-mock.js")).createPiCodingAgentBaseMock(),
	createReadTool: () => ({ execute: createReadToolExecuteMock }),
}));

vi.mock("../src/readseek-settings.js", () => ({
	resolveReadSeekImageMode: () => imageMode.value,
	resolveReadSeekJsonSettings: () => ({ settings: {}, warnings: [] }),
	resolveReadSeekSyntaxValidation: () => undefined,
	resolveReadSeekTimeoutMs: () => undefined,
}));

vi.mock("../src/readseek-client.js", () => ({
	readSeekMap: readSeekMapMock,
	readSeekRead: readSeekReadMock,
	readSeekDetect: readSeekDetectMock,
	readSeekImage: readSeekImageMock,
}));

const { executeRead } = await import("../src/read.js");

describe("executeRead anchor tracking", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		imageMode.value = "force";
	});

	async function writeImage(cwd: string): Promise<string> {
		const filePath = path.join(cwd, "image.png");
		const png = Buffer.from(
			"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC",
			"base64",
		);
		await writeFile(filePath, png);
		return filePath;
	}

	function imageDetectionFor(filePath: string) {
		return {
			kind: "image",
			type: "image/png",
			file: filePath,
			mime: "image/png",
			format: "png",
			width: 1,
			height: 1,
			animated: false,
		};
	}

	function mockImageDetection(filePath: string) {
		const imageDetection = imageDetectionFor(filePath);
		readSeekDetectMock.mockResolvedValue(imageDetection);
		readSeekImageMock.mockImplementation((_filePath: string, modes: string[]) =>
			Promise.resolve({
				...imageDetection,
				...(modes.includes("ocr") ? { ocr: "OCR TEXT" } : {}),
				...(modes.includes("caption") ? { caption: "A tiny test image." } : {}),
				...(modes.includes("objects") ? { objects: [{ label: "dot", bbox: [1, 2, 3, 4] }] } : {}),
			}),
		);
	}

	it("marks text reads with readseek lines as anchored", async () => {
		const cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-read-"));
		try {
			const filePath = path.join(cwd, "file.txt");
			await writeFile(filePath, "hello\nworld\n", "utf8");
			readSeekReadMock.mockResolvedValueOnce({
				file: filePath,
				language: "Text",
				line_count: 2,
				file_hash: "filehash",
				start_line: 1,
				end_line: 2,
				hashlines: [
					{ line: 1, hash: "aaa", text: "hello" },
					{ line: 2, hash: "bbb", text: "world" },
				],
			});
			const onSuccessfulRead = vi.fn();

			await executeRead({
				toolCallId: "test",
				params: { path: "file.txt" },
				signal: undefined,
				onUpdate: undefined,
				cwd,
				onSuccessfulRead,
			});

			expect(onSuccessfulRead).toHaveBeenCalledWith(filePath);
		} finally {
			await rm(cwd, { recursive: true, force: true });
		}
	});

	it("falls back to image attachment when image analysis crashes", async () => {
		const cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-read-"));
		try {
			const filePath = await writeImage(cwd);
			readSeekDetectMock.mockResolvedValue(imageDetectionFor(filePath));
			readSeekImageMock.mockRejectedValueOnce(new Error("readseek crashed with SIGFPE"));
			createReadToolExecuteMock.mockResolvedValueOnce({
				content: [{ type: "text", text: "image attachment" }],
			});

			const result = await executeRead({
				toolCallId: "test",
				params: { path: "image.png" },
				signal: undefined,
				onUpdate: undefined,
				cwd,
			});

			const text = (result.content as Array<{ type: string; text: string }>).map((part) => part.text).join("\n");
			expect(text).toContain("image attachment");
			expect(text).toContain("image analysis unavailable");
			expect(text).not.toContain("binary");
			const details = (result as any).details;
			expect(details?.readSeekValue).toBeUndefined();
		} finally {
			await rm(cwd, { recursive: true, force: true });
		}
	});

	it("appends image analysis (OCR, caption, objects) to image reads and does not mark them as anchored", async () => {
		const cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-read-"));
		try {
			const filePath = await writeImage(cwd);
			mockImageDetection(filePath);
			createReadToolExecuteMock.mockResolvedValueOnce({
				content: [{ type: "text", text: "image attachment" }],
			});
			const onSuccessfulRead = vi.fn();

			const result = await executeRead({
				toolCallId: "test",
				params: { path: "image.png" },
				signal: undefined,
				onUpdate: undefined,
				cwd,
				onSuccessfulRead,
			});

			expect(onSuccessfulRead).not.toHaveBeenCalled();
			const text = (result.content as Array<{ type: string; text: string }>).map((part) => part.text).join("\n");
			expect(text).toContain("image attachment");
			expect(text).toContain("OCR TEXT");
			expect(text).toContain("Image caption:\nA tiny test image.");
			expect(text).toContain("Detected objects:\n- dot [1, 2, 3, 4]");
		} finally {
			await rm(cwd, { recursive: true, force: true });
		}
	});

	it("skips image analysis when imageMode is off", async () => {
		const cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-read-"));
		try {
			imageMode.value = "off";
			const filePath = await writeImage(cwd);
			mockImageDetection(filePath);
			createReadToolExecuteMock.mockResolvedValueOnce({
				content: [{ type: "text", text: "image attachment" }],
			});

			const result = await executeRead({
				toolCallId: "test",
				params: { path: "image.png" },
				signal: undefined,
				onUpdate: undefined,
				cwd,
				modelSupportsImages: false,
			});

			const text = (result.content as Array<{ type: string; text: string }>).map((part) => part.text).join("\n");
			expect(text).toBe("image attachment");
			expect(readSeekImageMock).not.toHaveBeenCalled();
		} finally {
			await rm(cwd, { recursive: true, force: true });
		}
	});

	it("skips image analysis for image-capable models when imageMode is auto", async () => {
		const cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-read-"));
		try {
			imageMode.value = "auto";
			const filePath = await writeImage(cwd);
			mockImageDetection(filePath);
			createReadToolExecuteMock.mockResolvedValueOnce({
				content: [{ type: "text", text: "image attachment" }],
			});

			const result = await executeRead({
				toolCallId: "test",
				params: { path: "image.png" },
				signal: undefined,
				onUpdate: undefined,
				cwd,
				modelSupportsImages: true,
			});

			const text = (result.content as Array<{ type: string; text: string }>).map((part) => part.text).join("\n");
			expect(text).toBe("image attachment");
			expect(readSeekImageMock).not.toHaveBeenCalled();
		} finally {
			await rm(cwd, { recursive: true, force: true });
		}
	});

	it.each(["map", "local"])("treats %s bundle without symbol as a map read", async (bundle) => {
		const cwd = await mkdtemp(path.join(tmpdir(), "pi-readseek-read-"));
		try {
			const filePath = path.join(cwd, "file.ts");
			await writeFile(filePath, "const value = 1;\n", "utf8");
			readSeekReadMock.mockResolvedValueOnce({
				file: filePath,
				language: "TypeScript",
				line_count: 1,
				file_hash: "filehash",
				start_line: 1,
				end_line: 1,
				hashlines: [{ line: 1, hash: "aaa", text: "const value = 1;" }],
			});
			readSeekMapMock.mockResolvedValueOnce({
				path: filePath,
				totalLines: 1,
				totalBytes: 17,
				language: "TypeScript",
				symbols: [],
				detailLevel: "full",
			});

			const result = await executeRead({
				toolCallId: "test",
				params: { path: "file.ts", bundle },
				signal: undefined,
				onUpdate: undefined,
				cwd,
			});

			expect((result as { isError?: boolean }).isError).not.toBe(true);
			expect((result.details as any).readSeekValue.map).toEqual({
				requested: true,
				appended: true,
			});
		} finally {
			await rm(cwd, { recursive: true, force: true });
		}
	});
});
