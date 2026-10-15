#!/usr/bin/env node
// Set the app version everywhere it is written down:
//   pnpm version:set 0.9.0
// package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json (which CI and
// build_portable.ps1 read) and the `meetily` entry of the workspace Cargo.lock.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) {
  console.error('Usage: pnpm version:set <major.minor.patch>, e.g. pnpm version:set 0.9.0');
  process.exit(1);
}

const frontend = join(dirname(fileURLToPath(import.meta.url)), '..');
const files = [
  // [path, pattern matching the one version to replace]
  [join(frontend, 'package.json'), /^(  "version": ")[^"]+(",)$/m],
  [join(frontend, 'src-tauri', 'tauri.conf.json'), /^(    "version": ")[^"]+(",)$/m],
  [join(frontend, 'src-tauri', 'Cargo.toml'), /^(\[package\]\r?\nname = "meetily"\r?\nversion = ")[^"]+(")/m],
  [join(frontend, '..', 'Cargo.lock'), /^(name = "meetily"\r?\nversion = ")[^"]+(")/m],
];

for (const [path, pattern] of files) {
  const text = readFileSync(path, 'utf8');
  if (!pattern.test(text)) {
    console.error(`Version not found in ${path}; nothing was changed in it.`);
    process.exitCode = 1;
    continue;
  }
  writeFileSync(path, text.replace(pattern, `$1${version}$2`));
  console.log(`${version}  ${path}`);
}
