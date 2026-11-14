// Offline checks of the platform boundary; real archives run in Linux CI.
import test from 'node:test';
import assert from 'node:assert/strict';
import {verifyELF} from './linux-runtime.mjs';
for (const [arch,machine] of [['arm64',183],['x64',62]]) {
 test(`accepts only Linux ${arch} ELF64 little-endian executables`,()=>{
  const bytes=Buffer.alloc(64);bytes.set([127,69,76,70,2,1]);bytes.writeUInt16LE(2,16);bytes.writeUInt16LE(machine,18);
  verifyELF(bytes,arch);
  assert.throws(()=>verifyELF(bytes,arch==='arm64'?'x64':'arm64'),/architecture/);
  bytes[4]=1;assert.throws(()=>verifyELF(bytes,arch),/ELF64/);
  assert.throws(()=>verifyELF(Buffer.from('MZ'),arch),/ELF64/);
 });
}
