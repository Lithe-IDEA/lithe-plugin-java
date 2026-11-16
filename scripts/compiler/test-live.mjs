// The user's JDT LS + packaged SDK backend, with bounded process ownership. This tests
// native sources reaching the engine rather than only checking RPC invocation.
import assert from 'node:assert/strict';
import {createReport} from '../report.mjs';
import {capture} from './capture.mjs';
import {mkdtempSync,realpathSync,mkdirSync,rmSync,readFileSync,writeFileSync,renameSync,readdirSync,lstatSync,existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {runTool} from './process.mjs';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {Client} from '../package/lsp-client.mjs';
import {serverInputs,answeredExecutable} from '../package/jdtls-test-inputs.mjs';
const pkg=resolve(process.argv[2]??'.artifacts/package/arm64/io.github.lithe-idea.java-support');
const manifest=JSON.parse(readFileSync(join(pkg,'plugin.json')));
const stage=realpathSync.native(mkdtempSync(join(tmpdir(),'lithe-java-native-')));
const root=join(stage,'workspace');mkdirSync(root);let backend,server,configuration={};
const eclipse=process.env.LITHE_JAVA_PROJECT_KIND==='eclipse';
const sourceFile=join(root,eclipse?'src/Main.java':'Main.java');
if(eclipse){mkdirSync(join(root,'src'));writeFileSync(join(root,'.project'),'<projectDescription><name>FixtureNative</name><buildSpec><buildCommand><name>org.eclipse.jdt.core.javabuilder</name></buildCommand></buildSpec><natures><nature>org.eclipse.jdt.core.javanature</nature></natures></projectDescription>');writeFileSync(join(root,'.classpath'),'<classpath><classpathentry kind="src" path="src"/><classpathentry kind="con" path="org.eclipse.jdt.launching.JRE_CONTAINER"/><classpathentry kind="output" path="bin"/></classpath>');}
const context={contextID:'ctx-1',generation:1};let op=0;
const samePath=(left,right)=>process.platform==='win32'?resolve(left).toLowerCase()===resolve(right).toLowerCase():resolve(left)===resolve(right);
const snapshot=root=>{const items=[];const visit=(path,label)=>{const stat=lstatSync(path);assert.equal(stat.isSymbolicLink(),false);items.push([label,stat.mode,stat.isFile()?createHash('sha256').update(readFileSync(path)).digest('hex'):null]);if(stat.isDirectory())for(const name of readdirSync(path).sort())visit(join(path,name),`${label}/${name}`);};visit(root,'');return items;};
const inputs=serverInputs();
// LITHE_TEST_PROJECT_JAVA_HOME stands for the project JDK the host resolves for
// the java tool; without it JDT compiles against the JDK that runs it.
const projectJava=()=>{const home=process.env.LITHE_TEST_PROJECT_JAVA_HOME;if(!home)return {};const probe=spawnSync(join(home,'bin',process.platform==='win32'?'java.exe':'java'),['-version'],{encoding:'utf8',timeout:15000});const version=/version "(?:1\.)?(\d+(?:\.\d+)*)/.exec(probe.stderr)?.[1];assert.ok(version,`Cannot read the version of ${home}`);return {java:{executable:join(home,'bin','java'),home,version}};};
const baseline=snapshot(pkg);const serverBaseline=snapshot(inputs.home);
const folders=[{rootID:'root',name:'workspace',path:root,uri:pathToFileURL(root).href}];
const execute=(command,args=[])=>server.request('workspace/executeCommand',{command,arguments:args});
const report=createReport(eclipse?'native-jdt-eclipse':'native-jdt-unmanaged');
const step=report.step;
async function until(get,accept,label){const end=Date.now()+45000;let last;while(Date.now()<end){last=await get();if(accept(last))return last;await delay(100);}throw Error(`${label}: ${JSON.stringify(last)}`);}
try{
 const java=process.env.LITHE_TEST_JAVAC??'javac';
 const jarRoot=join(stage,'jar');mkdirSync(jarRoot);writeFileSync(join(jarRoot,'Library.java'),'package fixture; public class Library { public static String value() { return "jar-ok"; } }');await runTool(java,['--release','8','-d',jarRoot,join(jarRoot,'Library.java')],{timeout:30000});const jar=join(stage,'External Library.jar');await runTool(process.env.LITHE_TEST_JAR??'jar',['--create','--file',jar,'-C',jarRoot,'fixture'],{timeout:30000});
 writeFileSync(sourceFile,'public class Main { public static void main(String[] args) { System.out.print(fixture.Library.value()); } }\n');
 const hostRequests={
  'lithe/host/workspace/getContext':()=>({context,...context,folders}),
  'lithe/host/workspace/getFolders':()=>({folders}),
  'lithe/host/functions/providers':()=>({providers:[]}),
  'lithe/host/languageServer/request':async p=>({value:await server.request(p.method,p.params,p.timeoutMilliseconds??60000)}),
  'lithe/host/languageServer/notify':p=>{if(p.method==='workspace/didChangeConfiguration')configuration=p.params.settings;server.notify(p.method,p.params);return {};},
 };
 backend=new Client(join(pkg,manifest.backend.launch.path),[],{hostRequests});
 await backend.request('initialize',{processId:process.pid,rootUri:pathToFileURL(root).href,capabilities:{experimental:{litheFunctions:{versions:[0]},litheLogging:{versions:[0]}}}});backend.notify('initialized',{});
 const params={serverID:'java',platform:{os:{darwin:'macos',win32:'windows'}[process.platform]??process.platform,arch:process.env.LITHE_TEST_ARCH??(process.arch==='arm64'?'arm64':'x86_64')},settings:{logLevel:'WARNING',languageServerHome:inputs.home},tools:{...inputs.tools,...projectJava()}};
 for(const[key,name]of Object.entries({stateDirectory:'state',workspaceStateDirectory:'workspace-state',logDirectory:'logs',temporaryDirectory:'tmp'})){params[key]=join(stage,name);mkdirSync(params[key]);}
 const launch=await backend.request('lithe/function/invoke',{functionID:'java.resolveLaunch',operationID:`native-${++op}`,params,context});configuration=launch.workspaceConfiguration;
 server=new Client(answeredExecutable(launch,inputs),launch.arguments,{env:{...process.env,...launch.environment},hostRequests:{'workspace/configuration':p=>p.items.map(item=>item.section?item.section.split('.').reduce((value,key)=>value?.[key],configuration)??null:configuration)}});
 await server.request('initialize',{processId:process.pid,rootUri:pathToFileURL(root).href,workspaceFolders:[{uri:pathToFileURL(root).href,name:'workspace'}],capabilities:{workspace:{configuration:true,workspaceFolders:true},textDocument:{publishDiagnostics:{}}},initializationOptions:launch.initializationOptions},120000);server.notify('initialized',{});
 backend.notify('lithe/host/languageServer/didChangeState',{context,serverID:'java',state:'running',generation:1});
 const invoke=(functionID,params)=>backend.request('lithe/function/invoke',{functionID,operationID:`native-${++op}`,params,context},90000);
 let sources;
 await step('Discover the project from the real JDT server',async()=>{await until(()=>execute('java.project.getAll'),projects=>projects.some(uri=>(process.platform==='win32'?fileURLToPath(uri).replace(/[\\/]$/,'').toLowerCase()===root.toLowerCase():fileURLToPath(uri).replace(/\/$/,'')===root)),'JDT project import');sources=await invoke('libraries.settings',{context:{language:'zh-CN',builds:[]}});assert.equal(sources.sources[0].path,eclipse?'.classpath':'.vscode/settings.json');});
 await step('Save a referenced JAR and confirm JDT resolves the actual classpath',async()=>{const source=sources.sources[0],stageFile=join(stage,'libraries-stage.json');writeFileSync(stageFile,'');await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:source.id,revision:source.revision,values:{libraries:[jar]},outputFile:stageFile});mkdirSync(join(root,'.vscode'),{recursive:true});renameSync(stageFile,join(root,source.path));await invoke('libraries.settings',{operation:'readAfterSave',context:{language:'zh-CN',builds:[]}});if(!eclipse)await until(()=>execute('java.project.getSettings',[pathToFileURL(root).href,['org.eclipse.jdt.ls.core.referencedLibraries']]),value=>value['org.eclipse.jdt.ls.core.referencedLibraries']?.some(path=>samePath(path,jar)),'JDT classpath refresh');});
 await step('Save native compiler target and verify real JDT effective settings',async()=>{sources=await invoke('compiler.settings',{context:{language:'zh-CN',builds:[]}});const source=sources.sources.find(s=>s.id==='defaults'),stageFile=join(stage,'compiler-stage.prefs');writeFileSync(stageFile,'');await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:source.id,revision:source.revision,values:{target:'8'},outputFile:stageFile});mkdirSync(join(root,'.settings'),{recursive:true});renameSync(stageFile,join(root,source.path));await invoke('compiler.settings',{operation:'readAfterSave',context:{language:'zh-CN',builds:[]}});await until(()=>execute('java.project.getSettings',[pathToFileURL(root).href,['org.eclipse.jdt.core.compiler.codegen.targetPlatform']]),value=>value['org.eclipse.jdt.core.compiler.codegen.targetPlatform']==='1.8','JDT compiler refresh');});
 await step('Compile with JDT, inspect Java 8 bytecode and run with the external JAR',async()=>{
  const result=await execute('vscode.java.buildWorkspace',[JSON.stringify({mainClass:'Main',filePath:sourceFile,isFullBuild:true})]);assert.equal(result,1);
  const paths=await execute('vscode.java.resolveClasspath',['Main','']);
  const classes=paths.flat().find(path=>typeof path==='string'&&existsSync(join(path,'Main.class')));assert.ok(classes,JSON.stringify(paths));
  assert.equal(readFileSync(join(classes,'Main.class')).readUInt16BE(6),52);
  const output=await runTool(process.env.LITHE_TEST_JAVA??'java',['-cp',classes+(process.platform==='win32'?';':':')+jar,'Main'],{timeout:30000});assert.equal(output.toString(),'jar-ok');
 });
 capture('zh',await invoke('compiler.settings',{context:{language:'zh-CN',builds:[]}}));capture('en',await invoke('compiler.settings',{context:{language:'en',builds:[]}}));
 assert.deepEqual(snapshot(pkg),baseline,'Native settings and server startup must never mutate the installed package');
 assert.deepEqual(snapshot(inputs.home),serverBaseline,'Server startup must never write into the user\'s JDT LS installation');
 await server.request('shutdown',null);server.notify('exit',null);
}finally{
 try{if(server)await server.close();}finally{try{if(backend)await backend.close();}finally{report.write();rmSync(stage,{recursive:true,force:true});}}
}
