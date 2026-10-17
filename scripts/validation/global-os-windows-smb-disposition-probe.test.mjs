import{test,expect}from'bun:test';
import{windowsSmbDispositionControlTarget,windowsSmbDispositionProbeScript}from'./global-os-windows-smb-disposition-probe.mjs';
const nonce='11111111-1111-4111-8111-111111111111',preserved='34d53b78-f4d3-4128-b8a6-cb7e6b509895';
const input={nonce,physicalRoot:`C:\\Users\\muli\\AppData\\Local\\Temp\\nomifun-global-os-product-22222222-2222-4222-8222-222222222222\\smb-disposition-cases-${nonce}\\share-root`,shareName:`NomiFun_GlobalOS_Disposition_${nonce}`,preservedPhysicalRoot:`C:\\Users\\muli\\AppData\\Local\\Temp\\nomifun-global-os-product-ed5ef30f-a5bc-4ef2-8eb8-67bd4f4dd536\\unc-cases-${preserved}\\share-root`,preservedShareName:`NomiFun_GlobalOS_${preserved}`};
test('SMB disposition controls require a new exact physical fixture and localhost share',()=>{
  const target=windowsSmbDispositionControlTarget(input);expect(target.caseCount).toBe(16);expect(target.uncRoot).toBe(`\\\\localhost\\${input.shareName}`);
  for(const change of[{nonce:preserved},{nonce:'not-a-uuid'},{shareName:input.preservedShareName},{shareName:'C$'},{physicalRoot:input.preservedPhysicalRoot},{physicalRoot:input.physicalRoot+'\\child'},{physicalRoot:input.physicalRoot+'\\..'},{physicalRoot:input.physicalRoot.replace('22222222-2222-4222-8222-222222222222','ed5ef30f-a5bc-4ef2-8eb8-67bd4f4dd536')},{physicalRoot:input.physicalRoot.replace('C:','\\\\other-host\\disk')},{preservedShareName:'another-share'},{preservedPhysicalRoot:input.physicalRoot}])expect(()=>windowsSmbDispositionControlTarget({...input,...change})).toThrow();
});
test('SMB disposition script keeps SDK flags, explicit independent cases, raw errors and close observations without fallback',()=>{
  const script=windowsSmbDispositionProbeScript(input);
  expect(script).toContain('uint[] flags={0x13,0x03,0x01,0}');expect(script).toContain('FileDispositionInfo = 4, FileDispositionInfoEx = 21');
  expect(script).toContain('Delete|ReadAttributes|Synchronize,ShareReadWrite');expect(script).toContain('new[]{false,true}');expect(script).toContain('raw_os_error');
  expect(script).toContain('CREATE_NEW');expect(script).toContain('name_after_all_own_handles_closed');expect(script).toContain('"automatic_fallbacks",0');
  for(const forbidden of['Remove-SmbShare','New-SmbShare','DeleteFileW','Remove-Item','Move-Item','sample.txt','.backup','record_effect_terminal','Reconcile'])expect(script).not.toContain(forbidden);
  expect(script).not.toContain(input.preservedPhysicalRoot);expect(script).not.toContain(input.preservedShareName);
});
