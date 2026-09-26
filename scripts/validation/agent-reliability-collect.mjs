#!/usr/bin/env node
/** Frozen suite -> host capture -> pinned independent verifier -> signed
 * receipt -> existing statistical gate. Nothing runs unless explicitly invoked.
 * Keep the evidence signing key in the verifier harness, NEVER the Agent/app.
 */
import { createHash, createHmac, timingSafeEqual } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, statSync, realpathSync, mkdirSync, readdirSync } from 'node:fs';
import { resolve, dirname, relative, isAbsolute, basename } from 'node:path';
import { summarizeReliability } from './agent-reliability-report.mjs';

const must = (value, code) => { if (!value) throw new Error(code); };
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const sha = value => createHash('sha256').update(value).digest('hex');
const canonical = value => JSON.stringify(sort(value));
function sort(value) {
  if (Array.isArray(value)) return value.map(sort);
  if (object(value)) return Object.fromEntries(Object.keys(value).sort().map(key => [key, sort(value[key])]));
  return value;
}
function read(path, limit = 128 * 1024 * 1024) {
  const file = realpathSync(resolve(path));
  must(statSync(file).isFile() && statSync(file).size <= limit, 'evidence_file_not_bounded');
  const bytes = readFileSync(file);
  return { path: file, sha256: sha(bytes), value: JSON.parse(bytes.toString('utf8')) };
}
function save(path, value) {
  const output = resolve(path);
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(value, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' });
}
function inside(root, file) {
  const path = relative(root, file);
  return path === '' || (!isAbsolute(path) && path !== '..' && !path.startsWith('../') && !path.startsWith('..\\'));
}
function pinFile(path) {
  const file = realpathSync(resolve(path));
  must(statSync(file).isFile() && statSync(file).size <= 256 * 1024 * 1024, 'verifier_file_not_bounded');
  return { path: file, sha256: sha(readFileSync(file)) };
}
function signingKey() {
  const value = process.env.NOMIFUN_RELIABILITY_EVIDENCE_KEY;
  must(typeof value === 'string' && /^[a-fA-F0-9]{64}$/.test(value), 'collector_signing_key_unavailable');
  return Buffer.from(value, 'hex');
}
function signature(body) { return createHmac('sha256', signingKey()).update(canonical(body)).digest('hex'); }
function verified(receipt) {
  must(object(receipt) && object(receipt.body) && typeof receipt.hmac_sha256 === 'string'
    && /^[a-f0-9]{64}$/.test(receipt.hmac_sha256), 'invalid_signed_receipt');
  must(timingSafeEqual(Buffer.from(receipt.hmac_sha256, 'hex'), Buffer.from(signature(receipt.body), 'hex')), 'receipt_signature_mismatch');
  return receipt.body;
}
function argumentsFor(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 2) {
    must(args[index]?.startsWith('--') && args[index + 1] && !Object.hasOwn(options, args[index]), 'invalid_collector_arguments');
    options[args[index]] = args[index + 1];
  }
  return options;
}
function loadManifest(options) {
  const data = read(options['--manifest'], 4 * 1024 * 1024);
  must(data.sha256 === options['--manifest-sha256'], 'frozen_manifest_pin_mismatch');
  must(data.value.collector_version === 1, 'unknown_collector_manifest');
  summarizeReliability({ ...data.value, samples: [] });
  return data;
}
function checkVerifier(grader, workspace) {
  must(object(grader) && grader.independent_assertions === true && Array.isArray(grader.pins), 'invalid_independent_verifier');
  must(grader.pins.length > 0 && grader.pins.length <= 129, 'invalid_verifier_pin_count');
  for (const pin of grader.pins) {
    must(!inside(workspace, pin.path), 'verifier_is_inside_agent_workspace');
    must(pinFile(pin.path).sha256 === pin.sha256, 'independent_verifier_changed');
  }
  must(!inside(workspace, grader.cwd), 'verifier_cwd_is_inside_agent_workspace');
}
function payload(event) {
  if (object(event.resolved_payload)) return event.resolved_payload;
  if (event.payload?.storage === 'inline_json') return event.payload.value;
  if (typeof event.inline_json === 'string') return JSON.parse(event.inline_json);
  if (object(event.inline_json)) return event.inline_json;
  return null;
}
function inspectCapture(capture, manifest, trial) {
  must(capture.schema_version === 1 && capture.source === 'live_product'
    && capture.trial_id === trial.id && capture.suite_id === manifest.suite_id
    && capture.runtime_build_digest === manifest.runtime_build_digest && capture.model === manifest.model,
  'capture_does_not_match_frozen_trial');
  must(typeof capture.session_id === 'string' && /^[a-zA-Z0-9_.:-]{1,128}$/.test(capture.session_id), 'capture_session_invalid');
  must(Number.isSafeInteger(capture.duration_ms) && capture.duration_ms >= 0, 'capture_duration_invalid');
  must(Array.isArray(capture.observed_models) && capture.observed_models.length <= 128
    && capture.observed_models.every(model => model === manifest.model), 'capture_model_mismatch');
  if (trial.input_sha256) must(capture.input_sha256 === trial.input_sha256, 'capture_task_input_changed');
  must(Array.isArray(capture.events) && capture.events.length > 0 && capture.events.length <= 1_000_000, 'capture_event_window_invalid');
  let last = 0;
  let terminal = null;
  let operation = null;
  let pauses = 0;
  let recoveries = 0;
  let manual = 0;
  let proposedCompletions = 0;
  let toolStarts = 0;
  let modelRequests = 0;
  let compactionRequests = 0;
  let resumeAuthorizations = 0;
  let nativeRoots = 0;
  const effects = new Set();
  const ids = new Set();
  for (const event of capture.events) {
    must(event.agent_session_id === capture.session_id && event.seq === last + 1
      && typeof event.event_id === 'string' && !ids.has(event.event_id), 'capture_chain_is_incomplete_or_mixed');
    last = event.seq; ids.add(event.event_id);
    const data = payload(event);
    if (event.kind === 'turn/started') {
      must(operation === null, 'one_trial_cannot_pool_multiple_turns');
      must(typeof event.correlation_id === 'string' && event.correlation_id.length > 0
        && event.correlation_id === capture.operation_id, 'capture_turn_identity_mismatch');
      operation = event.correlation_id;
    }
    if (event.kind.startsWith('turn/') || event.kind === 'runtime/progress-recorded'
      || event.kind === 'runtime/effect-reconciliation-attested') {
      must(operation !== null && event.correlation_id === operation, 'capture_turn_identity_mismatch');
    }
    if (event.kind === 'tool/call-started') toolStarts++;
    if (event.kind === 'effect/started') {
      must(!effects.has(event.correlation_id), 'duplicate_effect_identity'); effects.add(event.correlation_id);
    }
    if (event.kind === 'turn/paused') pauses++;
    if (event.kind === 'turn/resume-authorized') resumeAuthorizations++;
    if (event.kind === 'runtime/effect-reconciliation-attested') manual++;
    if (event.kind === 'runtime/progress-recorded') {
      must(object(data?.event), 'capture_runtime_metadata_not_resolved');
      if (data.event.event === 'model_step_started') modelRequests++;
      if (data.event.event === 'compaction_started') compactionRequests++;
      if (data?.event?.event === 'execution_resumed') recoveries++;
      if (data?.event?.event === 'completion_reported') proposedCompletions++;
      if (data?.event?.event === 'turn_started') {
        nativeRoots++;
        must(nativeRoots === 1 && data.event.binding?.build_digest === manifest.runtime_build_digest, 'native_build_mismatch');
      }
    }
    if (['turn/completed', 'turn/failed', 'turn/cancelled'].includes(event.kind)) {
      must(event.correlation_id === operation && terminal === null, 'capture_terminal_is_ambiguous');
      terminal = event.kind;
    }
  }
  must(operation !== null, 'capture_has_no_accepted_turn');
  // Failed pre-model admissions remain recordable failures. A model-backed
  // run requires both a canonical native build root and a harness-observed
  // provider route; an empty every() check is not evidence of a live model.
  must((modelRequests === 0 && terminal !== 'turn/completed') || nativeRoots === 1, 'capture_native_root_missing');
  must(modelRequests === 0 ? capture.observed_models.length === 0 : capture.observed_models.length > 0, 'capture_model_observation_missing');
  return { terminal, pauses, recoveries, manual_reconciliations: manual, tool_admissions: toolStarts,
    model_requests: modelRequests, compaction_requests: compactionRequests, resume_authorizations: resumeAuthorizations,
    model_completion_reports: proposedCompletions, event_count: capture.events.length,
    model_reports_are_not_grades: true };
}
function emptyChecks(stratum, value) {
  return Object.fromEntries(stratum.metrics.map(metric => [metric,
    Object.fromEntries(stratum.checks[metric].map(check => [check, value]))]));
}
function normalizeChecks(stratum, grade) {
  must(object(grade) && object(grade.checks) && Object.keys(grade).every(key => key === 'checks'), 'verifier_output_invalid');
  const checks = emptyChecks(stratum, null);
  must(Object.keys(grade.checks).length === stratum.metrics.length, 'verifier_metric_coverage_changed');
  for (const metric of stratum.metrics) {
    must(object(grade.checks[metric]) && Object.keys(grade.checks[metric]).length === stratum.checks[metric].length, 'verifier_check_coverage_changed');
    for (const check of stratum.checks[metric]) {
      must([true, false, null].includes(grade.checks[metric][check]), 'verifier_check_not_observed');
      checks[metric][check] = grade.checks[metric][check];
    }
  }
  return checks;
}

