import{test,expect}from'bun:test';
import{preservedUncControlSource,validateSmbDispositionControlReceipt,windowsSmbControlPlan}from'./global-os-windows-smb-control-cases.mjs';
const nonce='11111111-1111-4111-8111-111111111111',old='34d53b78-f4d3-4128-b8a6-cb7e6b509895',sid='S-1-5-21-2723901165-780233200-1355648594-1001';
const fixture='C:\\Users\\muli\\AppData\\Local\\Temp\\nomifun-global-os-product-22222222-2222-4222-8222-222222222222';
const oldFixture='C:\\Users\\muli\\AppData\\Local\\Temp\\nomifun-global-os-product-ed5ef30f-a5bc-4ef2-8eb8-67bd4f4dd536';
const input={nonce,physicalRoot:`${fixture}\\smb-disposition-cases-${nonce}\\share-root`,shareName:`NomiFun_GlobalOS_Disposition_${nonce}`,preservedPhysicalRoot:`${oldFixture}\\unc-cases-${old}\\share-root`,preservedShareName:`NomiFun_GlobalOS_${old}`};
const sha='be97ccdfcde70b3ea454cd2fb29a682a0698dc089205f626946649a1a027ea11';
function preserved(){return{schema:'nomifun.global-os-native-regression.v2',platform:'win32',status:'failed',scenario:'unc',administrator:false,session_id:'01a14893-93f7-7a90-b573-bc0b31116207',login_identity:{sid},fixture:oldFixture,workspace:`${oldFixture}\\workspace`,failed_phase:{accepted:{agent_session_id:'01a14893-93f7-7a90-b573-bc0b31116207',replayed:false}},native_case_resources:{root:`${oldFixture}\\unc-cases-${old}`,physical_root:input.preservedPhysicalRoot,share_name:input.preservedShareName,unc_root:`\\\\localhost\\${input.preservedShareName}`},cleanup_intent:{only_after_canonical_settlement:true},failure:{message:'EXECUTION_CLEANUP_UNPROVEN'},checks:['unc-create-share','unc-write','unc-read'].map(step=>({step,status:'passed'}))};}
test('SMB control source pins the original failed report without treating it as permission to touch the target',()=>{
  expect(preservedUncControlSource(preserved(),sha,sid).session_id).toBe('01a14893-93f7-7a90-b573-bc0b31116207');
  expect(()=>preservedUncControlSource(preserved(),'0'.repeat(64),sid)).toThrow();
  for(const change of [{status:'completed'},{scenario:'service'},{administrator:true},{session_id:'other'},{login_identity:{sid:'other'}}])expect(()=>preservedUncControlSource({...preserved(),...change},sha,sid)).toThrow();
  const wrong=preserved();wrong.native_case_resources.physical_root='C:\\daily';expect(()=>preservedUncControlSource(wrong,sha,sid)).toThrow();
});
test('SMB control plan uses six canonical steps and preserves actual privilege and absence boundaries',()=>{
  const plan=windowsSmbControlPlan({probeInput:input,workspace:`${fixture}\\workspace`,loginIdentity:{sid,name:'muli',administrator:false}});
  expect(plan.steps.map(step=>step.name)).toEqual(['write_file','start_process','start_process','start_process','delete_path','read_file']);
  expect(plan.steps.map(step=>step.args.execution_identity)).toEqual([undefined,'administrator',undefined,'administrator',undefined,undefined]);
  expect(plan.steps[4].args.path).toBe(`${fixture}\\smb-disposition-cases-${nonce}`);expect(plan.steps[5].args).toEqual({path:plan.marker,missing_ok:true});
  for(const step of plan.steps.filter(step=>step.name==='start_process')){
    const script=Buffer.from(step.args.args.at(-1),'base64').toString('utf16le');expect(script).not.toContain(input.preservedPhysicalRoot);expect(script).not.toContain(input.preservedShareName);
    expect(step.args.args.at(-1).length).toBeLessThan(30000);expect(step.args.args).not.toContain('Bypass');
  }
  expect(()=>windowsSmbControlPlan({probeInput:input,workspace:`${oldFixture}\\workspace`,loginIdentity:{sid,name:'muli',administrator:false}})).toThrow();
});
test('SMB control receipt rejects unknown process and never treats marked success as unlink proof',()=>{
  const plan=windowsSmbControlPlan({probeInput:input,workspace:`${fixture}\\workspace`,loginIdentity:{sid,name:'muli',administrator:false}});
  const state={attributes:128,raw_os_error:0,absent:false},rows=[];
  for(const transport of ['local','unc'])for(const metadata_holder of [false,true])for(const flags of [0x13,3,1,null])rows.push({transport,metadata_holder,flags,information_class:flags===null?4:21,name:`control-${nonce}-${transport}-${metadata_holder?'pinned':'unheld'}-${(flags??0).toString(16).padStart(2,'0')}.bin`,open_access:0x110080,open_sharing:3,marked:true,all_own_handles_closed:true,steps:[{operation:'CREATE_NEW',success:true,raw_os_error:0},{operation:'set_disposition',success:true,raw_os_error:0}],name_after_all_own_handles_closed:state,local_name_after_all_own_handles_closed:state,unc_name_after_all_own_handles_closed:state});
  const receipt={schema:'nomifun.windows-smb-disposition-control.v1',nonce,automatic_fallbacks:0,manual_file_cleanup:0,original_effect_settlement:false,cases:rows};
  const value={state:'exited',exit_code:0,cleanup:{reaped:true,errors:[]},output:{text:JSON.stringify(receipt)}};
  expect(validateSmbDispositionControlReceipt(value,plan.target).cases.length).toBe(16);
  expect(()=>validateSmbDispositionControlReceipt({...value,state:'running'},plan.target)).toThrow();
  expect(()=>validateSmbDispositionControlReceipt({...value,cleanup:{reaped:false}},plan.target)).toThrow();
  const bad=structuredClone(receipt);bad.cases[0].all_own_handles_closed=false;expect(()=>validateSmbDispositionControlReceipt({...value,output:{text:JSON.stringify(bad)}},plan.target)).toThrow();
  const repeated=structuredClone(receipt);repeated.cases[1]=repeated.cases[0];expect(()=>validateSmbDispositionControlReceipt({...value,output:{text:JSON.stringify(repeated)}},plan.target)).toThrow();
});
