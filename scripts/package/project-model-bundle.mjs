#!/usr/bin/env node
// Builds the plugin's own JDT LS extension bundle (jdtls-extension/project-model)
// into <destination>/project-model/. It is compiled against the JDT LS release
// prepared by scripts/prepare-jdtls.sh, the release the tests run; the bundle
// only uses APIs JDT has kept since its 3.x releases, so a user's newer JDT LS
// loads it too. The jar is reproducible: entries are sorted and dated
// BUNDLE_DATE, so the same sources and JDK give the same bytes.
// Usage: node scripts/package/project-model-bundle.mjs <jdtls-home> <jdtls-extensions-dir>
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const BUNDLE_SOURCE = join(root, 'jdtls-extension/project-model');
export const BUNDLE_DIRECTORY = 'project-model';
export const BUNDLE_DATE = '2026-01-01T00:00:00Z';
// The oldest Java a supported JDT LS runs on is 21; 17 keeps the class files
// loadable by every JDK the user may still point JDT LS at.
const RELEASE = '17';

/** The bundle's jar name, from its manifest's symbolic name and version. */
export function bundleFileName() {
  const manifest = readFileSync(join(BUNDLE_SOURCE, 'META-INF/MANIFEST.MF'), 'utf8');
  const name = /^Bundle-SymbolicName:\s*([^;\s]+)/m.exec(manifest)?.[1];
  const version = /^Bundle-Version:\s*(\S+)/m.exec(manifest)?.[1];
  if (!name || !version) throw new Error('The project model bundle manifest names no symbolic name or version');
  return `${name}_${version}.jar`;
}

function tool(name) {
  const override = process.env[`LITHE_${name.toUpperCase()}`];
  if (override) return override;
  const home = process.env.JAVA_HOME;
  const candidate = home && join(home, 'bin', process.platform === 'win32' ? `${name}.exe` : name);
  return candidate && existsSync(candidate) ? candidate : name;
}

// JDT LS 1.61 is compiled for Java 21; an older javac cannot read its classes.
const MINIMUM_JAVAC = 21;

function checkedJavac() {
  const javac = tool('javac');
  const output = execFileSync(javac, ['-version'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 30_000 });
  const major = Number(/javac (?:1\.)?(\d+)/.exec(output)?.[1]);
  if (!(major >= MINIMUM_JAVAC)) {
    throw new Error(`${javac} reports "${output.trim()}"; the project model bundle needs javac ${MINIMUM_JAVAC} or later to read the JDT LS classes. Set JAVA_HOME or LITHE_JAVAC to a JDK ${MINIMUM_JAVAC}+.`);
  }
  return javac;
}

function sources(directory) {
  const found = [];
  for (const name of readdirSync(directory).sort()) {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) found.push(...sources(path));
    else if (name.endsWith('.java')) found.push(path);
  }
  return found;
}

/** Compiles and packs the bundle; returns the jar's path. */
export function buildProjectModelBundle(jdtlsHome, extensionsDirectory) {
  const plugins = join(jdtlsHome, 'plugins');
  if (!existsSync(plugins)) throw new Error(`No JDT LS plugins folder at ${plugins}`);
  const work = mkdtempSync(join(tmpdir(), 'lithe-project-model-'));
  try {
    const classes = join(work, 'classes');
    mkdirSync(classes);
    const classpath = readdirSync(plugins).filter(name => name.endsWith('.jar')).sort().map(name => join(plugins, name)).join(process.platform === 'win32' ? ';' : ':');
    execFileSync(checkedJavac(), ['--release', RELEASE, '-encoding', 'UTF-8', '-proc:none', '-Xlint:all,-options,-serial', '-Werror', '-classpath', classpath, '-d', classes, ...sources(join(BUNDLE_SOURCE, 'src'))], { stdio: ['ignore', 'inherit', 'inherit'], timeout: 180_000 });
    cpSync(join(BUNDLE_SOURCE, 'plugin.xml'), join(classes, 'plugin.xml'));
    const output = join(extensionsDirectory, BUNDLE_DIRECTORY);
    mkdirSync(output, { recursive: true });
    const jar = join(output, bundleFileName());
    rmSync(jar, { force: true });
    execFileSync(tool('jar'), ['--create', '--file', jar, '--manifest', join(BUNDLE_SOURCE, 'META-INF/MANIFEST.MF'), `--date=${BUNDLE_DATE}`, '-C', classes, '.'], { stdio: ['ignore', 'inherit', 'inherit'], timeout: 60_000 });
    return jar;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const [jdtlsHome, destination] = process.argv.slice(2);
  if (!jdtlsHome || !destination) {
    process.stderr.write('Usage: node scripts/package/project-model-bundle.mjs <jdtls-home> <jdtls-extensions-dir>\n');
    process.exit(2);
  }
  const jar = buildProjectModelBundle(resolve(jdtlsHome), resolve(destination));
  process.stdout.write(`${relative(process.cwd(), jar).split(sep).join('/')}\n`);
}
