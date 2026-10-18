import{test,expect}from'bun:test';
import{spawn}from'node:child_process';
import{windowsProcessObservation}from'./global-os-windows-cases.mjs';
import{windowsCloseProcessIdentity,windowsProcessExitObserverScript,validateWindowsExitReady,validateWindowsExitResult,beginWindowsProcessExitObservation}from'./global-os-windows-process-exit-observer.mjs';
const identity={pid:123,creationFiletime:'134366923802594258'};
const ready=()=>({schema:'nomifun.windows-retained-process-exit.v1',stage:'ready',pid:identity.pid,creation_filetime:identity.creationFiletime,access:'QUERY_LIMITED_INFORMATION|SYNCHRONIZE',handle_retained:true,running_before_dispatch:true});
const result=()=>({...ready(),stage:'result',ready_emitted:true,receipt:{status:'exited',wait_status:0,exit_code:23,raw_os_error:null},handle_closed:true,close_raw_os_error:0});
test('close requires the original ready creation identity and never grants terminate access',()=>{
  const report={app_pid:identity.pid,actual_app_identity:{native_process:{pid:identity.pid,state:'running',creation_filetime:identity.creationFiletime}}};
  expect(windowsCloseProcessIdentity(report)).toEqual(identity);expect(validateWindowsExitReady(ready(),identity).handle_retained).toBe(true);
  for(const mutate of [x=>{delete x.actual_app_identity;},x=>{x.actual_app_identity.native_process.creation_filetime='';},x=>{x.actual_app_identity.native_process.pid=999;}]){const x=structuredClone(report);mutate(x);expect(()=>windowsCloseProcessIdentity(x)).toThrow();}
  const script=windowsProcessExitObserverScript(identity);expect(script).toContain('OpenProcess(0x00101000,false,pid)');expect(script).toContain('[Console]::Out.Flush()');expect(script).not.toContain('TerminateProcess');expect(script).not.toContain('GetProcessById');expect(()=>windowsProcessExitObserverScript({...identity,timeoutMs:90001})).toThrow();
});
test('native exit proof requires the retained signaled handle and real code plus observer closure',()=>{
  const receipt=validateWindowsExitResult(result(),identity);expect(receipt.process_exit_proven).toBe(true);expect(receipt.exit_code).toBe(23);expect(receipt.overall_cleanup_proven).toBe(false);
  for(const mutate of [x=>{x.creation_filetime='1';},x=>{x.ready_emitted=false;},x=>{x.handle_closed=false;},x=>{x.close_raw_os_error=6;},x=>{x.receipt.wait_status=258;},x=>{x.receipt.status='timeout';},x=>{x.receipt.raw_os_error=5;},x=>{x.receipt.exit_code=null;}]){const x=result();mutate(x);expect(()=>validateWindowsExitResult(x,identity)).toThrow();}
  const code259=result();code259.receipt.exit_code=259;expect(validateWindowsExitResult(code259,identity).exit_code).toBe(259);
});
test.skipIf(process.platform!=='win32')('retained native observer reads code 23 from its own benign process after PID lifetime ends',async()=>{
  const child=spawn(process.execPath,['-e','setTimeout(()=>process.exit(24),30000);process.stdin.resume();process.stdin.on("end",()=>process.exit(23));'],{windowsHide:true,stdio:['pipe','ignore','ignore']});
  child.stdin.on('error',()=>{});const exit=new Promise((done,fail)=>{child.once('exit',(code,signal)=>done({code,signal}));child.once('error',fail);});
  try{
    const live=windowsProcessObservation(child.pid);expect(live.state).toBe('running');const observer=await beginWindowsProcessExitObservation({pid:child.pid,creationFiletime:live.creation_filetime});
    expect(observer.ready.handle_retained).toBe(true);child.stdin.end();expect(await exit).toEqual({code:23,signal:null});
    const receipt=await observer.completion;expect(receipt.process_exit_proven).toBe(true);expect(receipt.creation_filetime).toBe(live.creation_filetime);expect(receipt.exit_code).toBe(23);expect(receipt.handle_closed).toBe(true);expect(receipt.observer_reaped).toBe(true);expect(receipt.overall_cleanup_proven).toBe(false);
  }finally{child.stdin.end();await exit;}
},30000);
