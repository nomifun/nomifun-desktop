import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash, randomBytes } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Synthetic pipeline validation only. Every fixture is disposable and is
// never pooled with live product evidence or used to claim a reliability SLO.
const collector = fileURLToPath(new URL('./agent-reliability-collect.mjs', import.meta.url));
const checks = { tools: { artifact: true }, execution: { terminal: true }, quality: { scope: true } };
const graderSource = `
const fs = require('node:fs');
const path = require('node:path');
const workspace = process.argv[2];
const valid = fs.readFileSync(path.join(workspace,'answer.txt'),'utf8') === 'verified artifact';
if (process.env.NOMIFUN_RELIABILITY_EVIDENCE_KEY || process.env.NOMIFUN_LIVE_STEPFUN_API_KEY) process.exit(2);
console.log(JSON.stringify({ checks: { tools:{artifact:valid}, execution:{terminal:true}, quality:{scope:valid && !fs.existsSync(path.join(workspace,'forbidden.txt'))} } }));
`;

function fixture(t, source = graderSource, amendPlan = () => {}) {
  const root = mkdtempSync(join(tmpdir(),'nomifun-collector-test-'));
  t.after(() => rmSync(root,{recursive:true,force:true}));
  const workspace = join(root,'workspace'); const verifier = join(root,'verifier');
  mkdirSync(workspace); mkdirSync(verifier); mkdirSync(join(root,'receipts'));
  writeFileSync(join(workspace,'answer.txt'),'verified artifact');
  const grader = join(verifier,'grader.cjs'); writeFileSync(grader,source);
  const plan = { schema_version:1,suite_id:'synthetic-collector-test',runtime_build_digest:'a'.repeat(64),model:'step-3.7-flash',
    strata:[{id:'coding',minimum_samples:1,minimum_duration_ms:0,metrics:Object.keys(checks),
      checks:Object.fromEntries(Object.entries(checks).map(([metric,items])=>[metric,Object.keys(items)])),
      max_model_steps:2,max_compaction_requests:0,max_resume_authorizations:1}],
    scheduled_trials:[{id:'one',stratum:'coding'},{id:'missing',stratum:'coding'}],
    grader:{program:process.execPath,cwd:verifier,args:[grader,'{workspace}'],files:[grader],timeout_ms:2000,independent_assertions:true} };
  amendPlan(plan);
  const planPath = join(root,'plan.json'); writeFileSync(planPath,JSON.stringify(plan));
  const manifest = join(root,'manifest.json'); const receipt = join(root,'receipts','one.json');
  const key = randomBytes(32).toString('hex');
  const run = (command,args = [], environment = {}) => spawnSync(process.execPath,[collector,command,...args], {
    env:{...process.env,NOMIFUN_RELIABILITY_EVIDENCE_KEY:key,...environment},encoding:'utf8',windowsHide:true,timeout:10000,
  });
  const frozen = run('freeze',['--plan',planPath,'--output',manifest]); assert.equal(frozen.status,0,frozen.stderr);
  const pin = createHash('sha256').update(readFileSync(manifest)).digest('hex');
  const capture = {schema_version:1,source:'live_product',suite_id:plan.suite_id,trial_id:'one',session_id:'session-one',operation_id:'turn-one',
    runtime_build_digest:plan.runtime_build_digest,model:plan.model,observed_models:[plan.model],duration_ms:1,workspace,events:[]};
  const event = (kind,data={}) => ({agent_session_id:capture.session_id,event_id:`event-${capture.events.length+1}`,
    seq:capture.events.length+1,kind,correlation_id:'turn-one',resolved_payload:data});
  capture.events.push(event('turn/started'));
  capture.events.push(event('runtime/progress-recorded',{event:{event:'turn_started',binding:{build_digest:plan.runtime_build_digest}}}));
  capture.events.push(event('runtime/progress-recorded',{event:{event:'model_step_started',step:1}}));
  capture.events.push(event('turn/completed'));
  const capturePath = join(root,'capture.json');
  const base = ['--manifest',manifest,'--manifest-sha256',pin];
  const record = (overrides = {}, environment = {}) => {
    writeFileSync(capturePath,JSON.stringify({...capture,...overrides}));
    return run('record',[...base,'--trial','one','--capture',capturePath,'--workspace',workspace,'--output',receipt],environment);
  };
  const aggregate = () => run('aggregate',[...base,'--receipts',join(root,'receipts'),'--output',join(root,'evidence.json'),'--report',join(root,'report.json')]);
  return {root,workspace,grader,manifest,receipt,capture,record,aggregate,run,base,pin};
}

