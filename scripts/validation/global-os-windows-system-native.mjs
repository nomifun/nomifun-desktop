#!/usr/bin/env node
// Drives only the explicitly built library acceptance example. No App dataset,
// canonical Effect, user service, registry key or existing process is changed.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, mkdir, realpath, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { windowsLoginIdentity, windowsObservation, psLiteral } from './global-os-windows-cases.mjs';

const delay=ms=>new Promise(done=>setTimeout(done,ms));
const option=name=>{const i=process.argv.indexOf(name);return i<0?undefined:process.argv[i+1];};
export function validateNativeFixtureReceipt(value,{parentExit,exitCode,parentPid}) {
  assert.equal(value.canonical_data_involved,false,'A library fixture cannot settle canonical App effects');
  assert.equal(value.identity?.windows?.elevated,true);
  assert(Number.isSafeInteger(value.identity.windows.pid)&&value.identity.windows.pid>1);
  if(parentExit){
    assert.equal(exitCode,23);assert.equal(value.schema,'nomifun.windows-system-parent-exit-probe.v1');assert.equal(value.status,'ready_for_parent_exit');assert.equal(value.parent_pid,parentPid);assert.equal(value.parent_exit_code,23);assert.equal(value.parent_cleanup_requested,false);assert.equal(value.receipt_flushed,true);
    const pids=[value.identity.windows.pid,value.started?.pid,...value.descendant_pids];assert.equal(new Set(pids).size,pids.length);assert(pids.every(pid=>Number.isSafeInteger(pid)&&pid>1&&pid!==parentPid));
    assert.equal(value.exact_processes?.length,pids.length);
    for(const pid of pids){const identity=value.exact_processes.find(item=>item.pid===pid);assert(identity&&/^[0-9]{15,20}$/.test(identity.creation_filetime),'Every helper/child/descendant needs an exact native creation identity');}
  }else{
    assert.equal(exitCode,0);assert.equal(value.schema,'nomifun.windows-system-native-acceptance.v2');assert.equal(value.status,'passed');assert.equal(value.shutdown?.exact,true);assert.equal(value.facility_exit_proven,true);
    assert.deepEqual(value.cases?.map(item=>item.id),['readonly_complete_typed_receipt','pipe_cancel','pipe_stdin_close_natural_exit_late_terminal','native_deadline','conpty_resize_stdin_cancel']);
  }
  return value;
}
export async function runWindowsSystemNative({exe,reportDirectory,parentExit=false}) {
  assert.equal(process.platform,'win32');const login=await windowsLoginIdentity();exe=await realpath(resolve(exe));assert.equal(basename(exe),'windows_native.exe');const shape=await lstat(exe);assert(shape.isFile()&&!shape.isSymbolicLink());
  await mkdir(reportDirectory,{recursive:true});const nonce=randomUUID(),prefix=join(reportDirectory,`system-native-${parentExit?'parent-exit':'lifecycle'}-${nonce}`);
  const hash=createHash('sha256');for await(const bytes of createReadStream(exe))hash.update(bytes);
  const report={schema:'nomifun.windows-system-native-driver.v1',status:'running',scenario:parentExit?'parent_exit':'lifecycle',exe,exe_sha256:hash.digest('hex'),login_identity:login,canonical_data_involved:false,started_at:new Date().toISOString(),observations:[]};
  let stdout='',stderr='';const child=spawn(exe,parentExit?['--parent-exit-probe']:[],{cwd:dirname(exe),windowsHide:true,stdio:['ignore','pipe','pipe']});report.parent_pid=child.pid;
  child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
  const append=(stream,text)=>{if(Buffer.byteLength(stdout)+Buffer.byteLength(stderr)+Buffer.byteLength(text)>8*1024*1024)throw new Error('Native fixture receipt exceeded its output bound');if(stream==='stdout')stdout+=text;else stderr+=text;};
  child.stdout.on('data',text=>append('stdout',text));child.stderr.on('data',text=>append('stderr',text));
  try{
    const exit=await new Promise((done,fail)=>{child.once('error',fail);child.once('close',(code,signal)=>done({code,signal}));});report.exit=exit;
    await writeFile(`${prefix}.stdout.json`,stdout,{flag:'wx'});await writeFile(`${prefix}.stderr.log`,stderr,{flag:'wx'});
    const receipt=JSON.parse(stdout.trim());report.receipt=receipt;validateNativeFixtureReceipt(receipt,{parentExit,exitCode:exit.code,parentPid:child.pid});
    const identities=parentExit?receipt.exact_processes:[{pid:receipt.identity.windows.pid},...receipt.cases.flatMap(item=>item.started?[{pid:item.started.pid}]:item.receipt?.worker?.pid?[{pid:item.receipt.worker.pid}]:[])];
    const until=Date.now()+15000;
    while(Date.now()<until){
      const observation=await windowsObservation(`$targets=${psLiteral(JSON.stringify(identities))}|ConvertFrom-Json;$v=@();foreach($t in $targets){$p=$null;try{$p=Get-Process -Id $t.pid -ErrorAction Stop}catch{if($_.CategoryInfo.Category-ne'ObjectNotFound'){throw}};if($null-eq$p){$v+=[pscustomobject]@{pid=$t.pid;state='absent'}}else{$creation=$p.StartTime.ToUniversalTime().ToFileTimeUtc().ToString();$v+=[pscustomobject]@{pid=$t.pid;state=$(if($t.creation_filetime-and$creation-ne$t.creation_filetime){'pid_reused'}else{'same_process_alive'});creation_filetime=$creation}}};ConvertTo-Json -InputObject @($v) -Compress`);
      report.observations.push({at:new Date().toISOString(),processes:observation});
      if(observation.every(item=>['absent','pid_reused'].includes(item.state))){report.status='passed';report.all_exact_processes_gone=true;break;}await delay(100);
    }
    assert.equal(report.all_exact_processes_gone,true,'Native helper/child/descendant cleanup was not independently proven; no host signal or PID kill was sent');
    return report;
  }catch(error){report.status='failed';report.failure={name:error.name,message:error.message};throw error;}
  finally{report.finished_at=new Date().toISOString();report.stdout_bytes=Buffer.byteLength(stdout);report.stderr_bytes=Buffer.byteLength(stderr);await writeFile(`${prefix}.report.json`,`${JSON.stringify(report,null,2)}\n`,{flag:'wx'});console.log(JSON.stringify({status:report.status,report:`${prefix}.report.json`,parent_pid:child.pid}));}
}
if(process.argv[1]===fileURLToPath(import.meta.url)){
  if(process.argv.includes('--help'))console.log('node scripts/validation/global-os-windows-system-native.mjs --exe C:\\ABS\\windows_native.exe --report-dir C:\\ABS\\EVIDENCE [--parent-exit-probe]. Uses a new UAC facility; OS authentication remains native. Parent-exit mode intentionally exits its own library fixture 23, then independently checks exact helper/child/descendant PID+creation identities for 15 seconds. It never kills a PID or settles canonical App effects.');
  else{assert(option('--exe')&&option('--report-dir'));await runWindowsSystemNative({exe:option('--exe'),reportDirectory:option('--report-dir'),parentExit:process.argv.includes('--parent-exit-probe')});}
}