function freeze(options) {
  const plan = read(options['--plan'], 4 * 1024 * 1024).value;
  summarizeReliability({ ...plan, samples: [] });
  for (const stratum of plan.strata) {
    must(Number.isSafeInteger(stratum.max_model_steps) && stratum.max_model_steps > 0 && stratum.max_model_steps <= 4096
      && Number.isSafeInteger(stratum.max_compaction_requests) && stratum.max_compaction_requests >= 0 && stratum.max_compaction_requests <= 2048
      && Number.isSafeInteger(stratum.max_resume_authorizations) && stratum.max_resume_authorizations >= 0 && stratum.max_resume_authorizations <= 64,
    'frozen_execution_budget_required');
    const kinds = stratum.allowed_terminal_kinds ?? ['turn/completed'];
    must(Array.isArray(kinds) && kinds.length > 0 && kinds.every(kind => ['turn/completed', 'turn/failed', 'turn/cancelled'].includes(kind)), 'frozen_terminal_policy_invalid');
    must(stratum.allow_owner_reconciliation === undefined || typeof stratum.allow_owner_reconciliation === 'boolean', 'frozen_owner_policy_invalid');
  }
  const grader = plan.grader;
  must(object(grader) && grader.independent_assertions === true && isAbsolute(grader.program)
    && isAbsolute(grader.cwd) && Array.isArray(grader.args) && grader.args.every(value => typeof value === 'string')
    && grader.args.length <= 64 && Array.isArray(grader.files) && grader.files.length > 0 && grader.files.length <= 128,
  'freeze_requires_a_pinned_independent_grader');
  must(!/^(?:cmd|powershell|pwsh|bash|sh|zsh)(?:\.exe)?$/i.test(basename(grader.program)), 'shell_grader_not_allowed');
  must(Number.isSafeInteger(grader.timeout_ms) && grader.timeout_ms > 0 && grader.timeout_ms <= 600_000, 'verifier_timeout_not_bounded');
  const manifest = { ...plan, samples: undefined, collector_version: 1,
    frozen_at: new Date().toISOString(),
    grader: { ...grader, program: realpathSync(grader.program), cwd: realpathSync(grader.cwd),
      pins: [...new Set([grader.program, ...grader.files])].map(pinFile) } };
  save(options['--output'], manifest);
  console.log(`frozen_manifest_sha256=${sha(readFileSync(resolve(options['--output'])))}`);
}

