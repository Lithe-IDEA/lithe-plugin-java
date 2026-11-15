// Integration-test tools run in owned process trees with bounded output and cleanup.
import {spawn,execFile} from 'node:child_process';
const LIMIT=2*1024*1024;
export function runTool(tool,args,{cwd,env=process.env,timeout=120000}={}) {
 let command=tool,parameters=args,verbatim=false;
 if(process.platform==='win32'&&/\.(cmd|bat)$/i.test(tool)) {
  const quote=value=>{if(/["%\r\n]/.test(value))throw Error('Unsupported batch test argument');return `"${value}"`;};
  command=process.env.ComSpec??'cmd.exe';parameters=['/d','/v:off','/s','/c',`"${[tool,...args].map(quote).join(' ')}"`];verbatim=true;
 }
 return new Promise((resolve,reject)=>{
  const child=spawn(command,parameters,{cwd,env,detached:process.platform!=='win32',windowsHide:true,windowsVerbatimArguments:verbatim,stdio:['ignore','pipe','pipe']});
  let output=Buffer.alloc(0),errorOutput=Buffer.alloc(0),failure,done=false,stopping=false,cleanup;
  const kill=()=>{
   if(!child.pid)return;
   if(process.platform==='win32') {
    if(child.exitCode!==null)return;
    execFile('taskkill.exe',['/PID',String(child.pid),'/T','/F'],{timeout:5000,windowsHide:true},error=>{if(error&&child.exitCode===null)failure??=error;});
   } else {try{process.kill(-child.pid,'SIGKILL');}catch(error){if(error.code!=='ESRCH')failure??=error;}}
  };
  const finish=()=>{
   if(done)return;done=true;clearTimeout(timer);clearTimeout(cleanup);
   process.off('SIGINT',interrupt);process.off('SIGTERM',interrupt);
   if(failure)reject(failure);else if(child.exitCode!==0)reject(Error(`Tool failed (${child.exitCode}): ${errorOutput.toString()}\n${output.toString()}`));else resolve(output);
  };
  const stop=error=>{
   failure??=error;if(stopping)return;stopping=true;kill();
   cleanup=setTimeout(()=>{kill();child.stdout.destroy();child.stderr.destroy();child.unref();failure=Error('Tool cleanup timed out',{cause:failure});finish();},6000);
  };
  const interrupt=()=>stop(Error('Tool interrupted'));
  const timer=setTimeout(()=>stop(Error('Tool timed out')),timeout);
  process.once('SIGINT',interrupt);process.once('SIGTERM',interrupt);
  child.stdout.on('data',bytes=>{if(output.length+bytes.length>LIMIT)stop(Error('Tool output exceeds limit'));else output=Buffer.concat([output,bytes]);});
  child.stderr.on('data',bytes=>{if(errorOutput.length+bytes.length>LIMIT)stop(Error('Tool error output exceeds limit'));else errorOutput=Buffer.concat([errorOutput,bytes]);});
  child.once('error',error=>stop(error));
  child.once('close',()=>{kill();finish();});
 });
}
