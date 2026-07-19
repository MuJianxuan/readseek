#!/usr/bin/env node
// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) Jarkko Sakkinen 2026

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';

const [version] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) {
  throw new Error('usage: scripts/update-npm-integrity.mjs <version>');
}

const npm = process.env.NPM || 'npm';
const readseekPackage = '@jarkkojs/readseek';
const platformPackages = [
  '@jarkkojs/readseek-darwin-arm64',
  '@jarkkojs/readseek-linux-arm64',
  '@jarkkojs/readseek-linux-x64',
  '@jarkkojs/readseek-win32-x64',
];
const packageNames = [readseekPackage, ...platformPackages];
const distributions = new Map();

for (const packageName of packageNames) {
  const output = execFileSync(
    npm,
    ['view', `${packageName}@${version}`, 'dist', '--json'],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] },
  );
  const distribution = JSON.parse(output);
  if (!distribution?.tarball || !distribution?.integrity?.startsWith('sha512-')) {
    throw new Error(`npm returned incomplete distribution metadata for ${packageName}@${version}`);
  }
  distributions.set(packageName, distribution);
}

function updateLockedPackage(lock, packageName) {
  const lockedPackage = lock.packages[`node_modules/${packageName}`];
  if (!lockedPackage) {
    throw new Error(`package-lock.json does not contain ${packageName}`);
  }
  if (lockedPackage.version !== version) {
    throw new Error(`${packageName} lock version ${lockedPackage.version} does not match ${version}`);
  }

  const distribution = distributions.get(packageName);
  lockedPackage.resolved = distribution.tarball;
  lockedPackage.integrity = distribution.integrity;
}

function updateLock(lockPath, packageNamesToUpdate) {
  const lock = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
  for (const packageName of packageNamesToUpdate) {
    updateLockedPackage(lock, packageName);
  }
  fs.writeFileSync(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
}

updateLock('package-lock.json', platformPackages);
updateLock('packages/pi-readseek/package-lock.json', packageNames);
updateLock('packages/opencode-readseek/package-lock.json', packageNames);
