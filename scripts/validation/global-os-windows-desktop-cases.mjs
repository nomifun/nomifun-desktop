// All desktop effects use the existing Agent Computer actions. Host processes
// query only Calculator identity. Reports omit private pixels/window/UIA text.
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { sanitizeDesktopReport, normalizeCalculatorBidi } from './global-os-desktop-cases.mjs';
import { assertWindowsFixture, powershell, psArguments, psLiteral, windowsObservation, windowsProcessGone } from './global-os-windows-cases.mjs';

const delay=ms=>new Promise(done=>setTimeout(done,ms));
const resultText=value=>{assert.equal(typeof value.result?.text,'string');return value.result.text;};
const observeGeneration=value=>{assert(Number.isSafeInteger(value.generation)&&value.generation>0);return value.generation;};
const safeDesktopFailure=error=>{
  const code=/\b(?:SYSTEM_[A-Z_]+|ROLE_HOST_[A-Z_]+|COMPUTER_[A-Z_]+)\b/.exec(String(error?.message??''))?.[0],step=/^desktop-[a-z0-9-]+$/.test(error?.safe_step??'')?error.safe_step:undefined;
  return{name:error?.name??'Error',code,step,message:`Windows native desktop acceptance did not complete${code?` (${code})`:''}${step?` at ${step}`:''}; private UIA, pixels and raw provider bodies omitted.`};
};
export function sanitizeWindowsDesktopReport(report,nativeEvidence=[]){
  const sanitized=sanitizeDesktopReport(report,nativeEvidence),metadata={};
  for(const key of ['failure','failure_cleanup','recovery_cleanup_failure','recovery_cleanup_unproven']){const failure=report[key];if(!failure)continue;const code=/^(?:SYSTEM_|ROLE_HOST_|COMPUTER_)[A-Z_]+$/.test(failure.code??'')?failure.code:undefined,step=/^desktop-[a-z0-9-]+$/.test(failure.step??'')?failure.step:undefined;if(code||step)metadata[key]={code,step};}
  return{...sanitized,windows_failure_metadata:metadata};
}
const calculatorPids=async()=>windowsObservation("ConvertTo-Json -InputObject @(Get-Process -Name 'CalculatorApp','Calculator' -ErrorAction SilentlyContinue|ForEach-Object{[pscustomobject]@{pid=$_.Id;session_id=$_.SessionId}}) -Compress");
// Windows 10 can put a CalculatorApp child in an ApplicationFrameHost window.
// Bind that actual frame to the exact new Calculator PID before Computer may
// focus it, rather than assuming a process/app label identifies the window.
export async function windowsOwnedApplicationWindows(pid){
  assert(Number.isSafeInteger(pid)&&pid>1);
  return windowsObservation(`Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class NomiFunAcceptanceWindows {
  public sealed class Owned { public long handle; public uint owner_pid; public bool owned_descendant; public string title; }
  delegate bool EnumProc(IntPtr window, IntPtr state);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr state);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc proc, IntPtr state);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int maximum);
  public static Owned[] Find(uint pid) {
    var owned=new List<Owned>();
    EnumWindows((window, state)=>{
      if(!IsWindowVisible(window)) return true;
      uint owner; GetWindowThreadProcessId(window,out owner); bool descendant=false;
      if(owner!=pid) EnumChildWindows(window,(child,unused)=>{uint childPid;GetWindowThreadProcessId(child,out childPid);if(childPid==pid)descendant=true;return !descendant;},IntPtr.Zero);
      if(owner==pid||descendant){var text=new StringBuilder(1024);GetWindowText(window,text,text.Capacity);owned.Add(new Owned{handle=window.ToInt64(),owner_pid=owner,owned_descendant=descendant,title=text.ToString()});}
      return true;
    },IntPtr.Zero);
    return owned.ToArray();
  }
}
'@
ConvertTo-Json -InputObject @([NomiFunAcceptanceWindows]::Find(${pid})) -Compress`);
}
// TokenIntegrityLevel comes from the real process token, independently of the
// executable name, launch intent or membership in the Administrators group.
const integrityProbe=`Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class NomiFunAcceptanceIntegrity {
  [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool CloseHandle(IntPtr handle);
  [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
  [DllImport("advapi32.dll", SetLastError=true)] static extern bool GetTokenInformation(IntPtr token, int kind, IntPtr data, int size, out int needed);
  [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);
  [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr sid, uint index);
  public static uint Read(int pid) {
    IntPtr process=OpenProcess(0x1000, false, pid), token=IntPtr.Zero, data=IntPtr.Zero;
    if(process==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
    try {
      if(!OpenProcessToken(process, 8, out token)) throw new Win32Exception(Marshal.GetLastWin32Error());
      int needed; GetTokenInformation(token, 25, IntPtr.Zero, 0, out needed);
      if(needed<=0) throw new Win32Exception(Marshal.GetLastWin32Error());
      data=Marshal.AllocHGlobal(needed);
      if(!GetTokenInformation(token, 25, data, needed, out needed)) throw new Win32Exception(Marshal.GetLastWin32Error());
      IntPtr sid=Marshal.ReadIntPtr(data); byte count=Marshal.ReadByte(GetSidSubAuthorityCount(sid));
      if(count==0) throw new InvalidOperationException("Integrity SID has no authority");
      return unchecked((uint)Marshal.ReadInt32(GetSidSubAuthority(sid, (uint)count-1)));
    } finally { if(data!=IntPtr.Zero) Marshal.FreeHGlobal(data); if(token!=IntPtr.Zero) CloseHandle(token); CloseHandle(process); }
  }
}
'@`;
export const windowsIntegrityObservationScript=pid=>`${integrityProbe}\n[NomiFunAcceptanceIntegrity]::Read(${pid})|ConvertTo-Json -Compress`;
const quoted=token=>JSON.parse(token.replace(/\\u\{([0-9a-fA-F]{1,6})\}/g,(_,hex)=>String.fromCodePoint(Number.parseInt(hex,16))));
export function windowsUiaEntries(text){
  let window;
  const entries=[];
  for(const line of text.split('\n')){
    const owner=/^[│ └├─]*window\s+("(?:\\.|[^"\\\r\n])*")\s*$/.exec(line);
    if(owner){window=quoted(owner[1]);continue;}
    const item=/^[│ └├─]*\[(\d+)\]\s+(\w+)(?:\s+("(?:\\.|[^"\\\r\n])*"))?(?:\s+=\s+("(?:\\.|[^"\\\r\n])*"))?(?:\s+\(-?\d+,-?\d+\))?(?:\s+\[action: [a-z]+\])?(?:\s+\[[^\]\r\n]*\])?\s*$/.exec(line);
    if(item)entries.push({ref:Number(item[1]),role:item[2],name:item[3]?quoted(item[3]):undefined,value:item[4]?quoted(item[4]):undefined,window});
  }
  return entries;
}
export function windowsCalculatorDisplayHasExactValue(text,expected,window){
  if(!/^\d+$/.test(expected))return false;
  return windowsUiaEntries(text).some(entry=>{
    if(!['text','edit','document'].includes(entry.role)||(window!==undefined&&entry.window!==window))return false;
    const token=normalizeCalculatorBidi(entry.value??entry.name??'');
    return token===expected||token===`Display is ${expected}`||token===`显示为 ${expected}`||token===`显示为${expected}`;
  });
}
const findDigitOne=(text,window)=>{
  const labels=new Set(['1','One','一']);const matches=windowsUiaEntries(text).filter(entry=>entry.role==='button'&&entry.window===window&&labels.has(entry.name));
  assert.equal(matches.length,1,'A fresh Calculator UIA snapshot must expose exactly one digit-one button');return matches[0].ref;
};
export async function createWindowsDesktopScenario({fixture,workspace}){
  await assertWindowsFixture(fixture,workspace);assert.deepEqual(await calculatorPids(),[],'Calculator is already running; preserve the existing application');
  const nativeEvidence=[],resources={application:'calc.exe',pid:undefined,window_id:undefined,started_by_this_case:false,quit_confirmed:false,expected_calculation:'12+34=46'};
  let generation,oneRef;
  const inputResult=async value=>{assert.equal(value.generation,0);assert(resultText(value).length);await delay(350);};
  const window=id=>({id,name:'computer_observe',args:{action:'list_windows'},repeatable:true,maxAttempts:8,verify:async value=>{
    generation=observeGeneration(value);const pids=await calculatorPids();assert.equal(pids.length,1);assert.equal(pids[0].pid,resources.pid);
    const owned=await windowsOwnedApplicationWindows(resources.pid),matches=[...resultText(value).matchAll(/^- id=(\d+) app=("(?:\\.|[^"\\])*") title=("(?:\\.|[^"\\])*") at \((-?\d+), (-?\d+)\) size (\d+)x(\d+)( \[focused\])?$/gm)].filter(m=>owned.some(window=>window.handle===Number(m[1])&&window.title===JSON.parse(m[3])));
    if(!matches.length){await delay(200);return false;}assert.equal(matches.length,1);resources.window_id=Number(matches[0][1]);resources.window_title=JSON.parse(matches[0][3]);nativeEvidence.push({step:id,kind:'owned_calculator_native_window',pid:resources.pid,window_id:resources.window_id,independent:owned.find(window=>window.handle===resources.window_id),admitted_generation:generation});
  }});
  const focus=id=>({id,name:'computer_input',args:()=>({action:'focus_window',window_id:resources.window_id,expected_generation:generation}),verify:inputResult});
  const inspect=(id,expected,chooseOne=false)=>({id,name:'computer_inspect',args:{action:'observe'},verify:value=>{
    generation=observeGeneration(value);const text=resultText(value);assert(windowsUiaEntries(text).some(entry=>entry.window===resources.window_title),'Fresh UIA must include this exact owned Calculator frame before any input');
    if(expected!==undefined)assert(windowsCalculatorDisplayHasExactValue(text,expected,resources.window_title),`The fresh Calculator display did not establish expected value ${expected}`);if(chooseOne)oneRef=findDigitOne(text,resources.window_title);
    nativeEvidence.push({step:id,kind:'actual_calculator_uia',admitted_generation:generation,verified_display:expected,digit_one_ref:chooseOne?oneRef:undefined});
  }});
  const key=(id,combo)=>({id,name:'computer_input',args:()=>({action:'key',key:combo,expected_generation:generation}),verify:inputResult});
  const screenshot=id=>({id,name:'computer_observe',args:{action:'screenshot'},verify:(value,context)=>{
    observeGeneration(value);assert.equal(value.kind,'computer_screenshot');assert.equal(value.image_count,1);
    const dimensions=/Screenshot captured: (\d+)x(\d+) \(display (\d+), scaled from (\d+)x(\d+) physical\)/.exec(value.text);assert(dimensions,'Screenshot must report actual native dimensions');
    const images=[];for(const message of context.messages.filter(m=>m.role==='user'&&Array.isArray(m.content))){let call;for(const part of message.content){if(part.type==='text')call=/^Attachments returned by tool call (.+): $/.exec(part.text)?.[1];else if(call===context.callId&&part.type==='image_url')images.push(part.image_url?.url);}}
    assert.equal(images.length,1);assert(images[0].startsWith('data:image/png;base64,'));const data=images[0].slice('data:image/png;base64,'.length);assert.equal(data.length,value.encoded_bytes);const bytes=Buffer.from(data,'base64');assert(bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])));assert.equal(bytes.readUInt32BE(16),Number(dimensions[1]));assert.equal(bytes.readUInt32BE(20),Number(dimensions[2]));
    nativeEvidence.push({step:id,kind:'actual_native_png',width:Number(dimensions[1]),height:Number(dimensions[2]),physical_width:Number(dimensions[4]),physical_height:Number(dimensions[5]),bytes:bytes.length,pixels_retained:false});
  }});
  const quit=prefix=>[window(`${prefix}-window`),focus(`${prefix}-focus`),inspect(`${prefix}-uia`),key(`${prefix}-alt-f4`,'alt+f4'),{id:`${prefix}-confirm-exit`,name:'computer_observe',args:{action:'wait',seconds:0.25},repeatable:true,maxAttempts:20,verify:()=>{if(!windowsProcessGone(resources.pid))return false;resources.quit_confirmed=true;nativeEvidence.push({step:`${prefix}-confirm-exit`,kind:'exact_owned_pid_exit',pid:resources.pid,gone:true});}}];
  const steps=[{id:'desktop-launch',name:'launch_app',args:async()=>{assert.deepEqual(await calculatorPids(),[],'Calculator appeared before launch; preserve it');return {action:'launch',target:join(process.env.SystemRoot,'System32','calc.exe')};},verify:async value=>{
    assert(resultText(value).length);for(let i=0;i<80;i++){const pids=await calculatorPids();if(pids.length){assert.equal(pids.length,1);resources.pid=pids[0].pid;resources.started_by_this_case=true;nativeEvidence.push({step:'desktop-launch',kind:'independent_new_calculator_pid',...pids[0],preexisting:false});return;}await delay(100);}throw new Error('Calculator launch did not establish a unique native process');
  }},window('desktop-window'),focus('desktop-focus'),screenshot('desktop-before'),inspect('desktop-before-clear'),key('desktop-clear','escape'),inspect('desktop-after-clear','0',true),
    {id:'desktop-click-one',name:'computer_input',args:()=>({action:'click_element',ref:oneRef,expected_generation:generation}),verify:inputResult},inspect('desktop-after-click','1'),key('desktop-clear-again','escape'),inspect('desktop-clear-zero','0')];
  for(const [id,combo,expected]of[['one','1','1'],['two','2','12'],['plus','shift+=',undefined],['three','3','3'],['four','4','34'],['equals','enter','46']])steps.push(key(`desktop-key-${id}`,combo),inspect(`desktop-read-${id}`,expected));
  steps.push(screenshot('desktop-result'),...quit('desktop-quit'));
  return{title:'Windows ordinary Calculator UIA/focus/click/keyboard/native screenshot acceptance',resources,nativeEvidence,steps,modelTraits:['vision_input'],
    prompt:`只通过默认Agent Computer工具操作本次新启动的系统Calculator；已有Calculator时立即拒绝。真实list/focus/screenshot/UIA，清除当前算式后以fresh UIA ref点击1核验独立1，再键入12+34=46并以新UIA核验独立46；最后新观察后Alt+F4退出并确认本次PID退出。不读写剪贴板、历史或设置，不操作其他用户应用。截图和全局UIA只在运行期使用，不写入验收报告。本用例只测普通登录桌面，不声称高完整性或安全桌面通过。`,
    completionSummary:()=>`默认Agent在Windows普通登录桌面只操作本次Calculator PID ${resources.pid}：真实截图${nativeEvidence.filter(e=>e.kind==='actual_native_png').length}张；fresh UIA/ref点击1、键盘12+34及最终独立46均按步骤核验，新观察后Alt+F4和确切PID退出=${resources.quit_confirmed}。报告不保留原始像素/UIA/全局窗口文本。高完整性、UIPI/安全桌面及媒体未由本用例证明。`,
    cleanupNeeded:()=>resources.started_by_this_case&&!resources.quit_confirmed&&!windowsProcessGone(resources.pid),
    get cleanupSteps(){return resources.started_by_this_case&&!windowsProcessGone(resources.pid)?quit('desktop-cleanup'):[];},
    cleanupPrompt:'原桌面验收失败保持失败；原Turn已取消且效果和原生清理已结算。只取得fresh观察并通过Computer退出本次Calculator确切PID，不改已有应用或用户内容。',
    cleanupIntent:{temporary_process:()=>resources.pid,only_owned_calculator:true},
    pendingAcceptance:[{id:'elevated-secure-desktop',status:'not_run',reason:'Ordinary Calculator does not establish high-integrity UIA/input or secure-desktop support.'},{id:'multiscreen-media-clipboard',status:'not_run',reason:'These independent operations require separate native acceptance.'}],
    sanitizeReport:report=>sanitizeWindowsDesktopReport(report,nativeEvidence),sanitizeFailure:safeDesktopFailure,
  };
}

