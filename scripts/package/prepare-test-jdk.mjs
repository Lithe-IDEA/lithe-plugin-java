// Build/test support only. Production installation is the host's managed
// runtime service. Existing CI may compile the extension on JDK 21; this
// helper prepares the same pinned full JDK 25 to run the language service.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {closeSync,existsSync,mkdirSync,mkdtempSync,openSync,readFileSync,readSync,renameSync,rmSync} from 'node:fs';
import {basename,dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
function digest(path){
 const file=openSync(path,'r'),hash=createHash('sha256'),buffer=Buffer.alloc(64*1024);
 try{for(;;){const count=readSync(file,buffer);if(!count)break;hash.update(buffer.subarray(0,count));}return hash.digest('hex');}
 finally{closeSync(file);}
}
export function prepareTestJDK(){
 const os={darwin:'macos',win32:'windows'}[process.platform]??process.platform;
 const arch={arm64:'arm64',x64:'x86_64'}[process.arch];
 const manifest=JSON.parse(readFileSync(join(root,'io.github.lithe-idea.java-support/plugin.json')));
 const artifact=manifest.managedRuntimes.dependencies.find(value=>value.id==='jdk').artifacts.find(value=>value.os===os&&value.arch===arch);
 assert.ok(artifact,`No full test JDK is declared for ${os}/${arch}`);
 const cache=join(root,'.artifacts/managed-jdk'),directory=join(cache,artifact.sha256);
 const home=join(directory,artifact.stripPrefix,artifact.home??'');
 const executable=join(home,artifact.executable);
 const suffix=os==='windows'?'.exe':'';
 if([executable,join(home,`bin/javac${suffix}`),join(home,'release'),join(home,'lib/modules')].every(existsSync))return executable;
 mkdirSync(cache,{recursive:true});
 const archive=join(cache,basename(new URL(artifact.url).pathname));
 if(!existsSync(archive)||digest(archive)!==artifact.sha256){
  const partial=`${archive}.${process.pid}.partial`;
  try{
   execFileSync('curl',['--fail','--silent','--show-error','--location','--proto','=https','--proto-redir','=https','--connect-timeout','30','--max-time','300','--output',partial,artifact.url],{stdio:'inherit',timeout:320000,windowsHide:true});
   assert.equal(digest(partial),artifact.sha256,'Official full JDK archive checksum mismatch');
   renameSync(partial,archive);
  }finally{rmSync(partial,{force:true});}
 }
 const staging=mkdtempSync(join(cache,'.jdk-stage-'));
 try{
  // Only bytes matching a reviewed official archive digest are extracted.
  // Windows' system bsdtar supports the pinned ZIP; Unix inputs are tar.gz.
  const tar=os==='windows'?join(process.env.SystemRoot??'C:\\Windows','System32/tar.exe'):'tar';
  execFileSync(tar,['-xf',archive,'-C',staging],{stdio:'inherit',timeout:180000,windowsHide:true});
  assert.ok(existsSync(join(staging,artifact.stripPrefix,artifact.home??'',`bin/javac${suffix}`)),'Archive does not contain a full JDK');
  rmSync(directory,{recursive:true,force:true});renameSync(staging,directory);
  return executable;
 }finally{rmSync(staging,{recursive:true,force:true});}
}
