import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const root=resolve(fileURLToPath(new URL('../..',import.meta.url)));
const plugin='io.github.lithe-idea.java-support';
const manifest=JSON.parse(readFileSync(resolve(root,plugin,'plugin.json'),'utf8'));
const jsp=manifest.languages.find(x=>x.id==='jsp');
const grammar=JSON.parse(readFileSync(resolve(root,plugin,'grammars/jsp.tmLanguage.json'),'utf8'));
function visit(node,names=[]) {
  if (node&&typeof node==='object') {
    if (typeof node.include==='string' && node.include.startsWith('#')) names.push(node.include.slice(1));
    for (const k of ['match','begin','end']) if (typeof node[k]==='string') new RegExp(node[k]);
    for (const v of Object.values(node)) if (typeof v==='object') visit(v,names);
  }
  return names;
}
test('JSP language is syntax-only and leaves Java JDT LS alone',()=>{
  assert.deepEqual(jsp.fileExtensions,['jsp','jspf','tag','tagf']);
  assert.deepEqual(jsp.grammar,{kind:'textmate',path:'grammars/jsp.tmLanguage.json'});
  assert.equal(manifest.languageServers.length,1);
  assert.equal(manifest.languageServers[0].languageID,'java');
});
test('JSP TextMate grammar has valid local references and regular expressions',()=>{
  assert.equal(grammar.scopeName,'text.html.jsp');
  for(const name of visit(grammar)) assert.ok(Object.hasOwn(grammar.repository,name),name);
  assert.ok(JSON.stringify(grammar).length<16000);
});
test('Recognizes classic JSP, directives, tags and EL without matching escaped EL',()=>{
  const r=grammar.repository;
  const expect=(kind,source)=>assert.match(source,new RegExp(r[kind].begin));
  expect('jsp-directive','<%@ taglib prefix="c" uri="jakarta.tags.core" %>');
  expect('jsp-declaration','<%! int n = 0; %>');
  expect('jsp-expression','<%= user.getName() %>');
  expect('jsp-scriptlet','<% int n = 0; %>');
  assert.doesNotMatch('<%-- comment --%>',new RegExp(r['jsp-scriptlet'].begin));
  expect('tag','<c:forEach items="\\u0024{items}">');
  expect('el','\\u0024{bean.value}');
  expect('el','#{bean.value}');
  assert.doesNotMatch('\\\\\\u0024{literal}',new RegExp(r.el.begin));
});
test('Every platform packager includes the JSP grammar in the installable package',()=>{
  for(const path of ['scripts/assemble-package.sh','scripts/package/build-linux.mjs','scripts/package/build-windows.mjs','scripts/verify-package.sh'])
    assert.match(readFileSync(resolve(root,path),'utf8'),/grammars/,path);
});