test('fixed independent artifact checks are signed and missing runs remain in denominator', t => {
  const f = fixture(t);
  const result = f.record({}, {NOMIFUN_LIVE_STEPFUN_API_KEY:'test-sentinel-must-not-reach-grader'});
  assert.equal(result.status,0,result.stderr);
  assert.deepEqual(JSON.parse(readFileSync(f.receipt)).body.sample.checks,checks);
  assert.equal(f.aggregate().status,1);
  const report = JSON.parse(readFileSync(join(f.root,'report.json')));
  assert.equal(report.status,'not_proven'); assert.equal(report.missing_trials,1);
  assert.equal(report.metrics.execution.trials,2); assert.equal(report.metrics.execution.successes,1);
});

test('mock, changed model/build, missing observed model and mixed event chain are rejected', async t => {
  const variants = {
    mock:c=>({...c,source:'scripted_product'}), model:c=>({...c,model:'other'}), build:c=>({...c,runtime_build_digest:'b'.repeat(64)}),
    no_observed_model:c=>({...c,observed_models:[]}),
    seq_gap:c=>({...c,events:c.events.filter(e=>e.seq!==2)}),
    wrong_turn:c=>({...c,events:c.events.map(e=>e.seq===3?{...e,correlation_id:'other-turn'}:e)}),
    missing_native_root:c=>({...c,events:c.events.map(e=>e.seq===2?{...e,resolved_payload:{event:{event:'context_prepared'}}}:e)}),
  };
  for (const [name,change] of Object.entries(variants)) await t.test(name,t=>{
    const f=fixture(t); const result=f.record(change(f.capture));
    assert.equal(result.status,2,result.stdout); assert.equal(existsSync(f.receipt),false);
  });
});

test('pin and verifier drift fail before grading and no existing receipt is overwritten', t => {
  const f=fixture(t);
  const wrong=f.run('record',['--manifest',f.manifest,'--manifest-sha256','f'.repeat(64)]);
  assert.equal(wrong.status,2); assert.match(wrong.stderr,/pin_mismatch/);
  assert.equal(f.record().status,0);
  const before=readFileSync(f.receipt);
  assert.equal(f.record().status,2); assert.deepEqual(readFileSync(f.receipt),before);
  writeFileSync(f.grader,`${graderSource}\n// drift`);
  assert.match(f.record().stderr,/independent_verifier_changed/);
});

test('signature tampering and duplicate sessions cannot enter aggregation', async t => {
  await t.test('tampered receipt',t=>{
    const f=fixture(t); assert.equal(f.record().status,0);
    const receipt=JSON.parse(readFileSync(f.receipt)); receipt.body.sample.checks.quality.scope=false;
    writeFileSync(f.receipt,JSON.stringify(receipt));
    const result=f.aggregate(); assert.equal(result.status,2); assert.match(result.stderr,/signature_mismatch/);
  });
  await t.test('duplicate receipt',t=>{
    const f=fixture(t); assert.equal(f.record().status,0);
    writeFileSync(join(f.root,'receipts','duplicate.json'),readFileSync(f.receipt));
    assert.equal(f.aggregate().status,2);
  });
});

test('paused, manual and over-budget executions never inherit a successful grade', async t=>{
  for (const kind of ['turn/paused','runtime/effect-reconciliation-attested','runtime/progress-recorded']) await t.test(kind,t=>{
    const f=fixture(t); const events=f.capture.events.map(e=>({...e}));
    if(kind==='turn/paused') events[3].kind=kind;
    else { events.splice(3,0,{...events[3],kind,resolved_payload:{event:{event:'compaction_started'}}});
      events.forEach((e,i)=>{e.seq=i+1;e.event_id=`event-${i+1}`;}); }
    assert.equal(f.record({events}).status,0);
    assert.equal(JSON.parse(readFileSync(f.receipt)).body.sample.checks.execution.terminal,false);
  });
});

test('invalid output, missing checks, timeout and verifier mutation remain unverified', async t=>{
  for (const [name,source] of Object.entries({
    non_json:"console.log('not json');", missing_checks:"console.log(JSON.stringify({checks:{}}));",
    timeout:'setInterval(()=>{},1000);', changed_during_run:`require('node:fs').appendFileSync(__filename,'\\n// modified');console.log(JSON.stringify({checks:${JSON.stringify(checks)}}));`,
  })) await t.test(name,t=>{
    const f=fixture(t,source,plan=>{if(name==='timeout')plan.grader.timeout_ms=50;});
    assert.equal(f.record().status,0);
    const body=JSON.parse(readFileSync(f.receipt)).body;
    assert.equal(body.grader_status,'unverified'); assert.equal(body.sample.checks.quality.scope,null);
    if(name==='timeout') assert.equal(body.verifier_timeout_or_signal,true);
  });
});

test('replaced workspace and absent harness key fail before verifier execution',t=>{
  const f=fixture(t); const other=join(f.root,'other'); mkdirSync(other);
  assert.equal(f.record({workspace:other}).status,2);
  const result=f.record({}, {NOMIFUN_RELIABILITY_EVIDENCE_KEY:''});
  assert.equal(result.status,2); assert.match(result.stderr,/signing_key_unavailable/);
});
