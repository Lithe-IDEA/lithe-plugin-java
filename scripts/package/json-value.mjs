// Portable replacement for build-time plutil extraction on Linux.
import {readFileSync} from 'node:fs';
let value=JSON.parse(readFileSync(process.argv[2],'utf8'));
for(const part of process.argv[3].split('.'))value=value?.[part];
if(value===undefined || value===null || typeof value==='object')process.exit(1);
process.stdout.write(String(value)+'\n');
