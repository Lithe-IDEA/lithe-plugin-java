// Build-time Linux platform checks. The package ships no JDK: the backend runs
// the user's JDT LS on the user's JDK, so only the backend binary is native.
import {createHash} from 'node:crypto';
import {createReadStream,readFileSync,statSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
export const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
export const architectures={arm64:{runtime:'arm64',configuration:'config_linux_arm',machine:183},x64:{runtime:'x86_64',configuration:'config_linux',machine:62}};
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
