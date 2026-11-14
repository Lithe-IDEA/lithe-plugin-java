#!/usr/bin/env node
// Build-time only: the verified upstream staging tree is read-only input.
// Compile the narrowly patched EPL-2.0 integration against its exact original
// JDT distribution; package the changed source and hashes beside its notices.
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync,writeFileSync,readdirSync,mkdtempSync,mkdirSync,rmSync,cpSync} from 'node:fs';
import {resolve,join,dirname} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const source=join(root,'third_party/jdtls/patches/FernFlowerDecompiler.java');
const spec=JSON.parse(readFileSync(join(root,'third_party/jdtls/patches/manifest.json'),'utf8'));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const run=(exe,args,options={})=>execFileSync(exe,args,{timeout:120000,maxBuffer:8*1024*1024,...options});
const classPrefix='org/eclipse/jdt/ls/core/internal/decompiler/FernFlowerDecompiler';
const signature=name=>/^META-INF\/[^/]+\.(SF|RSA|DSA|EC)$/i.test(name);
function coreJar(packageRoot){
 const plugins=join(packageRoot,'language-server/plugins');
 const jars=readdirSync(plugins).filter(n=>/^org\.eclipse\.jdt\.ls\.core_1\.61\.0\..*\.jar$/.test(n));
 if(jars.length!==1)throw Error('The pinned JDT 1.61 core bundle is required');
 return join(plugins,jars[0]);
}
function entries(jar){return run('/usr/bin/unzip',['-Z1',jar],{encoding:'utf8'}).trim().split('\n');}

export function verifyPatch(packageRoot,io={}){
 const directory=join(packageRoot,'licenses/jdt-decompiler');
 const meta=JSON.parse(readFileSync(join(directory,'patch.json'),'utf8'));
 if(Object.keys(spec).some(key=>JSON.stringify(meta[key])!==JSON.stringify(spec[key])) || hash(readFileSync(source))!==spec.sourceSHA256 || hash(readFileSync(join(directory,'FernFlowerDecompiler.java')))!==spec.sourceSHA256)throw Error('Decompiler patch source identity mismatch');
 const jdk=JSON.parse(readFileSync(join(root,'third_party/jdk/manifest.json'),'utf8'));
 if(meta.compiler?.version!==jdk.version || jdk.platforms[meta.compiler?.platform]?.sha256!==meta.compiler?.archiveSHA256)throw Error('Decompiler compiler identity mismatch');
 const jar=coreJar(packageRoot);
 if(hash(readFileSync(jar))!==meta.patchedCoreJarSHA256)throw Error('Decompiler patch bundle changed');
 if((io.entries??entries)(jar).some(signature))throw Error('Modified JDT bundle retains upstream signature entries');
 if(meta.upstreamSignaturesRemoved!==true)throw Error('Missing upstream signature removal declaration');
 if(!meta.classes || JSON.stringify(meta.classes.map(item=>item.path).sort())!==JSON.stringify(spec.classEntries))throw Error('Missing patched decompiler classes');
 for(const item of meta.classes){
  if(!item.path.startsWith(classPrefix) || !item.path.endsWith('.class') || item.path.includes('..'))throw Error('Invalid patch class entry');
  if(hash((io.readEntry??((jar,path)=>run('/usr/bin/unzip',['-p',jar,path])))(jar,item.path))!==item.sha256)throw Error('Patched decompiler class changed');
 }
}
export function patch(packageRoot){
 packageRoot=resolve(packageRoot);
 const jar=coreJar(packageRoot);
 if(hash(readFileSync(source))!==spec.sourceSHA256 || hash(readFileSync(jar))!==spec.originalCoreJarSHA256)throw Error('Decompiler patch inputs do not match the lock');
 // Always use the pinned host-architecture JDK; the output is Java bytecode
 // shared by macOS and Windows, even when packaging another architecture.
 const jdk=run(join(root,'scripts/prepare-jdk.sh'),[],{encoding:'utf8',env:{...process.env,LITHE_JDK_TARGET_ARCH:process.arch==='arm64'?'arm64':'x86_64'}}).trim();
 const stage=mkdtempSync(join(tmpdir(),'lithe-jdt-decompiler-'));
 try{
  const classes=join(stage,'classes');mkdirSync(classes);
  run(join(jdk,'bin/javac'),['--release','21','-g:none','-classpath',join(dirname(jar),'*'),'-d',classes,source]);
  const original=entries(jar);
  const folder=join(classes,'org/eclipse/jdt/ls/core/internal/decompiler');
  const generated=readdirSync(folder).filter(n=>n.endsWith('.class')).sort().map(n=>classPrefix.substring(0,classPrefix.lastIndexOf('/')+1)+n);
  if(JSON.stringify(generated)!==JSON.stringify(spec.classEntries) || generated.some(n=>!original.includes(n)))throw Error('Unexpected decompiler class layout');
  const unpack=join(stage,'bundle');mkdirSync(unpack);run('/usr/bin/unzip',['-q',jar,'-d',unpack]);
  for(const name of original.filter(signature))rmSync(join(unpack,name));
  const manifestFile=join(unpack,'META-INF/MANIFEST.MF');
  const sections=readFileSync(manifestFile,'utf8').split(/\r?\n\r?\n/).filter(s=>s.trim());
  for(const section of sections.slice(1)){
   const fields=section.split(/\r?\n/).filter(l=>l&&!l.startsWith(' '));
   if(fields.some(l=>!l.startsWith('Name:')&&!/^SHA-\d+-Digest:/.test(l)))throw Error('Unexpected signed manifest section');
  }
  const mainManifest=join(stage,'MANIFEST.MF');writeFileSync(mainManifest,sections[0]+'\r\n\r\n');rmSync(manifestFile);
  cpSync(classes,unpack,{recursive:true});
  const target=join(stage,'patched.jar');run(join(jdk,'bin/jar'),['--create','--file',target,'--manifest',mainManifest,'-C',unpack,'.']);
  const directory=join(packageRoot,'licenses/jdt-decompiler');mkdirSync(directory,{recursive:true});cpSync(source,join(directory,'FernFlowerDecompiler.java'));
  const jdkSpec=JSON.parse(readFileSync(join(root,'third_party/jdk/manifest.json'),'utf8'));
  const compilerPlatform=process.arch==='arm64'?'macos-aarch64':'macos-x86_64';
  const metadata={...spec,compiler:{version:jdkSpec.version,platform:compilerPlatform,archiveSHA256:jdkSpec.platforms[compilerPlatform].sha256},patchedCoreJarSHA256:hash(readFileSync(target)),upstreamSignaturesRemoved:true,classes:generated.map(path=>({path,sha256:hash(readFileSync(join(classes,path)))}))};
  // The package staging directory is writable build output. Never patch an
  // installed bundle or the downloaded/cache copy of the upstream distribution.
  cpSync(target,jar);writeFileSync(join(directory,'patch.json'),JSON.stringify(metadata,null,2)+'\n');
  verifyPatch(packageRoot);
 } finally{rmSync(stage,{recursive:true,force:true});}
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 if(process.argv[2]==='--verify')verifyPatch(resolve(process.argv[3]));else patch(resolve(process.argv[2]));
}
