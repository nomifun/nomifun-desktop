// These checks parse source only. They never execute the UNC child probes.
import{test,expect}from'bun:test';
import{execFile}from'node:child_process';
import{createHash}from'node:crypto';
import{windowsUncCwdProbeSource}from'./global-os-windows-unc-cwd-probe.mjs';
const nonce='11111111-1111-4111-8111-111111111111';
const input={plainRoot:`\\\\localhost\\NomiFun_GlobalOS_${nonce}\\`,extendedRoot:`\\\\?\\UNC\\localhost\\NomiFun_GlobalOS_${nonce}\\`,markerName:`identity-${nonce}.txt`,markerContent:`UNC_FIXTURE_${nonce}\n`,powershell:'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe'};
const source=windowsUncCwdProbeSource(input);
const argumentsText=source.match(/^\$arguments='([^']+)'$/m)[1];
const child=Buffer.from(argumentsText.split(' ').at(-1),'base64').toString('utf16le');
test('UNC cwd source only permits the same unique localhost fixture and native Windows PowerShell',()=>{
  for(const change of [{plainRoot:input.plainRoot.slice(0,-1)},{plainRoot:input.plainRoot+'child\\'},{plainRoot:input.plainRoot.replace('localhost','foreign')},{extendedRoot:input.extendedRoot.replace(nonce,'22222222-2222-4222-8222-222222222222')},{markerName:'foreign.txt'},{markerContent:'different\n'},{powershell:'C:\\fixture\\powershell.exe'},{powershell:input.powershell+'\r\n'},{plainRoot:input.plainRoot+'\0'}])expect(()=>windowsUncCwdProbeSource({...input,...change})).toThrow();
  const preserved='34d53b78-f4d3-4128-b8a6-cb7e6b509895';expect(()=>windowsUncCwdProbeSource(Object.fromEntries(Object.entries(input).map(([key,value])=>[key,value.replaceAll(nonce,preserved)])))).toThrow('preserved unsettled share');
});
test('UNC cwd controls retain child identity, drain output, wait naturally and close handles within budgets',()=>{
  expect(Buffer.byteLength(source,'utf8')).toBeLessThanOrEqual(12*1024);
  const units=input.powershell.length+3+argumentsText.length+1;expect(units).toBeLessThanOrEqual(32767);expect(source).toContain(`command_line_utf16_units=${units}`);
  for(const expected of ['UseShellExecute=$false','WorkingDirectory=$case.cwd','$handle=$p.Handle','GetProcessTimes($handle','ReadToEndAsync()','WaitForExit(10000)','$p.Dispose()','handle_closed=$true','filesystem_mutations=0','nomifun.windows-unc-cwd-spelling-control.v1'])expect(source).toContain(expected);
  for(const stream of ['stdout','stderr']){const guard=source.indexOf(`if(!$${stream}Task.Wait(1000))`),result=source.indexOf(`$${stream}Task.GetAwaiter().GetResult()`);expect(guard).toBeGreaterThan(0);expect(result).toBeGreaterThan(guard);}
  for(const expected of ['GetCurrentDirectoryW','[Environment]::CurrentDirectory','(Get-Location).ProviderPath','[IO.File]::ReadAllText','Get-Item -LiteralPath','CategoryInfo.Category','FullyQualifiedErrorId'])expect(child).toContain(expected);
  for(const forbidden of ['Set-Location','SetCurrentDirectory','New-PSDrive','New-SmbShare','Remove-Item','Stop-Process','.Kill(','ExecutionPolicy','-Command ']){expect(source).not.toContain(forbidden);expect(child).not.toContain(forbidden);}
  expect(argumentsText).toStartWith('-NoLogo -NoProfile -NonInteractive -EncodedCommand ');
});
test('complete bounded child failures remain diagnostic rows instead of aborting the second spelling',()=>{
  for(const expected of ["$reasons+='child_exit_nonzero'","$observation=$null;$stdoutText=$stdout;$reasons+='missing_or_invalid_child_json'",'stderr_sha256=(TextSha256 $stderr)','protocol_error=$protocolError','nomifun.windows-unc-cwd-spelling-control-error.v1','completed_rows=$results.Count'])expect(source).toContain(expected);
  expect(source).not.toContain('child_stderr_nonempty');
  expect(source).not.toContain("throw 'Native child did not return a clean protocol result'");
  expect(source).toContain("$outBytes-gt16384-or$errBytes-gt8192");
  expect(source).toContain('$message.Substring(0,512)');
  expect(source.indexOf('$observation=$null;$stdoutText=$null')).toBeLessThan(source.indexOf('ConvertFrom-Json -InputObject $stdout'));
});
test('the flushed parent summary fits three KiB and exits according to both real child protocol results',()=>{
  for(const expected of ['foreach($limit in @(256,128))','UTF8.GetByteCount($candidate)-le3072','stderr_excerpt_truncated','stdout_excerpt_truncated','stdout_sha256=TextSha256 $raw.stdout','protocol_output_budget','protocol_succeeded=$success','[Console]::WriteLine($summary);[Console]::Out.Flush();exit $parentExit'])expect(source).toContain(expected);
  expect(source.indexOf('$result.handle_closed=$true')).toBeLessThan(source.indexOf('function EncodeSummary'));
  expect(source).toContain('$_.exit_code-ne0-or$null-ne$_.protocol_error-or$null-eq$_.observation');
  expect(source).not.toContain('$_.stderr_bytes-ne0');
  expect(source).toContain('$row.observation=$null');
});
test.skipIf(process.platform!=='win32')('native WinPS parses both source bodies without spawning the diagnostic children',async()=>{
  const parse="$sources=[Console]::In.ReadToEnd()|ConvertFrom-Json;$failures=@();foreach($s in $sources){$t=$null;$e=$null;[Management.Automation.Language.Parser]::ParseInput($s,[ref]$t,[ref]$e)|Out-Null;$failures+=@($e|ForEach-Object{$_.Message})};ConvertTo-Json -InputObject @($failures) -Compress";
  const output=await new Promise((done,fail)=>{const p=execFile(input.powershell,['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(parse,'utf16le').toString('base64')],{encoding:'utf8',windowsHide:true,timeout:10000},(error,stdout)=>error?fail(error):done(stdout));p.stdin.end(JSON.stringify([source,child]));});
  expect(JSON.parse(output)).toEqual([]);
},15000);
test.skipIf(process.platform!=='win32')('the actual summary function retains nonempty stderr without treating it as child protocol failure',async()=>{
  const hashFunction=source.match(/^function TextSha256[^\r\n]+/m)[0];
  const summaryFunction=source.match(/function EncodeSummary\([\s\S]*?(?=\n\$summary=)/)[0];
  const stderr='native diagnostic retained';
  const script=`${hashFunction}\n${summaryFunction}\n$results=@(@{exit_code=0;stderr_bytes=${Buffer.byteLength(stderr)};protocol_error=$null;observation=@{native_cwd='fixture';environment_cwd='fixture';provider_path='fixture';dotnet_marker=@{read=$true;content='marker';error_code=$null};ps_item=@{exists=$true;full_name='marker';length=6;error_code=$null}};stderr_sha256=(TextSha256 '${stderr}');stderr_text=$null;stderr_excerpt_truncated=$false;stdout_text=$null;stdout_sha256=$null;stdout_excerpt_truncated=$false});$rawOutputs=@(@{stdout='valid JSON retained as observation';stderr='${stderr}'});$checks=@();$checks+=,(EncodeSummary 256|ConvertFrom-Json);$results[0].exit_code=7;$checks+=,(EncodeSummary 256|ConvertFrom-Json);$results[0].exit_code=0;$results[0].observation=$null;$results[0].protocol_error='missing_or_invalid_child_json';$checks+=,(EncodeSummary 256|ConvertFrom-Json);ConvertTo-Json -InputObject $checks -Depth 12 -Compress`;
  const output=await new Promise((done,fail)=>{execFile(input.powershell,['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(script,'utf16le').toString('base64')],{encoding:'utf8',windowsHide:true,timeout:10000},(error,stdout)=>error?fail(error):done(stdout));});
  const checks=JSON.parse(output);expect(checks.map(value=>value.protocol_succeeded)).toEqual([true,false,false]);
  for(const check of checks){expect(check.results[0].stderr_text).toBe(stderr);expect(check.results[0].stderr_bytes).toBe(Buffer.byteLength(stderr));expect(check.results[0].stderr_sha256).toBe(createHash('sha256').update(stderr).digest('hex'));}
},15000);
