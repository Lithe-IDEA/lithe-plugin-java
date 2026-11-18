// Executes the Java-owned integration fixture against a corresponding host
// checkout without writing test code into that repository. Go's overlay is
// temporary; package, preferences, runtime cache and processes are test-owned.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {existsSync,mkdtempSync,mkdirSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const [hostArg,archiveArg]=process.argv.slice(2);
assert.ok(hostArg&&archiveArg,'Usage: node scripts/package/test-host-managed.mjs <host-checkout> <java-plugin.zip>');
assert.equal(process.platform,'linux');
assert.equal(process.arch,'x64');
const host=resolve(hostArg),archive=resolve(archiveArg);
const jdk=process.env.LITHE_MANAGED_TEST_JDK_ARCHIVE??join(root,'.artifacts/managed-jdk/microsoft-jdk-25.0.5.0.2-linux-x64.tar.gz');
const pins=JSON.parse(readFileSync(join(root,'third_party/jdtls/manifest.json')));
const server=process.env.LITHE_MANAGED_TEST_JDTLS_ARCHIVE??join(root,`.artifacts/jdtls-downloads/jdtls-${pins.version}-${pins.archiveSHA256}.tar.gz`);
assert.ok(existsSync(jdk)&&existsSync(server),'Download the exact official archives before running this integration lane');
mkdirSync(join(root,'.artifacts'),{recursive:true});
const stage=mkdtempSync(join(root,'.artifacts/host-managed-'));
try{
 const pkgRoot=join(stage,'package');mkdirSync(pkgRoot);
 execFileSync('unzip',['-q',archive,'-d',pkgRoot],{timeout:120000});
 const overlay=join(stage,'overlay.json');
 writeFileSync(overlay,JSON.stringify({Replace:{[join(host,'desktop/internal/pluginservice/zz_java_managed_integration_test.go')]:join(root,'scripts/package/host-managed-runtime_test.go')}}));
 execFileSync(join(host,'scripts/desktop/desktop-go.sh'),['test',`-overlay=${overlay}`,'./internal/pluginservice','-run','^TestJavaManagedRuntimeOfficialArchives$','-count=1','-v','-timeout=240s'],{
  cwd:host,stdio:'inherit',timeout:600000,
  env:{...process.env,LITHE_MANAGED_TEST_PACKAGE:join(pkgRoot,'io.github.lithe-idea.java-support'),LITHE_MANAGED_TEST_JDK_ARCHIVE:jdk,LITHE_MANAGED_TEST_JDTLS_ARCHIVE:server},
 });
}finally{rmSync(stage,{recursive:true,force:true});}
