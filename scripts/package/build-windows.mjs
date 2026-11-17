#!/usr/bin/env node
// Assemble on macOS, using the backend built from this checkout on Windows.
// The package carries the JDT LS extensions verified from their pins; JDT LS
// and the JDK it runs on are the user's. Installed packages are never
// written. Release CI supplies the backend artifact built from the same tag;
// the resulting unsigned ZIP is also used for native development validation.
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { collectNotices, verifyNotices } from './collect-notices.mjs';
import { buildProjectModelBundle } from './project-model-bundle.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
if (process.platform !== 'darwin') throw new Error('Assemble this development package on macOS.');
const backend = process.argv[2];
if (!backend) throw new Error('Usage: node scripts/package/build-windows.mjs <Windows x64 backend.exe>');
const artifacts = join(root, '.artifacts');
async function digest(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}
async function verifyPE(path) {
  const bytes = await readFile(path);
  const header = bytes.length >= 64 ? bytes.readUInt32LE(60) : bytes.length;
  if (bytes.toString('ascii', 0, 2) !== 'MZ' || header + 6 > bytes.length || bytes.readUInt32LE(header) !== 0x4550 || bytes.readUInt16LE(header + 4) !== 0x8664) {
    throw new Error('A Windows x64 PE executable is required.');
  }
}
await verifyPE(resolve(backend));
// This existing entry verifies Lombok, java-debug and java-test from their
// locked archives and licenses, next to the tested JDT LS release.
const jdt = execFileSync(join(root, 'scripts/prepare-jdtls.sh'), {
  encoding: 'utf8', env: { ...process.env, LITHE_ARCH: 'x86_64' },
  stdio: ['ignore', 'pipe', 'inherit'], timeout: 600_000,
}).trim();
const staging = join(artifacts, `windows-package-${process.pid}`);
const id = 'io.github.lithe-idea.java-support';
const packageRoot = join(staging, id);
try {
  await mkdir(packageRoot, { recursive: true });
  for (const name of ['icons', 'templates', 'l10n']) await cp(join(root, id, name), join(packageRoot, name), { recursive: true });
  const extensions = join(packageRoot, 'jdtls-extensions');
  await mkdir(extensions);
  for (const name of ['lombok', 'java-debug', 'java-test', 'manifest.json']) {
    await cp(join(jdt, name), join(extensions, name), { recursive: true });
  }
  // The plugin's own bundle, compiled against the prepared JDT LS release.
  buildProjectModelBundle(jdt, extensions);
  const manifest = JSON.parse(await readFile(join(root, id, 'plugin.json'), 'utf8'));
  manifest.backend.launch.path = 'bin/lithe-java-backend.exe';
  await writeFile(join(packageRoot, 'plugin.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  await mkdir(join(packageRoot, 'bin'));
  await cp(resolve(backend), join(packageRoot, manifest.backend.launch.path));
  collectNotices(packageRoot);
  verifyNotices(packageRoot);
  const target = join(artifacts, `Lithe-Java-Support-${manifest.version}-windows-x64.zip`);
  execFileSync('/usr/bin/ditto', ['-c', '-k', '--keepParent', packageRoot, target], { timeout: 120_000 });
  await writeFile(`${target}.sha256`, `${await digest(target)}  ${target.split('/').at(-1)}\n`);
  process.stdout.write(`${target}\n`);
} finally { await rm(staging, { recursive: true, force: true }); }
