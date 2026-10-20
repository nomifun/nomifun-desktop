import {test,expect} from 'bun:test';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {windowsBrowserCleanupSource} from './global-os-windows-cleanup.mjs';
const fixture=join(tmpdir(),'nomifun-global-os-product-11111111-1111-1111-1111-111111111111'),nonce='22222222-2222-2222-2222-222222222222',root=join(fixture,`browser-cases-${nonce}`);
function source(){return{schema:'nomifun.global-os-native-regression.v2',platform:'win32',status:'failed',scenario:'browser',administrator:false,fixture,workspace:join(fixture,'workspace'),session_id:'original',native_case_resources:{root},cleanup_intent:{path:root},scenario_host_resources_closed:true,checks:[],failure_settlement:{cleanup_proven:true,execution:{operation_id:'stopped',state:'cancelled',turn_state:'cancelled',producer_lease_live:false,checkpoint_retained:false,recovery_blocked:false,pause_requested:false,automatic_replay_authorized:false,pending_effects:0,unknown_effects:0,checkpoint_seq:null,checkpoint_digest:null,pause:null},head:{session_id:'original',status:'ready'}},observations:{'native-browser-confirm-disposed':{content:`Disposable upload document released ${nonce}`,target:{document_generation:2}}}};}
test('scoped Browser cleanup requires exact generated namespace, settled original Turn and real disposed-document evidence',()=>{
  expect(windowsBrowserCleanupSource(source()).root).toBe(root);
  for(const change of [value=>{value.failure_settlement.execution.unknown_effects=1;},value=>{value.native_case_resources.root='C:\\Users';},value=>{value.observations={};},value=>{value.scenario='desktop';},value=>{value.scenario_host_resources_closed=false;}]){const value=source();change(value);expect(()=>windowsBrowserCleanupSource(value)).toThrow();}
});
