import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

interface PackageJson {
  version: string;
  dependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
  resolved?: string;
}

interface PackageLock extends PackageJson {
  packages: Record<string, PackageJson>;
}

function readJson<T>(url: URL): T {
  return JSON.parse(readFileSync(url, "utf8")) as T;
}

function expectLockedPackage(
  lock: PackageLock,
  packageName: string,
  version: string,
): void {
  const lockedPackage = lock.packages[`node_modules/${packageName}`];
  expect(lockedPackage?.version).toBe(version);
  expect(lockedPackage?.resolved?.endsWith(`-${version}.tgz`)).toBe(true);
}

describe("package metadata", () => {
  it("matches the readseek release", () => {
    const pluginPackage = readJson<PackageJson>(new URL("../package.json", import.meta.url));
    const pluginLock = readJson<PackageLock>(new URL("../package-lock.json", import.meta.url));
    const readseekPackage = readJson<PackageJson>(new URL("../../../package.json", import.meta.url));
    const readseekLock = readJson<PackageLock>(
      new URL("../../../package-lock.json", import.meta.url),
    );
    const version = readseekPackage.version;
    const platformDependencies = Object.keys(readseekPackage.optionalDependencies ?? {});

    expect(pluginPackage.version).toBe(version);
    expect(pluginLock.version).toBe(version);
    expect(pluginLock.packages[""].version).toBe(version);
    expect(pluginPackage.dependencies?.["@jarkkojs/readseek"]).toBe(`^${version}`);
    expect(pluginLock.packages[""].dependencies?.["@jarkkojs/readseek"]).toBe(`^${version}`);

    for (const packageName of platformDependencies) {
      expectLockedPackage(readseekLock, packageName, version);
      expectLockedPackage(pluginLock, packageName, version);
    }
    expectLockedPackage(pluginLock, "@jarkkojs/readseek", version);
    expect(
      pluginLock.packages["node_modules/@jarkkojs/readseek"].optionalDependencies,
    ).toEqual(readseekPackage.optionalDependencies);
  });
});
