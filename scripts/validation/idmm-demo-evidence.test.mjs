import test from 'node:test';
import assert from 'node:assert/strict';
import {IDMM_DEMO_CASES, IDMM_DEMO_PREFIX, parseIdmmDemoCase, parseIdmmDemoEvidence} from './idmm-demo-evidence.mjs';

function evidence(name, index) {
  return {case:name,status:'pass',session_id:`0190f5fe-7c00-7a00-8000-${String(index + 1).padStart(12, '0')}`,
    elapsed_ms:20,idmm_turns:Number(!['off','sensitive_halt','provider_pause'].includes(name)),completed_turns:2,upstream_calls:2,
    native_resumes:Number(name==='provider_pause'),known_limitation:name==='provider_pause'?'provider_pause_requires_explicit_native_resume':null,
    sidecar_calls:Number(name==='sidecar_question'),injected_errors:Number(name==='provider_pause'),
    injected_stalls:Number(name==='stalled_recovery'),turns:[],interventions:[],transcript:[]};
}
const transcript = () => IDMM_DEMO_CASES.map((name,index)=>IDMM_DEMO_PREFIX+JSON.stringify(evidence(name,index))).join('\n');

test('a complete demo requires all distinct Sessions and real fault/sidecar evidence', () => {
  assert.equal(parseIdmmDemoEvidence(transcript()).cases.length, 7);
  assert.throws(()=>parseIdmmDemoEvidence(transcript().split('\n').slice(1).join('\n')), /INCOMPLETE/);
  const duplicate = transcript().replace('0190f5fe-7c00-7a00-8000-000000000007','0190f5fe-7c00-7a00-8000-000000000001');
  assert.throws(()=>parseIdmmDemoEvidence(duplicate), /INVALID/);
  const noFault = transcript().replace('"injected_errors":1','"injected_errors":0');
  assert.throws(()=>parseIdmmDemoEvidence(noFault), /INVALID/);
});

test('malformed, oversized or inconsistent evidence cannot produce a pass', () => {
  assert.equal(parseIdmmDemoCase(IDMM_DEMO_PREFIX+'{broken'),null);
  const wrong = evidence('sidecar_question', 3);
  wrong.sidecar_calls=0;
  assert.equal(parseIdmmDemoCase(IDMM_DEMO_PREFIX+JSON.stringify(wrong)),null);
  const oversized = evidence('off',0);
  oversized.transcript=[{state:'completed',content:'x'.repeat(4000)}];
  assert.equal(parseIdmmDemoCase(IDMM_DEMO_PREFIX+JSON.stringify(oversized)),null);
});

test('a bounded failed case is diagnostic evidence and cannot make the suite pass', () => {
  const failed = evidence('sidecar_question', 3);
  failed.status='fail';
  failed.failure_code='IDMM_SIDECAR_IGNORED_USER_PREFERENCE';
  failed.transcript=[{state:'accepted',content:'wrong title'}];
  assert.equal(parseIdmmDemoCase(IDMM_DEMO_PREFIX+JSON.stringify(failed)).status,'fail');
  const output=transcript().split('\n');
  output[3]=IDMM_DEMO_PREFIX+JSON.stringify(failed);
  assert.throws(()=>parseIdmmDemoEvidence(output.join('\n')),/INCOMPLETE/);
});
