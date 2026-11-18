// Native integration test of the extracted archive and its read-only boundary.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync,mkdirSync,rmSync,readFileSync,readdirSync,lstatSync,readlinkSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import {Client} from './lsp-client.mjs';
import {serverInputs,answeredExecutable,preparedRuntimes} from './jdtls-test-inputs.mjs';
const archive=resolve(process.argv[2]);
assert.equal(process.platform,'linux');
const inputs=serverInputs();
const stage=mkdtempSync(join(tmpdir(),'lithe-java-linux-'));let backend,server;
const serverBefore=()=>inventory(inputs.home);
const inventory=root=>{
 const entries=[];const walk=(path,relative)=>{const s=lstatSync(path);entries.push([relative,s.mode,s.isSymbolicLink()?readlinkSync(path):s.isFile()?createHash('sha256').update(readFileSync(path)).digest('hex'):null]);if(s.isDirectory())for(const name of readdirSync(path).sort())walk(join(path,name),`${relative}/${name}`);};walk(root,'');return entries;
};
try{
 execFileSync('unzip',['-q',archive,'-d',stage],{timeout:120000});
 const pkg=join(stage,'io.github.lithe-idea.java-support');const before=inventory(pkg);const serverBaseline=serverBefore();const jdkBaseline=inventory(inputs.jdkHome);
 const manifest=JSON.parse(readFileSync(join(pkg,'plugin.json')));
 const workspace=join(stage,'workspace');mkdirSync(workspace);writeFileSync(join(workspace,'Hello.java'),'class Hello { String value; }\n');
 const params={serverID:'java',platform:{os:'linux',arch:process.arch==='arm64'?'arm64':'x86_64'},settings:{logLevel:'WARNING'},tools:inputs.tools};
 for(const [key,name] of Object.entries({stateDirectory:'state',workspaceStateDirectory:'workspace-state',logDirectory:'logs',temporaryDirectory:'tmp'})){params[key]=join(stage,name);mkdirSync(params[key]);}
 backend=new Client(join(pkg,'bin/lithe-java-backend'));
 const initialized=await backend.request('initialize',{rootUri:pathToFileURL(workspace).href,capabilities:{experimental:{litheFunctions:{versions:[0]},litheLogging:{versions:[0]}}}});
 assert.ok(initialized.capabilities.experimental.litheFunctions);
 backend.notify('initialized',{});
 const first=performance.now();
 params.runtimes=await preparedRuntimes(backend,manifest,params.platform,inputs,'linux-prepare-first');
 const qualifiedMilliseconds=performance.now()-first;
 const again=performance.now();
 assert.deepEqual(await preparedRuntimes(backend,manifest,params.platform,inputs,'linux-prepare-again'),params.runtimes);
 const repeatedMilliseconds=performance.now()-again;
 const launch=await backend.request('lithe/function/invoke',{functionID:'java.resolveLaunch',operationID:'linux-smoke',params});
 assert.ok(launch.writablePaths.every(path=>!path.startsWith(pkg+'/')&&!path.startsWith(inputs.home+'/')));
 const runtime=answeredExecutable(launch,inputs);
 server=new Client(runtime,launch.arguments,{env:{...process.env,...launch.environment}});
 const result=await server.request('initialize',{processId:null,rootUri:pathToFileURL(workspace).href,workspaceFolders:[{uri:pathToFileURL(workspace).href,name:'workspace'}],capabilities:{},initializationOptions:launch.initializationOptions},120000);
 assert.ok(result.capabilities.definitionProvider);server.notify('initialized',{});
 await server.request('shutdown',null,30000);server.notify('exit',null);await server.close();server=undefined;
 await backend.close();backend=undefined;
 assert.deepEqual(inventory(pkg),before,'LSP startup must not change installed package files or permissions');
 assert.deepEqual(serverBefore(),serverBaseline,'LSP startup must not write into the user\'s JDT LS installation');
 assert.deepEqual(inventory(inputs.jdkHome),jdkBaseline,'LSP startup must not change the full JDK');
 console.log(`Verified Linux ${process.arch}: repeatable managed-runtime qualification (${qualifiedMilliseconds.toFixed(1)} / ${repeatedMilliseconds.toFixed(1)} ms), approved-map launch, initialize/shutdown, immutable package, JDT LS and full JDK`);
}finally{
 try{if(server)await server.close();}finally{try{if(backend)await backend.close();}finally{rmSync(stage,{recursive:true,force:true});}}
}
