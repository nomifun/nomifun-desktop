import {expect,test} from 'bun:test';
import {validateNativeFixtureReceipt} from './global-os-windows-system-native.mjs';
const probe=()=>({schema:'nomifun.windows-system-parent-exit-probe.v1',status:'ready_for_parent_exit',canonical_data_involved:false,parent_pid:100,parent_exit_code:23,parent_cleanup_requested:false,receipt_flushed:true,identity:{windows:{elevated:true,pid:101}},started:{pid:102},descendant_pids:[103],exact_processes:[101,102,103].map(pid=>({pid,creation_filetime:'134050000000000000'}))});
test('parent-exit evidence requires exact self-exit, native creation identities and no canonical claim',()=>{
  const args={parentExit:true,exitCode:23,parentPid:100};expect(validateNativeFixtureReceipt(probe(),args).status).toBe('ready_for_parent_exit');
  for(const change of [v=>v.canonical_data_involved=true,v=>v.parent_cleanup_requested=true,v=>v.receipt_flushed=false,v=>v.descendant_pids=[102],v=>v.exact_processes[0].creation_filetime='unknown',v=>v.identity.windows.elevated=false]){const v=probe();change(v);expect(()=>validateNativeFixtureReceipt(v,args)).toThrow();}
  expect(()=>validateNativeFixtureReceipt(probe(),{...args,exitCode:0})).toThrow();
});
test('native normal exit alone cannot pass missing Pipe/deadline/ConPTY lifecycle cases',()=>{
  const v={schema:'nomifun.windows-system-native-acceptance.v2',status:'passed',canonical_data_involved:false,identity:{windows:{elevated:true,pid:101}},shutdown:{exact:true},facility_exit_proven:true,cases:[]};
  expect(()=>validateNativeFixtureReceipt(v,{parentExit:false,exitCode:0,parentPid:100})).toThrow();
});
