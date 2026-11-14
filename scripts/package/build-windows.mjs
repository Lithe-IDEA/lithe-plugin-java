#!/usr/bin/env node
// Assemble on macOS, using the backend built from this checkout on Windows.
// All runtime inputs come from the committed manifests; installed packages are
// never written. Release CI supplies the backend artifact built from the same tag;
// the resulting unsigned ZIP is also used for native development validation.
import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { cp, mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
if (process.platform !== 'darwin') throw new Error('Assemble this development package on macOS.');
const backend = process.argv[2];
if (!backend) throw new Error('Usage: node scripts/package/build-windows.mjs <Windows x64 backend.exe>|--prepare-runtime');
const artifacts = join(root, '.artifacts');
const jdk = JSON.parse(await readFile(join(root, 'third_party/jdk/manifest.json'), 'utf8'));
const input = jdk.platforms['windows-x86_64'];
const cache = join(artifacts, 'jdk-downloads');
await mkdir(cache, { recursive: true });
const archive = join(cache, `jdk-windows-x86_64-${input.sha256}.zip`);
async function digest(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}
async function isVerified(path) {
  try { return await digest(path) === input.sha256; }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}
if (!await isVerified(archive)) {
  const temporary = `${archive}.${process.pid}.download`;
  try {
    const response = await fetch(input.url, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`JDK download failed: ${response.status}`);
    await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary, { flags: 'wx' }));
    if (!await isVerified(temporary)) throw new Error('Windows JDK SHA-256 mismatch.');
    await rename(temporary, archive);
  } finally { await rm(temporary, { force: true }); }
}
if (backend === '--prepare-runtime') {
  process.stdout.write(`${archive}\n`);
  process.exit(0);
}
async function verifyPE(path) {
  const bytes = await readFile(path);
  const header = bytes.length >= 64 ? bytes.readUInt32LE(60) : bytes.length;
  if (bytes.toString('ascii', 0, 2) !== 'MZ' || header + 6 > bytes.length || bytes.readUInt32LE(header) !== 0x4550 || bytes.readUInt16LE(header + 4) !== 0x8664) {
    throw new Error('A Windows x64 PE executable is required.');
  }
}
await verifyPE(resolve(backend));
// This existing entry verifies JDT LS, Lombok, java-debug and java-test from
// their locked archives and licenses, including config_win.
const jdt = execFileSync(join(root, 'scripts/prepare-jdtls.sh'), {
  encoding: 'utf8', env: { ...process.env, LITHE_ARCH: 'x86_64' },
  stdio: ['ignore', 'pipe', 'inherit'], timeout: 600_000,
}).trim();
const staging = join(artifacts, `windows-package-${process.pid}`);
const id = 'io.github.lithe-idea.java-support';
const packageRoot = join(staging, id);
try {
  await mkdir(packageRoot, { recursive: true });
  const unpack = join(staging, 'jdk');
  await mkdir(unpack);
  execFileSync('/usr/bin/unzip', ['-q', archive, '-d', unpack], { timeout: 120_000 });
  const runtime = join(unpack, input.jdkRoot);
  await verifyPE(join(runtime, 'bin/java.exe'));
  const release = await readFile(join(runtime, 'release'), 'utf8');
  if (!/^JAVA_VERSION="21\./m.test(release)) throw new Error('The pinned runtime must be Java 21.');
  await mkdir(join(packageRoot, 'runtime'), { recursive: true });
  await cp(runtime, join(packageRoot, 'runtime/x86_64'), { recursive: true });
  for (const name of ['icons', 'templates']) await cp(join(root, id, name), join(packageRoot, name), { recursive: true });
  const languageServer = join(packageRoot, 'language-server');
  await mkdir(languageServer);
  for (const name of ['plugins', 'features', 'lombok', 'java-debug', 'java-test', 'manifest.json', 'LICENSE-EPL-2.0.txt']) {
    await cp(join(jdt, name), join(languageServer, name), { recursive: true });
  }
  await mkdir(join(languageServer, 'configuration/x86_64'), { recursive: true });
  await cp(join(jdt, 'config_win/config.ini'), join(languageServer, 'configuration/x86_64/config.ini'));
  const manifest = JSON.parse(await readFile(join(root, id, 'plugin.json'), 'utf8'));
  manifest.backend.launch.path = 'bin/lithe-java-backend.exe';
  await writeFile(join(packageRoot, 'plugin.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  await mkdir(join(packageRoot, 'bin'));
  await cp(resolve(backend), join(packageRoot, manifest.backend.launch.path));
  const target = join(artifacts, `Lithe-Java-Support-${manifest.version}-windows-x64.zip`);
  execFileSync('/usr/bin/ditto', ['-c', '-k', '--keepParent', packageRoot, target], { timeout: 120_000 });
  await writeFile(`${target}.sha256`, `${await digest(target)}  ${target.split('/').at(-1)}\n`);
  process.stdout.write(`${target}\n`);
} finally { await rm(staging, { recursive: true, force: true }); }
