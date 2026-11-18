// Pin the files loaded by the language service independently of plugin releases.
// Run only after prepare-jdtls.sh has checked the upstream archive SHA-256.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync,readdirSync,lstatSync,writeFileSync} from 'node:fs';
import {dirname,join,relative,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const server=resolve(process.argv[2]??join(root,'.artifacts/jdtls'));
const pins=JSON.parse(readFileSync(join(root,'third_party/jdtls/manifest.json'),'utf8'));
const target=join(root,'third_party/jdtls/runtime-files.json');
const record=path=>{
 const stat=lstatSync(path);
 assert.ok(stat.isFile()&&!stat.isSymbolicLink(),`Expected upstream regular file: ${path}`);
 return {path:relative(server,path).split('\\').join('/'),size:stat.size,sha256:createHash('sha256').update(readFileSync(path)).digest('hex')};
};
const files=[];
const walk=path=>{
 const stat=lstatSync(path);assert.ok(!stat.isSymbolicLink(),`Unexpected upstream link: ${path}`);
 if(stat.isDirectory())for(const name of readdirSync(path).sort())walk(join(path,name));
 else files.push(record(path));
};
walk(join(server,'plugins'));
const configurations={};
for(const name of readdirSync(server).filter(name=>name.startsWith('config_')).sort())configurations[name]=record(join(server,name,'config.ini'));
const profile={version:pins.version,archiveSHA256:pins.archiveSHA256,files,configurations};
if(process.argv.includes('--write'))writeFileSync(target,JSON.stringify(profile,null,2)+'\n');
else assert.deepEqual(profile,JSON.parse(readFileSync(target,'utf8')),'The verified JDT LS file profile differs; update it only with a reviewed upstream version change');
