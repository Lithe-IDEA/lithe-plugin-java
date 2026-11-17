// Project models JDT LS cannot import on its own (issue 1lck/Lithe-IDEA#2063),
// run through the packaged backend, its project model bundle and the user's
// JDT LS: an Eclipse WTP project with Web App Libraries, Server Runtime and a
// TOMCAT_HOME variable, a legacy web folder with only WEB-INF/lib, and an
// IntelliJ IDEA project with a non-standard source folder and module and
// project libraries. Each must compile with JDT without any project file
// being written, the package and the JDT LS installation must stay unchanged,
// and the libraries page must show the model. The legacy web scenario also
// saves the detected entries through the page and builds again from the file.
// A fake servlet container outside the workspace stands for Tomcat 9.
// Usage: node scripts/compiler/test-project-models.mjs <package> [scenario,...]
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
const SCENARIOS=['eclipse-wtp','legacy-web','idea'];
const selected=(process.argv[3]??SCENARIOS.join(',')).split(',');
for(const name of selected)assert.ok(SCENARIOS.includes(name),`Unknown scenario ${name}`);
const inputs=serverInputs();
const javac=process.env.LITHE_TEST_JAVAC??'javac',jarTool=process.env.LITHE_TEST_JAR??'jar';
const write=(path,content)=>{mkdirSync(dirname(path),{recursive:true});writeFileSync(path,content);};
// Files below `directory`, with content digests; build output folders are left out.
const OUTPUT=new Set(['bin','build','out','classes']);
const snapshot=(directory,skip=OUTPUT)=>{const items=[];const visit=(path,label)=>{const stat=lstatSync(path);assert.equal(stat.isSymbolicLink(),false);items.push([label,stat.isFile()?createHash('sha256').update(readFileSync(path)).digest('hex'):null]);if(stat.isDirectory())for(const name of readdirSync(path).sort())if(!skip.has(name))visit(join(path,name),`${label}/${name}`);};visit(directory,'');return items;};
const samePath=(left,right)=>process.platform==='win32'?resolve(left).toLowerCase()===resolve(right).toLowerCase():resolve(left)===resolve(right);
async function until(get,accept,label,timeout=90000){const end=Date.now()+timeout;let last;while(Date.now()<end){last=await get();if(accept(last))return last;await delay(200);}throw Error(`${label}: ${JSON.stringify(last)}`);}
async function jar(stage,name,files,output){
 const directory=join(stage,`jar-${name}`);
 for(const[path,source]of Object.entries(files))write(join(directory,path),source);
 await runTool(javac,['--release','8','-d',directory,...Object.keys(files).map(path=>join(directory,path))],{timeout:60000});
 mkdirSync(dirname(output),{recursive:true});
 await runTool(jarTool,['--create','--file',output,'-C',directory,'.'],{timeout:60000});
}
const HELLO='package demo;\npublic class Hello extends javax.servlet.http.HttpServlet {\n public static void main(String[] args) { System.out.print(com.acme.Util.value()); }\n}\n';
const ECLIPSE_PROJECT=name=>`<projectDescription><name>${name}</name><buildSpec><buildCommand><name>org.eclipse.jdt.core.javabuilder</name></buildCommand></buildSpec><natures><nature>org.eclipse.jdt.core.javanature</nature><nature>org.eclipse.wst.common.project.facet.core.nature</nature></natures></projectDescription>`;

