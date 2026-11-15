// Captures only the public non-sensitive projection for plugin-owned visual checks.
import {mkdirSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
export function capture(language,snapshot){
 const root=resolve('.artifacts/compiler');mkdirSync(root,{recursive:true});
 writeFileSync(join(root,`visual-${language}.json`),JSON.stringify(snapshot,null,2));
}
