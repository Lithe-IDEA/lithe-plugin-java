#!/usr/bin/env node
// Copy notices from Cargo's locked build graph. No runtime download or source
// modification: registry/Git inputs stay owned by Cargo, output by the package.
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync,readFileSync,readdirSync,lstatSync,mkdirSync,writeFileSync,chmodSync,rmSync} from 'node:fs';
import {dirname,isAbsolute,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const repository=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const sha=data=>createHash('sha256').update(data).digest('hex');
const isNotice=name=>/^(LICENSE|COPYING|NOTICE)(?:[._-]|$)/i.test(name);
const isLicense=name=>/^(LICENSE|COPYING)(?:[._-]|$)/i.test(name);
function documents(directory,recursive=false,prefix='') {
  return readdirSync(directory,{withFileTypes:true}).flatMap(entry=>{
    const path=join(prefix,entry.name);
    if(entry.isDirectory() && recursive) return documents(join(directory,entry.name),true,path);
    return entry.isFile() && isNotice(entry.name) ? [path] : [];
  }).sort();
}
function readDocument(path) {
  if(!lstatSync(path).isFile() || lstatSync(path).size>1024*1024) throw Error('Invalid or oversized notice');
  return readFileSync(path);
}

export function collectNotices(packageRoot,metadata) {
  packageRoot=resolve(packageRoot);
  const manifest=JSON.parse(readFileSync(join(packageRoot,'plugin.json'),'utf8'));
  if(manifest.id!=='io.github.lithe-idea.java-support') throw Error('A Java Support package is required');
  metadata??=JSON.parse(execFileSync('cargo',['metadata','--locked','--format-version','1'],{
    cwd:repository,encoding:'utf8',timeout:180000,maxBuffer:16*1024*1024,
  }));
  const nodes=new Map(metadata.resolve.nodes.map(node=>[node.id,node]));
  const backend=metadata.packages.find(pkg=>pkg.name==='lithe-java-backend');
  if(!backend) throw Error('Missing backend build graph');
  const selected=new Set();
  function visit(id) {
    if(selected.has(id)) return;
    selected.add(id);
    const node=nodes.get(id);
    if(!node) throw Error('Incomplete Cargo graph');
    for(const dep of node.deps) if(dep.dep_kinds.some(kind=>kind.kind!=='dev')) visit(dep.pkg);
  }
  visit(backend.id);
  const fallbacks=JSON.parse(readFileSync(join(repository,'third_party/rust-notices/manifest.json'),'utf8'));
  const destination=join(packageRoot,'licenses/rust');
  rmSync(destination,{recursive:true,force:true});
  mkdirSync(destination,{recursive:true});
  for(const name of ['LICENSE','NOTICE']) {
    writeFileSync(join(packageRoot,name),readDocument(join(metadata.workspace_root,name)),{mode:0o644});
    chmodSync(join(packageRoot,name),0o644);
  }
  const records=[];
  for(const pkg of metadata.packages.filter(pkg=>selected.has(pkg.id)).sort((a,b)=>`${a.name}@${a.version}`<`${b.name}@${b.version}`?-1:1)) {
    const key=`${pkg.name}-${pkg.version}`;
    if(!/^[a-zA-Z0-9_.+-]+$/.test(key)) throw Error('Invalid Cargo package identity');
    const source=dirname(pkg.manifest_path);
    let primary=source;
    if(metadata.workspace_members.includes(pkg.id)) primary=metadata.workspace_root;
    let names=documents(primary);
    if(!names.some(isLicense) && pkg.source?.startsWith('git+')) {
      // SDKs may inherit a shared license above their crate. Stop at the Git
      // checkout boundary rather than accepting a license from the user home.
      for(let level=0;level<8;level++) {
        if(existsSync(join(primary,'.git')) || dirname(primary)===primary) break;
        primary=dirname(primary); names=documents(primary);
        if(names.some(isLicense)) break;
      }
    }
    const files=[];
    function copy(name,data) {
      const target=join(destination,key,name);
      mkdirSync(dirname(target),{recursive:true});
      writeFileSync(target,data,{mode:0o644}); chmodSync(target,0o644);
      files.push({path:`${key}/${name.split('\\').join('/')}`,sha256:sha(data)});
    }
    if(names.some(isLicense)) {
      for(const name of names) copy(name,readDocument(join(primary,name)));
    } else {
      const fallback=fallbacks.find(item=>item.name===pkg.name && item.version===pkg.version);
      if(!fallback) throw Error(`Missing primary license for ${key}`);
      const vcs=JSON.parse(readFileSync(join(source,'.cargo_vcs_info.json'),'utf8'));
      if(vcs.git.sha1!==fallback.revision) throw Error(`License source revision mismatch for ${key}`);
      const data=readDocument(join(repository,'third_party/rust-notices',fallback.file));
      if(sha(data)!==fallback.sha256) throw Error(`License hash mismatch for ${key}`);
      copy('LICENSE',data);
    }
    // Retain embedded notices too, such as tree-sitter's Unicode license.
    for(const name of metadata.workspace_members.includes(pkg.id)?[]:documents(source,true)) {
      if(primary===source && names.includes(name)) continue;
      copy(join('source',name),readDocument(join(source,name)));
    }
    records.push({name:pkg.name,version:pkg.version,license:pkg.license,files});
  }
  writeFileSync(join(destination,'manifest.json'),JSON.stringify({schemaVersion:1,packages:records},null,2)+'\n');
  return records;
}
export function verifyNotices(packageRoot) {
  for(const name of ['LICENSE','NOTICE']) {
    if(!readDocument(join(packageRoot,name)).equals(readDocument(join(repository,name)))) throw Error(`Missing or modified ${name}`);
  }
  const root=join(packageRoot,'licenses/rust');
  const manifest=JSON.parse(readDocument(join(root,'manifest.json')).toString('utf8'));
  if(manifest.schemaVersion!==1 || !manifest.packages?.length) throw Error('Missing Rust build notices');
  for(const pkg of manifest.packages) {
    if(!pkg.files?.length) throw Error('Missing dependency notice files');
    for(const file of pkg.files) {
      if(typeof file.path!=='string' || isAbsolute(file.path) || file.path.split(/[\\/]/).includes('..')) throw Error('Unsafe notice path');
      if(sha(readDocument(join(root,file.path)))!==file.sha256) throw Error('Modified dependency notice');
    }
  }
}
if(process.argv[1] && resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  if(process.argv.length===4 && process.argv[2]==='--verify') verifyNotices(process.argv[3]);
  else {
    if(process.argv.length!==3) throw Error('Usage: collect-notices.mjs [--verify] <package-directory>');
    const records=collectNotices(process.argv[2]);
    verifyNotices(process.argv[2]);
    process.stderr.write(`Collected locked Rust notices for ${records.length} build packages\n`);
  }
}
