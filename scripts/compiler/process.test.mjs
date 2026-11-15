import {test} from 'node:test';
import assert from 'node:assert/strict';
import {runTool} from './process.mjs';
test('bounded tool runner returns output and rejects a failed command',async()=>{
 assert.equal((await runTool(process.execPath,['-e','process.stdout.write("ready")'])).toString(),'ready');
 await assert.rejects(runTool(process.execPath,['-e','process.exit(3)']),/Tool failed/);
});
test('a tool deadline terminates an owned tree including its descendant',async()=>{
 const code=`const {spawn}=require('node:child_process');spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'inherit'});setInterval(()=>{},1000);`;
 await assert.rejects(runTool(process.execPath,['-e',code],{timeout:2500}),/Tool timed out/);
});
