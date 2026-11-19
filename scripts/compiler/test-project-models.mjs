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
  write(join(root,'app.iml'),'<?xml version="1.0" encoding="UTF-8"?><module type="JAVA_MODULE" version="4"><component name="NewModuleRootManager" inherit-compiler-output="true"><exclude-output/><content url="file://$MODULE_DIR$"><sourceFolder url="file://$MODULE_DIR$/java" isTestSource="false"/></content><orderEntry type="inheritedJdk"/><orderEntry type="sourceFolder" forTests="false"/><orderEntry type="module-library"><library><CLASSES><root url="jar://$MODULE_DIR$/third-party/util.jar!/"/></CLASSES><JAVADOC/><SOURCES/></library></orderEntry><orderEntry type="module-library"><library name="Maximo"><CLASSES><root url="file://$MODULE_DIR$/../MAXIMO.ear/businessobjects.jar"/><root url="file://$MODULE_DIR$/../MAXIMO.ear/web.war/WEB-INF/classes"/><root url="file://$MODULE_DIR$/third-party"/></CLASSES><jarDirectory url="file://$MODULE_DIR$/third-party" recursive="false"/></library></orderEntry><orderEntry type="library" name="servlet" level="project"/></component></module>');
  write(join(root,'.idea/libraries/servlet.xml'),'<component name="libraryTable"><library name="servlet"><CLASSES><root url="jar://$PROJECT_DIR$/vendor/servlet-api.jar!/"/></CLASSES><JAVADOC/><SOURCES/></library></component>');
  await jar(stage,'util',{'com/acme/Util.java':'package com.acme; public class Util { public static String value() { return "idea-ok"; } }'},join(root,'third-party/util.jar'));
  await jar(stage,'servlet-copy',{'javax/servlet/http/HttpServlet.java':'package javax.servlet.http; public abstract class HttpServlet { }'},join(root,'vendor/servlet-api.jar'));
  const classFolders=['businessobjects.jar','web.war/WEB-INF/classes'].map(path=>join(stage,'MAXIMO.ear',path));
  for(const [index,folder] of classFolders.entries()){
   const name=`Legacy${index}`, source=join(stage,`${name}.java`);
   write(source,`package acme.legacy; public class ${name} { public static void touch() {} }`);
   mkdirSync(folder,{recursive:true});
   await runTool(javac,['--release','8','-d',folder,source],{timeout:60000});
  }
  write(join(root,'java/demo/Hello.java'),HELLO.replace('System.out.print','acme.legacy.Legacy0.touch(); acme.legacy.Legacy1.touch(); System.out.print'));
  return {source:join(root,'java/demo/Hello.java'),project:root,descriptor:'.idea/modules.xml',model:'IntelliJ IDEA',library:'third-party/util.jar',sourcePath:'java',projectServlet:true,classFolders};
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
  let blockedRequests;
  const hostRequests={
   'lithe/host/workspace/getContext':()=>({context,...context,folders}),
   'lithe/host/workspace/getFolders':()=>({folders}),
   'lithe/host/functions/providers':()=>({providers:[]}),
   'lithe/host/languageServer/request':async p=>{
    if(blockedRequests){
     blockedRequests.push([p.params.command,p.timeoutMilliseconds]);
     throw Error('JDT indexing: request timed out');
    }
    return {value:await server.request(p.method,p.params,p.timeoutMilliseconds??60000)};
   },
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
   if(fixture.projectServlet){
    assert.deepEqual(model?.classFolders?.map(path=>resolve(path)),fixture.classFolders.map(path=>resolve(path)));
    assert.equal(model.serverRuntimeHome,'');
   }
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
   if(fixture.classFolders)await until(()=>execute('java.project.getClasspaths',[projectUri,'{"scope":"test"}']),value=>fixture.classFolders.every(folder=>value.classpaths.some(path=>samePath(path,folder))),'JDT class folders');
   await compiles();
  });
  if(fixture.classFolders)await step('JDT completes and navigates to classes in both directories',async()=>{
   const uri=pathToFileURL(fixture.source).href,text=readFileSync(fixture.source,'utf8');
   const textDocument={uri};
   server.notify('textDocument/didOpen',{textDocument:{uri,languageId:'java',version:1,text}});
   const line=text.split('\n').findIndex(value=>value.includes('Legacy0'));
   const use=text.split('\n')[line];
   for(const index of [0,1]){
    const name=`Legacy${index}`, character=use.indexOf(name);
    await until(()=>server.request('textDocument/definition',{textDocument,position:{line,character:character+2}},15000),
     value=>[value].flat().some(location=>(location?.uri??location?.targetUri??'').includes(name)),'JDT definition '+name);
    await until(()=>server.request('textDocument/completion',{textDocument,position:{line,character:character+name.length+1}},15000),
     value=>(value?.items??value??[]).some(item=>item.label.startsWith('touch')),'JDT completion '+name);
   }
   server.notify('textDocument/didClose',{textDocument});
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
  if(fixture.classFolders)await step('the IDEA structure opens while JDT cannot answer',async()=>{
   blockedRequests=[];
   try{
    const page=await invoke('libraries.settings',{context:{language:'zh-CN',builds:[]}});
    const module=page.sources.find(source=>source.group==='modules'&&source.path==='app.iml');
    assert.equal(module?.writable,false);
    const dependencies=module.fields.find(field=>field.id==='dependencies').value;
    assert.ok(dependencies.some(row=>row.kind==='classes'&&row.detail.includes('JDT 状态未确认')));
    assert.match(module.fields.find(field=>field.id==='jdt-status').value,/JDT 类路径未确认/);
    assert.ok(page.sources.some(source=>source.path==='.vscode/settings.json'&&source.writable));
    assert.deepEqual(blockedRequests.map(([command])=>command),['java.project.getClasspaths']);
    assert.ok(blockedRequests[0][1]<=1500);
   }finally{blockedRequests=undefined;}
  });
  await step('the libraries page shows the model read-only',async()=>{
   const page=await invoke('libraries.settings',{context:{language:'zh-CN',builds:[]}});
   for(const source of page.sources){
    for(const field of source.fields){
     assert.ok(Object.hasOwn(field,'value')&&Object.hasOwn(field,'effectiveValue')&&typeof field.effectiveSource==='string',
      `Missing source-state field in ${source.path}: ${field.id}`);
    }
   }
   const model=page.sources.find(source=>source.group==='model'&&source.path===fixture.descriptor);
   assert.ok(model,JSON.stringify(page.sources.map(source=>[source.group,source.path])));
   assert.equal(model.writable,false);
   const value=id=>model.fields.find(field=>field.id===id)?.value;
   assert.match(value('model'),new RegExp(fixture.model));
   if(fixture.projectServlet)assert.equal(value('servlet'),undefined);
   else assert.match(value('servlet'),/Apache Tomcat 9\.0\.98/);
   if(fixture.sourcePath)assert.deepEqual(value('source-folders'),[fixture.sourcePath]);
   if(fixture.library)assert.ok(value('detected').some(line=>line.startsWith(fixture.library)),JSON.stringify(value('detected')));
   if(fixture.classFolders){
    const modules=page.sources.filter(source=>source.group==='modules');
    assert.deepEqual(modules.map(source=>[source.title,source.path,source.writable]),[['app','app.iml',false]]);
    const dependencies=modules[0].fields.find(field=>field.id==='dependencies').value;
    assert.match(modules[0].fields.find(field=>field.id==='jdt-status').value,/JDT 已返回工作区类路径/);
    for(const folder of fixture.classFolders){
     const entry=dependencies.find(item=>item.kind==='classes'&&samePath(item.value,folder));
     assert.match(entry?.detail??'',/JDT 已生效/);
    }
    const directory=dependencies.filter(item=>item.value==='third-party/*.jar');
    assert.equal(directory.length,1);
    assert.match(directory[0].detail,/JDT 已加载 1 个 JAR/);
    assert.ok(page.sources.some(source=>source.path==='.vscode/settings.json'&&source.writable));
   }
   if(name==='eclipse-wtp')assert.ok(value('containers')?.some(line=>line.startsWith('Web App Libraries')),JSON.stringify(model.fields));
   assert.deepEqual(projectFiles(),projectBaseline,'Detection and the libraries page must not write project files');
   // The public projection, rendered by the native host in scripts/compiler/visual.mjs.
   capture(`libraries-${name}-zh`,page);
   capture(`libraries-${name}-en`,await invoke('libraries.settings',{context:{language:'en',builds:[]}}));
  });
  if(name==='eclipse-wtp')await step('saving an Eclipse module still writes its own .classpath',async()=>{
   const page=await invoke('libraries.settings',{context:{language:'en',builds:[]}});
   const module=page.sources.find(item=>item.group==='modules'&&item.path==='WebApp/.classpath');
   assert.equal(module?.writable,true);
   const stageFile=join(stage,'classpath-stage.xml');writeFileSync(stageFile,'');
   const edit=await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:module.id,revision:module.revision,values:{output:'build/verified'},outputFile:stageFile});
   assert.equal(edit.changed,true);
   const contents=readFileSync(stageFile,'utf8');
   assert.match(contents,/kind="output" path="build\/verified"/);
   assert.match(contents,/org\.eclipse\.jst\.j2ee\.internal\.web\.container/);
   renameSync(stageFile,join(root,module.path));
   const after=await invoke('libraries.settings',{operation:'readAfterSave',context:{language:'en',builds:[]}});
   assert.equal(after.sources.find(item=>item.id===module.id).fields.find(field=>field.id==='output').value,'build/verified');
   await compiles();
   assert.ok(existsSync(join(fixture.project,'build/verified/demo/Hello.class')));
  });
  if(name==='legacy-web')await step('saving the detected entries writes them and JDT builds from the file',async()=>{
   const page=await invoke('libraries.settings',{context:{language:'en',builds:[]}});
   const source=page.sources.find(item=>item.path==='.vscode/settings.json');assert.ok(source,JSON.stringify(page.sources));
   assert.equal(source.group,'modules');
   assert.equal(source.title,root.split(/[\\/]/).at(-1));
   assert.deepEqual([...new Set(source.fields.map(field=>field.section))],['Sources','Paths','Dependencies']);
   const initialOutput=source.fields.find(field=>field.id==='output');
   assert.equal(initialOutput.value,null);
   assert.ok(initialOutput.effectiveValue, 'JDT reports the default output location');
   assert.equal(initialOutput.inheritedValue,initialOutput.effectiveValue);
   assert.ok(source.fields.find(field=>field.id==='roots').inheritedValue.some(row=>row.value==='src'));
   assert.deepEqual(source.fields.find(field=>field.id==='dependencies').entryKinds.map(kind=>[kind.id,kind.input]),
    [['archive','file'],['jar-directory','directory'],['classes','directory']]);
   assert.equal(source.fields.find(field=>field.id==='store-detected')?.value,false);
   await jar(stage,'manual',{'demo/manual/Extra.java':'package demo.manual; public class Extra {}'},join(stage,'manual.jar'));
   const classFolder=join(stage,'jar-manual');
   const resolved=()=>execute('java.project.getClasspaths',[projectUri,'{"scope":"test"}']);
   const hasClassFolder=value=>value.classpaths.some(path=>samePath(path,classFolder));
   const stageFile=join(stage,'settings-stage.json');writeFileSync(stageFile,'');
   const selectedLibrary=join(stage,'manual.jar');
   const selectedDirectory=join(root,'WebRoot/WEB-INF/lib');
   write(join(root,'test/demo/ExampleTest.java'),'package demo; public class ExampleTest {}');
   write(join(root,'src/demo/excluded/Invalid.java'),'intentionally invalid Java');
   write(join(root,'resources/verified.properties'),'verified=true');
   write(join(root,'test-resources/verified-test.properties'),'verified=true');
   write(join(root,'resources/NotJava.java'),'intentionally invalid Java');
   const edit=await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:source.id,revision:source.revision,values:{
    'store-detected':true,output:'build/verified',
    roots:[{kind:'folder',value:join(root,'src')},{kind:'test',value:join(root,'test')},
     {kind:'excluded',value:join(root,'src/demo/excluded')},{kind:'resource',value:join(root,'resources')},
     {kind:'test-resource',value:join(root,'test-resources')}],
    dependencies:[{kind:'jar-directory',value:selectedDirectory},{kind:'archive',value:selectedLibrary},{kind:'classes',value:classFolder}]
   },outputFile:stageFile});
   assert.equal(edit.changed,true);
   const saved=JSON.parse(readFileSync(stageFile,'utf8'));
   assert.deepEqual(saved['java.project.referencedLibraries'],[fixture.library,selectedLibrary],'The library directory stores a JAR pattern and the server libraries stay implicit');
   assert.deepEqual(saved['java.lithe.classFolders'],[classFolder]);
   assert.equal(saved['java.project.outputPath'],'build/verified');
   assert.deepEqual(saved['java.project.sourcePaths'],['src','test','resources','test-resources']);
   mkdirSync(join(root,'.vscode'));renameSync(stageFile,join(root,source.path));
   const after=await invoke('libraries.settings',{operation:'readAfterSave',context:{language:'en',builds:[]}});
   const savedSource=after.sources.find(item=>item.path==='.vscode/settings.json');
   assert.equal(savedSource.fields.find(field=>field.id==='store-detected').value,true);
   assert.equal(savedSource.fields.find(field=>field.id==='output').value,'build/verified');
   await until(resolved,value=>hasClassFolder(value)&&value.classpaths.some(path=>samePath(path,selectedLibrary)),
    'JDT loads the saved JAR and class folder');
   const marked=await until(()=>execute('java.project.getSettings',[projectUri,['org.eclipse.jdt.ls.core.classpathEntries']]),
    value=>JSON.stringify(value).includes('demo/excluded/')&&JSON.stringify(value).includes('**/*.java'),
    'JDT applied the test and excluded folder markers');
   assert.equal(await execute('java.project.isTestFile',[pathToFileURL(join(root,'test/demo/ExampleTest.java')).href]),true);
   const current=await invoke('libraries.settings',{context:{language:'en',builds:[]}});
   const rows=current.sources.find(item=>item.id===source.id).fields.find(field=>field.id==='dependencies').value;
   assert.equal(rows.filter(row=>row.kind==='jar-directory').length,1);
   assert.match(rows.find(row=>row.kind==='jar-directory').detail,/1 JARs? loaded by JDT/);
   assert.match(rows.find(row=>row.kind==='classes').detail,/Loaded by JDT/);
   capture('workspace-dependencies-en',current);
   await compiles();
   const jdtOutput=(await execute('java.project.getSettings',[projectUri,['org.eclipse.jdt.ls.core.outputPath']]))['org.eclipse.jdt.ls.core.outputPath'];
   assert.ok(samePath(jdtOutput,join(root,'build/verified')), 'JDT uses the configured production output');
   assert.ok(existsSync(join(jdtOutput,'verified.properties')));
   const testOutput=marked['org.eclipse.jdt.ls.core.classpathEntries'].find(entry=>samePath(entry.path,join(root,'test-resources'))).output;
   assert.ok(testOutput&&!samePath(testOutput,jdtOutput));
   assert.ok(existsSync(join(testOutput,'verified-test.properties')));
   writeFileSync(stageFile,'');
   await invoke('compiler.prepareSettings',{operation:'editSource',sourceID:savedSource.id,revision:savedSource.revision,values:{
    dependencies:[{kind:'jar-directory',value:fixture.library}]
   },outputFile:stageFile});
   assert.deepEqual(JSON.parse(readFileSync(stageFile,'utf8'))['java.project.referencedLibraries'],[fixture.library]);
   renameSync(stageFile,join(root,source.path));
   await invoke('libraries.settings',{operation:'readAfterSave',context:{language:'en',builds:[]}});
   await until(resolved,value=>!hasClassFolder(value)&&!value.classpaths.some(path=>samePath(path,selectedLibrary)),
    'JDT drops the removed JAR and class folder');
  });
  await server.request('shutdown',null,30000);server.notify('exit',null);
  await Promise.race([server.exited,delay(2000,undefined,{ref:false})]);
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
