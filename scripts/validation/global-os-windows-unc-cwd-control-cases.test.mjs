import{test,expect}from'bun:test';
import{mkdir,rmdir,realpath}from'node:fs/promises';
import{tmpdir}from'node:os';
import{join}from'node:path';
import{randomUUID,createHash}from'node:crypto';
import{validateWindowsUncCwdControlReceipt,validateWindowsUncCwdControlLifecycle,createWindowsUncCwdControlScenario}from'./global-os-windows-unc-cwd-control-cases.mjs';
import{nativeUncCwdControlOptions}from'./global-os-native.mjs';
const nonce='11111111-1111-4111-8111-111111111111',plainRoot=`\\\\localhost\\NomiFun_GlobalOS_${nonce}\\`,extendedRoot=`\\\\?\\UNC\\localhost\\NomiFun_GlobalOS_${nonce}\\`,markerContent=`UNC_FIXTURE_${nonce}\n`;
const expected={plainRoot,extendedRoot,markerContent};
const digest=text=>createHash('sha256').update(text).digest('hex');
function protocol(){return{schema:'nomifun.windows-unc-cwd-spelling-control.v1',protocol_succeeded:true,filesystem_mutations:0,results:['plain','verbatim'].map((spelling,index)=>({spelling,requested_cwd:[plainRoot,extendedRoot][index],pid:100+index,creation_filetime:`13436698559738594${index}`,exit_code:0,wait_completed:true,stdout_bytes:900,stderr_bytes:0,stderr_text:'',stderr_sha256:digest(''),stderr_excerpt_truncated:false,stdout_text:null,stdout_sha256:null,stdout_excerpt_truncated:false,command_line_utf16_units:3884,handle_closed:true,protocol_error:null,observation:{native_cwd:[plainRoot,extendedRoot][index],environment_cwd:[plainRoot,extendedRoot][index],provider_path:index?'C:\\':plainRoot,dotnet_marker:{read:true,content:markerContent,error_code:null},ps_item:index?{exists:false,full_name:null,length:null,error_code:'ObjectNotFound:PathNotFound'}:{exists:true,full_name:`${plainRoot}identity-${nonce}.txt`,length:Buffer.byteLength(markerContent),error_code:null}}}))};}
const receipt=value=>({state:'exited',exit_code:0,cleanup:{reaped:true,errors:[]},output:{text:JSON.stringify(value),decode_errors:0,dropped_bytes:0}});
test('UNC cwd diagnostic retains ProviderPath differences while requiring exact complete child protocol and owned marker',()=>{
  expect(validateWindowsUncCwdControlReceipt(receipt(protocol()),expected).results[1].observation.provider_path).toBe('C:\\');
  const stderr=protocol();stderr.results[0].stderr_text='diagnostic';stderr.results[0].stderr_bytes=10;stderr.results[0].stderr_sha256=digest('diagnostic');
  expect(validateWindowsUncCwdControlReceipt(receipt(stderr),expected).results[0].stderr_text).toBe('diagnostic');
  for(const mutate of [x=>{x.results.pop();},x=>{x.results[1].pid=x.results[0].pid;},x=>{x.results[0].creation_filetime='';},x=>{x.results[0].requested_cwd='C:\\';},x=>{x.results[0].exit_code=1;},x=>{x.results[0].wait_completed=false;},x=>{x.results[0].handle_closed=false;},x=>{x.results[0].stderr_bytes=1;},x=>{x.results[0].observation.dotnet_marker.content='foreign';},x=>{x.schema='other';}]){const changed=protocol();mutate(changed);expect(()=>validateWindowsUncCwdControlReceipt(receipt(changed),expected)).toThrow();}
  for(const mutate of [x=>{x.state='running';},x=>{x.cleanup.reaped=false;},x=>{x.output.dropped_bytes=1;},x=>{x.output.decode_errors=1;}]){const changed=receipt(protocol());mutate(changed);expect(()=>validateWindowsUncCwdControlReceipt(changed,expected)).toThrow();}
});
test('UNC spelling control requires exact original private native App identity and independent scenario',()=>{
  const options={scenario:'unc-cwd-spelling-control',platform:'win32',nativeDataRoot:join(tmpdir(),'fixture','native-data')};
  expect(()=>nativeUncCwdControlOptions(options)).not.toThrow();
  for(const change of [{platform:'darwin'},{nativeDataRoot:undefined},{administrator:true},{protectedUserDirectory:true},{retainServiceForAppExit:true},{continueSession:'old'},{installerReport:'other'},{missingDefault:true}])expect(()=>nativeUncCwdControlOptions({...options,...change})).toThrow();
});
test('failed child data can retain exact lifecycle proof without certifying protocol success or accepting missing exits',()=>{
  for(const failure of ['reported_error','nonzero','missing_json']){
    const changed=protocol();changed.protocol_succeeded=false;changed.results[0].protocol_error=failure;
    if(failure==='nonzero')changed.results[0].exit_code=5;if(failure==='missing_json'){changed.results[0].observation=null;changed.results[0].stdout_text='not JSON';changed.results[0].stdout_sha256=digest('not JSON');changed.results[0].stdout_bytes=8;}
    const failed={...receipt(changed),exit_code:1};expect(validateWindowsUncCwdControlLifecycle(failed,expected).protocol_succeeded).toBe(false);expect(()=>validateWindowsUncCwdControlReceipt(failed,expected)).toThrow();
    for(const mutate of [x=>{x.results[0].handle_closed=false;},x=>{delete x.results[0].pid;},x=>{delete x.results[0].exit_code;},x=>{delete x.results[0].creation_filetime;},x=>{x.results[0].wait_completed=false;}]){const missing=structuredClone(changed);mutate(missing);expect(()=>validateWindowsUncCwdControlLifecycle({...receipt(missing),exit_code:1},expected)).toThrow();}
  }
});
test.skipIf(process.platform!=='win32')('control factory writes its fixed probe through canonical tools and retains live original share cleanup getter',async()=>{
  const parent=await realpath(tmpdir()),fixture=join(parent,`nomifun-global-os-product-${randomUUID()}`);await mkdir(fixture);
  try{
    const scenario=await createWindowsUncCwdControlScenario({fixture,workspace:join(fixture,'workspace'),loginIdentity:{administrator:false,name:'TEST\\fixture',sid:'S-1-5-21-1-2-3-1000'}});
    expect(scenario.steps.map(step=>step.id)).toEqual(['unc-existing-facilities','unc-physical-fixture','unc-create-share','unc-cwd-control-write-probe','unc-cwd-two-spellings','unc-remove-share','unc-delete-physical-fixture']);
    expect(scenario.steps[3].name).toBe('write_file');const probeArguments=scenario.steps[4].args();expect(probeArguments.args).toEqual(['-NoLogo','-NoProfile','-NonInteractive','-File',scenario.resources.probe_path]);
    expect(probeArguments.cwd).toBe(scenario.resources.physical_root);expect(Object.getOwnPropertyDescriptor(scenario,'cleanupSteps').get).toBeFunction();
    expect(scenario.cleanupSteps).toHaveLength(1);scenario.steps[2].args();expect(scenario.cleanupSteps).toHaveLength(2);
    expect(await scenario.cleanupNeeded()).toBe(false);expect(scenario.nativeEvidence.find(value=>value.phase==='diagnostic_child_cleanup_guard').automatic_fixture_cleanup_allowed).toBe(false);
    expect(scenario.completionSummary('cleanup')).toContain('原UNC验收失败保留');expect(scenario.completionSummary('main')).toContain('不代表产品UNC16项通过');
  }finally{await rmdir(fixture);}
});