export async function createWindowsHighDesktopScenario({fixture,workspace,loginIdentity}) {
  await assertWindowsFixture(fixture,workspace);
  const nonce=randomUUID(),title=`NomiFun high integrity ${nonce}`,inputName=`NomiFun input ${nonce}`,buttonName=`Apply ${nonce}`,pending=`NOMIFUN_PENDING_${nonce}`,literal='NomiFun High 中文 12+34',expected=`NOMIFUN_RESULT_${nonce}:${literal}`;
  const script=`${integrityProbe}\nAdd-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $form=New-Object Windows.Forms.Form; $form.Text=${psLiteral(title)}; $form.Width=880; $form.Height=600; $form.StartPosition='CenterScreen'; $field=New-Object Windows.Forms.TextBox; $field.AccessibleName=${psLiteral(inputName)}; $field.Location=New-Object Drawing.Point(30,40); $field.Width=700; $form.Controls.Add($field); $button=New-Object Windows.Forms.Button; $button.Text=${psLiteral(buttonName)}; $button.Location=New-Object Drawing.Point(30,90); $button.Width=650; $form.Controls.Add($button); $label=New-Object Windows.Forms.Label; $label.Text=${psLiteral(pending)}; $label.Location=New-Object Drawing.Point(30,150); $label.AutoSize=$true; $form.Controls.Add($label); $button.Add_Click({$label.Text=${psLiteral(`NOMIFUN_RESULT_${nonce}:`)}+$field.Text}); $form.Add_Shown({$i=[Security.Principal.WindowsIdentity]::GetCurrent(); $p=New-Object Security.Principal.WindowsPrincipal($i); [pscustomobject]@{fixture=${psLiteral(nonce)};pid=$PID;sid=$i.User.Value;elevated=$p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);integrity_rid=[NomiFunAcceptanceIntegrity]::Read($PID);session_id=(Get-Process -Id $PID).SessionId;window_handle=$form.Handle.ToInt64().ToString();profile=$env:USERPROFILE}|ConvertTo-Json -Compress|ForEach-Object{[Console]::WriteLine($_)}}); [Windows.Forms.Application]::Run($form)`;
  const nativeEvidence=[],resources={application:'Windows PowerShell WinForms isolated fixture',fixture_nonce:nonce,pid:undefined,window_id:undefined,started_by_this_case:false,quit_confirmed:false,actual_high_integrity:false};
  let processId,cursor=0,output='',generation,fieldRef,buttonRef;
  const settle=async value=>{assert.equal(value.generation,0);assert(resultText(value).length);await delay(350);};
  const inspect=(id,{field=false,button=false,result=false}={})=>({id,name:'computer_inspect',args:{action:'observe'},verify:value=>{
    generation=observeGeneration(value);const items=windowsUiaEntries(resultText(value)).filter(item=>item.window===title);assert(items.length>0,'Fresh UIA must include controls in this exact owned high-integrity fixture window');
    if(field){const matches=items.filter(item=>item.role==='edit'&&item.name===inputName);assert.equal(matches.length,1);fieldRef=matches[0].ref;}
    if(button){const matches=items.filter(item=>item.role==='button'&&item.name===buttonName);assert.equal(matches.length,1);buttonRef=matches[0].ref;}
    if(result){assert(items.some(item=>item.role!=='button'&&(item.value??item.name)===expected),'The exact high-integrity UIA result label must reflect actual entered text and button action');resources.result_verified=true;}
    nativeEvidence.push({step:id,kind:'exact_high_integrity_fixture_uia',pid:resources.pid,window_id:resources.window_id,admitted_generation:generation,result_verified:result||undefined});
  }});
  const listWindows=id=>({id,name:'computer_observe',args:{action:'list_windows'},verify:value=>{generation=observeGeneration(value);const matches=[...resultText(value).matchAll(/^- id=(\d+) app=("(?:\\.|[^"\\])*") title=("(?:\\.|[^"\\])*") at \((-?\d+), (-?\d+)\) size (\d+)x(\d+)( \[focused\])?$/gm)].filter(match=>Number(match[1])===resources.window_id&&JSON.parse(match[3])===title);assert.equal(matches.length,1,'The actual Computer window list must identify the exact elevated fixture HWND/title');}});
  const steps=[{id:'desktop-high-start',name:'start_process',args:{command:powershell,args:['-STA',...psArguments(script)],cwd:workspace,execution_identity:'administrator',wait_ms:0,timeout_ms:180000},verify:value=>{
    assert.equal(value.state,'running');processId=value.process_id;resources.pid=value.pid;cursor=value.output.next_cursor;output=value.output.text;resources.started_by_this_case=true;assert(!windowsProcessGone(resources.pid));
  }},{id:'desktop-high-poll-ready',name:'poll_process',repeatable:true,maxAttempts:20,args:()=>({process_id:processId,cursor,wait_ms:1000}),verify:async value=>{
    assert.equal(value.state,'running');cursor=value.output.next_cursor;output+=value.output.text;const lines=output.replaceAll('\r\n','\n').trim().split('\n');const line=lines.find(line=>line.startsWith('{')&&line.includes(nonce));if(!line)return false;const identity=JSON.parse(line);
    assert.equal(identity.fixture,nonce);assert.equal(identity.pid,resources.pid);assert.equal(identity.sid,loginIdentity.sid);assert.equal(identity.elevated,true);assert.equal(identity.integrity_rid,12288,'The actual fixture token must have High Mandatory integrity');assert.equal(identity.session_id,loginIdentity.session_id);assert.equal(identity.profile.toLowerCase(),loginIdentity.profile.toLowerCase());assert(/^\d+$/.test(identity.window_handle));resources.window_id=Number(identity.window_handle);assert(Number.isSafeInteger(resources.window_id)&&resources.window_id>0);
    const observed=await windowsObservation(`$p=Get-Process -Id ${resources.pid} -ErrorAction Stop;[pscustomobject]@{pid=$p.Id;session_id=$p.SessionId;window_handle=$p.MainWindowHandle.ToInt64().ToString()}|ConvertTo-Json -Compress`);assert.equal(observed.pid,resources.pid);assert.equal(observed.session_id,identity.session_id);assert.equal(observed.window_handle,identity.window_handle);resources.actual_high_integrity=true;nativeEvidence.push({step:'desktop-high-poll-ready',kind:'actual_elevated_owned_window',...identity,independent:observed});
  }},listWindows('desktop-high-list'),
    {id:'desktop-high-route-admission',name:'computer_input',expectedComputerNoEffectAdmission:true,args:()=>({action:'focus_window',window_id:resources.window_id,expected_generation:generation}),verifyNoEffect:proof=>{generation=undefined;nativeEvidence.push({step:'desktop-high-route-admission',kind:'typed_canonical_no_effect_bridge_admission',...proof});}},
    listWindows('desktop-high-bridge-list'),
    {id:'desktop-high-focus',name:'computer_input',args:()=>({action:'focus_window',window_id:resources.window_id,expected_generation:generation}),verify:settle},
    inspect('desktop-high-field',{field:true}),{id:'desktop-high-click-field',name:'computer_input',args:()=>({action:'click_element',ref:fieldRef,expected_generation:generation}),verify:settle},
    inspect('desktop-high-before-type'),{id:'desktop-high-type',name:'computer_input',args:()=>({action:'type',text:literal,expected_generation:generation}),verify:settle},
    inspect('desktop-high-button',{button:true}),{id:'desktop-high-click-button',name:'computer_input',args:()=>({action:'click_element',ref:buttonRef,expected_generation:generation}),verify:settle},inspect('desktop-high-result',{result:true}),
    {id:'desktop-high-screenshot',name:'computer_observe',args:{action:'screenshot'},verify:(value,context)=>{observeGeneration(value);assert.equal(value.kind,'computer_screenshot');assert.equal(value.image_count,1);const images=[];for(const message of context.messages.filter(m=>m.role==='user'&&Array.isArray(m.content))){let call;for(const part of message.content){if(part.type==='text')call=/^Attachments returned by tool call (.+): $/.exec(part.text)?.[1];else if(call===context.callId&&part.type==='image_url')images.push(part.image_url?.url);}}assert.equal(images.length,1);assert(images[0].startsWith('data:image/png;base64,'));const bytes=Buffer.from(images[0].slice('data:image/png;base64,'.length),'base64');assert(bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])));nativeEvidence.push({step:'desktop-high-screenshot',kind:'actual_native_png',width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20),bytes:bytes.length,pixels_retained:false});}},
    {id:'desktop-high-cancel',name:'cancel_process',args:()=>({process_id:processId}),verify:value=>{assert.equal(value.state,'cancelled');assert.equal(value.cleanup.reaped,true);cursor=value.output.next_cursor;}},
    {id:'desktop-high-confirm-exit',name:'poll_process',allowCancelledObservation:true,args:()=>({process_id:processId,cursor,wait_ms:0}),verify:value=>{assert.equal(value.state,'cancelled');assert.equal(value.cleanup.reaped,true);assert(windowsProcessGone(resources.pid));resources.quit_confirmed=true;nativeEvidence.push({step:'desktop-high-confirm-exit',kind:'exact_owned_elevated_pid_exit',pid:resources.pid,gone:true});}},
  ];
  return{title:'Windows authenticated high-integrity Computer bridge acceptance',steps,resources,nativeEvidence,modelTraits:['vision_input'],
    prompt:`仅通过现役administrator start_process启动本次唯一WinForms测试窗口 ${title}，同ProcessSupervisor保留生命周期。核对真实elevated token、登录session、USERPROFILE和准确PID/HWND；默认Agent用Computer真实list/focus/fresh UIA/ref点击输入框、type本次文本、fresh ref点击唯一Apply按钮，再fresh UIA核对结果label已按真实文本变化，取得真实PNG。不得用宿主UI脚本或进程脚本代替Computer点击/输入；不得操作安全桌面或既有用户应用。最后现役cancel及terminal poll证明本次确切PID退出/reaped。`,
    completionSummary:()=>`默认Agent经真实管理员start_process创建本次WinForms PID ${resources.pid}/HWND ${resources.window_id}，自报elevated token、session/profile与独立process metadata一致。Computer fresh list/focus/UIA/ref click/type/button action后精确结果label核验=${resources.result_verified===true}；真实PNG不保存像素；同ProcessSupervisor cancel/terminal reaped与准确PID退出=${resources.quit_confirmed}。未触碰安全桌面，UAC允许不单独算桌面能力成功。`,
    cleanupNeeded:()=>false,cleanupSteps:[],cleanupIntent:{temporary_process:()=>({process_id:processId,pid:resources.pid}),action:'Only the original canonical Turn owner cancels/reaps this managed process; never reuse its process_id in a new Turn or send host PID signals.'},
    pendingAcceptance:[{id:'uac-deny-secure-desktop',status:'not_run',reason:'This allowed high-integrity fixture does not prove UAC rejection/cancel or permit security-desktop automation.'}],sanitizeReport:report=>sanitizeWindowsDesktopReport(report,nativeEvidence),sanitizeFailure:safeDesktopFailure,
  };
}
