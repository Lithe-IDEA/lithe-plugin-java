import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,writeFileSync,cpSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {verifyPatch} from './patch-jdt-decompiler.mjs';
const root=fileURLToPath(new URL('../../',import.meta.url));
const hash=value=>createHash('sha256').update(value).digest('hex');
function fixture(t){
 const pkg=mkdtempSync(join(tmpdir(),'jdt-patch-test-'));t.after(()=>rmSync(pkg,{recursive:true,force:true}));
 const notices=join(pkg,'licenses/jdt-decompiler');mkdirSync(notices,{recursive:true});mkdirSync(join(pkg,'language-server/plugins'),{recursive:true});
 const spec=JSON.parse(readFileSync(join(root,'third_party/jdtls/patches/manifest.json'),'utf8'));
 const jdk=JSON.parse(readFileSync(join(root,'third_party/jdk/manifest.json'),'utf8'));
 const jar=join(pkg,'language-server/plugins/org.eclipse.jdt.ls.core_1.61.0.test.jar');writeFileSync(jar,'locked generated jar');
 cpSync(join(root,'third_party/jdtls/patches/FernFlowerDecompiler.java'),join(notices,'FernFlowerDecompiler.java'));
 const meta={...spec,compiler:{version:jdk.version,platform:'macos-aarch64',archiveSHA256:jdk.platforms['macos-aarch64'].sha256},patchedCoreJarSHA256:hash(readFileSync(jar)),upstreamSignaturesRemoved:true,classes:spec.classEntries.map(path=>({path,sha256:hash(path)}))};
 const manifest=join(notices,'patch.json');const save=()=>writeFileSync(manifest,JSON.stringify(meta));save();
 return {pkg,jar,meta,manifest,save,notices,io:{entries:()=>spec.classEntries,readEntry:(_,path)=>Buffer.from(path)}};
}
test('verifies the complete pinned patch and compiler identities without running tools',t=>{const f=fixture(t);verifyPatch(f.pkg,f.io);});
test('rejects changes to the generated jar or packaged source',t=>{
 const f=fixture(t);writeFileSync(f.jar,'modified');assert.throws(()=>verifyPatch(f.pkg,f.io),/bundle changed/);
 const g=fixture(t);writeFileSync(join(g.notices,'FernFlowerDecompiler.java'),'modified');assert.throws(()=>verifyPatch(g.pkg,g.io),/source identity/);
});
test('rejects missing classes, altered class bytes and stale signing entries',t=>{
 const f=fixture(t);f.meta.classes.pop();f.save();assert.throws(()=>verifyPatch(f.pkg,f.io),/Missing patched/);
 const h=fixture(t);h.meta.upstreamSignaturesRemoved=false;h.save();assert.throws(()=>verifyPatch(h.pkg,h.io),/signature removal/);
 const g=fixture(t);assert.throws(()=>verifyPatch(g.pkg,{...g.io,readEntry:()=>Buffer.from('modified')}),/class changed/);
 assert.throws(()=>verifyPatch(g.pkg,{...g.io,entries:()=>['META-INF/ECLIPSE_.SF']}),/signature entries/);
});
test('rejects an unpinned upstream revision or compiler',t=>{
 const f=fixture(t);f.meta.upstreamRevision='other';f.save();assert.throws(()=>verifyPatch(f.pkg,f.io),/source identity/);
 const g=fixture(t);g.meta.compiler.archiveSHA256='0'.repeat(64);g.save();assert.throws(()=>verifyPatch(g.pkg,g.io),/compiler identity/);
});