// Each scenario writes its fixture and returns the source file, the classes
// JDT must see, and the descriptor the model rows are bound to.
const fixtures={
 async 'eclipse-wtp'(root,stage){
  const project=join(root,'WebApp');
  write(join(project,'.project'),ECLIPSE_PROJECT('WebApp'));
  write(join(project,'.classpath'),'<classpath><classpathentry kind="src" path="src"/><classpathentry kind="con" path="org.eclipse.jdt.launching.JRE_CONTAINER"/><classpathentry kind="con" path="org.eclipse.jst.server.core.container/org.eclipse.jst.server.tomcat.runtimeTarget/Apache Tomcat v9.0"/><classpathentry kind="con" path="org.eclipse.jst.j2ee.internal.web.container"/><classpathentry kind="con" path="org.eclipse.jst.j2ee.internal.module.container"/><classpathentry kind="var" path="TOMCAT_HOME/lib/jsp-api.jar"/><classpathentry kind="output" path="build/classes"/></classpath>');
  write(join(project,'.settings/org.eclipse.wst.common.component'),'<?xml version="1.0" encoding="UTF-8"?><project-modules id="moduleCoreId" project-version="1.5.0"><wb-module deploy-name="WebApp"><wb-resource deploy-path="/" source-path="/WebContent" tag="defaultRootSource"/><wb-resource deploy-path="/WEB-INF/classes" source-path="/src"/></wb-module></project-modules>');
  await jar(stage,'util',{'com/acme/Util.java':'package com.acme; public class Util { public static String value() { return "web-ok"; } }'},join(project,'WebContent/WEB-INF/lib/util.jar'));
  write(join(project,'src/demo/Hello.java'),HELLO);
  return {source:join(project,'src/demo/Hello.java'),project,descriptor:'WebApp/.project',model:'Eclipse'};
 },
 async 'legacy-web'(root,stage){
  await jar(stage,'util',{'com/acme/Util.java':'package com.acme; public class Util { public static String value() { return "web-ok"; } }'},join(root,'WebRoot/WEB-INF/lib/util.jar'));
  write(join(root,'WebRoot/WEB-INF/web.xml'),'<?xml version="1.0" encoding="UTF-8"?><web-app xmlns="http://java.sun.com/xml/ns/javaee" version="3.0"/>');
  write(join(root,'src/demo/Hello.java'),HELLO);
  return {source:join(root,'src/demo/Hello.java'),project:root,descriptor:'WebRoot/WEB-INF/web.xml',model:'传统 Web',library:'WebRoot/WEB-INF/lib/*.jar'};
 },
 async idea(root,stage){
  write(join(root,'.idea/modules.xml'),'<?xml version="1.0" encoding="UTF-8"?><project version="4"><component name="ProjectModuleManager"><modules><module fileurl="file://$PROJECT_DIR$/app.iml" filepath="$PROJECT_DIR$/app.iml"/></modules></component></project>');
  write(join(root,'app.iml'),'<?xml version="1.0" encoding="UTF-8"?><module type="JAVA_MODULE" version="4"><component name="NewModuleRootManager" inherit-compiler-output="true"><exclude-output/><content url="file://$MODULE_DIR$"><sourceFolder url="file://$MODULE_DIR$/java" isTestSource="false"/></content><orderEntry type="inheritedJdk"/><orderEntry type="sourceFolder" forTests="false"/><orderEntry type="module-library"><library><CLASSES><root url="jar://$MODULE_DIR$/third-party/util.jar!/"/></CLASSES><JAVADOC/><SOURCES/></library></orderEntry><orderEntry type="library" name="servlet" level="project"/></component></module>');
  write(join(root,'.idea/libraries/servlet.xml'),'<component name="libraryTable"><library name="servlet"><CLASSES><root url="jar://$PROJECT_DIR$/vendor/servlet-api.jar!/"/></CLASSES><JAVADOC/><SOURCES/></library></component>');
  await jar(stage,'util',{'com/acme/Util.java':'package com.acme; public class Util { public static String value() { return "idea-ok"; } }'},join(root,'third-party/util.jar'));
  await jar(stage,'servlet-copy',{'javax/servlet/http/HttpServlet.java':'package javax.servlet.http; public abstract class HttpServlet { }'},join(root,'vendor/servlet-api.jar'));
  write(join(root,'java/demo/Hello.java'),HELLO);
  return {source:join(root,'java/demo/Hello.java'),project:root,descriptor:'.idea/modules.xml',model:'IntelliJ IDEA',library:'third-party/util.jar',sourcePath:'java',projectServlet:true};
 },
};