function record(options) {
  signingKey(); // Fail before any verifier execution if the harness is not configured.
  const manifestData = loadManifest(options);
  const manifest = manifestData.value;
  const trial = manifest.scheduled_trials.find(item => item.id === options['--trial']);
  must(trial, 'unscheduled_trial');
  const stratum = manifest.strata.find(item => item.id === trial.stratum);
  const captureData = read(options['--capture']);
  const capture = captureData.value;
  const observed = inspectCapture(capture, manifest, trial);
  const workspace = realpathSync(resolve(options['--workspace']));
  must(statSync(workspace).isDirectory(), 'agent_workspace_missing');
  must(typeof capture.workspace === 'string' && realpathSync(resolve(capture.workspace)) === workspace, 'capture_workspace_changed');
  must(!inside(workspace, captureData.path) && !inside(workspace, manifestData.path)
    && !inside(workspace, resolve(options['--output'])), 'evidence_must_be_outside_agent_workspace');
  checkVerifier(manifest.grader, workspace);
  const substitutions = { '{workspace}': workspace, '{capture}': captureData.path, '{trial}': trial.id };
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => /^(?:PATH|PATHEXT|SYSTEMROOT|WINDIR|TEMP|TMP|HOME|USERPROFILE|LANG|LC_ALL)$/i.test(key)));
  const result = spawnSync(manifest.grader.program, manifest.grader.args.map(value => substitutions[value] ?? value), {
    cwd: manifest.grader.cwd, env, encoding: 'utf8', shell: false, windowsHide: true,
    timeout: manifest.grader.timeout_ms, maxBuffer: 1024 * 1024,
  });
  let checks = emptyChecks(stratum, null);
  let graderStatus = 'unverified';
  try {
    checkVerifier(manifest.grader, workspace);
    must(!result.error && result.status === 0, 'verifier_execution_failed');
    checks = normalizeChecks(stratum, JSON.parse(result.stdout));
    graderStatus = 'observed';
  } catch { /* Nulls remain failures/unverified, never silently dropped. */ }
  const allowedTerminals = stratum.allowed_terminal_kinds ?? ['turn/completed'];
  if ((!allowedTerminals.includes(observed.terminal)
    || (observed.terminal === 'turn/completed' && observed.model_requests === 0)) && checks.execution) {
    checks.execution = Object.fromEntries(Object.keys(checks.execution).map(key => [key, false]));
  }
  if (observed.manual_reconciliations > 0 && stratum.allow_owner_reconciliation !== true && checks.execution) {
    checks.execution = Object.fromEntries(Object.keys(checks.execution).map(key => [key, false]));
  }
  if ((observed.model_requests > stratum.max_model_steps || observed.compaction_requests > stratum.max_compaction_requests
    || observed.resume_authorizations > stratum.max_resume_authorizations) && checks.execution) {
    checks.execution = Object.fromEntries(Object.keys(checks.execution).map(key => [key, false]));
  }
  const sample = { id: trial.id, session_id: capture.session_id, suite_id: manifest.suite_id,
    runtime_build_digest: manifest.runtime_build_digest, model: manifest.model, stratum: trial.stratum,
    source: 'live_product', grader: 'independent_assertions', duration_ms: capture.duration_ms, checks };
  const body = { schema_version: 1, manifest_sha256: manifestData.sha256, sample,
    capture_sha256: captureData.sha256, grader_status: graderStatus, observations: observed,
    verifier_exit_code: result.status, verifier_direct_process_exit_observed: result.status !== null,
    verifier_timeout_or_signal: Boolean(result.error || result.signal), verifier_descendant_cleanup: 'fixture_owner_responsibility',
    stdout_sha256: sha(result.stdout ?? ''), stderr_sha256: sha(result.stderr ?? ''),
    recorded_at: new Date().toISOString() };
  save(options['--output'], { body, hmac_sha256: signature(body) });
  console.log(`trial_recorded=${trial.id} grader_status=${graderStatus}`);
}

