#!/usr/bin/env node

import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const unsupportedFlags = new Set(["--runInBand"]);
const args = process.argv.slice(2).filter((arg) => !unsupportedFlags.has(arg));
const vitestBin = fileURLToPath(new URL("../node_modules/vitest/vitest.mjs", import.meta.url));

const child = spawn(process.execPath, [vitestBin, "run", ...args], {
  stdio: "inherit",
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

child.on("error", (error) => {
  console.error(error.message);
  process.exit(1);
});
