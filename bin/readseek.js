#!/usr/bin/env node
// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) Jarkko Sakkinen 2026

const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

function resolveBinary() {
  const platform = process.platform;

  let packageName;
  let binaryName = 'readseek';

  switch (platform) {
    case 'linux':
      packageName = '@jarkkojs/readseek-linux-x64';
      break;
    case 'darwin':
      packageName = '@jarkkojs/readseek-darwin-arm64';
      break;
    case 'win32':
      packageName = '@jarkkojs/readseek-win32-x64';
      binaryName = 'readseek.exe';
      break;
    default:
      throw new Error(`unsupported platform: ${platform}`);
  }

  let packageDir;
  try {
    packageDir = path.dirname(
      require.resolve(`${packageName}/package.json`, { paths: [__dirname] })
    );
  } catch {
    throw new Error(
      `package ${packageName} is not installed; ` +
        'install the optional platform-specific dependency'
    );
  }

  const binaryPath = path.join(packageDir, 'bin', binaryName);
  if (!fs.existsSync(binaryPath)) {
    throw new Error(`binary not found: ${binaryPath}`);
  }

  return binaryPath;
}

try {
  const binary = resolveBinary();
  const args = process.argv.slice(2);
  execFileSync(binary, args, { stdio: 'inherit' });
} catch (err) {
  if (typeof err.status === 'number') {
    process.exit(err.status);
  }
  console.error(err.message);
  process.exit(1);
}
