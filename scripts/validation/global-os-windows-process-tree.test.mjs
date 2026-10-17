import test from 'node:test';
import assert from 'node:assert/strict';
import {assertWindowsClosedProcessObservations,classifyWindowsProcessTree} from './global-os-windows-process-tree.mjs';
test('owned process closure needs exact native identity and never treats access denial or live same creation as gone',()=>{
  const before=[{pid:100,creation_filetime:'134000000000000001'}];
  assertWindowsClosedProcessObservations(before,[{pid:100,state:'absent'}]);
  assertWindowsClosedProcessObservations(before,[{pid:100,state:'exited',creation_filetime:before[0].creation_filetime}]);
  assertWindowsClosedProcessObservations(before,[{pid:100,state:'running',creation_filetime:'134000000000000002'}]);
  for(const after of [[],[{pid:101,state:'absent'}],[{pid:100,native_error:'AccessDenied'}],[{pid:100,state:'running',creation_filetime:before[0].creation_filetime}],[{pid:100,state:'unknown'}]])assert.throws(()=>assertWindowsClosedProcessObservations(before,after));
  assert.throws(()=>assertWindowsClosedProcessObservations([{pid:100,native_error:'Denied'}],[{pid:100,state:'absent'}]));
});
function candidates(){return{appPid:100,appCreationFiletime:'1000',processes:[{pid:100,state:'running',creation_filetime:'1000'},{pid:200,state:'running',creation_filetime:'2000'},{pid:300,state:'running',creation_filetime:'1500'},{pid:400,state:'running',creation_filetime:'2500'},{pid:500,state:'running',creation_filetime:'3000'}],edges:[{parent_pid:100,pid:200},{parent_pid:200,pid:300},{parent_pid:300,pid:400},{parent_pid:100,pid:500}]};}
test('a proven reused creator PID excludes its entire disconnected subtree while preserving valid current App branches',()=>{
  const result=classifyWindowsProcessTree(candidates());assert.equal(result.complete,true);assert.deepEqual(result.processes.map(row=>row.pid),[100,200,500]);assert.deepEqual(result.excluded_nodes.map(row=>row.pid),[300,400]);assert.deepEqual(result.edges,[{parent_pid:100,pid:200},{parent_pid:100,pid:500}]);assert.deepEqual(result.excluded_edges,[{parent_pid:200,pid:300,parent_creation_filetime:'2000',child_creation_filetime:'1500',reason:'child_predates_current_parent_pid'}]);
});
test('denied missing and unobserved creation identities remain unproven and cannot be excluded as a reused creator edge',()=>{
  for(const mutate of [row=>{row.native_error='AccessDenied';},row=>{row.creation_filetime='';},row=>{row.state='absent';delete row.creation_filetime;}]){const input=candidates();mutate(input.processes[2]);const result=classifyWindowsProcessTree(input);assert.equal(result.complete,false);assert.equal(result.excluded_edges.length,0);assert(result.processes.some(row=>row.pid===300));assert(result.processes.some(row=>row.pid===400));}
  const input=candidates();input.processes[3].native_error='AccessDenied';assert.equal(classifyWindowsProcessTree(input).complete,false);
});
test('tree classification retains the exact current root identity and refuses a stale or already exited root',()=>{
  for(const mutate of [input=>{input.appCreationFiletime='999';},input=>{input.processes[0].state='exited';},input=>{input.processes.shift();}]){const input=candidates();mutate(input);assert.throws(()=>classifyWindowsProcessTree(input));}
});
