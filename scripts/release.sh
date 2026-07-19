#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Copyright (C) Jarkko Sakkinen 2026

set -euo pipefail

die() {
	printf '%s\n' "$1" >&2
	exit 1
}

ver_gt() {
	if (( $1 > $4 )); then return 0
	elif (( $1 == $4 && $2 > $5 )); then return 0
	elif (( $1 == $4 && $2 == $5 && $3 > $6 )); then return 0
	else return 1
	fi
}

version_parts() {
	[[ "$1" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] \
		|| die "invalid version: $1"
	VERSION_A="${BASH_REMATCH[1]}"
	VERSION_B="${BASH_REMATCH[2]}"
	VERSION_C="${BASH_REMATCH[3]}"
}

release_files=(
	Cargo.toml
	Cargo.lock
	package.json
	package-lock.json
	npm/darwin-arm64/package.json
	npm/linux-arm64/package.json
	npm/linux-x64/package.json
	npm/win32-x64/package.json
	man/man1/readseek.1
	packages/pi-readseek/package.json
	packages/pi-readseek/package-lock.json
	packages/opencode-readseek/package.json
	packages/opencode-readseek/package-lock.json
	autoload/readseek/config.vim
)
committed=0
signing_check_tag=""

cleanup() {
	local status=$?

	if [[ -n "$signing_check_tag" ]] \
		&& git rev-parse --verify --quiet "refs/tags/$signing_check_tag" >/dev/null; then
		git tag -d "$signing_check_tag" >/dev/null 2>&1 || true
	fi
	if (( status != 0 && !committed )); then
		git restore --staged -- "${release_files[@]}" 2>/dev/null || true
		git restore -- "${release_files[@]}" 2>/dev/null || true
	fi
	return "$status"
}
trap cleanup EXIT

next_ver="${1:-}"
[[ -n "$next_ver" ]] || die "usage: scripts/release.sh <next-version>"
version_parts "$next_ver"
next_a="$VERSION_A"
next_b="$VERSION_B"
next_c="$VERSION_C"

branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null)" \
	|| die "HEAD is detached; check out a branch before releasing"
[[ -z "$(git status --porcelain)" ]] \
	|| die "working directory is not clean"
[[ -z "$(git tag -l "$next_ver")" ]] \
	|| die "tag $next_ver already exists"

signing_check_tag="readseek-signing-check-$next_ver-$$"
[[ -z "$(git tag -l "$signing_check_tag")" ]] \
	|| die "temporary signing-check tag already exists: $signing_check_tag"
git tag -s "$signing_check_tag" -m "readseek release signing check" \
	|| die "cannot sign release tags; renew or configure the Git signing key"
git tag -d "$signing_check_tag" >/dev/null
signing_check_tag=""

core_ver="$(sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' Cargo.toml | head -1)"
[[ -n "$core_ver" ]] || die "cannot find version in Cargo.toml"
version_parts "$core_ver"
ver_gt "$next_a" "$next_b" "$next_c" "$VERSION_A" "$VERSION_B" "$VERSION_C" \
	|| die "$next_ver is not greater than readseek $core_ver"

pi_ver="$(node -p 'require("./packages/pi-readseek/package.json").version')" \
	|| die "cannot find pi-readseek version"
version_parts "$pi_ver"
ver_gt "$next_a" "$next_b" "$next_c" "$VERSION_A" "$VERSION_B" "$VERSION_C" \
	|| die "$next_ver is not greater than pi-readseek $pi_ver"

	opencode_ver="$(node -p 'require("./packages/opencode-readseek/package.json").version')" \
	|| die "cannot find opencode-readseek version"
version_parts "$opencode_ver"
	ver_gt "$next_a" "$next_b" "$next_c" "$VERSION_A" "$VERSION_B" "$VERSION_C" \
	|| die "$next_ver is not greater than opencode-readseek $opencode_ver"

