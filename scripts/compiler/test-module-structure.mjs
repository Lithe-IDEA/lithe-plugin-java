// The project structure page (1lck/Lithe-IDEA#2086) run through the packaged
// backend and the user's JDT LS: two Eclipse projects and a Maven module.
// The page must list them as modules, show the Maven module read-only with
// the classpath JDT resolved, and save an edit of the Eclipse module's source
// folders, output folder and dependencies (another module, exported, and a
// JAR outside the workspace) to its .classpath, keeping what the page does
// not show. JDT must then compile the module against the new classpath.
// Reading the page writes nothing, and the package and the JDT LS
// installation stay unchanged.
// Usage: node scripts/compiler/test-module-structure.mjs <package>
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {existsSync,lstatSync,mkdirSync,mkdtempSync,readFileSync,readdirSync,realpathSync,renameSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname,join,resolve} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {Client} from '../package/lsp-client.mjs';
import {answeredExecutable,serverInputs} from '../package/jdtls-test-inputs.mjs';
import {createReport} from '../report.mjs';
import {capture} from './capture.mjs';
import {runTool} from './process.mjs';

const pkg=resolve(process.argv[2]??'.artifacts/package/arm64/io.github.lithe-idea.java-support');
const manifest=JSON.parse(readFileSync(join(pkg,'plugin.json')));
const inputs=serverInputs();
const javac=process.env.LITHE_TEST_JAVAC??'javac',jarTool=process.env.LITHE_TEST_JAR??'jar';
const write=(path,content)=>{mkdirSync(dirname(path),{recursive:true});writeFileSync(path,content);};
const OUTPUT=new Set(['bin','build','target','classes']);
// Files below `directory`, with content digests; build output, JDT's own
// metadata and the named files are left out.
const snapshot=(directory,skip=OUTPUT)=>{const items=[];const visit=(path,label)=>{const stat=lstatSync(path);assert.equal(stat.isSymbolicLink(),false);items.push([label,stat.isFile()?createHash('sha256').update(readFileSync(path)).digest('hex'):null]);if(stat.isDirectory())for(const name of readdirSync(path).sort())if(!skip.has(name))visit(join(path,name),`${label}/${name}`);};visit(directory,'');return items;};
const samePath=(left,right)=>process.platform==='win32'?resolve(left).toLowerCase()===resolve(right).toLowerCase():resolve(left)===resolve(right);
const slashes=path=>path.replaceAll('\\','/');
async function until(get,accept,label,timeout=120000){const end=Date.now()+timeout;let last;while(Date.now()<end){last=await get();if(accept(last))return last;await delay(250);}throw Error(`${label}: ${JSON.stringify(last)}`);}
async function jar(stage,name,files,output){
 const directory=join(stage,`jar-${name}`);
 for(const[path,source]of Object.entries(files))write(join(directory,path),source);
 await runTool(javac,['--release','8','-d',directory,...Object.keys(files).map(path=>join(directory,path))],{timeout:60000});
 mkdirSync(dirname(output),{recursive:true});
 await runTool(jarTool,['--create','--file',output,'-C',directory,'.'],{timeout:60000});
}
const PROJECT=name=>`<projectDescription><name>${name}</name><buildSpec><buildCommand><name>org.eclipse.jdt.core.javabuilder</name></buildCommand></buildSpec><natures><nature>org.eclipse.jdt.core.javanature</nature></natures></projectDescription>`;
const JRE='org.eclipse.jdt.launching.JRE_CONTAINER';

