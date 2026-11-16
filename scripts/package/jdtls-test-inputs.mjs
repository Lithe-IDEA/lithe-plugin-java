// What an integration test passes for the user's own JDT LS and the JDK it
// runs on, which the package does not ship. LITHE_TEST_JDTLS_HOME names the
// JDT LS folder (default: the tested release scripts/prepare-jdtls.sh
// prepared in .artifacts/jdtls); LITHE_TEST_JDTLS_JAVA names a Java 21+
// executable (default: JAVA_HOME's, then java on PATH). The launch answers the
// jdtls-runtime tool, which the host would resolve; tests resolve it here.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
export const SERVER_RUNTIME_TOOL='jdtls-runtime';
export function serverInputs(){
 const home=resolve(process.env.LITHE_TEST_JDTLS_HOME??join(root,'.artifacts/jdtls'));
 assert.ok(existsSync(join(home,'plugins')),`No JDT LS installation at ${home}; run scripts/prepare-jdtls.sh or set LITHE_TEST_JDTLS_HOME`);
 const exe=process.platform==='win32'?'java.exe':'java';
 const java=process.env.LITHE_TEST_JDTLS_JAVA??(process.env.JAVA_HOME?join(process.env.JAVA_HOME,'bin',exe):'java');
 // `java -version` prints to stderr; the pattern is the tool definition's.
 const probe=spawnSync(java,['-version'],{encoding:'utf8',timeout:15000,windowsHide:true});
 assert.equal(probe.status,0,`${java} -version failed: ${probe.error?.message??probe.stderr}`);
 const version=/version "(?:1\.)?(\d+(?:\.\d+)*)/.exec(probe.stderr)?.[1];
 assert.ok(version && Number(version.split('.')[0])>=21,`JDT LS needs Java 21 or later, ${java} is ${version}`);
 return {home,java,tools:{[SERVER_RUNTIME_TOOL]:{executable:java,version}}};
}
// The program a launch answer runs: the runtime tool resolves to the test JDK.
export function answeredExecutable(launch,inputs){
 assert.deepEqual(launch.executable,{kind:'tool',tool:SERVER_RUNTIME_TOOL});
 return inputs.java;
}
