import{test,expect}from'bun:test';
import{nativeControlTerminalFailureObservation}from'./global-os-native.mjs';
function fixture(receipt={state:'exited',exit_code:1,cleanup:{reaped:true,errors:[]},output:{text:'complete typed diagnostic',dropped_bytes:0,decode_errors:0}}){
  const callId='control-call',sessionId='control-session',operationId='control-turn',callArguments={command:'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',args:['-NoProfile','-File','C:\\fixture\\probe.ps1'],cwd:'C:\\fixture'};
  const body=JSON.stringify(receipt),raw=`[tool error]\n${body}`,event=(seq,value)=>({seq,agent_session_id:sessionId,correlation_id:operationId,kind:'runtime/progress-recorded',payload:{storage:'inline_json',value:{event:value}}});
  const events=[event(1,{event:'tool_call_completed',step:1,call:{call_id:callId,name:'exec_command',arguments:callArguments}}),event(2,{event:'tool_started',step:1,call_id:callId}),event(3,{event:'host_tool_dispatch',dispatch:{call_id:callId,operation_id:`${operationId}:tool:${callId}`,action_id:'workspace.process/exec'}}),event(4,{event:'tool_completed',step:1,result:{call_id:callId,is_error:true,output:[{type:'text',text:body}]}})];
  return{raw,callId,sessionId,operationId,callArguments,events};
}
test('control diagnostics can observe only their exact dispatched terminal failure while preserving the failure',()=>{
  const proof=nativeControlTerminalFailureObservation(fixture());expect(proof.receipt.exit_code).toBe(1);expect(proof.provenance.original_failure_preserved).toBe(true);expect(proof.provenance.failed_call_accepted_as_success).toBe(false);
  for(const change of [x=>{x.callId='foreign';},x=>{x.operationId='foreign';},x=>{x.events[3].agent_session_id='foreign';},x=>{x.callArguments={...x.callArguments,cwd:'C:\\foreign'};},x=>{x.events[3].payload.value.event.result.is_error=false;},x=>{x.events[3].payload.value.event.runtime_hold='runtime_gate';},x=>{x.events[2].payload.value.event.dispatch.action_id='other/action';},x=>{x.events[2].payload.value.event.dispatch.operation_id='foreign';},x=>{x.events.splice(1,1);},x=>{x.events[0].payload.value.event.call.name='read_file';}]){const input=fixture();change(input);expect(()=>nativeControlTerminalFailureObservation(input)).toThrow();}
});
test('running unknown unjoined or truncated control output cannot authorize diagnostic cleanup',()=>{
  for(const change of [x=>{x.state='running';},x=>{x.state='unknown';},x=>{delete x.cleanup;},x=>{x.cleanup.reaped=false;},x=>{x.cleanup.errors=['unproven'];},x=>{x.exit_code=0;},x=>{x.output.dropped_bytes=1;},x=>{x.output.decode_errors=1;}]){const receipt={state:'exited',exit_code:1,cleanup:{reaped:true,errors:[]},output:{text:'complete typed diagnostic',dropped_bytes:0,decode_errors:0}};change(receipt);expect(()=>nativeControlTerminalFailureObservation(fixture(receipt))).toThrow();}
  const input=fixture();input.raw=input.raw.slice(0,-1);input.events[3].payload.value.event.result.output[0].text=input.raw.slice('[tool error]\n'.length);expect(()=>nativeControlTerminalFailureObservation(input)).toThrow();
});
