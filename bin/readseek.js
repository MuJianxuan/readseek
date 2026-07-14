#!/usr/bin/env node
// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) Jarkko Sakkinen 2026

const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

function resolveBinary() {
  const platform = `${process.platform}-${process.arch}`;
  const packages = {
    'darwin-arm64': '@jarkkojs/readseek-darwin-arm64',
    'linux-arm64': '@jarkkojs/readseek-linux-arm64',
    'linux-x64': '@jarkkojs/readseek-linux-x64',
    'win32-x64': '@jarkkojs/readseek-win32-x64',
  };
  const packageName = packages[platform];
  if (!packageName) {
    throw new Error(
      `unsupported platform: ${platform}; supported platforms: ${Object.keys(packages).join(', ')}`
    );
  }
  const binaryName = process.platform === 'win32' ? 'readseek.exe' : 'readseek';

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
