import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';
import {collectNotices,verifyNotices} from './collect-notices.mjs';

function fixture(t) {
 const root=mkdtempSync(join(tmpdir(),'java-notices-'));
 t.after(()=>rmSync(root,{recursive:true,force:true}));
 const output=join(root,'output'); mkdirSync(output);
 writeFileSync(join(output,'plugin.json'),JSON.stringify({id:'io.github.lithe-idea.java-support'}));
 writeFileSync(join(root,'LICENSE'),readFileSync(new URL('../../LICENSE',import.meta.url)));writeFileSync(join(root,'NOTICE'),readFileSync(new URL('../../NOTICE',import.meta.url)));
 const packages=[{id:'app',name:'lithe-java-backend',version:'0.1.0',manifest_path:join(root,'Cargo.toml'),source:null,license:'Apache-2.0'}];
 const metadata={workspace_root:root,workspace_members:['app'],packages,resolve:{nodes:[{id:'app',deps:[]}]}};
 return {root,output,metadata};
}
test('copies notices and hashes normal build dependencies while excluding dev-only packages',t=>{
 const f=fixture(t);const dep=join(f.root,'dep');mkdirSync(dep);
 writeFileSync(join(dep,'LICENSE-MIT'),'dependency license');
 f.metadata.packages.push({id:'dep',name:'dependency',version:'1.0.0',manifest_path:join(dep,'Cargo.toml'),source:'registry+test',license:'MIT'});
 f.metadata.packages.push({id:'dev',name:'test-only',version:'1.0.0',manifest_path:join(f.root,'absent','Cargo.toml'),source:'registry+test',license:'MIT'});
 f.metadata.resolve.nodes[0].deps=[{pkg:'dep',dep_kinds:[{kind:null}]},{pkg:'dev',dep_kinds:[{kind:'dev'}]}];
 f.metadata.resolve.nodes.push({id:'dep',deps:[]});
 const records=collectNotices(f.output,f.metadata);
 assert.deepEqual(records.map(p=>p.name),['dependency','lithe-java-backend']);
 assert.equal(readFileSync(join(f.output,'NOTICE'),'utf8'),readFileSync(new URL('../../NOTICE',import.meta.url),'utf8'));
 const doc=records[0].files[0];
 assert.equal(doc.sha256,createHash('sha256').update('dependency license').digest('hex'));
 assert.equal(readFileSync(join(f.output,'licenses/rust',doc.path),'utf8'),'dependency license');
 assert.equal(JSON.stringify(records).includes(f.root),false);
 verifyNotices(f.output);
 writeFileSync(join(f.output,'licenses/rust',doc.path),'tampered');
 assert.throws(()=>verifyNotices(f.output),/Modified dependency notice/);
});
test('fails rather than omitting an unknown dependency without a primary license',t=>{
 const f=fixture(t);const dep=join(f.root,'missing');mkdirSync(dep);
 f.metadata.packages.push({id:'missing',name:'missing',version:'1.0.0',manifest_path:join(dep,'Cargo.toml'),source:'registry+test',license:'MIT'});
 f.metadata.resolve.nodes[0].deps=[{pkg:'missing',dep_kinds:[{kind:null}]}];f.metadata.resolve.nodes.push({id:'missing',deps:[]});
 assert.throws(()=>collectNotices(f.output,f.metadata),/Missing primary license for missing-1.0.0/);
});
test('retains nested notices in addition to the primary license',t=>{
 const f=fixture(t);const dep=join(f.root,'dep');mkdirSync(join(dep,'unicode'),{recursive:true});
 writeFileSync(join(dep,'LICENSE'),'MIT');writeFileSync(join(dep,'unicode/LICENSE'),'Unicode');
 f.metadata.packages.push({id:'dep',name:'dependency',version:'1.0.0',manifest_path:join(dep,'Cargo.toml'),source:'registry+test',license:'MIT'});
 f.metadata.resolve.nodes[0].deps=[{pkg:'dep',dep_kinds:[{kind:'build'}]}];f.metadata.resolve.nodes.push({id:'dep',deps:[]});
 const records=collectNotices(f.output,f.metadata);
 assert.equal(records[0].files.length,2);
 assert.equal(readFileSync(join(f.output,'licenses/rust/dependency-1.0.0/source/unicode/LICENSE'),'utf8'),'Unicode');
});
