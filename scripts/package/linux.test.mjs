// Offline checks of the platform boundary; real archives run in Linux CI.
import test from 'node:test';
import assert from 'node:assert/strict';
import {verifyELF,verifyBinary} from './linux-runtime.mjs';
for (const [arch,machine] of [['arm64',183],['x64',62]]) {
 test(`accepts only Linux ${arch} ELF64 little-endian executables`,()=>{
  const bytes=Buffer.alloc(64);bytes.set([127,69,76,70,2,1]);bytes.writeUInt16LE(2,16);bytes.writeUInt16LE(machine,18);
  verifyELF(bytes,arch);
  assert.throws(()=>verifyELF(bytes,arch==='arm64'?'x64':'arm64'),/architecture/);
  bytes[4]=1;assert.throws(()=>verifyELF(bytes,arch),/ELF64/);
  assert.throws(()=>verifyELF(Buffer.from('MZ'),arch),/ELF64/);
 });
}

import {mkdtempSync,writeFileSync,chmodSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
test('allows read-only ELF shared libraries but requires execute permission on launchers',t=>{
 const stage=mkdtempSync(join(tmpdir(),'linux-elf-'));t.after(()=>rmSync(stage,{recursive:true,force:true}));
 const file=join(stage,'libjvm.so');const bytes=Buffer.alloc(64);bytes.set([127,69,76,70,2,1]);bytes.writeUInt16LE(3,16);bytes.writeUInt16LE(183,18);writeFileSync(file,bytes);chmodSync(file,0o644);
 verifyBinary(file,'arm64');assert.throws(()=>verifyBinary(file,'arm64',{executable:true}),/Executable permission/);
 chmodSync(file,0o755);verifyBinary(file,'arm64',{executable:true});
});
