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
	npm/linux-x64/package.json
	npm/win32-x64/package.json
	man/man1/readseek.1
	packages/pi-readseek/package.json
	packages/pi-readseek/package-lock.json
)
committed=0

cleanup() {
	local status=$?

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

readseek_log="$(git log --first-parent --format='- %s (%an)' --no-merges "$core_ver"..HEAD -- . ':(exclude)packages/pi-readseek')"
pi_log="$(git log --format='- %s (%an)' --no-merges "$core_ver"..HEAD -- packages/pi-readseek)"
[[ -n "$readseek_log" ]] || readseek_log='- No source changes.'
[[ -n "$pi_log" ]] || pi_log='- Merged pi-readseek into this repository.'

npm install --prefix packages/pi-readseek --package-lock=false --ignore-scripts

npm --prefix packages/pi-readseek run typecheck
npm --prefix packages/pi-readseek test

node - "$next_ver" <<'NODE'
const fs = require('node:fs');

const [nextVersion] = process.argv.slice(2);
const corePackagePaths = [
  'package.json',
  'npm/darwin-arm64/package.json',
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

const coreLockPath = 'package-lock.json';
const coreLock = JSON.parse(fs.readFileSync(coreLockPath, 'utf8'));
coreLock.version = nextVersion;
coreLock.packages[''].version = nextVersion;
for (const name of Object.keys(root.optionalDependencies)) {
  coreLock.packages[''].optionalDependencies[name] = nextVersion;
}
fs.writeFileSync(coreLockPath, `${JSON.stringify(coreLock, null, 2)}\n`);

const piPackagePath = 'packages/pi-readseek/package.json';
const piPackage = JSON.parse(fs.readFileSync(piPackagePath, 'utf8'));
piPackage.version = nextVersion;
piPackage.dependencies['@jarkkojs/readseek'] = `^${nextVersion}`;
fs.writeFileSync(piPackagePath, `${JSON.stringify(piPackage, null, 2)}\n`);

const piLockPath = 'packages/pi-readseek/package-lock.json';
const piLock = JSON.parse(fs.readFileSync(piLockPath, 'utf8'));
piLock.version = nextVersion;
piLock.packages[''].version = nextVersion;
piLock.packages[''].dependencies['@jarkkojs/readseek'] = `^${nextVersion}`;
fs.writeFileSync(piLockPath, `${JSON.stringify(piLock, null, 2)}\n`);
NODE

sed -E -i.bak "s/^([[:space:]]*version[[:space:]]*=[[:space:]]*)\"${core_ver//./\\.}\"/\1\"$next_ver\"/" Cargo.toml
rm -f Cargo.toml.bak
grep -q "^[[:space:]]*version = \"$next_ver\"" Cargo.toml \
	|| die "failed to update version in Cargo.toml"

date="$(date '+%Y-%m-%d')"
sed -E -i.bak 's/^(\.TH [^[:space:]]* [0-9][0-9]*) "[^"]*"/\1 "'"$date"'"/' man/man1/readseek.1
sed -E -i.bak 's/"readseek [0-9][^"]*"/"readseek '"$next_ver"'"/' man/man1/readseek.1
rm -f man/man1/readseek.1.bak

grep -q '^\.TH .* "'"$date"'"' man/man1/readseek.1 \
	|| die "failed to update man/man1/readseek.1 date"
grep -q '^\.TH .* "readseek '"$next_ver"'"' man/man1/readseek.1 \
	|| die "failed to update man/man1/readseek.1 version"

cargo build
cargo test
cargo clippy --all-targets --all-features
cargo fmt --check

git add -- "${release_files[@]}"
git commit -s -m "Bump the version to $next_ver"
committed=1


release_notes="$(git rev-parse --git-path "readseek-$next_ver-tag-message.txt")"
cat >"$release_notes" <<EOF
readseek $next_ver

readseek:
$readseek_log

pi-readseek:
$pi_log
EOF

printf 'Created %s. Create the signed tag with:\n\n' "$(git rev-parse --short HEAD)"
printf 'git tag -s %q -F %q\n' "$next_ver" "$release_notes"
printf '\nTag message:\n\n'
cat "$release_notes"
