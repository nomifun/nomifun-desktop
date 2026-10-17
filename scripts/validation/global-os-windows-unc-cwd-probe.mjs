// Source builder only. Importing this module never starts a process or reads a fixture.
import assert from 'node:assert/strict';
import {win32} from 'node:path';

const literal=value=>`'${value.replaceAll("'","''")}'`;
const uuid='[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}';
const rootPattern=new RegExp(`^\\\\\\\\localhost\\\\NomiFun_GlobalOS_(${uuid})\\\\$`);
const preservedNonce='34d53b78-f4d3-4128-b8a6-cb7e6b509895';

export function windowsUncCwdProbeSource({plainRoot,extendedRoot,markerName,markerContent,powershell}) {
  for(const value of [plainRoot,extendedRoot,markerName,powershell])assert(typeof value==='string'&&!/[\x00-\x1f]/.test(value),'Expected a literal path without control characters');
  const match=rootPattern.exec(plainRoot);assert(match,'Expected an exact unique localhost UNC share root with a trailing separator');
  const nonce=match[1];assert.notEqual(nonce,preservedNonce,'Never probe the preserved unsettled share');
  assert.equal(extendedRoot,`\\\\?\\UNC\\localhost\\NomiFun_GlobalOS_${nonce}\\`,'UNC spellings must identify the same exact fixture');
  assert.equal(markerName,`identity-${nonce}.txt`);
  assert.equal(markerContent,`UNC_FIXTURE_${nonce}\n`);
  assert(/^[a-z]:\\Windows\\System32\\WindowsPowerShell\\v1\.0\\powershell\.exe$/i.test(powershell),'Use the native System32 Windows PowerShell executable');
  assert.equal(win32.normalize(powershell),powershell);

  const child=String.raw`$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)
Add-Type -TypeDefinition 'using System;using System.Text;using System.Runtime.InteropServices;public static class NomiUncCwdNative{[DllImport("kernel32.dll",CharSet=CharSet.Unicode,ExactSpelling=true,SetLastError=true)]public static extern uint GetCurrentDirectoryW(uint n,StringBuilder b);}'
$b=New-Object Text.StringBuilder(32768);$n=[NomiUncCwdNative]::GetCurrentDirectoryW(32768,$b);if(!$n-or$n-ge32768){throw 'GetCurrentDirectoryW failed'}
$dotnet=@{read=$false;content=$null;error_code=$null}
try{$dotnet.content=[IO.File]::ReadAllText(${literal(markerName)},[Text.Encoding]::UTF8);$dotnet.read=$true}catch{$dotnet.error_code=([string]$_.CategoryInfo.Category)+':'+$_.FullyQualifiedErrorId}
$item=@{exists=$false;full_name=$null;length=$null;error_code=$null}
try{$f=Get-Item -LiteralPath ${literal(markerName)} -ErrorAction Stop;if($f.PSIsContainer){throw 'Marker is a directory'};$item.exists=$true;$item.full_name=$f.FullName;$item.length=$f.Length}catch{$item.error_code=([string]$_.CategoryInfo.Category)+':'+$_.FullyQualifiedErrorId}
$o=@{native_cwd=$b.ToString();environment_cwd=[Environment]::CurrentDirectory;provider_path=(Get-Location).ProviderPath;dotnet_marker=$dotnet;ps_item=$item}
[Console]::WriteLine(($o|ConvertTo-Json -Depth 5 -Compress))`;
  const encoded=Buffer.from(child,'utf16le').toString('base64');
  const argumentsText=`-NoLogo -NoProfile -NonInteractive -EncodedCommand ${encoded}`;
  const commandLineUnits=powershell.length+3+argumentsText.length+1;
  assert(commandLineUnits<=32767,'Each child must fit the Windows UTF-16 command-line limit including executable and terminator');
  const source=String.raw`$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class NomiUncCwdTimes{[DllImport("kernel32.dll",SetLastError=true)]public static extern bool GetProcessTimes(IntPtr h,out long c,out long e,out long k,out long u);[DllImport("kernel32.dll",SetLastError=true)]public static extern bool GetExitCodeProcess(IntPtr h,out uint code);}'
$arguments=${literal(argumentsText)}
$cases=@(@{spelling='plain';cwd=${literal(plainRoot)}},@{spelling='verbatim';cwd=${literal(extendedRoot)}})
function TextSha256([string]$text){$sha=[Security.Cryptography.SHA256]::Create();try{return(-join($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text))|ForEach-Object{$_.ToString('x2')}))}finally{$sha.Dispose()}}
$results=@();$rawOutputs=@()
foreach($case in $cases){
  $info=New-Object Diagnostics.ProcessStartInfo;$info.FileName=${literal(powershell)};$info.Arguments=$arguments;$info.WorkingDirectory=$case.cwd;$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true;$info.StandardOutputEncoding=New-Object Text.UTF8Encoding($false);$info.StandardErrorEncoding=New-Object Text.UTF8Encoding($false)
  $p=New-Object Diagnostics.Process;$p.StartInfo=$info;$result=$null;$childPid=$null;$created=$null;$outBytes=$null;$errBytes=$null
  try{
    if(!$p.Start()){throw 'Native child did not start'}
    $handle=$p.Handle;$childPid=$p.Id;$created=0L;$ended=0L;$kernel=0L;$user=0L
    if(![NomiUncCwdTimes]::GetProcessTimes($handle,[ref]$created,[ref]$ended,[ref]$kernel,[ref]$user)){throw 'GetProcessTimes failed'}
    $stdoutTask=$p.StandardOutput.ReadToEndAsync();$stderrTask=$p.StandardError.ReadToEndAsync()
    if(!$p.WaitForExit(10000)){throw 'Child deadline exceeded; the canonical parent Job owns cancellation and cleanup'}
    if(!$stdoutTask.Wait(1000)){throw 'Child stdout drain deadline exceeded; the canonical parent Job owns cleanup'}
    if(!$stderrTask.Wait(1000)){throw 'Child stderr drain deadline exceeded; the canonical parent Job owns cleanup'}
    $stdout=$stdoutTask.GetAwaiter().GetResult();$stderr=$stderrTask.GetAwaiter().GetResult();$outBytes=[Text.Encoding]::UTF8.GetByteCount($stdout);$errBytes=[Text.Encoding]::UTF8.GetByteCount($stderr)
    if($outBytes-gt16384-or$errBytes-gt8192){throw 'Child output exceeds the bounded protocol'}
    [uint32]$exitCode=0;if(![NomiUncCwdTimes]::GetExitCodeProcess($handle,[ref]$exitCode)){throw 'GetExitCodeProcess failed'}
    $sameCreated=0L;if(![NomiUncCwdTimes]::GetProcessTimes($handle,[ref]$sameCreated,[ref]$ended,[ref]$kernel,[ref]$user)-or$sameCreated-ne$created){throw 'Child creation identity changed'}
    $reasons=@();if($exitCode-ne0){$reasons+='child_exit_nonzero'}
    $observation=$null;$stdoutText=$null
    try{
      $observation=ConvertFrom-Json -InputObject $stdout -ErrorAction Stop
      if($null-eq$observation-or@($observation).Count-ne1-or$observation.native_cwd-isnot[string]-or$observation.environment_cwd-isnot[string]-or$observation.provider_path-isnot[string]-or$observation.dotnet_marker.read-isnot[bool]-or$observation.ps_item.exists-isnot[bool]){throw 'Incomplete child JSON'}
    }catch{$observation=$null;$stdoutText=$stdout;$reasons+='missing_or_invalid_child_json'}
    $protocolError=$null;if($reasons.Count){$protocolError=$reasons-join';'}
    $result=@{spelling=$case.spelling;requested_cwd=$case.cwd;pid=$childPid;creation_filetime=([string]$created);exit_code=$exitCode;wait_completed=$true;stdout_bytes=$outBytes;stderr_bytes=$errBytes;stderr_text=$null;stderr_sha256=(TextSha256 $stderr);stderr_excerpt_truncated=$false;stdout_text=$null;stdout_sha256=$null;stdout_excerpt_truncated=$false;protocol_error=$protocolError;handle_closed=$false;command_line_utf16_units=${commandLineUnits};observation=$observation}
  }catch{
    $message=[string]$_.Exception.Message;if($message.Length-gt512){$message=$message.Substring(0,512)}
    [Console]::Error.WriteLine((@{schema='nomifun.windows-unc-cwd-spelling-control-error.v1';spelling=$case.spelling;requested_cwd=$case.cwd;pid=$childPid;creation_filetime=$created;stdout_bytes=$outBytes;stderr_bytes=$errBytes;completed_rows=$results.Count;message=$message}|ConvertTo-Json -Depth 3 -Compress))
    throw
  }finally{$p.Dispose()}
  $result.handle_closed=$true;$results+=,$result;$rawOutputs+=,@{stdout=$stdout;stderr=$stderr}
}
function EncodeSummary([int]$limit){
  for($i=0;$i-lt$results.Count;$i++){
    $row=$results[$i];$raw=$rawOutputs[$i]
    $row.stderr_text=$raw.stderr.Substring(0,[Math]::Min($limit,$raw.stderr.Length));$row.stderr_excerpt_truncated=$raw.stderr.Length-gt$limit
    if($null-eq$row.observation){$row.stdout_text=$raw.stdout.Substring(0,[Math]::Min($limit,$raw.stdout.Length));$row.stdout_sha256=TextSha256 $raw.stdout;$row.stdout_excerpt_truncated=$raw.stdout.Length-gt$limit}
  }
  $success=@($results|Where-Object{$_.exit_code-ne0-or$null-ne$_.protocol_error-or$null-eq$_.observation}).Count-eq0
  return(@{schema='nomifun.windows-unc-cwd-spelling-control.v1';results=$results;filesystem_mutations=0;protocol_succeeded=$success}|ConvertTo-Json -Depth 8 -Compress)
}
$summary=$null
foreach($limit in @(256,128)){$candidate=EncodeSummary $limit;if([Text.Encoding]::UTF8.GetByteCount($candidate)-le3072){$summary=$candidate;break}}
if($null-eq$summary){
  foreach($row in $results){$row.observation=$null;if($null-eq$row.protocol_error){$row.protocol_error='protocol_output_budget'}else{$row.protocol_error+=';protocol_output_budget'}}
  foreach($limit in @(128,64,0)){$candidate=EncodeSummary $limit;if([Text.Encoding]::UTF8.GetByteCount($candidate)-le3072){$summary=$candidate;break}}
}
if($null-eq$summary){throw 'Even the bounded lifecycle summary exceeds 3072 bytes'}
$parentExit=0;if(@($results|Where-Object{$_.exit_code-ne0-or$null-ne$_.protocol_error-or$null-eq$_.observation}).Count){$parentExit=1}
[Console]::WriteLine($summary);[Console]::Out.Flush();exit $parentExit
`;
  assert(Buffer.byteLength(source,'utf8')<=12*1024,'The parent script must fit the canonical write-file source budget');
  return source;
}