function aggregate(options) {
  const manifest = loadManifest(options);
  const samples = [];
  const directory = realpathSync(resolve(options['--receipts']));
  for (const name of readdirSync(directory).filter(name => name.endsWith('.json')).sort()) {
    const body = verified(read(resolve(directory, name), 4 * 1024 * 1024).value);
    must(body.manifest_sha256 === manifest.sha256 && body.schema_version === 1, 'receipt_from_another_frozen_suite');
    samples.push(body.sample);
  }
  const evidence = { schema_version: 1, suite_id: manifest.value.suite_id,
    runtime_build_digest: manifest.value.runtime_build_digest, model: manifest.value.model,
    strata: manifest.value.strata, scheduled_trials: manifest.value.scheduled_trials, samples };
  const report = summarizeReliability(evidence);
  save(options['--output'], evidence);
  if (options['--report']) save(options['--report'], report);
  console.log(`agent_reliability_status=${report.status} missing_trials=${report.missing_trials}`);
  if (report.status !== 'pass') process.exitCode = 1;
}

try {
  const [command, ...args] = process.argv.slice(2);
  const options = argumentsFor(args);
  if (command === 'freeze') freeze(options);
  else if (command === 'record') record(options);
  else if (command === 'aggregate') aggregate(options);
  else throw new Error('usage_freeze_record_or_aggregate');
} catch (error) {
  // Never print arbitrary capture contents, verifier stdout or secret values.
  const code = error instanceof Error && /^[a-z0-9_]{1,100}$/.test(error.message) ? error.message : 'invalid_or_unavailable';
  console.error(`agent_reliability_collector=${code}`);
  process.exitCode = 2;
}