vim_ver="$(sed -n "s/^export const PluginVersion = '\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)'$/\1/p" autoload/readseek/config.vim)"
[[ -n "$vim_ver" ]] || die "cannot find readseek.vim version"
version_parts "$vim_ver"
ver_gt "$next_a" "$next_b" "$next_c" "$VERSION_A" "$VERSION_B" "$VERSION_C" \
	|| die "$next_ver is not greater than readseek.vim $vim_ver"

readseek_log="$(git log --first-parent --format='- %s (%an)' --no-merges "$core_ver"..HEAD -- . ':(exclude)autoload' ':(exclude)doc' ':(exclude)plugin' ':(exclude)test' ':(exclude)LICENSE-MIT' ':(exclude)packages/pi-readseek' ':(exclude)packages/opencode-readseek')"
pi_log="$(git log --format='- %s (%an)' --no-merges "$core_ver"..HEAD -- packages/pi-readseek)"
opencode_log="$(git log --format='- %s (%an)' --no-merges "$core_ver"..HEAD -- packages/opencode-readseek)"
vim_log="$(git log --format='- %s (%an)' --no-merges "$core_ver"..HEAD -- autoload doc plugin test LICENSE-MIT)"
[[ -n "$readseek_log" ]] || readseek_log='- No source changes.'
[[ -n "$pi_log" ]] || pi_log='- Merged pi-readseek into this repository.'
[[ -n "$opencode_log" ]] || opencode_log='- Merged opencode-readseek into this repository.'
[[ -n "$vim_log" ]] || vim_log='- Merged readseek.vim into this repository.'

npm install --prefix packages/pi-readseek --package-lock=false --ignore-scripts
npm install --prefix packages/opencode-readseek --package-lock=false --ignore-scripts

node - "$next_ver" <<'NODE'
const fs = require('node:fs');

const [nextVersion] = process.argv.slice(2);
const corePackagePaths = [
  'package.json',
  'npm/darwin-arm64/package.json',
  'npm/linux-arm64/package.json',
  'npm/linux-x64/package.json',
  'npm/win32-x64/package.json',
];
const corePackages = corePackagePaths.map((packagePath) => [
  packagePath,
  JSON.parse(fs.readFileSync(packagePath, 'utf8')),
]);
const root = corePackages[0][1];

root.version = nextVersion;
for (const [, data] of corePackages.slice(1)) {
  data.version = nextVersion;
  root.optionalDependencies[data.name] = nextVersion;
}
for (const [packagePath, data] of corePackages) {
  fs.writeFileSync(packagePath, `${JSON.stringify(data, null, 2)}\n`);
}

const readseekDependency = '@jarkkojs/readseek';
const readseekRange = `^${nextVersion}`;
const platformDependencies = Object.keys(root.optionalDependencies);

function updateLockedPackage(lock, packageName) {
  const lockedPackage = lock.packages[`node_modules/${packageName}`];
  if (!lockedPackage) {
    throw new Error(`package-lock.json does not contain ${packageName}`);
  }

  const unscopedName = packageName.slice(packageName.indexOf('/') + 1);
  lockedPackage.version = nextVersion;
  lockedPackage.resolved =
    `https://registry.npmjs.org/${packageName}/-/${unscopedName}-${nextVersion}.tgz`;
  delete lockedPackage.integrity;
}

function updatePluginLock(lock) {
  updateLockedPackage(lock, readseekDependency);
  for (const packageName of platformDependencies) updateLockedPackage(lock, packageName);

  const lockedReadseek = lock.packages[`node_modules/${readseekDependency}`];
  for (const packageName of platformDependencies) {
    lockedReadseek.optionalDependencies[packageName] = nextVersion;
  }
}

const coreLockPath = 'package-lock.json';
const coreLock = JSON.parse(fs.readFileSync(coreLockPath, 'utf8'));
coreLock.version = nextVersion;
coreLock.packages[''].version = nextVersion;
for (const name of Object.keys(root.optionalDependencies)) {
  coreLock.packages[''].optionalDependencies[name] = nextVersion;
}
for (const packageName of platformDependencies) updateLockedPackage(coreLock, packageName);
fs.writeFileSync(coreLockPath, `${JSON.stringify(coreLock, null, 2)}\n`);

