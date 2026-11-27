#!/usr/bin/env node
// Linux packages carry the JDT LS extensions verified from their pins and a
// native backend from this source checkout; the host prepares JDT LS and its JDK separately.
// All writes are build outputs, never installed files.
import {execFileSync} from 'node:child_process';
import {chmodSync,cpSync,mkdirSync,readFileSync,rmSync,mkdtempSync,readdirSync,lstatSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {architectures,root,digest,verifyELF,verifyBinary} from './linux-runtime.mjs';
import {collectNotices,verifyNotices} from './collect-notices.mjs';
import {BUNDLE_DIRECTORY,buildProjectModelBundle} from './project-model-bundle.mjs';
const arch=process.argv[2];const target=architectures[arch];
if(!target)throw Error('Usage: node scripts/package/build-linux.mjs arm64|x64 [native-backend]');
const run=(exe,args,options={})=>execFileSync(exe,args,{cwd:root,timeout:600000,stdio:['ignore','pipe','inherit'],...options});
let backend=process.argv[3];
if(!backend){
 if(process.platform!=='linux' || process.arch!==(arch==='x64'?'x64':'arm64'))throw Error('A native Linux runner or a matching backend binary is required');
 run('cargo',['build','--release','--locked','-p','lithe-java-backend']);
 backend=join(root,'target/release/lithe-java-backend');
}
backend=resolve(backend);verifyELF(readFileSync(backend),arch);
const jdt=run(join(root,'scripts/prepare-jdtls.sh'),[],{encoding:'utf8',env:{...process.env,LITHE_ARCH:target.runtime}}).trim();
const id='io.github.lithe-idea.java-support';
const manifest=JSON.parse(readFileSync(join(root,id,'plugin.json')));
const staging=mkdtempSync(join(root,'.artifacts',`linux-${arch}-`));
const pkg=join(staging,id);
const extensions=['lombok','java-debug','java-test','manifest.json'];
// The plugin's own bundle, compiled against the prepared JDT LS release.
const packaged=[...extensions,BUNDLE_DIRECTORY];
function verify(directory){
 const actual=JSON.parse(readFileSync(join(directory,'plugin.json')));
 if(JSON.stringify(actual)!==JSON.stringify(manifest))throw Error('Packaged manifest changed');
 for(const name of ['LICENSE','NOTICE'])if(!readFileSync(join(directory,name)).equals(readFileSync(join(root,name))))throw Error(`Missing or modified ${name}`);
 verifyBinary(join(directory,manifest.backend.launch.path),arch,{executable:true});
 if(readdirSync(join(directory,'jdtls-extensions')).sort().join()!==[...packaged].sort().join())throw Error('Unexpected JDT LS extension entries');
 const walk=path=>{for(const name of readdirSync(path)){const file=join(path,name);const s=lstatSync(file);if(s.isSymbolicLink())throw Error(`Package link: ${file}`);
  // JDT LS and the JDK belong in the managed cache, not in the plugin package.
  if((s.isFile()&&/^org\.eclipse\.equinox\.launcher_.*\.jar$|^java$|^libjvm\.so$/.test(name))||(s.isDirectory()&&/^config_/.test(name)))throw Error(`Package must not contain JDT LS or a JDK: ${file}`);
  if(s.isDirectory())walk(file);}};walk(directory);
 verifyNotices(directory);
}
try{
 mkdirSync(pkg);
 cpSync(join(root,id,'plugin.json'),join(pkg,'plugin.json'));
 for(const name of ['icons','templates','l10n','grammars'])cpSync(join(root,id,name),join(pkg,name),{recursive:true});
 const destination=join(pkg,'jdtls-extensions');mkdirSync(destination);
 for(const name of extensions)cpSync(join(jdt,name),join(destination,name),{recursive:true});
 buildProjectModelBundle(jdt,destination);
 mkdirSync(join(pkg,'bin'));cpSync(backend,join(pkg,manifest.backend.launch.path));chmodSync(join(pkg,manifest.backend.launch.path),0o755);
 collectNotices(pkg);verify(pkg);
 const dist=join(root,'dist');mkdirSync(dist,{recursive:true});
 const asset=join(dist,`Lithe-Java-Support-${manifest.version}-linux-${arch}.zip`);rmSync(asset,{force:true});
 run('zip',['-qrXy',asset,id],{cwd:staging,timeout:120000});
 const check=join(staging,'extracted');mkdirSync(check);
 run('unzip',['-q',asset,'-d',check],{timeout:120000});
 if(readdirSync(check).join()!==id)throw Error('Archive must contain only the plugin directory');
 verify(join(check,id));
 writeFileSync(`${asset}.sha256`,`${await digest(asset)}  ${asset.split('/').at(-1)}\n`);
 console.log(asset);
}finally{rmSync(staging,{recursive:true,force:true});}
