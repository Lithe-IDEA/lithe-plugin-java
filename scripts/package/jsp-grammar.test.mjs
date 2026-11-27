// Static checks of the JSP language declaration and grammar. How the grammar colors
// real JSP is checked by the host's own tokenizer in scripts/grammar/host-jsp.mjs.
import assert from 'node:assert/strict';
import {readFileSync,statSync} from 'node:fs';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const root=resolve(fileURLToPath(new URL('../..',import.meta.url)));
const plugin='io.github.lithe-idea.java-support';
const manifest=JSON.parse(readFileSync(resolve(root,plugin,'plugin.json'),'utf8'));
const jsp=manifest.languages.find(x=>x.id==='jsp');
const grammar=JSON.parse(readFileSync(resolve(root,plugin,'grammars/jsp.tmLanguage.json'),'utf8'));
// Grammars of the host this one includes by scope; the host bundles them.
const HOST_SCOPES=new Set(['text.html.basic','source.java']);
function visit(node,includes=[]) {
  if (node&&typeof node==='object') {
    if (typeof node.include==='string') includes.push(node.include);
    // JavaScript has no lookbehind-free subset check; it does compile the Oniguruma
    // patterns this grammar uses (lookahead, lookbehind, classes).
    for (const k of ['match','begin','end']) if (typeof node[k]==='string') new RegExp(node[k]);
    for (const v of Object.values(node)) if (typeof v==='object') visit(v,includes);
  }
  return includes;
}
const begin=name=>new RegExp(grammar.repository[name].begin);

test('JSP language is syntax-only and leaves Java JDT LS alone',()=>{
  assert.deepEqual(jsp.fileExtensions,['jsp','jspf','tag','tagf']);
  assert.deepEqual(jsp.grammar,{kind:'textmate',path:'grammars/jsp.tmLanguage.json'});
  assert.deepEqual(jsp.configuration.blockComment,['<%--','--%>']);
  assert.equal(manifest.languageServers.length,1);
  assert.equal(manifest.languageServers[0].languageID,'java');
});
test('JSP grammar builds on the host HTML grammar and resolves every include',()=>{
  assert.equal(grammar.scopeName,'text.html.jsp');
  // HTML, with its embedded CSS and JavaScript, stays the host's: .jsp opened as HTML
  // before this language existed and must lose none of those colors.
  assert.equal(grammar.patterns.at(-1).include,'text.html.basic');
  assert.ok(Object.keys(grammar.injections).every(selector=>selector.startsWith('L:text.html.jsp')));
  for(const include of visit(grammar)){
    if(include.startsWith('#'))assert.ok(Object.hasOwn(grammar.repository,include.slice(1)),include);
    else assert.ok(HOST_SCOPES.has(include.split('#')[0]),include);
  }
  assert.ok(JSON.stringify(grammar).length<16000);
});
test('JSP blocks end at the first %> even inside Java comments',()=>{
  const java=grammar.repository.java.patterns;
  const line=java.find(rule=>rule.name==='comment.line.double-slash.java');
  assert.equal('<% n++; // note %><p>'.match(new RegExp(line.match))[0],'// note ');
  const block=java.find(rule=>rule.name==='comment.block.java');
  assert.match(' open %>',new RegExp(block.end));
  assert.doesNotMatch('<%-- comment --%>',begin('jsp-scriptlet'));
  for(const [name,source] of [['jsp-directive','<%@ taglib prefix="c" uri="jakarta.tags.core" %>'],['jsp-declaration','<%! int n = 0; %>'],['jsp-expression','<%= user.getName() %>'],['jsp-scriptlet','<% int n = 0; %>']])
    assert.match(source,begin(name),name);
});
test('EL and tag-library tags are recognized; escaped EL is not',()=>{
  const dollar=String.fromCharCode(36);
  assert.match(dollar+'{bean.value}',begin('el'));
  assert.match('#{bean.value}',begin('el'));
  assert.doesNotMatch(String.fromCharCode(92)+dollar+'{literal}',begin('el'));
  assert.match('<c:forEach items="x">',begin('custom-tag'));
  assert.match('<jsp:include page="/f.jsp"/>',begin('custom-tag'));
  assert.doesNotMatch('<div class="x">',begin('custom-tag'));
});
test('Every platform packager includes the JSP grammar and keeps its scripts executable',()=>{
  for(const path of ['scripts/assemble-package.sh','scripts/package/build-linux.mjs','scripts/package/build-windows.mjs','scripts/verify-package.sh'])
    assert.match(readFileSync(resolve(root,path),'utf8'),/grammars/,path);
  if(process.platform!=='win32')for(const path of ['scripts/assemble-package.sh','scripts/verify-package.sh'])
    assert.ok(statSync(resolve(root,path)).mode&0o111,`${path} must stay executable: build-package.sh and archive-package.sh run it directly`);
});
