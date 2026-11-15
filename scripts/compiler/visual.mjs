// Plugin-owned tests are injected through a Go overlay; no test or plugin data
// is copied into the host repository. Pass an explicit host checkout to validate.
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {runTool} from './process.mjs';
const root=fileURLToPath(new URL('../..',import.meta.url));
const host=resolve(process.argv[2]??'');if(!process.argv[2])throw Error('Usage: node scripts/compiler/visual.mjs <Lithe host checkout>');
const manifest=JSON.parse(readFileSync(join(root,'compiler-host.json')));
const stage=join(root,'.artifacts/compiler');mkdirSync(stage,{recursive:true});
const target=join(host,'desktop/internal/settingswindow/plugin_native_source_test.go');
const overlay=join(stage,'overlay.json');writeFileSync(overlay,JSON.stringify({Replace:{[target]:join(root,'e2e/host/desktop/internal/settingswindow/plugin_native_source_test.go')}}));
const revision=(await runTool('git',['-C',host,'rev-parse','HEAD'])).toString().trim();
writeFileSync(join(stage,'visual-host.json'),JSON.stringify({repository:manifest.repository,revision,core:process.env.LITHE_CORE_RUST_DIR??'host pin'},null,2));
const result=await runTool(process.env.LITHE_TEST_BASH??'bash',['scripts/desktop/desktop-go.sh','test','-overlay',overlay,'-tags','vivahead lithevisual','./internal/settingswindow','-run','^TestPluginNativeSourceVisual$','-count=1','-timeout=120s'],{cwd:host,env:{...process.env,LITHE_NATIVE_PACKAGE:join(root,manifest.package),LITHE_NATIVE_SNAPSHOTS:stage,LITHE_NATIVE_OUTPUT:join(stage,'visual')},timeout:1200000});
process.stdout.write(result);