const report=createReport('module-structure');
const packageBaseline=snapshot(pkg,new Set()),serverBaseline=snapshot(inputs.home,new Set());
const stage=realpathSync.native(mkdtempSync(join(tmpdir(),'lithe-java-modules-')));
let backend,server;
try{
 const root=join(stage,'workspace');mkdirSync(root);
 const vendor=join(stage,'vendor/acme-util.jar');
 await jar(stage,'acme',{'com/acme/Util.java':'package com.acme; public class Util { public static String value() { return "acme"; } }'},vendor);
 const common=join(root,'common'),app=join(root,'app'),service=join(root,'service');
 write(join(common,'.project'),PROJECT('common'));
 write(join(common,'.classpath'),`<classpath><classpathentry kind="src" path="src"/><classpathentry kind="con" path="${JRE}"/><classpathentry kind="output" path="bin"/></classpath>`);
 write(join(common,'src/com/common/Greeter.java'),'package com.common; public class Greeter { public static String greet() { return "hi"; } }');
 await jar(stage,'old',{'com/old/Old.java':'package com.old; public class Old { }'},join(app,'lib/old.jar'));
 write(join(app,'.project'),PROJECT('app'));
 const original=`<?xml version="1.0" encoding="UTF-8"?>\n<classpath>\n\t<classpathentry kind="src" path="src"/>\n\t<classpathentry kind="con" path="${JRE}"/>\n\t<!-- keep this comment -->\n\t<classpathentry kind="lib" path="lib/old.jar" sourcepath="lib/old-sources.jar"/>\n\t<classpathentry kind="output" path="bin"/>\n</classpath>\n`;
 write(join(app,'.classpath'),original);
 write(join(app,'src/demo/App.java'),'package demo; public class App { public static void main(String[] args) { System.out.print(com.common.Greeter.greet() + com.acme.Util.value() + demo.extra.Extra.NAME); } }');
 write(join(app,'extra/demo/extra/Extra.java'),'package demo.extra; public class Extra { public static final String NAME = "extra"; }');
 write(join(service,'pom.xml'),'<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>test.lithe</groupId><artifactId>service</artifactId><version>1</version><properties><maven.compiler.source>17</maven.compiler.source><maven.compiler.target>17</maven.compiler.target></properties></project>');
 write(join(service,'src/main/java/demo/Service.java'),'package demo; public class Service { }');

 const context={contextID:'ctx-modules',generation:1};let op=0,configuration={};
 const folders=[{rootID:'root',name:'workspace',path:root,uri:pathToFileURL(root).href}];
 const hostRequests={
  'lithe/host/workspace/getContext':()=>({context,...context,folders}),
  'lithe/host/workspace/getFolders':()=>({folders}),
  'lithe/host/functions/providers':()=>({providers:[]}),
  'lithe/host/languageServer/request':async p=>({value:await server.request(p.method,p.params,p.timeoutMilliseconds??60000)}),
  'lithe/host/languageServer/notify':p=>{if(p.method==='workspace/didChangeConfiguration')configuration=p.params.settings;server.notify(p.method,p.params);return {};},
 };
 backend=new Client(join(pkg,manifest.backend.launch.path),[],{hostRequests});
 await backend.request('initialize',{processId:process.pid,rootUri:pathToFileURL(root).href,capabilities:{experimental:{litheFunctions:{versions:[0]},litheLogging:{versions:[0]}}}});backend.notify('initialized',{});
 const params={serverID:'java',platform:{os:{darwin:'macos',win32:'windows'}[process.platform]??process.platform,arch:process.env.LITHE_TEST_ARCH??(process.arch==='arm64'?'arm64':'x86_64')},settings:{logLevel:'WARNING',languageServerHome:inputs.home},tools:inputs.tools};
 for(const[key,directory]of Object.entries({stateDirectory:'state',workspaceStateDirectory:'workspace-state',logDirectory:'logs',temporaryDirectory:'tmp'})){params[key]=join(stage,directory);mkdirSync(params[key]);}
 const invoke=(functionID,args,timeout=120000)=>backend.request('lithe/function/invoke',{functionID,operationID:`modules-${++op}`,params:args,context},timeout);
 const launch=await invoke('java.resolveLaunch',params);configuration=launch.workspaceConfiguration;
 const problems=new Map();
 server=new Client(answeredExecutable(launch,inputs),launch.arguments,{notifications:(method,p)=>{if(method==='textDocument/publishDiagnostics')problems.set(p.uri,p.diagnostics.filter(item=>item.severity===1).map(item=>item.message));},env:{...process.env,...launch.environment},hostRequests:{'workspace/configuration':p=>p.items.map(item=>item.section?item.section.split('.').reduce((value,key)=>value?.[key],configuration)??null:configuration)}});
 await server.request('initialize',{processId:process.pid,rootUri:pathToFileURL(root).href,workspaceFolders:[{uri:pathToFileURL(root).href,name:'workspace'}],capabilities:{workspace:{configuration:true,workspaceFolders:true},textDocument:{publishDiagnostics:{}}},initializationOptions:launch.initializationOptions},180000);server.notify('initialized',{});
 backend.notify('lithe/host/languageServer/didChangeState',{context,serverID:'java',state:'running',generation:1});
 const execute=(command,args=[],timeout=60000)=>server.request('workspace/executeCommand',{command,arguments:args},timeout);
 const page=language=>invoke('libraries.settings',{context:{language,builds:[]}});
 const module=(snapshot,title)=>snapshot.sources.find(source=>source.group==='modules'&&source.title===title);
 const field=(source,id)=>source.fields.find(item=>item.id===id);

 await report.step('JDT imports the two Eclipse projects and the Maven module',async()=>{
  await until(()=>execute('java.project.getAll'),projects=>[common,app,service].every(project=>projects.some(uri=>samePath(fileURLToPath(uri),project))),'JDT project import',240000);
 });
 // JDT LS adds its resource filter to an Eclipse .project on import.
 const projectFiles=()=>snapshot(root,new Set([...OUTPUT,'.project','.settings']));
 let before;
 await report.step('the structure page lists the modules and writes nothing',async()=>{
  const baseline=projectFiles();
  before=await page('zh-CN');
  assert.deepEqual(before.groups[0],{id:'modules',title:'模块',layout:'sidebar'},JSON.stringify(before.groups));
  const appModule=module(before,'app');assert.ok(appModule,JSON.stringify(before.sources.map(source=>[source.group,source.title])));
  assert.equal(appModule.writable,true);
  assert.equal(appModule.path,'app/.classpath');
  assert.deepEqual(field(appModule,'roots').value.map(entry=>entry.value),['src']);
  assert.equal(field(appModule,'jre').value,JRE);
  assert.deepEqual(field(appModule,'dependencies').value.map(entry=>`${entry.kind}:${entry.value}`),['archive:lib/old.jar']);
  const moduleChoices=field(appModule,'dependencies').entryKinds.find(kind=>kind.id==='module').choices.map(choice=>choice.value);
  assert.deepEqual(moduleChoices,['/common']);
  assert.equal(module(before,'common').writable,true);
  const serviceModule=module(before,'service');
  assert.ok(serviceModule,JSON.stringify(before.sources.map(source=>source.title)));
  assert.equal(serviceModule.writable,false);
  assert.equal(serviceModule.path,'service/pom.xml');
  const resolved=field(serviceModule,'dependencies').value;
  assert.ok(resolved.some(entry=>entry.kind==='classes'&&slashes(entry.value).endsWith('target/classes')),JSON.stringify(resolved));
  assert.deepEqual(projectFiles(),baseline,'Reading the structure page must not write project files');
  // The public projection, rendered by the native host in scripts/compiler/visual.mjs.
  capture('libraries-modules-zh',before);
  capture('libraries-modules-en',await page('en'));
 });
 await report.step('saving the module writes its .classpath and keeps what the page does not show',async()=>{
  const appModule=module(before,'app');
  const stageFile=join(stage,'classpath-stage.xml');writeFileSync(stageFile,'');
  const edit=await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:appModule.id,revision:appModule.revision,outputFile:stageFile,values:{
   roots:[{kind:'folder',value:'src'},{kind:'folder',value:join(app,'extra')}],
   output:'build/classes',
   dependencies:[{kind:'module',value:'/common',attributes:{exported:true}},{kind:'archive',value:'lib/old.jar'},{kind:'archive',value:vendor}],
  }});
  assert.equal(edit.changed,true);
  const text=readFileSync(stageFile,'utf8');
  for(const kept of['<!-- keep this comment -->','sourcepath="lib/old-sources.jar"','path="extra"','path="build/classes"','exported="true"','combineaccessrules="false"',`path="${slashes(realpathSync.native(vendor))}"`])assert.ok(text.includes(kept),`${kept}\n${text}`);
  assert.equal(snapshot(root).find(([label])=>label==='/app/.classpath')[1],createHash('sha256').update(original).digest('hex'),'Preparing an edit must not touch the file');
  renameSync(stageFile,join(app,'.classpath'));
  const after=await invoke('libraries.settings',{operation:'readAfterSave',context:{language:'en',builds:[]}});
  const saved=module(after,'app');
  assert.deepEqual(field(saved,'roots').value.map(entry=>entry.value),['src','extra']);
  assert.equal(field(saved,'output').value,'build/classes');
  assert.deepEqual(field(saved,'dependencies').value.map(entry=>[entry.kind,entry.title,entry.attributes?.exported??false]),[['module','common',true],['archive','old.jar',false],['archive','acme-util.jar',false]]);
 });
 await report.step('JDT compiles the module against the saved classpath',async()=>{
  const errors=()=>JSON.stringify([...problems].filter(([,messages])=>messages.length));
  // A full build cleans every project first and may build the module before
  // the module it now depends on; the incremental builds that follow, as
  // JDT LS's own auto-build would run them, settle it.
  let full=true;
  const build=()=>{const request=server.request('java/buildWorkspace',full,180000);full=false;return request;};
  await until(build,status=>status===1,'JDT build after the .classpath edit',60000).catch(error=>{throw Error(`${error.message}: ${errors()}`);});
  assert.ok(existsSync(join(app,'build/classes/demo/App.class')),'demo/App.class is not in the saved output folder');
  assert.ok(existsSync(join(app,'build/classes/demo/extra/Extra.class')),'the added source folder was not compiled');
 });
 await server.request('shutdown',null,30000);server.notify('exit',null);
 await report.step('the package and the JDT LS installation stay unchanged',async()=>{
  assert.deepEqual(snapshot(pkg,new Set()),packageBaseline,'The structure page must never write into the installed package');
  assert.deepEqual(snapshot(inputs.home,new Set()),serverBaseline,'The structure page must never write into the user\'s JDT LS installation');
 });
}finally{
 try{if(server)await server.close();}finally{try{if(backend)await backend.close();}finally{rmSync(stage,{recursive:true,force:true});report.write();}}
}
