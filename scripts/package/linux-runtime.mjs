// Build-time Linux runtime preparation. Downloaded inputs are immutable and
// keyed by SHA-256; extracted runtimes belong only to this checkout.
import {createHash} from 'node:crypto';
import {createReadStream,createWriteStream,readFileSync,mkdirSync,existsSync,rmSync,mkdtempSync,cpSync,renameSync,statSync} from 'node:fs';
import {join,dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {Readable} from 'node:stream';
import {pipeline} from 'node:stream/promises';
export const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
export const architectures={arm64:{runtime:'arm64',jdk:'linux-aarch64',configuration:'config_linux_arm',machine:183},x64:{runtime:'x86_64',jdk:'linux-x86_64',configuration:'config_linux',machine:62}};
export function verifyELF(bytes,arch){
 const target=architectures[arch];
 if(!target)throw Error(`Unsupported Linux architecture: ${arch}`);
 if(bytes.length<64 || !bytes.subarray(0,6).equals(Buffer.from([127,69,76,70,2,1])))throw Error('An ELF64 little-endian binary is required');
 if(bytes.readUInt16LE(18)!==target.machine)throw Error(`ELF architecture differs from Linux ${arch}`);
 if(![2,3].includes(bytes.readUInt16LE(16)))throw Error('An ELF executable or shared object is required');
}
// Shared libraries are mapped by the loader and need no execute permission.
export function verifyBinary(path,arch,{executable=false}={}){
 verifyELF(readFileSync(path),arch);
 if(executable && !(statSync(path).mode & 0o111))throw Error(`Executable permission missing: ${path}`);
}
export async function digest(path){
 const hash=createHash('sha256');for await(const bytes of createReadStream(path))hash.update(bytes);return hash.digest('hex');
}
export function copyRuntime(source,destination){
 // Both BSD and GNU cp resolve the JDK's legal/ links with -RL. Node's
 // recursive copy can retain links, which become dangling after staging ends.
 execFileSync('cp',['-RL',source,destination],{timeout:120000});
}
export async function prepareJDK(arch){
 const target=architectures[arch];if(!target)throw Error(`Unsupported Linux architecture: ${arch}`);
 const manifest=JSON.parse(readFileSync(join(root,'third_party/jdk/manifest.json')));
 const input=manifest.platforms[target.jdk];if(!input)throw Error(`Missing pinned JDK: ${target.jdk}`);
 const cache=join(root,'.artifacts/jdk-downloads');mkdirSync(cache,{recursive:true});
 const archive=join(cache,`jdk-${target.jdk}-${input.sha256}.tar.gz`);
 if(!existsSync(archive) || await digest(archive)!==input.sha256){
  const temporary=`${archive}.${process.pid}.download`;
  try{
   const response=await fetch(input.url,{signal:AbortSignal.timeout(180000)});
   if(!response.ok)throw Error(`JDK download failed: HTTP ${response.status}`);
   await pipeline(Readable.fromWeb(response.body),createWriteStream(temporary,{flags:'wx'}));
   if(await digest(temporary)!==input.sha256)throw Error('Linux JDK SHA-256 mismatch');
   renameSync(temporary,archive);
  }finally{rmSync(temporary,{force:true});}
 }
 const output=join(root,'.artifacts',`jdk-${target.jdk}-${input.sha256}`);
 // Re-extract the checksum-verified input: no mutable runtime tree is trusted.
 const staging=mkdtempSync(join(cache,'extract-'));
 try{
  execFileSync('tar',['-xzf',archive,'-C',staging],{timeout:120000});
  const runtime=join(staging,input.jdkRoot);
  for(const path of ['bin/java','lib/server/libjvm.so'])verifyELF(readFileSync(join(runtime,path)),arch);
  if(!/^JAVA_VERSION="21\./m.test(readFileSync(join(runtime,'release'),'utf8')))throw Error('The runtime must be Java 21');
  rmSync(output,{recursive:true,force:true});
  // The plugin contract forbids links. Dereference upstream JDK links only
  // during packaging; runtime code never modifies these installed resources.
  copyRuntime(runtime,output);
  return output;
 }finally{rmSync(staging,{recursive:true,force:true});}
}
if(process.argv[1] && resolve(process.argv[1])===fileURLToPath(import.meta.url))console.log(await prepareJDK(process.argv[2]));
