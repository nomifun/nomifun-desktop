import { expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { windowsUacCancellationProof, windowsUacCancelOptions } from './global-os-windows-uac-cancel-native.mjs';

const stable = value => JSON.stringify(value, (_, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item);

function fixture() {
  const session = 'session', operation = 'operation', accepted = {message_id:'root',operation_id:operation};
  const call = {id:'unexecuted-admin',args:{command:'fixture.exe',args:['unique-marker'],cwd:'C:\\fixture',execution_identity:'administrator'}};
  const diagnostic = {code:'SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED',message:'native operation cancelled',kind:'Uncategorized',raw_os_error:1223,user_code_not_started:true,cleanup_proven:true};
  const error = {code:diagnostic.code,retryable:false,systemExecutionError:diagnostic};
  const progress = (seq, event) => ({agent_session_id:session,correlation_id:operation,seq,kind:'runtime/progress-recorded',payload:{value:{event}}});
  const carrier = stable({code:diagnostic.code,message:diagnostic.message,details:{system_execution_error:diagnostic}});
  const events = [
    {agent_session_id:session,correlation_id:operation,seq:1,kind:'turn/started',payload:{value:{source_message_id:accepted.message_id}}},
    progress(2,{event:'tool_call_completed',step:1,call:{call_id:call.id,name:'exec_command',arguments:call.args}}),
    progress(3,{event:'tool_call_completed',step:0,call:{call_id:'agent-instructions:100',name:'read_file',arguments:{path:call.args.cwd,format:'instruction_scope',execution_identity:'administrator'}}}),
    progress(4,{event:'tool_started',step:0,call_id:'agent-instructions:100',capability_id:'workspace.files',action_id:'workspace.files/read'}),
    progress(5,{event:'host_tool_dispatch',dispatch:{operation_id:`${operation}:tool:agent-instructions:100`,call_id:'agent-instructions:100',capability_id:'workspace.files',action_id:'workspace.files/read',model_name:'read_file'}}),
    progress(6,{event:'tool_completed',step:0,result:{call_id:'agent-instructions:100',is_error:true,output:[{type:'text',text:carrier}]}}),
    progress(7,{event:'host_cleanup_proven'}),
    progress(8,{event:'turn_failed',failure:{kind:'system_execution',diagnostic}}),
    {agent_session_id:session,correlation_id:operation,seq:9,kind:'turn/failed',payload:{value:{error}}},
  ];
  return {events,session,accepted,call,markerAbsent:true,execution:{operation_id:operation,state:'failed',pending_effects:0,unknown_effects:0,producer_lease_live:false,checkpoint_retained:false,automatic_replay_authorized:false},
    summary:{content:{turn_summary:true,turn_id:accepted.message_id,turn_state:'failed',error}}};
}

test('cancelled native instruction reads may have ToolStarted while proposed mutations have zero dispatch', () => {
  const value = fixture();
  expect(windowsUacCancellationProof(value)).toMatchObject({code:'SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED',mutation_effect_count:0,user_mutation_dispatch_count:0,administrator_preflight_count:1,internal_read_dispatch_count:1});
  const completed = value.events[5], output = completed.payload.value.event.result.output, bytes = Buffer.from(stable(output));
  completed.payload.value.event.result.output = null;
  const digest = createHash('sha256').update(bytes).digest('hex');
  completed.payload.value._canonical_shared_content = {version:1,references:[{pointer:'/event/result/output',kind:'tool_output',digest,byte_len:bytes.length,payload_id:`shared-tool:${value.session}:output:${digest}`}]};
  expect(windowsUacCancellationProof(value).preflight_result_seq).toBe(6);
  completed.payload.value._canonical_shared_content.references[0].digest = 'f'.repeat(64);
  expect(() => windowsUacCancellationProof(value)).toThrow();
});

test('unknown, ordinary ACL, binding, fabricated prose and user Stop cannot qualify as UAC cancellation', () => {
  const mutations = [
    value => { value.events[8].payload.value.error.code = 'PRESET_RESOURCE_NOT_BOUND'; },
    value => { value.events[8].payload.value.error.code = 'NOMIFUN_INTERNAL_ERROR'; },
    value => { value.events[8].payload.value.error.systemExecutionError.code = 'SYSTEM_READ_RESULT_UNKNOWN'; },
    value => { value.events[8].payload.value.error.systemExecutionError.raw_os_error = 5; },
    value => { value.events[8].payload.value.error.systemExecutionError.raw_os_error = null; },
    value => { value.events[8].payload.value.error.systemExecutionError.user_code_not_started = false; },
    value => { value.events[8].payload.value.error.systemExecutionError.cleanup_proven = null; },
    value => { delete value.events[8].payload.value.error.systemExecutionError; value.events[8].payload.value.error.message = 'raw_os_error=1223 cleanup_proven=true'; },
    value => { value.events[8].kind = 'turn/cancelled'; },
    value => { value.markerAbsent = false; },
    value => { value.execution.unknown_effects = 1; },
    value => { value.execution.producer_lease_live = true; },
    value => { value.events.push({agent_session_id:value.session,kind:'effect/started',payload:{value:{turn_id:value.accepted.operation_id}}}); },
    value => { value.events[3].payload.value.event.call_id = value.call.id; },
    value => { value.events[4].payload.value.event.dispatch.action_id = 'workspace.process/exec'; },
    value => { value.events[6].seq = 10; },
    value => { value.summary.content.error = {code:'NOMIFUN_INTERNAL_ERROR'}; },
    value => { value.events[3].agent_session_id = 'foreign'; },
  ];
  for (const mutate of mutations) { const value = fixture(); mutate(value); expect(() => windowsUacCancellationProof(value)).toThrow(); }
});

test('native driver requires explicit execution, exact ready report, source and executable identities', () => {
  expect(() => windowsUacCancelOptions([])).toThrow();
  expect(() => windowsUacCancelOptions(['--execute','--execute'])).toThrow();
  expect(() => windowsUacCancelOptions(['--yes'])).toThrow();
  const args = ['--execute','--fixture-report','ready.json','--report-root','reports','--repo-root','repo','--expected-source-head','a'.repeat(40),'--expected-exe-sha256','b'.repeat(64)];
  expect(windowsUacCancelOptions(args)['--expected-source-head']).toBe('a'.repeat(40));
});
