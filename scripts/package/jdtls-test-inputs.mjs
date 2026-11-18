// What an integration test passes for the separately prepared JDT LS and JDK.
// Neither is stored inside the plugin ZIP. LITHE_TEST_JDTLS_HOME names the
// JDT LS folder (default: the tested release scripts/prepare-jdtls.sh
// prepared in .artifacts/jdtls); LITHE_TEST_JDTLS_JAVA names a full Java 25
// executable (default: JAVA_HOME's, then java on PATH). The launch answers the
// jdtls-runtime tool, which the host would resolve; tests resolve it here.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync,realpathSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {prepareTestJDK} from './prepare-test-jdk.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
export const SERVER_RUNTIME_TOOL='jdtls-runtime';
export function serverInputs(){
 const home=resolve(process.env.LITHE_TEST_JDTLS_HOME??join(root,'.artifacts/jdtls'));
 assert.ok(existsSync(join(home,'plugins')),`No JDT LS installation at ${home}; run scripts/prepare-jdtls.sh or set LITHE_TEST_JDTLS_HOME`);
 const exe=process.platform==='win32'?'java.exe':'java';
 let java=process.env.LITHE_TEST_JDTLS_JAVA??(process.env.JAVA_HOME?join(process.env.JAVA_HOME,'bin',exe):'java');
 // `java -version` prints to stderr; the pattern is the tool definition's.
 const run=()=>spawnSync(java,['-XshowSettings:properties','-version'],{encoding:'utf8',timeout:15000,windowsHide:true});
 let probe=run();
 // Honor explicit test input failures. Without one, prepare the manifest pin
 // when the existing runner only has the JDK used for extension compilation.
 if(!process.env.LITHE_TEST_JDTLS_JAVA&&(probe.status!==0||!/version "25(?:[.\-+_"])/.test(probe.stderr))){java=prepareTestJDK();probe=run();}
 assert.equal(probe.status,0,`${java} -version failed: ${probe.error?.message??probe.stderr}`);
 const version=/version "(?:1\.)?(\d+(?:\.\d+)*)/.exec(probe.stderr)?.[1];
 assert.ok(version && Number(version.split('.')[0])===25,`This verified JDT LS environment uses Java 25, ${java} is ${version}`);
 const jdkHome=/^\s*java\.home = (.+)$/m.exec(probe.stderr)?.[1]?.trim();
 assert.ok(jdkHome&&existsSync(join(jdkHome,'bin',process.platform==='win32'?'javac.exe':'javac')),'The language service needs a full JDK 25');
 return {home,java,jdkHome:realpathSync(jdkHome),tools:{[SERVER_RUNTIME_TOOL]:{executable:java,home:realpathSync(jdkHome),version}}};
}

// Exercise the same read-only hook the host calls before downloading anything.
// The caller owns fetching and integrity checking; these paths are already
// prepared inputs. Using the returned map also tests the normal launch path.
export async function preparedRuntimes(backend,manifest,platform,inputs,operationID){
 const dependencies=manifest.managedRuntimes.dependencies.map(dependency=>{
  const artifact=dependency.artifacts.find(value=>value.os===platform.os&&value.arch===platform.arch);
  assert.ok(artifact,`No managed runtime for ${platform.os}/${platform.arch}`);
  const {artifacts,...declaration}=dependency;
  return {...declaration,...artifact};
 });
 const params={platform,dependencies,candidates:{jdk:inputs.jdkHome,jdtls:inputs.home},tools:inputs.tools,settings:{},language:'en'};
 const result=await backend.request('lithe/function/invoke',{functionID:manifest.managedRuntimes.resolve.function,operationID,params});
 assert.deepEqual(result.missing,[]);
 assert.equal(result.runtimes.length,2);
 const runtimes=Object.fromEntries(result.runtimes.map(({id,...value})=>[id,value]));
 assert.equal(realpathSync(runtimes.jdk.home),inputs.jdkHome);
 assert.equal(realpathSync(runtimes.jdtls.home),realpathSync(inputs.home));
 return runtimes;
}
// The program a launch answer runs: the runtime tool resolves to the test JDK.
export function answeredExecutable(launch,inputs){
 assert.deepEqual(launch.executable,{kind:'tool',tool:SERVER_RUNTIME_TOOL});
 return inputs.java;
}
