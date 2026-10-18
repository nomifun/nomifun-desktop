// A read-only observer keeps the original process object alive across exit.
// It never requests terminate rights or infers exit codes from a later PID.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {powershell,psArguments,psLiteral} from './global-os-windows-cases.mjs';
const schema='nomifun.windows-retained-process-exit.v1';
const access='QUERY_LIMITED_INFORMATION|SYNCHRONIZE';
export function windowsCloseProcessIdentity(report){
  assert(Number.isSafeInteger(report.app_pid)&&report.app_pid>1);
  const native=report.actual_app_identity?.native_process;
  assert(native&&native.pid===report.app_pid&&native.state==='running','A ready fixture must already record the exact App creation identity');
  assert.match(native.creation_filetime,/^[1-9][0-9]+$/);
  return{pid:report.app_pid,creationFiletime:native.creation_filetime};
}
export function windowsProcessExitObserverScript({pid,creationFiletime,timeoutMs=60000}){
  assert(Number.isSafeInteger(pid)&&pid>1);assert.match(creationFiletime,/^[1-9][0-9]+$/);
  assert(Number.isSafeInteger(timeoutMs)&&timeoutMs>0&&timeoutMs<=60000);
  return String.raw`Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public sealed class NomiFunRetainedProcessExit {
  [StructLayout(LayoutKind.Sequential)] struct FT { public uint Low; public uint High; }
  [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetProcessTimes(IntPtr handle,out FT creation,out FT exit,out FT kernel,out FT user);
  [DllImport("kernel32.dll",SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr handle,out uint code);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool CloseHandle(IntPtr handle);
  IntPtr handle;
  public NomiFunRetainedProcessExit(uint pid,string expected) {
    handle=OpenProcess(0x00101000,false,pid);
    if(handle==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
    try {
      FT creation,exit,kernel,user;
      if(!GetProcessTimes(handle,out creation,out exit,out kernel,out user)) throw new Win32Exception(Marshal.GetLastWin32Error());
      if((((ulong)creation.High<<32)|creation.Low).ToString()!=expected) throw new InvalidOperationException("Creation identity differs from the original ready fixture");
      uint state=WaitForSingleObject(handle,0);
      if(state!=258) throw new InvalidOperationException("The exact process was not running before dispatch");
    } catch { CloseHandle(handle);handle=IntPtr.Zero;throw; }
  }
  public object Wait(uint timeout) {
    uint state=WaitForSingleObject(handle,timeout);
    if(state==258) return new {status="timeout",wait_status=state,exit_code=(uint?)null,raw_os_error=(int?)null};
    if(state!=0) return new {status="wait_failed",wait_status=state,exit_code=(uint?)null,raw_os_error=state==0xffffffff?(int?)Marshal.GetLastWin32Error():null};
    uint code;if(!GetExitCodeProcess(handle,out code)) return new {status="exit_code_unavailable",wait_status=state,exit_code=(uint?)null,raw_os_error=(int?)Marshal.GetLastWin32Error()};
    return new {status="exited",wait_status=state,exit_code=(uint?)code,raw_os_error=(int?)null};
  }
  public int Close() {
    IntPtr closing=handle;handle=IntPtr.Zero;
    if(closing==IntPtr.Zero) return 0;
    return CloseHandle(closing)?0:Marshal.GetLastWin32Error();
  }
}
'@
$observer=$null;$receipt=$null;$ready=$false;$closeError=$null;
try {
  $observer=[NomiFunRetainedProcessExit]::new(${pid},${psLiteral(creationFiletime)});
  $ready=$true;
  [Console]::Out.WriteLine((([ordered]@{schema='${schema}';stage='ready';pid=${pid};creation_filetime=${psLiteral(creationFiletime)};access='${access}';handle_retained=$true;running_before_dispatch=$true})|ConvertTo-Json -Compress));[Console]::Out.Flush();
  $receipt=$observer.Wait(${timeoutMs});
} catch {
  $raw=$null;if($_.Exception.InnerException-is[ComponentModel.Win32Exception]){$raw=$_.Exception.InnerException.NativeErrorCode};
  $receipt=[pscustomobject]@{status='observer_failed';wait_status=$null;exit_code=$null;raw_os_error=$raw};
} finally {if($null-ne$observer){$closeError=$observer.Close()}}
[Console]::Out.WriteLine((([ordered]@{schema='${schema}';stage='result';pid=${pid};creation_filetime=${psLiteral(creationFiletime)};access='${access}';ready_emitted=$ready;receipt=$receipt;handle_closed=($null-ne$observer-and$closeError-eq0);close_raw_os_error=$closeError})|ConvertTo-Json -Compress -Depth 6));[Console]::Out.Flush();`;
}
export function validateWindowsExitReady(value,identity){
  assert.equal(value.schema,schema);assert.equal(value.stage,'ready');assert.equal(value.pid,identity.pid);assert.equal(value.creation_filetime,identity.creationFiletime);
  assert.equal(value.access,access);assert.equal(value.handle_retained,true);assert.equal(value.running_before_dispatch,true);return value;
}
export function validateWindowsExitResult(value,identity){
  assert.equal(value.schema,schema);assert.equal(value.stage,'result');assert.equal(value.pid,identity.pid);assert.equal(value.creation_filetime,identity.creationFiletime);assert.equal(value.access,access);
  assert.equal(value.ready_emitted,true);assert.equal(value.handle_closed,true);assert.equal(value.close_raw_os_error,0);
  const receipt=value.receipt;assert.equal(receipt.status,'exited');assert.equal(receipt.wait_status,0);assert.equal(receipt.raw_os_error,null);
  assert(Number.isSafeInteger(receipt.exit_code)&&receipt.exit_code>=0&&receipt.exit_code<=0xffffffff);
  return{...value,process_exit_proven:true,overall_cleanup_proven:false,exit_code:receipt.exit_code};
}
export async function beginWindowsProcessExitObservation(identity){
  assert.equal(process.platform,'win32');const script=windowsProcessExitObserverScript(identity),scriptSha=createHash('sha256').update(script).digest('hex');
  const child=spawn(powershell,psArguments(script),{windowsHide:true,stdio:['ignore','pipe','pipe']});
  let readyResolve,readyReject,resultResolve,resultReject,buffer='',byteLength=0,ready,result,failure,stderrBytes=0;
  const readyPromise=new Promise((done,fail)=>{readyResolve=done;readyReject=fail;}),completion=new Promise((done,fail)=>{resultResolve=done;resultReject=fail;});
  // A failed readiness waiter must not leave an unhandled completion promise.
  completion.catch(()=>{});
  const reject=reason=>{failure??=reason;readyReject(failure);};
  const readyTimer=setTimeout(()=>reject(new Error('Retained process observer readiness was not proven before dispatch')),15000);
  const finalTimer=setTimeout(()=>{const error=new Error('Retained process exit/observer cleanup was not proven within 90 seconds');reject(error);resultReject(error);},90000);
  child.stdout.setEncoding('utf8');child.stdout.on('data',text=>{try{
    byteLength+=Buffer.byteLength(text);assert(byteLength<=8192,'Bounded retained-process observation exceeded');buffer+=text;
    for(let at;(at=buffer.indexOf('\n'))>=0;){const line=buffer.slice(0,at).trim();buffer=buffer.slice(at+1);if(!line)continue;const value=JSON.parse(line);
      if(value.stage==='ready'){assert(!ready&&!result);ready=validateWindowsExitReady(value,identity);clearTimeout(readyTimer);readyResolve(ready);}
      else{assert(!result);result=value;if(!ready)failure??=new Error('The exact original process handle was not acquired; no exit request is authorized');}
    }
  }catch{reject(new Error('Retained process observer returned an invalid bounded protocol'));}});
  child.stderr.on('data',bytes=>{stderrBytes+=bytes.length;if(stderrBytes>16384)reject(new Error('Retained process observer diagnostic budget exceeded'));});
  child.once('error',()=>reject(new Error('Read-only retained process observer could not start')));
  child.once('close',(code,signal)=>{clearTimeout(readyTimer);clearTimeout(finalTimer);try{
    if(failure)throw failure;assert.equal(buffer.trim(),'');assert.equal(code,0);assert.equal(signal,null);assert(ready&&result);
    const value=validateWindowsExitResult(result,identity);resultResolve({...value,observer_pid:child.pid,observer_exit_code:code,observer_reaped:true,script_sha256:scriptSha});
  }catch{const error=failure??new Error('The retained process exit code or observer cleanup remains unproven');error.observation={ready:ready??null,result:result??null,observer_pid:child.pid,observer_exit_code:code,observer_signal:signal,script_sha256:scriptSha};readyReject(error);resultReject(error);}});
  await readyPromise;return{ready,observer_pid:child.pid,script_sha256:scriptSha,completion};
}