async function scenario(name,report){
 const step=(label,body)=>report.step(`${name}: ${label}`,body);
 const stage=realpathSync.native(mkdtempSync(join(tmpdir(),`lithe-java-model-${name}-`)));
 const root=join(stage,'workspace');mkdirSync(root);
 let backend,server,configuration={};
 try{
  const tomcat=join(stage,'servers/apache-tomcat-9.0.98');
  await jar(stage,'servlet',{'javax/servlet/http/HttpServlet.java':'package javax.servlet.http; public abstract class HttpServlet { }'},join(tomcat,'lib/servlet-api.jar'));
  await jar(stage,'jsp',{'javax/servlet/jsp/JspPage.java':'package javax.servlet.jsp; public interface JspPage { }'},join(tomcat,'lib/jsp-api.jar'));
  const fixture=await fixtures[name](root,stage);
  // JDT LS itself adds its resource filter (filteredResources) to an Eclipse
  // .project on import; every other file must stay as the fixture wrote it.
  const projectFiles=()=>snapshot(root,new Set([...OUTPUT,...(name==='eclipse-wtp'?['.project']:[])]));
  const projectBaseline=projectFiles();
  const context={contextID:`ctx-${name}`,generation:1};let op=0;
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
  const params={serverID:'java',platform:{os:{darwin:'macos',win32:'windows'}[process.platform]??process.platform,arch:process.env.LITHE_TEST_ARCH??(process.arch==='arm64'?'arm64':'x86_64')},settings:{logLevel:'WARNING',languageServerHome:inputs.home,projectModelDetection:true,servletContainerHome:tomcat},tools:inputs.tools};
  for(const[key,directory]of Object.entries({stateDirectory:'state',workspaceStateDirectory:'workspace-state',logDirectory:'logs',temporaryDirectory:'tmp'})){params[key]=join(stage,directory);mkdirSync(params[key]);}
  const invoke=(functionID,args,timeout=90000)=>backend.request('lithe/function/invoke',{functionID,operationID:`model-${++op}`,params:args,context},timeout);
  const launch=await invoke('java.resolveLaunch',params);configuration=launch.workspaceConfiguration;
  await step('the launch loads the bundle and configures the servlet container',async()=>{
   assert.ok(launch.initializationOptions.bundles.some(bundle=>/io\.github\.lithe_idea\.java\.projectmodel_.*\.jar$/.test(bundle)),JSON.stringify(launch.initializationOptions.bundles));
   const model=configuration.java.lithe?.projectModel;
   // The IDEA fixture takes the servlet API from a project library, so it
   // needs no servlet container.
   if(fixture.projectServlet)assert.equal(model,undefined);
   else assert.ok(model&&samePath(model.serverRuntimeHome,tomcat),JSON.stringify(configuration.java.lithe));
   if(fixture.library)assert.ok(JSON.stringify(configuration.java.project.referencedLibraries).includes(fixture.library),JSON.stringify(configuration.java.project));
   if(fixture.sourcePath)assert.deepEqual(configuration.java.project.sourcePaths,[fixture.sourcePath]);
  });
  server=new Client(answeredExecutable(launch,inputs),launch.arguments,{env:{...process.env,...launch.environment},hostRequests:{'workspace/configuration':p=>p.items.map(item=>item.section?item.section.split('.').reduce((value,key)=>value?.[key],configuration)??null:configuration)}});
  await server.request('initialize',{processId:process.pid,rootUri:pathToFileURL(root).href,workspaceFolders:[{uri:pathToFileURL(root).href,name:'workspace'}],capabilities:{workspace:{configuration:true,workspaceFolders:true},textDocument:{publishDiagnostics:{}}},initializationOptions:launch.initializationOptions},180000);server.notify('initialized',{});
  backend.notify('lithe/host/languageServer/didChangeState',{context,serverID:'java',state:'running',generation:1});
  const execute=(command,args=[],timeout=60000)=>server.request('workspace/executeCommand',{command,arguments:args},timeout);
  const projectUri=pathToFileURL(fixture.project).href;
  const imported=()=>until(()=>execute('java.project.getAll'),projects=>projects.some(uri=>samePath(fileURLToPath(uri),fixture.project)),'JDT project import');
  const compiles=async()=>{
   assert.equal(await server.request('java/buildWorkspace',true,180000),1,'JDT reports build errors');
   const paths=await execute('vscode.java.resolveClasspath',['demo.Hello','']);
   const classes=paths.flat().find(path=>typeof path==='string'&&existsSync(join(path,'demo/Hello.class')));
   assert.ok(classes,`demo/Hello.class was not compiled: ${JSON.stringify(paths)}`);
  };
  await step('JDT imports the project and compiles it against the detected classpath',async()=>{
   await imported();
   if(fixture.library)await until(()=>execute('java.project.getSettings',[projectUri,['org.eclipse.jdt.ls.core.referencedLibraries']]),value=>(value['org.eclipse.jdt.ls.core.referencedLibraries']??[]).some(path=>path.endsWith('servlet-api.jar')),'JDT referenced libraries');
   if(fixture.sourcePath)await until(()=>execute('java.project.getSettings',[projectUri,['org.eclipse.jdt.ls.core.sourcePaths']]),value=>(value['org.eclipse.jdt.ls.core.sourcePaths']??[]).some(path=>samePath(path,join(root,fixture.sourcePath))),'JDT source paths');
   await compiles();
  });
  if(name==='eclipse-wtp')await step('the bundle resolves the WTP containers and the TOMCAT_HOME variable',async()=>{
   const described=await execute('lithe.java.projectModel.describe');
   const project=described.projects.find(item=>item.name==='WebApp');assert.ok(project,JSON.stringify(described));
   const entry=prefix=>project.entries.find(item=>item.path.startsWith(prefix));
   assert.equal(entry('org.eclipse.jst.j2ee.internal.web.container').status,'resolved');
   assert.equal(entry('org.eclipse.jst.j2ee.internal.web.container').libraries,1);
   assert.equal(entry('org.eclipse.jst.server.core.container').status,'resolved');
   assert.ok(samePath(entry('TOMCAT_HOME').resolvedPath,join(tomcat,'lib/jsp-api.jar')),JSON.stringify(entry('TOMCAT_HOME')));
  });
  await step('the libraries page shows the model read-only',async()=>{
   const page=await invoke('libraries.settings',{context:{language:'zh-CN',builds:[]}});
   const model=page.sources.find(source=>source.group==='model'&&source.path===fixture.descriptor);
   assert.ok(model,JSON.stringify(page.sources.map(source=>[source.group,source.path])));
   assert.equal(model.writable,false);
   const value=id=>model.fields.find(field=>field.id===id)?.value;
   assert.match(value('model'),new RegExp(fixture.model));
   if(fixture.projectServlet)assert.equal(value('servlet'),undefined);
   else assert.match(value('servlet'),/Apache Tomcat 9\.0\.98/);
   if(fixture.sourcePath)assert.deepEqual(value('source-folders'),[fixture.sourcePath]);
   if(fixture.library)assert.ok(value('detected').some(line=>line.startsWith(fixture.library)),JSON.stringify(value('detected')));
   if(name==='eclipse-wtp')assert.ok(value('containers')?.some(line=>line.startsWith('Web App Libraries')),JSON.stringify(model.fields));
   assert.deepEqual(projectFiles(),projectBaseline,'Detection and the libraries page must not write project files');
   // The public projection, rendered by the native host in scripts/compiler/visual.mjs.
   capture(`libraries-${name}-zh`,page);
   capture(`libraries-${name}-en`,await invoke('libraries.settings',{context:{language:'en',builds:[]}}));
  });
  if(name==='legacy-web')await step('saving the detected entries writes them and JDT builds from the file',async()=>{
   const page=await invoke('libraries.settings',{context:{language:'en',builds:[]}});
   const source=page.sources.find(item=>item.path==='.vscode/settings.json');assert.ok(source,JSON.stringify(page.sources));
   assert.equal(source.fields.find(field=>field.id==='store-detected')?.value,false);
   const stageFile=join(stage,'settings-stage.json');writeFileSync(stageFile,'');
   const edit=await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:source.id,revision:source.revision,values:{'store-detected':true},outputFile:stageFile});
   assert.equal(edit.changed,true);
   const saved=JSON.parse(readFileSync(stageFile,'utf8'));
   assert.deepEqual(saved['java.project.referencedLibraries'],['lib/**',fixture.library],'Only portable entries are saved, never the machine\'s servlet container');
   mkdirSync(join(root,'.vscode'));renameSync(stageFile,join(root,source.path));
   const after=await invoke('libraries.settings',{operation:'readAfterSave',context:{language:'en',builds:[]}});
   assert.equal(after.sources.find(item=>item.path==='.vscode/settings.json').fields.find(field=>field.id==='store-detected').value,true);
   await compiles();
  });
  await server.request('shutdown',null,30000);server.notify('exit',null);
 }finally{
  try{if(server)await server.close();}finally{try{if(backend)await backend.close();}finally{rmSync(stage,{recursive:true,force:true});}}
 }
}

const report=createReport('project-models');
const packageBaseline=snapshot(pkg,new Set()),serverBaseline=snapshot(inputs.home,new Set());
try{
 for(const name of selected)await scenario(name,report);
 await report.step('the package and the JDT LS installation stay unchanged',async()=>{
  assert.deepEqual(snapshot(pkg,new Set()),packageBaseline,'Project models must never write into the installed package');
  assert.deepEqual(snapshot(inputs.home,new Set()),serverBaseline,'Project models must never write into the user\'s JDT LS installation');
 });
}finally{report.write();}