const piPackagePath = 'packages/pi-readseek/package.json';
const piPackage = JSON.parse(fs.readFileSync(piPackagePath, 'utf8'));
piPackage.version = nextVersion;
piPackage.dependencies[readseekDependency] = readseekRange;
fs.writeFileSync(piPackagePath, `${JSON.stringify(piPackage, null, 2)}\n`);

const piLockPath = 'packages/pi-readseek/package-lock.json';
const piLock = JSON.parse(fs.readFileSync(piLockPath, 'utf8'));
piLock.version = nextVersion;
piLock.packages[''].version = nextVersion;
piLock.packages[''].dependencies[readseekDependency] = readseekRange;
updatePluginLock(piLock);
fs.writeFileSync(piLockPath, `${JSON.stringify(piLock, null, 2)}\n`);

const opencodePackagePath = 'packages/opencode-readseek/package.json';
const opencodePackage = JSON.parse(fs.readFileSync(opencodePackagePath, 'utf8'));
opencodePackage.version = nextVersion;
opencodePackage.dependencies[readseekDependency] = readseekRange;
fs.writeFileSync(opencodePackagePath, `${JSON.stringify(opencodePackage, null, 2)}\n`);

const opencodeLockPath = 'packages/opencode-readseek/package-lock.json';
const opencodeLock = JSON.parse(fs.readFileSync(opencodeLockPath, 'utf8'));
opencodeLock.version = nextVersion;
opencodeLock.packages[''].version = nextVersion;
opencodeLock.packages[''].dependencies[readseekDependency] = readseekRange;
updatePluginLock(opencodeLock);
fs.writeFileSync(opencodeLockPath, `${JSON.stringify(opencodeLock, null, 2)}\n`);
NODE

sed -E -i.bak "s/^([[:space:]]*version[[:space:]]*=[[:space:]]*)\"${core_ver//./\\.}\"/\1\"$next_ver\"/" Cargo.toml
rm -f Cargo.toml.bak
grep -q "^[[:space:]]*version = \"$next_ver\"" Cargo.toml \
	|| die "failed to update version in Cargo.toml"

sed -E -i.bak "s/^(export const PluginVersion = ')[^']+(')$/\1$next_ver\2/" autoload/readseek/config.vim
rm -f autoload/readseek/config.vim.bak
grep -q "^export const PluginVersion = '$next_ver'$" autoload/readseek/config.vim \
	|| die "failed to update readseek.vim version"

date="$(date '+%Y-%m-%d')"
sed -E -i.bak 's/^(\.TH [^[:space:]]* [0-9][0-9]*) "[^"]*"/\1 "'"$date"'"/' man/man1/readseek.1
sed -E -i.bak 's/"readseek [0-9][^"]*"/"readseek '"$next_ver"'"/' man/man1/readseek.1
rm -f man/man1/readseek.1.bak

grep -q '^\.TH .* "'"$date"'"' man/man1/readseek.1 \
	|| die "failed to update man/man1/readseek.1 date"
grep -q '^\.TH .* "readseek '"$next_ver"'"' man/man1/readseek.1 \
	|| die "failed to update man/man1/readseek.1 version"

npm --prefix packages/pi-readseek run typecheck
npm --prefix packages/pi-readseek test
npm --prefix packages/opencode-readseek run typecheck
bun test packages/opencode-readseek/tests
vim -Nu NONE -n -i NONE -es -S test/readseek.vim

cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check

git add -- "${release_files[@]}"
git commit -s -m "Bump the version to $next_ver"
committed=1


sob="Signed-off-by: $(git config user.name) <$(git config user.email)>"
release_notes="$(git rev-parse --git-path "readseek-$next_ver-tag-message.txt")"
cat >"$release_notes" <<EOF
readseek $next_ver

readseek:
$readseek_log

pi-readseek:
$pi_log

opencode-readseek:
$opencode_log

readseek.vim:
$vim_log

$sob
EOF

git tag -s "$next_ver" -F "$release_notes"

printf 'tagged %s\n' "$next_ver"
printf 'push the commit and tag, wait for CI, then run make publish\n'
