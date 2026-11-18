// Checks the plugin's JSP grammar with a Lithe host's own TextMate tokenizer and the
// bundled HTML, CSS, JavaScript and Java grammars it includes. The plugin-owned test is
// added through a Go overlay; nothing is copied into the host checkout. The editor
// packages are pure Go, so this needs neither the Rust Core nor a display.
// Usage: node scripts/grammar/host-jsp.mjs <Lithe host checkout>
import {mkdirSync,realpathSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {runTool} from '../compiler/process.mjs';

const root=fileURLToPath(new URL('../..',import.meta.url));
if(!process.argv[2])throw Error('Usage: node scripts/grammar/host-jsp.mjs <Lithe host checkout>');
// Go matches overlay keys against real paths (macOS /tmp is a symlink).
const host=realpathSync.native(resolve(process.argv[2]));
const stage=join(root,'.artifacts/grammar');mkdirSync(stage,{recursive:true});
const name='internal/editor/syntax/plugin_jsp_grammar_test.go';
const overlay=join(stage,'overlay.json');
writeFileSync(overlay,JSON.stringify({Replace:{[join(host,'desktop',name)]:join(root,'e2e/host/desktop',name)}}));
const output=await runTool('go',['test','-overlay',overlay,'-tags','lithejsp','./internal/editor/syntax','-run','^TestPluginJSPGrammarOnHostTokenizer$','-count=1','-timeout=120s','-v'],{
 cwd:join(host,'desktop'),
 env:{...process.env,CGO_ENABLED:'0',LITHE_JSP_GRAMMAR:join(root,'io.github.lithe-idea.java-support/grammars/jsp.tmLanguage.json')},
 timeout:600000,
});
process.stdout.write(output);
