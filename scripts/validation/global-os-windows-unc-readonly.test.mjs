import {test,expect} from 'bun:test';
import {windowsUncReadonlyTarget} from './global-os-windows-unc-readonly.mjs';
test('UNC metadata probe accepts only the exact generated share and matching local fixture namespace',()=>{
  const nonce='11111111-1111-4111-8111-111111111111',input={uncRoot:`\\\\localhost\\NomiFun_GlobalOS_${nonce}`,extendedUncRoot:`\\\\?\\UNC\\localhost\\NomiFun_GlobalOS_${nonce}`,physicalRoot:`C:\\Users\\muli\\AppData\\Local\\Temp\\nomifun-global-os-product-22222222-2222-4222-8222-222222222222\\unc-cases-${nonce}\\share-root`};
  expect(windowsUncReadonlyTarget(input)).toEqual({nonce,root:input.extendedUncRoot});
  for(const change of [{uncRoot:'\\\\localhost\\C$'},{uncRoot:input.uncRoot+'\\child'},{extendedUncRoot:input.extendedUncRoot+'\\child'},{extendedUncRoot:input.extendedUncRoot.replace('localhost','other-host')},{physicalRoot:'C:\\Users\\muli'},{physicalRoot:input.physicalRoot.replace(`unc-cases-${nonce}`,'unc-cases-33333333-3333-4333-8333-333333333333')}])expect(()=>windowsUncReadonlyTarget({...input,...change})).toThrow();
});
