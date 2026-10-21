#!/usr/bin/env bun

/**
 * Compile and run an opt-in Nomi-core or native-browser live-provider smoke without exposing
 * its credential to Cargo, build scripts, argv, files, logs, or tool children.
 *
 * The smoke is intentionally pinned to StepFun Coding Plan
 * (`stepfun-plan` / `step-3.7-flash`, or explicit `step-5-preview`) in the Rust fixture.
 * NOMIFUN_LIVE_STEPFUN_MODEL is a non-secret, explicitly selected model;
 * it never enables fallback or changes the official endpoint. The only
 * secret input is NOMIFUN_LIVE_STEPFUN_API_KEY from the environment.
 *
 * Usage:
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --compile-only
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --browser
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --browser --compile-only
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --browser-gui --data-dir C:/new-disposable-gui-data
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --cron-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --model-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --file-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --component-smoke --report .tmp-component-smoke.json
 * The component smoke defaults to step-5-preview and checks a real managed
 * OfficeCLI launch followed by an ordinary turn in the same Session.
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --resource-wait-smoke
 * The resource wait smoke runs an isolated managed Node authorization helper,
 * verifies zero model polling during waiting, then verifies authorization and cancellation.
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --input-queue-smoke
 * Uses step-5-preview to verify queue/steer, exact Stop, durable hold across a
 * complete Host restart, explicit resume and original-key replay fences.
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --input-handoff-smoke
 * The input handoff smoke defaults to step-5-preview. It requires an existing
 * dev component selected by NOMIFUN_OFFICECLI_PATH or an embedded OfficeCLI
 * build; this runner does not download components or discover them on PATH.
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --reasoning-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --reasoning-smoke --reasoning-navigation
 * Navigation also requires NOMIFUN_LIVE_PLAYWRIGHT_MODULE pointing at an existing
 * Playwright ESM entry, and optionally NOMIFUN_LIVE_BROWSER_NODE for its Node host.
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --coding-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --app-hosting-smoke --report .tmp-hosting.json
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --app-hosting-smoke --app-hosting-browser-hold --artifact-dir C:/new-generated-project-evidence
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --app-workspace-ui-fixture
 * The UI fixture boots an isolated production Host with a public test login
 * and a real configured provider. Product validation remains null; explicitly
 * POST its finish URL after independent browser QA to shut it down.
 * Browser hold emits a safe loopback QA callback and waits up to fifteen minutes per stage
 * for independent increment/refresh/clear checks. It does not fabricate clicks.
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --long-coding-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --game-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --general-desktop-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --companion-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --creative-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --idmm-smoke --report .tmp-idmm-demo-report.json
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --before-tool-smoke
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --compaction-smoke
 *   NOMIFUN_LIVE_FIXTURE_PARENT=/absolute/repo/.git/hook-product-validation \
 *     bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --before-tool-smoke --retain-native-fixture
 * Retention is only for this run's native acceptance: app/Node processes still
 * close, and the isolated Provider credential remains encrypted in its store.
 * A cleaned and credential-audited failing run may also retain its fixture;
 * its failure status and exit code remain unchanged.
 *
 * Cargo always runs first with a credential-free environment and emits JSON
 * metadata. The runner resolves the freshly-built test executable from that
 * metadata, then launches it directly. Only that executable receives the
 * credential, once, over stdin; its environment remains credential-free.
 */

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, realpathSync, statSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute, basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { IDMM_DEMO_CASES, parseIdmmDemoCase, parseIdmmDemoEvidence } from './idmm-demo-evidence.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const WINDOWS_TOOLCHAIN_MODULE_URL = pathToFileURL(
  resolve(ROOT, 'scripts/run-dev.mjs'),
).href;
const API_KEY_ENVIRONMENT_NAME = 'NOMIFUN_LIVE_STEPFUN_API_KEY';
const browser = process.argv.includes('--browser');
const browserGui = process.argv.includes('--browser-gui');
const guiDataIndex = process.argv.indexOf('--data-dir');
const guiDataDir = guiDataIndex >= 0 && process.argv[guiDataIndex + 1] ? resolve(process.argv[guiDataIndex + 1]) : null;
const TEST_TARGET = browserGui ? 'browser_gui_fixture' : browser ? 'browser_workspace_smoke' : 'nomi_core_live_provider_smoke';
const TARGET_KIND = browser || browserGui ? 'example' : 'test';
const FIXTURE_PARENT_ENVIRONMENT_NAME = 'NOMIFUN_LIVE_FIXTURE_PARENT';
const RETAIN_FIXTURE_ENVIRONMENT_NAME = 'NOMIFUN_LIVE_RETAIN_NATIVE_FIXTURE';
const NATIVE_FIXTURE_MARKER = 'NOMIFUN_LIVE_SMOKE_NATIVE_FIXTURE ';
const MODEL_ENVIRONMENT_NAME = 'NOMIFUN_LIVE_STEPFUN_MODEL';
const DEFAULT_MODEL = 'step-3.7-flash';
const ALLOWED_MODELS = new Set([DEFAULT_MODEL, 'step-5-preview']);
const CRON_TEST_NAME = 'nomi_core_scheduled_tasks_reach_live_stepfun';
const MODEL_TEST_NAME = 'nomi_core_selected_model_reaches_live_stepfun';
const FILE_TEST_NAME = 'nomi_core_workspace_file_reaches_live_stepfun';
const COMPONENT_TEST_NAME = 'nomi_core_managed_component_reaches_live_stepfun';
const INPUT_HANDOFF_TEST_NAME = 'nomi_core_user_input_handoff_reaches_live_stepfun';
const RESOURCE_WAIT_TEST_NAME = 'nomi_core_resource_wait_reaches_live_stepfun';
const INPUT_QUEUE_TEST_NAME = 'nomi_core_input_queue_reaches_live_stepfun';
const INPUT_QUEUE_PHASES = ['mixed_turns_verified', 'stop_hold_verified', 'restart_hold_verified', 'resume_verified'];
const INPUT_QUEUE_PROOF = 'NOMIFUN_INPUT_QUEUE_LIVE_OK model=step-5-preview queue_turns=2 exact_steer=true stop_hold=true host_restarts=1 resumed_turns=1 idempotency=true cancelled_replay_fenced=true';
const COMPONENT_EVIDENCE = 'NOMIFUN_LIVE_COMPONENT_EVIDENCE ';
const REASONING_TEST_NAME = 'nomi_core_reasoning_lifecycle_reaches_live_stepfun';
const CODING_TEST_NAME = 'nomi_core_official_coding_agent_reaches_live_stepfun';
const LONG_CODING_TEST_NAME = 'nomi_core_long_coding_reaches_live_stepfun';
const GAME_TEST_NAME = 'nomi_core_snake_game_reaches_live_stepfun';
const GENERAL_DESKTOP_TEST_NAME = 'nomi_core_general_desktop_reaches_live_stepfun';
const COMPANION_TEST_NAME = 'nomi_core_official_companion_reaches_live_stepfun';
const CREATIVE_TEST_NAME = 'nomi_core_official_creative_studio_reaches_live_stepfun';
const BEFORE_TOOL_TEST_NAME = 'nomi_core_product_before_tool_reaches_live_stepfun';
const IDMM_TEST_NAME = 'nomi_core_idmm_reaches_live_stepfun';
const APP_HOSTING_TEST_NAME = 'nomi_core_app_generation_and_hosting_reaches_live_stepfun';
const APP_WORKSPACE_UI_TEST_NAME = 'nomi_core_app_workspace_ui_fixture';
const APP_WORKSPACE_UI_READY = 'NOMIFUN_LIVE_APP_WORKSPACE_UI_READY ';
const APP_WORKSPACE_UI_CLOSED = 'NOMIFUN_LIVE_APP_WORKSPACE_UI_CLOSED validation=null';
const APP_HOSTING_READY = 'NOMIFUN_LIVE_APP_HOSTING_READY ';
const APP_HOSTING_EVIDENCE = 'NOMIFUN_LIVE_APP_HOSTING_EVIDENCE ';
const APP_HOSTING_ARTIFACT_ENV = 'NOMIFUN_LIVE_APP_HOSTING_ARTIFACT_DIR';
const APP_HOSTING_HOLD_ENV = 'NOMIFUN_LIVE_APP_HOSTING_BROWSER_HOLD';
const BEFORE_TOOL_STAGE_PHASES = ['before_tool.publish_select', 'before_tool.allow', 'before_tool.deny', 'before_tool.continuation'];
const GLOBAL_TIMEOUT_MS = (process.argv.includes('--long-coding-smoke') ? 75 : process.argv.includes('--app-hosting-smoke') ? 60 : process.argv.includes('--app-workspace-ui-fixture') ? 45 : 30) * 60 * 1000;
const CARGO_OUTPUT_LIMIT_BYTES = 32 * 1024 * 1024;
const TEST_OUTPUT_LIMIT_BYTES = 8 * 1024 * 1024;
const FAILURE_SENTINEL =
  /^NOMIFUN_LIVE_SMOKE_FAILURE phase=([a-z0-9_.-]+) code=([A-Z0-9_]+) status=([0-9]{3})$/;
const compileOnly = process.argv.includes('--compile-only');
const selfTest = process.argv.includes('--self-test');
const cronSmoke = process.argv.includes('--cron-smoke');
const modelSmoke = process.argv.includes('--model-smoke');
const fileSmoke = process.argv.includes('--file-smoke');
const componentSmoke = process.argv.includes('--component-smoke');
const inputHandoffSmoke = process.argv.includes('--input-handoff-smoke');
const resourceWaitSmoke = process.argv.includes('--resource-wait-smoke');
const inputQueueSmoke = process.argv.includes('--input-queue-smoke');
const reasoningSmoke = process.argv.includes('--reasoning-smoke');
const reasoningNavigation = process.argv.includes('--reasoning-navigation');
const codingSmoke = process.argv.includes('--coding-smoke');
const longCodingSmoke = process.argv.includes('--long-coding-smoke');
const gameSmoke = process.argv.includes('--game-smoke');
const generalDesktopSmoke = process.argv.includes('--general-desktop-smoke');
const companionSmoke = process.argv.includes('--companion-smoke');
const creativeSmoke = process.argv.includes('--creative-smoke');
const beforeToolSmoke = process.argv.includes('--before-tool-smoke');
const idmmSmoke = process.argv.includes('--idmm-smoke');
const appHostingSmoke = process.argv.includes('--app-hosting-smoke');
const appWorkspaceUiFixture = process.argv.includes('--app-workspace-ui-fixture');
const appHostingBrowserHold = process.argv.includes('--app-hosting-browser-hold');
const artifactIndex = process.argv.indexOf('--artifact-dir');
const artifactDir = artifactIndex >= 0 && process.argv[artifactIndex + 1] ? resolve(process.argv[artifactIndex + 1]) : null;
const reportIndex = process.argv.indexOf('--report');
const reportPath = reportIndex >= 0 && process.argv[reportIndex + 1] ? resolve(process.argv[reportIndex + 1]) : null;
const selectedSmokeTest = cronSmoke ? CRON_TEST_NAME : appWorkspaceUiFixture ? APP_WORKSPACE_UI_TEST_NAME : appHostingSmoke ? APP_HOSTING_TEST_NAME : reasoningSmoke ? REASONING_TEST_NAME : idmmSmoke ? IDMM_TEST_NAME : beforeToolSmoke ? BEFORE_TOOL_TEST_NAME : fileSmoke ? FILE_TEST_NAME : componentSmoke ? COMPONENT_TEST_NAME : inputQueueSmoke ? INPUT_QUEUE_TEST_NAME : resourceWaitSmoke ? RESOURCE_WAIT_TEST_NAME : inputHandoffSmoke ? INPUT_HANDOFF_TEST_NAME : codingSmoke ? CODING_TEST_NAME : longCodingSmoke ? LONG_CODING_TEST_NAME : gameSmoke ? GAME_TEST_NAME : generalDesktopSmoke ? GENERAL_DESKTOP_TEST_NAME : companionSmoke ? COMPANION_TEST_NAME : creativeSmoke ? CREATIVE_TEST_NAME : MODEL_TEST_NAME;
const selectedSmokeMode = cronSmoke ? 'scheduled_tasks' : appWorkspaceUiFixture ? 'app_workspace_ui_host' : appHostingSmoke ? 'app_workspace_generation' : reasoningSmoke ? 'reasoning_lifecycle' : idmmSmoke ? 'idmm' : beforeToolSmoke ? 'before_tool' : fileSmoke ? 'workspace_file' : componentSmoke ? 'managed_component' : inputQueueSmoke ? 'input_queue' : resourceWaitSmoke ? 'resource_wait' : inputHandoffSmoke ? 'user_input_handoff' : codingSmoke ? 'coding_agent' : longCodingSmoke ? 'long_coding' : gameSmoke ? 'snake_game' : generalDesktopSmoke ? 'general_desktop' : companionSmoke ? 'companion' : creativeSmoke ? 'creative_studio' : 'selected_model';
const retainNativeFixture = process.argv.includes('--retain-native-fixture');
const globalDeadline = Date.now() + GLOBAL_TIMEOUT_MS;

function selectedSmokeModel(value) {
  return value ?? (cronSmoke || componentSmoke || inputHandoffSmoke || resourceWaitSmoke || inputQueueSmoke ? 'step-5-preview' : DEFAULT_MODEL);
}

function cargoFailureCode(stderr) {
  return stderr.includes('rust-lld: error: failed to write output') && /permission denied/i.test(stderr)
    ? 'CARGO_LINK_OUTPUT_UNAVAILABLE'
    : 'CARGO_BUILD_FAILED';
}

function isCredentialEnvironmentName(name) {
  return name.toUpperCase() === API_KEY_ENVIRONMENT_NAME;
}

function environmentWithoutCredential(source) {
  const environment = {};
  // Enumerate names before values so the credential value is never copied
  // while constructing an environment for Cargo, vcvars, or the test process.
  for (const name of Object.keys(source)) {
    if (isCredentialEnvironmentName(name)) continue;
    const value = source[name];
    if (typeof value === 'string') environment[name] = value;
  }
  return environment;
}

function removeCredentialFromCurrentEnvironment() {
  for (const name of Object.keys(process.env)) {
    if (isCredentialEnvironmentName(name)) delete process.env[name];
  }
}

function emitFailure(scope, code, status) {
  console.error(`${scope} code=${code} status=${status}`);
}

function terminateProcessTree(child, environment) {
  if (!child.pid) return;
  if (process.platform === 'win32') {
    spawnSync(
      'taskkill.exe',
      ['/pid', String(child.pid), '/t', '/f'],
      {
        env: environmentWithoutCredential(environment),
        stdio: 'ignore',
        timeout: 5000,
        windowsHide: true,
      },
    );
    return;
  }
  try {
    process.kill(-child.pid, 'SIGKILL');
  } catch {
    try {
      child.kill('SIGKILL');
    } catch {
      // The process may already have exited between the timeout and cleanup.
    }
  }
}

function runCaptured(
  command,
  args,
  { environment, input = null, outputLimitBytes, onStdoutLine = null, onStderrLine = null, timeoutMs = null },
) {
  const remainingMs = Math.min(globalDeadline - Date.now(), timeoutMs ?? Infinity);
  if (remainingMs <= 0) {
    return Promise.resolve({
      status: null,
      stdout: '',
      stderr: '',
      timedOut: true,
      overflowed: false,
      spawnError: false,
    });
  }

  return new Promise((complete) => {
    let settled = false;
    let timer = null;
    let bytes = 0;
    const stdout = [];
    const stderr = [];
    let pendingLine = '';
    let pendingErrorLine = '';
    const child = spawn(command, args, {
      cwd: ROOT,
      env: environmentWithoutCredential(environment),
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
      detached: process.platform !== 'win32',
    });

    const finish = (result) => {
      if (settled) return;
      settled = true;
      if (timer !== null) clearTimeout(timer);
      complete({
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
        timedOut: false,
        overflowed: false,
        spawnError: false,
        ...result,
      });
    };

    const stop = (result) => {
      terminateProcessTree(child, environment);
      child.stdin?.destroy();
      child.stdout?.destroy();
      child.stderr?.destroy();
      child.unref();
      finish(result);
    };

    const collect = (target) => (chunk) => {
      if (settled) return;
      const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      bytes += buffer.length;
      if (bytes > outputLimitBytes) {
        stop({ status: null, overflowed: true });
        return;
      }
      target.push(buffer);
    };

    child.stdout.on('data', chunk => {
      collect(stdout)(chunk);
      if (onStdoutLine && !settled) {
        pendingLine += chunk.toString('utf8');
        const lines = pendingLine.split(/\r?\n/);
        pendingLine = lines.pop() ?? '';
        if (pendingLine.length > 16000) pendingLine = '';
        for (const line of lines) if (line.length < 16000) onStdoutLine(line);
      }
    });
    child.stderr.on('data', chunk => {
      collect(stderr)(chunk);
      if (onStderrLine && !settled) {
        pendingErrorLine += chunk.toString('utf8');
        const lines = pendingErrorLine.split(/\r?\n/);
        pendingErrorLine = lines.pop() ?? '';
        if (pendingErrorLine.length > 24000) pendingErrorLine = '';
        for (const line of lines) if (line.length < 24000) onStderrLine(line);
      }
    });
    child.once('error', () => finish({ status: null, spawnError: true }));
    child.once('close', (status, signal) => finish({ status, signal }));

    child.stdin.on('error', () => {
      // A fast child failure may close stdin before the one-shot write lands.
      // The exit status and safe sentinel remain the authoritative result.
    });
    if (input === null) child.stdin.end();
    else child.stdin.end(input);

    timer = setTimeout(
      () => stop({ status: null, timedOut: true }),
      remainingMs,
    );
  });
}

function testExecutableFromCargoJson(stdout) {
  let executable = null;
  for (const line of stdout.split(/\r?\n/)) {
    if (!line.startsWith('{')) continue;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }
    if (
      message.reason === 'compiler-artifact' &&
      message.target?.name === TEST_TARGET &&
      Array.isArray(message.target?.kind) &&
      message.target.kind.includes(TARGET_KIND) &&
      typeof message.executable === 'string'
    ) {
      executable = message.executable;
    }
  }
  return executable;
}

function browserEvidenceFromOutput(output) {
  const prefix = 'NOMIFUN_BROWSER_LIVE_EVIDENCE ';
  const line = output.split(/\r?\n/).find(line => line.startsWith(prefix) && line.length < 12000);
  if (!line) return null;
  try {
    const data = JSON.parse(line.slice(prefix.length));
    const smallNumber = value => Number.isInteger(value) && value >= -100 && value <= 100 ? value : null;
    const choice = (value, allowed, fallback = null) => allowed.includes(value) ? value : fallback;
    return {
      count: smallNumber(data.count),
      values: Array.isArray(data.values) ? data.values.slice(0, 128).map(smallNumber) : [],
      served_versions: smallNumber(data.served_versions),
      source_changed: data.source_changed === true,
      presented: data.presented === true,
      presentation_failed: data.presentation_failed === true,
      tools: Array.isArray(data.tools) ? data.tools.slice(0, 100).map(tool => ({
        name: choice(tool.name, ['Browser', 'Read', 'Write', 'Edit', 'apply_patch', 'update_plan', 'AskUserQuestion'], 'OTHER'),
        status: choice(tool.status, ['completed', 'failed'], 'other'),
        operation: choice(tool.operation, ['navigate', 'observe', 'act', 'tab', 'diagnostics', 'screenshot']),
        action: choice(tool.action, ['click', 'type', 'press', 'scroll', 'hover']),
        error_present: tool.error_present === true,
        result_error: typeof tool.result_error === 'boolean' ? tool.result_error : null,
        error_codes: Array.isArray(tool.error_codes) ? tool.error_codes.filter(code => ['INVALID_PAYLOAD', 'CAPABILITY_NOT_SELECTED', 'BROWSER_STALE_OBSERVATION', 'BROWSER_STALE_TARGET', 'BROWSER_NOT_ACTIONABLE', 'BROWSER_NATIVE_COMMAND_FAILED', 'BROWSER_UNSUPPORTED_ACTION', 'TOOL_NOT_FOUND', 'PERMISSION_DENIED'].includes(code)).slice(0, 9) : [],
        error_hints: Array.isArray(tool.error_hints) ? tool.error_hints.filter(hint => ['one_of', 'required', 'additional_property', 'element', 'target', 'observation_generation', 'ref_id', 'operation', 'action'].includes(hint)).slice(0, 9) : [],
      })) : [],
      tool_shapes: Array.isArray(data.tool_shapes) ? data.tool_shapes.slice(0, 32).map(shape => ({
        class: choice(shape.class, ['non_object', 'legacy_operation', 'missing_action_wrapper', 'nested_action_wrapper', 'private_attached', 'legacy_reference', 'raw_reference', 'canonical_element', 'partial_observed_element', 'other_element_object', 'scalar_element', 'other_canonical_variant', 'missing_reference'], 'other'),
        tag: choice(shape.tag, ['click', 'hover', 'type', 'press', 'select', 'scroll', 'drag', 'dialog']),
        outer_unknown_keys: smallNumber(shape.outer_unknown_keys),
        action_unknown_keys: smallNumber(shape.action_unknown_keys),
      })) : [],
    };
  } catch { return null; }
}
function browserProofFromOutput(output) {
  const prefix = 'BROWSER_WORKSPACE_SMOKE_PASS ';
  const line = output.split(/\r?\n/).find(line => line.startsWith(prefix) && line.length < 16000);
  try {
    const proof = JSON.parse(line?.slice(prefix.length) ?? 'null');
    return proof?.scope === 'live-agent-only' && proof.native_fixture_profile_cleanup === true && proof.agent?.real_provider === true && proof.agent?.native_auto_open === true && proof.agent?.trusted_click_value === 2 && proof.agent?.canonical_action_shape === true && proof.agent?.evidence_driven_cancel === true && proof.agent?.terminal_before_unlock === true;
  } catch { return false; }
}
function typedFailureFromOutput(output) {
  for (const line of output.split(/\r?\n/)) {
    const browserFailure = browser && line.match(/^BROWSER_WORKSPACE_SMOKE_FAIL (LIVE_[A-Z0-9_]+)$/);
    if (browserFailure) return { phase: 'browser.frontend', code: browserFailure[1], status: '422' };
    const match = line.match(FAILURE_SENTINEL);
    if (match) {
      return { phase: match[1], code: match[2], status: match[3] };
    }
  }
  return null;
}

function selectedTestPassed(stdout, selected) {
  return stdout.split(/\r?\n/).some((line) => line === `test ${selected} ... ok`);
}

function pathIsInside(parent, child) {
  const suffix = relative(parent, child);
  return suffix.length > 0 && suffix !== '..' && !suffix.startsWith('../') &&
    !suffix.startsWith('..\\') && !isAbsolute(suffix);
}

function canonicalDirectory(value) {
  if (typeof value !== 'string' || value.length === 0 || !isAbsolute(value) || /[\x00-\x1f\x7f]/.test(value)) {
    throw new Error('invalid fixture path');
  }
  const canonical = realpathSync(value);
  if (!statSync(canonical).isDirectory()) throw new Error('invalid fixture directory');
  return canonical;
}

function validateFixtureParent(value) {
  const gitRoot = canonicalDirectory(resolve(ROOT, '.git'));
  const parent = canonicalDirectory(value);
  if (!pathIsInside(gitRoot, parent)) throw new Error('fixture parent outside repository git directory');
  return parent;
}

function retainedFixtureFromOutput(output, parent) {
  const lines = output.split(/\r?\n/).filter((line) => line.startsWith(NATIVE_FIXTURE_MARKER));
  if (lines.length !== 1) throw new Error('missing or duplicate fixture evidence');
  const payload = JSON.parse(lines[0].slice(NATIVE_FIXTURE_MARKER.length));
  if (!payload || Array.isArray(payload) || typeof payload !== 'object' ||
      Object.keys(payload).sort().join(',') !== 'data_root,fixture_root,work_root') {
    throw new Error('invalid fixture evidence');
  }
  const fixtureRoot = canonicalDirectory(payload.fixture_root);
  const dataRoot = canonicalDirectory(payload.data_root);
  const workRoot = canonicalDirectory(payload.work_root);
  if (dirname(fixtureRoot) !== parent || !basename(fixtureRoot).startsWith('before-tool-native-') ||
      !pathIsInside(parent, fixtureRoot) || !pathIsInside(fixtureRoot, dataRoot) || !pathIsInside(fixtureRoot, workRoot) ||
      dataRoot !== resolve(fixtureRoot, 'data') || workRoot !== resolve(fixtureRoot, 'work')) {
    throw new Error('fixture evidence is outside the exact isolated root');
  }
  return { fixture_root: fixtureRoot, data_root: dataRoot, work_root: workRoot };
}

function beforeToolStagesFromOutput(stdout) {
  const stages = [];
  for (const line of stdout.split(/\r?\n/)) {
    const match = line.match(/^NOMIFUN_LIVE_SMOKE_STAGE phase=(before_tool\.(?:publish_select|allow|deny|continuation)) status=pass$/);
    if (match) stages.push(match[1]);
  }
  return stages;
}

function inputQueueEvidence(output) {
  const lines = output.split(/\r?\n/);
  const phases = lines.filter(line => line.startsWith('NOMIFUN_INPUT_QUEUE_PHASE '));
  if (phases.length !== INPUT_QUEUE_PHASES.length || INPUT_QUEUE_PHASES.some((phase, index) => phases[index] !== `NOMIFUN_INPUT_QUEUE_PHASE phase=${phase}`)
      || lines.filter(line => line.startsWith('NOMIFUN_INPUT_QUEUE_LIVE_OK')).length !== 1
      || !lines.includes(INPUT_QUEUE_PROOF)) throw new Error('input queue lifecycle evidence incomplete');
  return {schema_version:1,provider:'stepfun-plan',model:'step-5-preview',queue_turns:2,exact_steer:true,
    stop_hold:true,host_restarts:1,resumed_turns:1,idempotency:true,cancelled_replay_fenced:true};
}

function loopbackUrl(value, expectedPath = null) {
  if (typeof value !== 'string' || value.length > 180 || /[\r\n]/.test(value)) return false;
  try {
    const url = new URL(value);
    return url.protocol === 'http:' && url.hostname === '127.0.0.1' && /^[0-9]{1,5}$/.test(url.port) &&
      Number(url.port) > 0 && Number(url.port) <= 65535 && !url.username && !url.password && !url.search && !url.hash &&
      (expectedPath === null || url.pathname === expectedPath);
  } catch { return false; }
}

function appHostingReady(line) {
  if (!line.startsWith(APP_HOSTING_READY) || /[\r\n]/.test(line)) return null;
  try {
    const raw = JSON.parse(line.slice(APP_HOSTING_READY.length));
    const id = /^[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
    if (!loopbackUrl(raw.app_url) || !loopbackUrl(raw.host_url, '/') ||
        !loopbackUrl(raw.qa_callback, '/__app-hosting-smoke/ui-state') ||
        !raw.qa_callback.startsWith(raw.host_url + '/') || !id.test(raw.session_id) || !id.test(raw.app_id)) return null;
    return {app_url:raw.app_url, host_url:raw.host_url, session_id:raw.session_id, app_id:raw.app_id, qa_callback:raw.qa_callback,
      ...(['generated','repaired'].includes(raw.stage) ? {stage:raw.stage} : {}),
      ...(raw.web_test_username === 'app-workspace-smoke' ? {web_test_username:raw.web_test_username} : {})};
  } catch { return null; }
}

function appWorkspaceUiReady(line) {
  if (!line.startsWith(APP_WORKSPACE_UI_READY) || /[\r\n]/.test(line)) return null;
  try {
    const raw = JSON.parse(line.slice(APP_WORKSPACE_UI_READY.length));
    if (!loopbackUrl(raw.host_url, '/') || !loopbackUrl(raw.finish_url, '/__app-workspace-fixture/finish') ||
        !raw.finish_url.startsWith(raw.host_url + '/') || raw.web_test_username !== 'app-workspace-smoke' ||
        !ALLOWED_MODELS.has(raw.model) || raw.validation !== null || typeof raw.provider_id !== 'string' ||
        !/^[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(raw.provider_id)) return null;
    return {host_url:raw.host_url,finish_url:raw.finish_url,web_test_username:raw.web_test_username,
      provider_id:raw.provider_id,model:raw.model,validation:null};
  } catch { return null; }
}

function appHostingEvidence(output) {
  const lines = output.split(/\r?\n/).filter(line => line.startsWith(APP_HOSTING_EVIDENCE));
  if (lines.length !== 1) throw new Error('missing hosting evidence');
  const raw = JSON.parse(lines[0].slice(APP_HOSTING_EVIDENCE.length));
  if (raw.schema_version !== 3 || raw.provider !== 'stepfun-plan' || !ALLOWED_MODELS.has(raw.model) ||
      raw.template !== 'app.developer' || raw.runtime_terminal !== 'turn/completed' || !loopbackUrl(raw.app_url) ||
      raw.default_hosting_grant !== true || raw.fixed_app_agent !== true || raw.manual_create_before_model_configuration !== true ||
      raw.ordinary_session !== true || raw.current_workspace_is_managed_directory !== true || raw.application_count !== 1 || raw.application_id_preserved !== true || typeof raw.preview !== 'boolean' ||
      typeof raw.current_workspace !== 'string' || !isAbsolute(raw.current_workspace) || raw.current_workspace.length > 4096 || /[\x00-\x1f\x7f]/.test(raw.current_workspace) ||
      raw.external_unauthenticated_status !== 403 || raw.connection?.connected !== true || raw.connection?.app_count !== 1 ||
      !Number.isSafeInteger(raw.verified_host_requests) || raw.verified_host_requests < 1 || raw.verified_host_requests > 100 ||
      raw.stop?.returned_state !== 'stopped' || raw.stop?.returned_pid !== null || raw.stop?.root_pid_gone !== true ||
      raw.stop?.direct_tcp_connect !== false || raw.stop?.no_proxy !== true || raw.stop?.fresh_http_connection !== true || raw.stop?.after_stop_response !== null ||
      raw.script_syntax_checked !== true || raw.http_status !== 200 ||
      raw.http_bytes_equal !== true || raw.stop_verified !== true || !/^[a-f0-9]{64}$/.test(raw.source_sha256) ||
      !/^[a-f0-9]{64}$/.test(raw.backend_sha256) ||
      !Number.isSafeInteger(raw.source_bytes) || raw.source_bytes < 100 || raw.source_bytes > 8*1024*1024 ||
      !Array.isArray(raw.settlements) || raw.settlements.length > 128) throw new Error('invalid hosting evidence');
  const projectSettlement = item => {
    if (!['workspace.files/write','workspace.files/patch','app.hosting/import','app.hosting/update','app.hosting/preview','app.hosting/rebuild','app.hosting/start','app.hosting/stop','app.hosting/status','app.hosting/list','app.hosting/current','app.hosting/guide'].includes(item.action_id) ||
        typeof item.operation_id !== 'string' || item.operation_id.length > 180 || /[\r\n]/.test(item.operation_id) ||
        !Number.isSafeInteger(item.dispatch_seq) || !Number.isSafeInteger(item.result_seq) || item.result_seq <= item.dispatch_seq ||
        typeof item.result_success !== 'boolean' || !/^[a-f0-9]{64}$/.test(item.result_sha256) ||
        (item.effect_succeeded_seq !== null && (!Number.isSafeInteger(item.effect_succeeded_seq) ||
          item.effect_succeeded_seq <= item.dispatch_seq || item.effect_succeeded_seq >= item.result_seq))) throw new Error('invalid settlement');
    return {action_id:item.action_id, operation_id:item.operation_id, dispatch_seq:item.dispatch_seq,result_seq:item.result_seq,
      result_success:item.result_success,result_sha256:item.result_sha256,effect_succeeded_seq:item.effect_succeeded_seq,app_id:item.app_id ?? null};
  };
  const settlements = raw.settlements.map(projectSettlement);
  for (const action of ['workspace.files/write','app.hosting/update']) {
    if (!settlements.some(item => item.action_id === action && item.result_success && item.effect_succeeded_seq !== null)) throw new Error('missing exact settlement');
  }
  if (!settlements.some(item => ['app.hosting/preview','app.hosting/start'].includes(item.action_id) && item.result_success && item.effect_succeeded_seq !== null && item.app_id === raw.app_id)) throw new Error('missing actual run settlement');
  if (!settlements.some(item => item.action_id === 'app.hosting/current' && item.result_success && item.app_id === raw.app_id)) throw new Error('current application was not identified');
  if (!settlements.some(item => item.action_id === 'app.hosting/update' && item.result_success && item.app_id === raw.app_id) ||
      settlements.some(item => item.action_id === 'app.hosting/import')) throw new Error('application workspace was duplicated or not updated');
  if (!settlements.some(item => item.action_id === 'app.hosting/guide' && item.result_success)) throw new Error('guide was not read');
  if (raw.browser_qa !== null && (!raw.browser_qa || raw.browser_qa.independent_callback !== true ||
      raw.browser_qa.increment !== true || raw.browser_qa.refresh_persisted !== true || raw.browser_qa.clear !== true || raw.browser_qa.connected !== true)) throw new Error('invalid browser callback');
  const repair = raw.repair;
  if (!repair || repair.runtime_terminal !== 'turn/completed' || repair.app_id_preserved !== true || repair.session_id_preserved !== true ||
      repair.prior_history_unchanged !== true || repair.source_changed !== true || repair.original_pid_gone !== true ||
      repair.upstream_failure?.injected_http_status !== 503 || repair.upstream_failure?.app_http_status !== 503 ||
      repair.upstream_failure?.body?.connected !== false || repair.upstream_failure?.body?.app_count !== 0 ||
      repair.upstream_failure?.recovered_success !== true || !Number.isSafeInteger(repair.upstream_failure?.verified_injected_requests) ||
      repair.upstream_failure.verified_injected_requests < 1 || !Number.isSafeInteger(repair.upstream_failure?.recovered_verified_requests) ||
      repair.upstream_failure.recovered_verified_requests < 1 || !Array.isArray(repair.settlements) || repair.settlements.length > 128 ||
      repair.managed_pid !== raw.managed_pid || repair.managed_pid !== raw.stop.observed_root_pid ||
      !Array.isArray(raw.turns) || raw.turns.length !== 2 || raw.turns[0].operation_id !== raw.operation_id ||
      raw.turns[1].operation_id !== repair.operation_id || raw.turns[0].operation_id === raw.turns[1].operation_id ||
      raw.turns.some(turn => turn.runtime_terminal !== 'turn/completed')) throw new Error('invalid application repair evidence');
  const repairSettlements = repair.settlements.map(projectSettlement);
  const stop = repairSettlements.find(item => item.action_id === 'app.hosting/stop' && item.result_success && item.effect_succeeded_seq !== null && item.app_id === raw.app_id);
  const write = repairSettlements.find(item => ['workspace.files/write','workspace.files/patch'].includes(item.action_id) && item.result_success && item.effect_succeeded_seq !== null && item.dispatch_seq > stop?.result_seq);
  const run = repairSettlements.find(item => ['app.hosting/preview','app.hosting/rebuild','app.hosting/start'].includes(item.action_id) && item.result_success && item.effect_succeeded_seq !== null && item.app_id === raw.app_id && item.dispatch_seq > write?.result_seq);
  if (!stop || !write || !run || repairSettlements.some(item => item.action_id === 'app.hosting/import')) throw new Error('repair owner order is not proven');
  if (repair.browser_qa !== null && (!repair.browser_qa || repair.browser_qa.independent_callback !== true || repair.browser_qa.increment !== true ||
      repair.browser_qa.refresh_persisted !== true || repair.browser_qa.clear !== true || repair.browser_qa.connected !== true)) throw new Error('invalid repair browser callback');
  // Explicit projection: never forward unrecognized child fields or outputs.
  return {schema_version:3, provider:raw.provider,model:raw.model,template:raw.template,default_hosting_grant:true,
    fixed_app_agent:true,manual_create_before_model_configuration:true,ordinary_session:true,current_workspace:raw.current_workspace,
    current_workspace_is_managed_directory:true,application_count:1,application_id_preserved:true,session_id:raw.session_id,
    operation_id:raw.operation_id,app_id:raw.app_id,app_name:typeof raw.app_name === 'string' ? raw.app_name.slice(0,160) : null,
    app_url:raw.app_url,managed_pid:raw.managed_pid,preview:raw.preview,runtime_terminal:raw.runtime_terminal,settlements,
    source_sha256:raw.source_sha256,source_bytes:raw.source_bytes,script_syntax_checked:true,
    backend_sha256:raw.backend_sha256,external_unauthenticated_status:403,connection:{connected:true,app_count:1},verified_host_requests:raw.verified_host_requests,
    stop:{returned_state:'stopped',returned_pid:null,observed_root_pid:raw.stop.observed_root_pid,root_pid_gone:true,
      direct_tcp_connect:false,no_proxy:true,fresh_http_connection:true,after_stop_response:null},
    http_status:200,http_bytes_equal:true,stop_verified:true,browser_qa:raw.browser_qa === null ? null : {
      independent_callback:true,increment:true,refresh_persisted:true,clear:true,connected:true},
    turns:[{operation_id:raw.operation_id,runtime_terminal:'turn/completed',settlements},
      {operation_id:repair.operation_id,runtime_terminal:'turn/completed',settlements:repairSettlements}],
    repair:{operation_id:repair.operation_id,runtime_terminal:'turn/completed',app_id_preserved:true,session_id_preserved:true,
      prior_history_unchanged:true,source_changed:true,original_pid_gone:true,managed_pid:repair.managed_pid,settlements:repairSettlements,
      upstream_failure:{injected_http_status:503,app_http_status:503,body:{connected:false,app_count:0},recovered_success:true,
        verified_injected_requests:repair.upstream_failure.verified_injected_requests,recovered_verified_requests:repair.upstream_failure.recovered_verified_requests},browser_qa:repair.browser_qa === null ? null : {
          independent_callback:true,increment:true,refresh_persisted:true,clear:true,connected:true}},
    generated_project:typeof raw.generated_project === 'string' ? raw.generated_project : null,
    repaired_project:typeof raw.repaired_project === 'string' ? raw.repaired_project : null};
}

async function resolveToolchainEnvironment() {
  const credentialFreeInput = environmentWithoutCredential(process.env);
  if (process.platform !== 'win32') {
    return { environment: credentialFreeInput };
  }
  const source = `
    import { loadWindowsToolchainEnvironment } from ${JSON.stringify(WINDOWS_TOOLCHAIN_MODULE_URL)};
    const environment = loadWindowsToolchainEnvironment(process.env, process.platform);
    for (const name of Object.keys(environment)) {
      if (name.toUpperCase() === ${JSON.stringify(API_KEY_ENVIRONMENT_NAME)}) {
        delete environment[name];
      }
    }
    process.stdout.write(JSON.stringify(environment));
  `;
  const probe = await runCaptured(
    process.execPath,
    ['--eval', source],
    {
      environment: credentialFreeInput,
      outputLimitBytes: 2 * 1024 * 1024,
    },
  );
  if (probe.timedOut) return { failure: 'RUNNER_GLOBAL_TIMEOUT' };
  if (probe.overflowed) return { failure: 'WINDOWS_TOOLCHAIN_OUTPUT_LIMIT_EXCEEDED' };
  if (probe.spawnError || probe.status !== 0) {
    return { failure: 'WINDOWS_TOOLCHAIN_UNAVAILABLE' };
  }
  try {
    const parsed = JSON.parse(probe.stdout);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { failure: 'WINDOWS_TOOLCHAIN_UNAVAILABLE' };
    }
    return { environment: environmentWithoutCredential(parsed) };
  } catch {
    return { failure: 'WINDOWS_TOOLCHAIN_UNAVAILABLE' };
  }
}

function managedComponentEvidence(stderr) {
  const lines = stderr.split(/\r?\n/).filter(line => line.startsWith(COMPONENT_EVIDENCE));
  if (lines.length !== 1) throw new Error('component evidence missing or duplicated');
  const value = JSON.parse(lines[0].slice(COMPONENT_EVIDENCE.length));
  if (value.schema_version !== 1 || value.provider !== 'stepfun-plan' || value.model !== 'step-5-preview' ||
      value.component !== 'officecli' || value.action_id !== 'workspace.process/start' || value.source !== 'managed' ||
      typeof value.version !== 'string' || value.version.length > 64 || !/^[0-9]+\.[0-9]+\.[0-9]+(?:[+-][0-9A-Za-z.+-]+)?$/.test(value.version) ||
      !/^[a-f0-9]{64}$/.test(value.digest) || value.canonical_component_argument !== true || value.exit_code !== 0 ||
      value.cleanup_reaped !== true || value.completed_turns !== 2 || value.ordinary_continuation_no_tools !== true ||
      value.same_session_binding !== true || !Number.isSafeInteger(value.call_started_seq) || value.call_started_seq <= 0 ||
      !Number.isSafeInteger(value.result_seq) || value.result_seq <= value.call_started_seq) {
    throw new Error('component evidence invalid');
  }
  return {schema_version:1,provider:'stepfun-plan',model:'step-5-preview',component:'officecli',
    action_id:'workspace.process/start',version:value.version,digest:value.digest,source:'managed',
    canonical_component_argument:true,exit_code:0,cleanup_reaped:true,completed_turns:2,
    ordinary_continuation_no_tools:true,same_session_binding:true,
    call_started_seq:value.call_started_seq,result_seq:value.result_seq};
}

async function main() {
  const userArgs = process.argv.slice(2);
  const allowedFlags = ['--compile-only', '--self-test', '--browser', '--browser-gui', '--cron-smoke', '--model-smoke', '--file-smoke', '--component-smoke', '--input-handoff-smoke', '--resource-wait-smoke', '--input-queue-smoke', '--reasoning-smoke', '--reasoning-navigation', '--coding-smoke', '--long-coding-smoke', '--game-smoke', '--general-desktop-smoke', '--companion-smoke', '--creative-smoke', '--before-tool-smoke', '--idmm-smoke', '--retain-native-fixture', '--app-hosting-smoke', '--app-hosting-browser-hold', '--app-workspace-ui-fixture'];
  if (userArgs.some((arg, index) => {
    if (arg === '--report') return (!idmmSmoke && !appHostingSmoke && !componentSmoke && !inputQueueSmoke) || !userArgs[index + 1] || userArgs[index + 1].startsWith('--');
    if (index > 0 && userArgs[index - 1] === '--report') return false;
    if (arg === '--artifact-dir') return !appHostingSmoke || !userArgs[index + 1] || userArgs[index + 1].startsWith('--') || !isAbsolute(userArgs[index + 1]);
    if (index > 0 && userArgs[index - 1] === '--artifact-dir') return false;
    if (arg === '--data-dir') return !browserGui || !userArgs[index + 1] || userArgs[index + 1].startsWith('--');
    if (index > 0 && userArgs[index - 1] === '--data-dir') return false;
    return !allowedFlags.includes(arg);
  }) || userArgs.filter(arg => arg === '--data-dir').length > 1 || userArgs.filter(arg => arg === '--report').length > 1 || userArgs.filter(arg => arg === '--artifact-dir').length > 1 ||
      (artifactDir && (existsSync(artifactDir) || !existsSync(dirname(artifactDir)) || !statSync(dirname(artifactDir)).isDirectory())) ||
      (reportPath && (!reportPath.endsWith('.json') || !existsSync(dirname(reportPath))))) {
    emitFailure('live_smoke_status=not_run', 'RUNNER_ARGUMENT_INVALID', 400);
    process.exitCode = 2;
    return;
  }
  if (([browser, browserGui, cronSmoke, modelSmoke, fileSmoke, componentSmoke, inputHandoffSmoke, resourceWaitSmoke, inputQueueSmoke, reasoningSmoke, codingSmoke, longCodingSmoke, gameSmoke, generalDesktopSmoke, companionSmoke, creativeSmoke, beforeToolSmoke, idmmSmoke, appHostingSmoke, appWorkspaceUiFixture].filter(Boolean).length > 1) || (retainNativeFixture && !beforeToolSmoke) || (reasoningNavigation && !reasoningSmoke) || (appHostingBrowserHold && !appHostingSmoke)) {
    emitFailure('live_smoke_status=not_run', 'RUNNER_MODE_SELECTION_INVALID', 400);
    process.exitCode = 2;
    return;
  }
  let fixtureParent = null;
  if (retainNativeFixture) {
    try { fixtureParent = validateFixtureParent(process.env[FIXTURE_PARENT_ENVIRONMENT_NAME]); }
    catch {
      emitFailure('live_smoke_status=not_run', 'NATIVE_FIXTURE_PARENT_INVALID', 400);
      process.exitCode = 2;
      return;
    }
  }
  const model = selectedSmokeModel(process.env[MODEL_ENVIRONMENT_NAME]);
  // The separate native-browser fixture still pins its own provider model.
  // Never label that existing acceptance path as a different selected model.
  if (!ALLOWED_MODELS.has(model) || (browser && model !== DEFAULT_MODEL) || ((componentSmoke || cronSmoke || inputQueueSmoke) && model !== 'step-5-preview')) {
    emitFailure('live_smoke_status=not_run', 'LIVE_MODEL_INVALID', 400);
    process.exitCode = 2;
    return;
  }
  if (browserGui && (!guiDataDir || existsSync(guiDataDir))) {
    emitFailure('browser_gui_status=not_run', 'NEW_DATA_DIRECTORY_REQUIRED', 412);
    process.exitCode = 2;
    return;
  }
  const toolchain = await resolveToolchainEnvironment();
  if (!toolchain.environment) {
    emitFailure(
      compileOnly ? 'live_smoke_compile_status=not_run' : 'live_smoke_status=not_run',
      toolchain.failure,
      toolchain.failure === 'RUNNER_GLOBAL_TIMEOUT' ? 504 : 503,
    );
    process.exitCode = 2;
    return;
  }
  const environment = toolchain.environment;
  environment[MODEL_ENVIRONMENT_NAME] = model;
  // Only the explicit CLI mode can request retention; inherited test variables
  // must not silently change the normal cleanup contract.
  for (const name of Object.keys(environment)) {
    if ([RETAIN_FIXTURE_ENVIRONMENT_NAME, FIXTURE_PARENT_ENVIRONMENT_NAME, 'NOMIFUN_LIVE_REASONING_NAVIGATION', APP_HOSTING_ARTIFACT_ENV, APP_HOSTING_HOLD_ENV].includes(name.toUpperCase())) delete environment[name];
  }
  if (retainNativeFixture) {
    environment[RETAIN_FIXTURE_ENVIRONMENT_NAME] = '1';
    environment[FIXTURE_PARENT_ENVIRONMENT_NAME] = fixtureParent;
  }
  if (reasoningNavigation) environment.NOMIFUN_LIVE_REASONING_NAVIGATION = '1';
  if (appHostingSmoke && artifactDir) environment[APP_HOSTING_ARTIFACT_ENV] = artifactDir;
  if (appHostingBrowserHold) environment[APP_HOSTING_HOLD_ENV] = '1';

  environment.CARGO_TERM_COLOR = 'never';
  environment.RUST_BACKTRACE = '0';
  const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
  console.log('live_smoke_phase=compile');
  const compile = await runCaptured(
    cargo,
    browserGui ? ['build', '--locked', '-p', 'nomifun-app', '--example', TEST_TARGET, '--features', 'browser-use', '--message-format=json-render-diagnostics'] : browser ? ['build', '--locked', '-p', 'nomifun-desktop', '--example', TEST_TARGET, '--no-default-features', '--message-format=json-render-diagnostics'] : [
      'test',
      '--locked',
      '-p',
      'nomifun-app',
      '--test',
      TEST_TARGET,
      ...((resourceWaitSmoke || cronSmoke || inputQueueSmoke) ? ['--no-default-features'] : []),
      ...(generalDesktopSmoke ? ['--features', 'browser-use,computer-use'] : []),
      '--no-run',
      '--message-format=json-render-diagnostics',
    ],
    {
      environment,
      outputLimitBytes: CARGO_OUTPUT_LIMIT_BYTES,
    },
  );

  if (compile.timedOut) {
    emitFailure(
      compileOnly ? 'live_smoke_compile_status=fail' : 'live_smoke_status=not_run',
      'RUNNER_GLOBAL_TIMEOUT',
      504,
    );
    process.exitCode = 2;
    return;
  }
  if (compile.overflowed) {
    emitFailure(
      compileOnly ? 'live_smoke_compile_status=fail' : 'live_smoke_status=not_run',
      'CARGO_OUTPUT_LIMIT_EXCEEDED',
      503,
    );
    process.exitCode = 2;
    return;
  }
  if (compile.spawnError) {
    emitFailure(
      compileOnly ? 'live_smoke_compile_status=fail' : 'live_smoke_status=not_run',
      'CARGO_SPAWN_FAILED',
      503,
    );
    process.exitCode = 2;
    return;
  }
  if (compile.status !== 0) {
    emitFailure(
      compileOnly ? 'live_smoke_compile_status=fail' : 'live_smoke_status=not_run',
      cargoFailureCode(compile.stderr),
      503,
    );
    process.exitCode = compile.status ?? 1;
    return;
  }

  const executable = testExecutableFromCargoJson(compile.stdout);
  if (!executable || !existsSync(executable)) {
    emitFailure(
      compileOnly ? 'live_smoke_compile_status=fail' : 'live_smoke_status=not_run',
      'TEST_EXECUTABLE_NOT_FOUND',
      503,
    );
    process.exitCode = 2;
    return;
  }
  if (compileOnly) {
    console.log('live_smoke_compile_status=pass code=OK status=200');
    process.exitCode = 0;
    return;
  }
  console.log('live_smoke_compile_status=pass code=OK status=200');

  // Deliberately read the credential only after Cargo and every build script
  // have exited. Remove it from this runner's environment before launching any
  // further process, then send one newline-terminated value over stdin.
  let credential = process.env[API_KEY_ENVIRONMENT_NAME];
  if (typeof credential !== 'string' || credential.trim().length === 0) {
    emitFailure(
      'live_smoke_status=not_run',
      'LIVE_CREDENTIAL_MISSING',
      412,
    );
    process.exitCode = 2;
    return;
  }
  credential = credential.trim();
  if (/[\r\n]/.test(credential)) {
    emitFailure(
      'live_smoke_status=not_run',
      'LIVE_CREDENTIAL_INVALID',
      412,
    );
    process.exitCode = 2;
    return;
  }
  removeCredentialFromCurrentEnvironment();
  const credentialInput = Buffer.from(`${credential}\n`, 'utf8');
  credential = '';

  let test;
  let navigationRun = null;
  const navigationReady = line => {
    const ready = navigationFixtureReady(line);
    if (!ready || navigationRun) return;
    console.log('live_smoke_navigation_phase=browser');
    navigationRun = runCaptured(environment.NOMIFUN_LIVE_BROWSER_NODE || process.execPath,
      [resolve(ROOT, 'scripts/validation/reasoning-navigation-browser.mjs'), ready.port, ready.session],
      { environment, outputLimitBytes: 1024 * 1024, timeoutMs: 240_000 });
  };
  try {
    console.log(`live_smoke_phase=execute mode=${browserGui ? 'browser_gui' : browser ? 'browser_frontend' : selectedSmokeMode} model=${model}`);
    test = await runCaptured(
      executable,
      browserGui ? [guiDataDir, '--live-frontend'] : browser ? ['--live-agent-only'] : [
        selectedSmokeTest,
        '--exact',
        '--ignored',
        '--test-threads=1',
        '--nocapture',
      ],
      {
        environment,
        input: credentialInput,
        outputLimitBytes: TEST_OUTPUT_LIMIT_BYTES,
        onStderrLine: inputQueueSmoke ? line => {
          const phase = line.match(/^NOMIFUN_INPUT_QUEUE_PHASE phase=(mixed_turns_verified|stop_hold_verified|restart_hold_verified|resume_verified)$/);
          if (phase) console.log('live_smoke_input_queue_phase=' + phase[1]);
          if (line === INPUT_QUEUE_PROOF) {
            console.log('live_smoke_input_queue_evidence=' + line);
          }
        } : appWorkspaceUiFixture ? line => {
          const ready = appWorkspaceUiReady(line);
          if (ready) console.log('live_smoke_app_workspace_ui_ready=' + JSON.stringify(ready));
          if (line === APP_WORKSPACE_UI_CLOSED) console.log('live_smoke_app_workspace_ui_closed=true validation=null');
          const preserved = line.match(/^NOMIFUN_LIVE_APP_WORKSPACE_SHUTDOWN_FILES_PRESERVED app_count=([0-9]{1,5}) file_count=([0-9]{1,5}) sha256=([a-f0-9,]*)$/);
          if (preserved && (preserved[3] === '' || preserved[3].split(',').every(hash => /^[a-f0-9]{64}$/.test(hash))) &&
              (preserved[3] === '' ? 0 : preserved[3].split(',').length) === Number(preserved[2])) {
            console.log(`live_smoke_app_workspace_shutdown_files_preserved=app_count=${preserved[1]} file_count=${preserved[2]} sha256=${preserved[3]}`);
          }
        } : appHostingSmoke ? line => {
          const ready = appHostingReady(line);
          if (ready && appHostingBrowserHold) console.log(`live_smoke_app_hosting_ready=${JSON.stringify(ready)}`);
          const stage = line.match(/^NOMIFUN_LIVE_APP_HOSTING_STAGE stage=(generate|repair) status=running$/);
          if (stage) console.log('live_smoke_app_hosting_stage=' + stage[1] + ' status=running');
          const rejection = line.match(/^NOMIFUN_LIVE_APP_HOSTING_REJECTION phase=(domain|principal|capability|action|store|session_read|session_owner|resolved_snapshot_ref|resource_key|replay_read|canonical_tool_causation|effect_started_append|import_source_canonicalization|effect_replay_read)$/);
          if (rejection) console.log('live_smoke_app_hosting_rejection=phase=' + rejection[1]);
          if (line.startsWith('NOMIFUN_LIVE_APP_HOSTING_STOP_DIAGNOSTIC ')) {
            try {
              const raw = JSON.parse(line.slice('NOMIFUN_LIVE_APP_HOSTING_STOP_DIAGNOSTIC '.length));
              if (loopbackUrl(raw.app_url) && loopbackUrl(raw.host_url)) console.log('live_smoke_app_hosting_stop_diagnostic=' + JSON.stringify({
                app_url:raw.app_url,host_url:raw.host_url,root_pid:raw.stop?.observed_root_pid,
                root_pid_gone:raw.stop?.root_pid_gone,direct_tcp_connect:raw.stop?.direct_tcp_connect,
                http_status:raw.stop?.after_stop_response?.status ?? null,
                matches_generated_html:raw.stop?.after_stop_response?.matches_generated_html ?? false}));
            } catch { /* No arbitrary child output is forwarded. */ }
          }
        } : reasoningNavigation ? navigationReady : idmmSmoke ? line => {
          const active = line.match(/^NOMIFUN_IDMM_DEMO_ACTIVE case=([a-z_]+)$/);
          if (active && IDMM_DEMO_CASES.includes(active[1])) console.log(`idmm_demo_case=${active[1]} status=running`);
          const item = parseIdmmDemoCase(line);
          if (item) console.log(`idmm_demo_case=${item.case} status=${item.status} elapsed_ms=${item.elapsed_ms} idmm_turns=${item.idmm_turns} sidecar_calls=${item.sidecar_calls}`);
        } : null,
        onStdoutLine: browserGui ? line => {
          const prefix = 'BROWSER_GUI_FIXTURE_READY ';
          if (!line.startsWith(prefix)) return;
          try {
            const data = JSON.parse(line.slice(prefix.length));
            const page = new URL(data.page);
            if (resolve(data.data_dir) !== guiDataDir || resolve(data.work_dir) !== resolve(guiDataDir, 'work') ||
                data.real_provider !== true || page.protocol !== 'http:' || page.hostname !== '127.0.0.1' ||
                page.username || page.password || page.pathname !== '/' || data.control !== page.origin ||
                !/^[a-f0-9-]{36}$/.test(data.session_id)) return;
            console.log('browser_gui_ready=' + JSON.stringify({data_dir:guiDataDir,work_dir:resolve(guiDataDir,'work'),page:page.href,control:page.origin,session_id:data.session_id,real_provider:true}));
          } catch { /* Never forward arbitrary child output or credentials. */ }
        } : null,
      },
    );
  } finally {
    credentialInput.fill(0);
  }
  if (reasoningNavigation) {
    const navigation = navigationRun ? await navigationRun : null;
    const proof = navigation?.stdout.split(/\r?\n/).find(line =>
      /^NOMIFUN_LIVE_REASONING_NAVIGATION leaves=2 returns=2 returned_thinking=[0-9]{1,5} returned_done=[0-9]{1,5} clock_advances=true returned_expanded=true completed_collapsed=true terminal=true$/.test(line));
    if (proof) console.log('live_smoke_navigation_evidence=' + proof.slice('NOMIFUN_LIVE_REASONING_NAVIGATION '.length));
    if (!navigation || navigation.status !== 0 || !proof) {
      const failure = navigation?.stderr.split(/\r?\n/).find(line =>
        /^NOMIFUN_LIVE_REASONING_NAVIGATION_FAILURE code=NAVIGATION_[A-Z_]+_FAILED$/.test(line));
      if (failure) console.error(failure);
      emitFailure('live_smoke_navigation_status=fail', 'REASONING_NAVIGATION_FAILED', 422);
      if (test.status === 0) test.status = 1;
    }
  }

  if (test.timedOut) {
    emitFailure('live_smoke_status=fail', 'RUNNER_GLOBAL_TIMEOUT', 504);
    process.exitCode = 2;
    return;
  }
  if (test.overflowed) {
    emitFailure('live_smoke_status=fail', 'TEST_OUTPUT_LIMIT_EXCEEDED', 503);
    process.exitCode = 2;
    return;
  }
  if (test.spawnError) {
    emitFailure('live_smoke_status=not_run', 'TEST_SPAWN_FAILED', 503);
    process.exitCode = 2;
    return;
  }
  const stages = beforeToolSmoke ? beforeToolStagesFromOutput(test.stderr) : [];
  for (const phase of stages) console.log(`live_smoke_stage=${phase} status=pass`);
  for (const line of test.stderr.split(/\r?\n/)) {
    const reasoning = line.match(/^NOMIFUN_LIVE_REASONING_EVIDENCE messages=([0-9]{1,5}) deltas=([0-9]{1,6}) done=([0-9]{1,5}) reopened=([0-9]{1,5}) tools_started=([0-9]{1,5}) tools_completed=([0-9]{1,5}) active_samples=([0-9]{1,5}) elapsed_ms=([0-9]{1,8}) all_closed=(true|false) terminal=(true|false)$/);
    if (reasoning) console.log(`live_smoke_reasoning_evidence=${reasoning[0].slice('NOMIFUN_LIVE_REASONING_EVIDENCE '.length)}`);
    const reasoningRecovery = line.match(/^NOMIFUN_LIVE_REASONING_RECOVERY reconnects=([0-9]{1,3}) recovered_frames=([0-9]{1,6}) active_reads=([0-9]{1,5}) history_active=([0-9]{1,5}) history_done=([0-9]{1,5}) stable_anchor=(true|false) terminal_history_closed=(true|false)$/);
    if (reasoningRecovery) console.log('live_smoke_reasoning_recovery=' + reasoningRecovery[0].slice('NOMIFUN_LIVE_REASONING_RECOVERY '.length));
    const build = line.match(/^NOMIFUN_LIVE_SMOKE_BUILD digest=([a-f0-9]{64})$/);
    if (build) console.log(`live_smoke_build_digest=${build[1]}`);
    const codingTrace = line.match(/^NOMIFUN_LIVE_SMOKE_CODING_TRACE phase=(file|coding|snake_game|long_first|long_repair|long_second) read=([0-9]{1,4}) write=([0-9]{1,4}) patch=([0-9]{1,4}) exec=([0-9]{1,4}) plan=([0-9]{1,4}) completion=([0-9]{1,4}) tool_errors=([0-9]{1,4}) final_replies=([0-9]{1,4}) file_exists=(true|false) file_bytes=([0-9]{1,8}) html=(true|false) script=(true|false) canvas=(true|false) keydown=(true|false)$/);
    if (codingTrace) console.log(`live_smoke_coding_trace=${codingTrace[0].slice('NOMIFUN_LIVE_SMOKE_CODING_TRACE '.length)}`);
    const codingFlow = line.match(/^NOMIFUN_LIVE_SMOKE_CODING_FLOW phase=(file|coding|snake_game|long_first|long_repair|long_second) flow=([RWPEUCO01x]{0,96})$/);
    if (codingFlow) console.log(`live_smoke_coding_flow=phase=${codingFlow[1]} flow=${codingFlow[2]}`);
    const codingCheck = line.match(/^NOMIFUN_LIVE_SMOKE_CODING_CHECK phase=long_first attempt=([0-2]) quote=(true|false) newline=(true|false) summary=(true|false) malformed=(true|false) syntax=(true|false) import=(true|false) assertion=(true|false) zero_tests=(true|false)$/);
    if (codingCheck) console.log(`live_smoke_coding_check=${codingCheck[0].slice('NOMIFUN_LIVE_SMOKE_CODING_CHECK '.length)}`);
    const codingHistory = line.match(/^NOMIFUN_LIVE_SMOKE_CODING_HISTORY phase=long_coding\.(?:first|second)\.history instruction=([0-9]{1,4}) source=([0-9]{1,4}) tests=([0-9]{1,4}) other=([0-9]{1,4}) errors=([0-9]{1,4}) invalid_payload=([0-9]{1,4}) not_found=([0-9]{1,4}) scope_rejected=([0-9]{1,4}) capability_unavailable=([0-9]{1,4}) admission=([0-9]{1,4}) process_error=([0-9]{1,4}) command_exit=([0-9]{1,4}) tool_search=([0-9]{1,4}) discovery_revealed_write=([0-9]{1,4}) other_tools=([0-9]{1,4}) exec_failed=([0-9]{1,4}) exec_succeeded=([0-9]{1,4}) exec_unknown=([0-9]{1,4}) test_launches=([0-9]{1,4})$/);
    if (codingHistory) console.log(`live_smoke_coding_history=${codingHistory[0].slice('NOMIFUN_LIVE_SMOKE_CODING_HISTORY '.length)}`);
    const codingFailures = line.match(/^NOMIFUN_LIVE_SMOKE_CODING_FAILURES phase=long_coding\.(?:first|second)\.history names=([a-z_,]{0,180}) diagnosis=([A-Z0-9_]{1,96})$/);
    if (codingFailures && codingFailures[1].split(',').filter(Boolean).every(name => ['read_file', 'write_file', 'apply_patch', 'exec_command', 'start_process', 'poll_process', 'git_status', 'git_diff', 'search_files', 'update_plan', 'report_completion', 'other'].includes(name))) {
      console.log(`live_smoke_coding_failures=${codingFailures[0].slice('NOMIFUN_LIVE_SMOKE_CODING_FAILURES '.length)}`);
    }
    const codingTerminal = line.match(/^NOMIFUN_LIVE_SMOKE_CODING_TERMINAL phase=long_coding\.(?:first|repair|second)\.result replies=([0-9]{1,4}) tools=([0-9]{1,4}) failed_turn=(true|false) code=([A-Z0-9_]{1,96}) diagnosis=([A-Z0-9_]{1,96})$/);
    if (codingTerminal) console.log(`live_smoke_coding_terminal=${codingTerminal[0].slice('NOMIFUN_LIVE_SMOKE_CODING_TERMINAL '.length)}`);
    const turnFailure = line.match(/^NOMIFUN_LIVE_SMOKE_TURN_FAILURE index=([0-7]) code=([A-Z0-9_]{1,96}) diagnosis=([A-Z0-9_]{1,96}) steps=([0-9]{1,4}) detail=([A-Za-z0-9_:().-]{0,240})$/);
    if (turnFailure) console.log(`live_smoke_turn_failure=${turnFailure[0].slice('NOMIFUN_LIVE_SMOKE_TURN_FAILURE '.length)}`);
    const runtimeProgress = line.match(/^NOMIFUN_LIVE_SMOKE_RUNTIME_PROGRESS steps=([0-9]{1,5}) compact_calls=([0-9]{1,5}) compacted=([0-9]{1,5}) degraded=([0-9]{1,5}) reads=([0-9]{1,5}) execs=([0-9]{1,5}) writes=([0-9]{1,5}) reports=([0-9]{1,5})$/);
    if (runtimeProgress) console.log(`live_smoke_runtime_progress=${runtimeProgress[0].slice('NOMIFUN_LIVE_SMOKE_RUNTIME_PROGRESS '.length)}`);

    const controlErrors = line.match(/^NOMIFUN_LIVE_SMOKE_CONTROL_ERRORS sequence=([A-Z_:,]{1,450})$/);
    if (controlErrors) console.log(`live_smoke_control_errors=${controlErrors[1]}`);
    const toolRejections = line.match(/^NOMIFUN_LIVE_SMOKE_TOOL_REJECTIONS invalid_arguments=([0-9]{1,5}) unavailable_names=([0-9]{1,5}) kernel_rejections=([0-9]{1,5}) delegations=([0-9]{1,5}) text_events=([0-9]{1,5})$/);
    if (toolRejections) console.log(`live_smoke_tool_rejections=${toolRejections[0].slice('NOMIFUN_LIVE_SMOKE_TOOL_REJECTIONS '.length)}`);
    const collaborationShapes = line.match(/^NOMIFUN_LIVE_SMOKE_COLLAB_ARGUMENT_SHAPES sequence=(NONE|(?:(?:parallel|planned|other):(?:missing|null|boolean|string|array|object|number):(?:missing|null|boolean|string|array|object|number))(?:,(?:parallel|planned|other):(?:missing|null|boolean|string|array|object|number):(?:missing|null|boolean|string|array|object|number)){0,5})$/);
    if (collaborationShapes) console.log(`live_smoke_collaboration_argument_shapes=${collaborationShapes[1]}`);
    const compact = line.match(/^NOMIFUN_LIVE_SMOKE_COMPACTION summaries=([0-9]{1,4}) replacements=([0-9]{1,4})$/);
    if (compact) console.log(`live_smoke_compaction summaries=${compact[1]} replacements=${compact[2]}`);
    const recovery = line.match(/^NOMIFUN_LIVE_SMOKE_RECOVERY phase=(engine\.(?:nomi|coding)\.(?:create|patch|exec|continue)) controls=([0-9]{1,4}) pre_execution=([0-9]{1,4})$/);
    if (recovery) console.log(`live_smoke_recovery phase=${recovery[1]} controls=${recovery[2]} pre_execution=${recovery[3]}`);
  }
  if (test.status === 0) {
    if (browserGui) {
      console.log('browser_gui_fixture_status=stopped');
      process.exitCode = 0; return; // A stopped fixture is not a GUI acceptance claim.
    }
    if (browser) {
      if (!browserProofFromOutput(test.stdout)) {
        emitFailure('live_smoke_status=fail', 'BROWSER_EVIDENCE_MISSING', 422);
        process.exitCode = 1;
        return;
      }
      console.log('browser_live_frontend_status=pass native_click=true observed_value=2 canonical_action_shape=true evidence_cancel=true terminal_unlock=true');
      console.log('live_smoke_status=pass code=OK status=200');
      process.exitCode = 0;
      return;
    }
    // libtest exits successfully even when an exact filter matches zero tests.
    const selected = selectedSmokeTest;
    if (!selectedTestPassed(test.stdout, selected)) {
      emitFailure('live_smoke_status=not_run', 'SELECTED_TEST_DID_NOT_PASS', 503);
      process.exitCode = 2;
      return;
    }
    if (inputQueueSmoke) {
      try {
        const evidence = inputQueueEvidence(test.stderr);
        if (reportPath) {
          writeFileSync(reportPath, JSON.stringify({...evidence,status:'pass',completed_at:new Date().toISOString()},null,2)+'\n',{mode:0o600});
          console.log('live_smoke_input_queue_report=' + JSON.stringify(reportPath));
        }
      } catch {
        emitFailure('live_smoke_status=fail', 'INPUT_QUEUE_EVIDENCE_OR_REPORT_INVALID', 503);
        process.exitCode = 2; return;
      }
    }
    if (beforeToolSmoke && (stages.length !== BEFORE_TOOL_STAGE_PHASES.length ||
        BEFORE_TOOL_STAGE_PHASES.some((phase, index) => stages[index] !== phase))) {
      emitFailure('live_smoke_status=fail', 'BEFORE_TOOL_STAGE_EVIDENCE_INCOMPLETE', 503);
      process.exitCode = 2;
      return;
    }
    if (componentSmoke) {
      try {
        const evidence = managedComponentEvidence(test.stderr);
        console.log('live_smoke_component_evidence=' + JSON.stringify(evidence));
        if (reportPath) {
          writeFileSync(reportPath, JSON.stringify({...evidence,status:'pass',completed_at:new Date().toISOString()},null,2)+'\n',{mode:0o600});
          console.log('live_smoke_component_report=' + JSON.stringify(reportPath));
        }
      } catch {
        emitFailure('live_smoke_status=fail', 'COMPONENT_EVIDENCE_OR_REPORT_INVALID', 503);
        process.exitCode = 2; return;
      }
    }
    if (idmmSmoke) {
      try {
        const evidence = parseIdmmDemoEvidence(test.stderr);
        console.log('idmm_demo_limitation=provider_pause_requires_explicit_native_resume');
        if (reportPath) {
          writeFileSync(reportPath, JSON.stringify({...evidence, model, completed_at:new Date().toISOString()}, null, 2) + '\n', {mode:0o600});
          console.log(`idmm_demo_report=${JSON.stringify(reportPath)}`);
        }
      } catch {
        emitFailure('live_smoke_status=fail', 'IDMM_DEMO_EVIDENCE_OR_REPORT_INVALID', 503);
        process.exitCode = 2;
        return;
      }
    }
    if (appHostingSmoke) {
      try {
        const evidence = appHostingEvidence(test.stderr);
        if (evidence.model !== model || (appHostingBrowserHold && (evidence.browser_qa === null || evidence.repair.browser_qa === null))) throw new Error('hosting mode evidence mismatch');
        console.log('live_smoke_app_hosting_evidence=' + JSON.stringify(evidence));
        if (reportPath) {
          writeFileSync(reportPath, JSON.stringify({...evidence,status:'pass',completed_at:new Date().toISOString()},null,2)+'\n',{mode:0o600});
          console.log('live_smoke_app_hosting_report=' + JSON.stringify(reportPath));
        }
      } catch {
        emitFailure('live_smoke_status=fail', 'APP_HOSTING_EVIDENCE_OR_REPORT_INVALID', 503);
        process.exitCode = 2; return;
      }
    }
    if (appWorkspaceUiFixture) {
      const ready = test.stderr.split(/\r?\n/).map(appWorkspaceUiReady).filter(Boolean);
      const closed = test.stderr.split(/\r?\n/).filter(line => line === APP_WORKSPACE_UI_CLOSED);
      if (ready.length !== 1 || closed.length !== 1) {
        emitFailure('live_smoke_status=fail','UI_FIXTURE_LIFECYCLE_EVIDENCE_INVALID',503);
        process.exitCode = 2; return;
      }
      console.log('live_smoke_scope=production_host_lifecycle validation=null');
    }
    if (retainNativeFixture) {
      try {
        const paths = retainedFixtureFromOutput(test.stderr, validateFixtureParent(fixtureParent));
        // Paths only: the retained disposable Provider credential remains
        // encrypted by the ordinary store for this native acceptance session.
        console.log(`live_smoke_native_fixture=${JSON.stringify(paths)}`);
      } catch {
        emitFailure('live_smoke_status=fail', 'NATIVE_FIXTURE_EVIDENCE_INVALID', 503);
        process.exitCode = 2;
        return;
      }
    } else if (test.stderr.split(/\r?\n/).some((line) => line.startsWith(NATIVE_FIXTURE_MARKER))) {
      emitFailure('live_smoke_status=fail', 'NATIVE_FIXTURE_RETENTION_UNEXPECTED', 503);
      process.exitCode = 2;
      return;
    }
    console.log(`live_smoke_mode=${selectedSmokeMode} model=${model}`);
    console.log('live_smoke_status=pass code=OK status=200');
    process.exitCode = 0;
    return;
  }

  const typed = typedFailureFromOutput(`${test.stdout}\n${test.stderr}`);
  if (inputQueueSmoke && reportPath) {
    try { writeFileSync(reportPath, JSON.stringify({schema_version:1,provider:'stepfun-plan',model,
      status:'fail',completed_at:new Date().toISOString(),failure:typed ?? {code:'TEST_EXECUTION_FAILED'}},null,2)+'\n',{mode:0o600}); }
    catch { /* Failure remains authoritative if report writing fails. */ }
  }
  if (componentSmoke && reportPath) {
    try { writeFileSync(reportPath, JSON.stringify({schema_version:1,provider:'stepfun-plan',model,
      component:'officecli',status:'fail',completed_at:new Date().toISOString(),
      failure:typed ?? {code:'TEST_EXECUTION_FAILED'}},null,2)+'\n',{mode:0o600}); }
    catch { /* Failure remains authoritative if report writing fails. */ }
  }
  if (appHostingSmoke && reportPath) {
    try { writeFileSync(reportPath, JSON.stringify({schema_version:3,provider:'stepfun-plan',model,template:'app.developer',fixed_app_agent:true,status:'fail',
      completed_at:new Date().toISOString(),failure:typed ?? {code:'TEST_EXECUTION_FAILED'},artifact_directory:artifactDir},null,2)+'\n',{mode:0o600}); }
    catch { /* Failure remains authoritative if report writing fails. */ }
  }
  if (idmmSmoke && reportPath) {
    try {
      const cases = test.stderr.split(/\r?\n/).map(parseIdmmDemoCase).filter(Boolean);
      writeFileSync(reportPath, JSON.stringify({schema_version:1,provider:'stepfun-plan',model,
        status:'fail',completed_at:new Date().toISOString(),cases,
        failure:typed ?? {code:'TEST_EXECUTION_FAILED'}},null,2)+'\n',{mode:0o600});
      console.log(`idmm_demo_report=${JSON.stringify(reportPath)}`);
    } catch { /* The exit failure remains authoritative if report writing fails. */ }
  }
  if (!typed) {
    // Never forward arbitrary child diagnostics: they may contain provider
    // content. Exit/signal and a fixed classification identify harness faults.
    const signal = typeof test.signal === 'string' && /^SIG[A-Z0-9]{1,16}$/.test(test.signal) ? test.signal : 'none';
    const exit = Number.isInteger(test.status) && test.status >= 0 && test.status <= 255 ? test.status : 'none';
    const stackOverflow = /stack overflow|overflowed its stack/i.test(`${test.stdout}\n${test.stderr}`);
    console.error(`live_smoke_harness exit=${exit} signal=${signal} stack_overflow=${stackOverflow}`);
    const panicLine = `${test.stdout}\n${test.stderr}`.split(/\r?\n/)
      .find(line => line.includes('panicked at '));
    if (panicLine) {
      const location = panicLine.match(/(?:nomi_core_live_provider_smoke|turn|engine|history_process_display)\.rs:(\d{1,5}):(\d{1,5})/);
      console.error(`live_smoke_panic=${location ? `known_source_line_${location[1]}` : 'location_unavailable'}`);
    }
  }
  if (browser) {
    const evidence = browserEvidenceFromOutput(test.stderr);
    if (evidence) console.error(`browser_live_evidence=${JSON.stringify(evidence)}`);
  }
  if (retainNativeFixture && test.stderr.split(/\r?\n/).some((line) => line.startsWith(NATIVE_FIXTURE_MARKER))) {
    try {
      const paths = retainedFixtureFromOutput(test.stderr, validateFixtureParent(fixtureParent));
      console.log(`live_smoke_native_fixture=${JSON.stringify(paths)}`);
    } catch {
      emitFailure('live_smoke_status=fail', 'NATIVE_FIXTURE_EVIDENCE_INVALID', 503);
      process.exitCode = 2;
      return;
    }
  }
  if (typed) {
    console.error(
      `live_smoke_status=fail phase=${typed.phase} code=${typed.code} status=${typed.status}`,
    );
  } else {
    emitFailure(
      'live_smoke_status=not_run',
      'TEST_HARNESS_FAILED_WITHOUT_SENTINEL',
      503,
    );
  }
  process.exitCode = test.status ?? 1;
}

function navigationFixtureReady(line) {
  if (/[\r\n]/.test(line)) return null;
  const ready = line.match(/^NOMIFUN_LIVE_REASONING_UI_READY port=([0-9]{1,5}) session=([a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12})$/);
  return ready && Number(ready[1]) >= 1 && Number(ready[1]) <= 65535
    ? { port: ready[1], session: ready[2] } : null;
}

function runSelfTest() {
  const component = {schema_version:1,provider:'stepfun-plan',model:'step-5-preview',component:'officecli',
    action_id:'workspace.process/start',source:'managed',version:'1.0.156',digest:'a'.repeat(64),
    canonical_component_argument:true,exit_code:0,cleanup_reaped:true,completed_turns:2,
    ordinary_continuation_no_tools:true,same_session_binding:true,call_started_seq:5,result_seq:9,secret:'DO_NOT_EMIT'};
  const componentEvidence = managedComponentEvidence(COMPONENT_EVIDENCE+JSON.stringify(component));
  if (componentEvidence.action_id !== 'workspace.process/start') {
    throw new Error('component evidence must preserve the current process Action');
  }
  if (JSON.stringify(componentEvidence).includes('DO_NOT_EMIT')) {
    throw new Error('component evidence projection exposed arbitrary data');
  }
  for (const invalid of [{...component,exit_code:1},{...component,canonical_component_argument:false},
      {...component,ordinary_continuation_no_tools:false},{...component,result_seq:5},
      {...component,action_id:'workspace.process/exec'}]) {
    let rejected = false; try { managedComponentEvidence(COMPONENT_EVIDENCE+JSON.stringify(invalid)); } catch { rejected = true; }
    if (!rejected) throw new Error('invalid component evidence accepted');
  }
  const hostingId = '0190f5fe-7c00-7a00-8abc-012345678911';
  const fixtureReady = APP_WORKSPACE_UI_READY + JSON.stringify({host_url:'http://127.0.0.1:43124',
    finish_url:'http://127.0.0.1:43124/__app-workspace-fixture/finish',web_test_username:'app-workspace-smoke',
    provider_id:hostingId,model:DEFAULT_MODEL,validation:null,secret:'DO_NOT_EMIT'});
  if (!appWorkspaceUiReady(fixtureReady) || JSON.stringify(appWorkspaceUiReady(fixtureReady)).includes('DO_NOT_EMIT') ||
      appWorkspaceUiReady(fixtureReady.replace('127.0.0.1','example.com')) || appWorkspaceUiReady(fixtureReady.replace('null','true'))) {
    throw new Error('UI fixture readiness must not manufacture acceptance');
  }
  const hostingReadyLine = APP_HOSTING_READY + JSON.stringify({app_url:'http://127.0.0.1:43123',host_url:'http://127.0.0.1:43124',
    session_id:hostingId,app_id:hostingId,qa_callback:'http://127.0.0.1:43124/__app-hosting-smoke/ui-state',secret:'DO_NOT_EMIT'});
  if (!appHostingReady(hostingReadyLine) || JSON.stringify(appHostingReady(hostingReadyLine)).includes('DO_NOT_EMIT') ||
      appHostingReady(hostingReadyLine.replace('43123','0')) || appHostingReady(hostingReadyLine.replace('127.0.0.1','example.com')) ||
      appHostingReady(hostingReadyLine+'\n')) throw new Error('hosting ready signal projection failed');
  const hostingEvidence = {schema_version:3,provider:'stepfun-plan',model:DEFAULT_MODEL,template:'app.developer',runtime_terminal:'turn/completed',
    default_hosting_grant:true,fixed_app_agent:true,manual_create_before_model_configuration:true,ordinary_session:true,
    current_workspace:resolve(ROOT,'.git','app-workspace-validation','synthetic'),current_workspace_is_managed_directory:true,application_count:1,application_id_preserved:true,
    app_id:hostingId,preview:true,managed_pid:1234,operation_id:'first-operation',
    external_unauthenticated_status:403,connection:{connected:true,app_count:1},verified_host_requests:1,
    stop:{returned_state:'stopped',returned_pid:null,observed_root_pid:1234,root_pid_gone:true,direct_tcp_connect:false,
      no_proxy:true,fresh_http_connection:true,after_stop_response:null},
    app_url:'http://127.0.0.1:43123',script_syntax_checked:true,http_status:200,http_bytes_equal:true,
    stop_verified:true,source_sha256:'a'.repeat(64),backend_sha256:'c'.repeat(64),source_bytes:1024,browser_qa:null,secret:'DO_NOT_EMIT',
    settlements:['workspace.files/write','app.hosting/update','app.hosting/start','app.hosting/guide','app.hosting/current'].map((action_id,i)=>({action_id,operation_id:'test-operation-'+i,
      dispatch_seq:i*10+1,result_seq:i*10+3,result_success:true,result_sha256:'b'.repeat(64),effect_succeeded_seq:i*10+2,app_id:hostingId,output:'DO_NOT_EMIT'}))};
  hostingEvidence.repair = {operation_id:'repair-operation',runtime_terminal:'turn/completed',app_id_preserved:true,session_id_preserved:true,
    prior_history_unchanged:true,source_changed:true,original_pid_gone:true,managed_pid:1234,browser_qa:null,
    upstream_failure:{injected_http_status:503,app_http_status:503,body:{connected:false,app_count:0},verified_injected_requests:1,recovered_success:true,recovered_verified_requests:1},
    settlements:['app.hosting/stop','workspace.files/patch','app.hosting/preview'].map((action_id,i)=>({action_id,operation_id:'repair-tool-'+i,
      dispatch_seq:100+i*10+1,result_seq:100+i*10+3,result_success:true,result_sha256:'b'.repeat(64),effect_succeeded_seq:100+i*10+2,app_id:hostingId}))};
  hostingEvidence.turns = [{operation_id:'first-operation',runtime_terminal:'turn/completed'},{operation_id:'repair-operation',runtime_terminal:'turn/completed'}];
  if (JSON.stringify(appHostingEvidence(APP_HOSTING_EVIDENCE+JSON.stringify(hostingEvidence))).includes('DO_NOT_EMIT')) throw new Error('hosting evidence redaction failed');
  for (const invalid of [{...hostingEvidence,http_status:404},{...hostingEvidence,settlements:hostingEvidence.settlements.slice(1)},
      {...hostingEvidence,template:'coding.codex'}, {...hostingEvidence,default_hosting_grant:false},
      {...hostingEvidence,current_workspace_is_managed_directory:false}, {...hostingEvidence,application_id_preserved:false},
      {...hostingEvidence,repair:null},{...hostingEvidence,repair:{...hostingEvidence.repair,prior_history_unchanged:false}},
      {...hostingEvidence,repair:{...hostingEvidence.repair,upstream_failure:{...hostingEvidence.repair.upstream_failure,app_http_status:200}}},
      {...hostingEvidence,settlements:[...hostingEvidence.settlements,{...hostingEvidence.settlements[1],action_id:'app.hosting/import'}]},
      {...hostingEvidence,settlements:hostingEvidence.settlements.map(item=>({...item,effect_succeeded_seq:null}))},
      {...hostingEvidence,stop:{...hostingEvidence.stop,root_pid_gone:false}},
      {...hostingEvidence,stop:{...hostingEvidence.stop,direct_tcp_connect:true}},
      {...hostingEvidence,stop:{...hostingEvidence.stop,after_stop_response:{status:200}}},
      {...hostingEvidence,stop:{...hostingEvidence.stop,no_proxy:false}},
      {...hostingEvidence,stop:{...hostingEvidence.stop,fresh_http_connection:false}}]) {
    let rejected = false; try { appHostingEvidence(APP_HOSTING_EVIDENCE+JSON.stringify(invalid)); } catch { rejected = true; }
    if (!rejected) throw new Error('incomplete hosting evidence accepted');
  }
  if (cargoFailureCode("rust-lld: error: failed to write output 'test.exe': permission denied") !== 'CARGO_LINK_OUTPUT_UNAVAILABLE' ||
      cargoFailureCode('error[E0308]: mismatched types') !== 'CARGO_BUILD_FAILED') {
    throw new Error('cargo failure classification self-test failed');
  }
  const proof = 'BROWSER_WORKSPACE_SMOKE_PASS ' + JSON.stringify({ scope: 'live-agent-only', native_fixture_profile_cleanup: true, agent: { real_provider: true, native_auto_open: true, trusted_click_value: 2, canonical_action_shape: true, evidence_driven_cancel: true, terminal_before_unlock: true } });
  if (!browserProofFromOutput(proof) || browserProofFromOutput('') || browserProofFromOutput(proof.replace('"trusted_click_value":2', '"trusted_click_value":4'))) throw new Error('browser proof parsing failed');
  const evidence = browserEvidenceFromOutput('NOMIFUN_BROWSER_LIVE_EVIDENCE ' + JSON.stringify({ count: 3, secret: 'DO_NOT_EMIT', tools: [{ name: 'DO_NOT_EMIT', args: 'DO_NOT_EMIT' }] }));
  if (evidence?.count !== 3 || JSON.stringify(evidence).includes('DO_NOT_EMIT')) throw new Error('browser evidence redaction failed');
  const selfTestRoot = mkdtempSync(resolve(ROOT, '.git', 'before-tool-runner-self-test-'));
  try {
    const parent = validateFixtureParent(selfTestRoot);
    const root = mkdtempSync(resolve(parent, 'before-tool-native-'));
    mkdirSync(resolve(root, 'data')); mkdirSync(resolve(root, 'work'));
    const marker = NATIVE_FIXTURE_MARKER + JSON.stringify({fixture_root: root, data_root: resolve(root, 'data'), work_root: resolve(root, 'work')});
    if (retainedFixtureFromOutput(marker, parent).fixture_root !== root) throw new Error('fixture proof self-test failed');
    for (const invalid of [marker + '\n' + marker, NATIVE_FIXTURE_MARKER + JSON.stringify({fixture_root: root, data_root: parent, work_root: resolve(root, 'work')}),
        NATIVE_FIXTURE_MARKER + JSON.stringify({fixture_root: '', data_root: resolve(root, 'data'), work_root: resolve(root, 'work')})]) {
      let rejected = false;
      try { retainedFixtureFromOutput(invalid, parent); } catch { rejected = true; }
      if (!rejected) throw new Error('unsafe fixture proof accepted');
    }
    for (const invalid of ['', ROOT, resolve(ROOT, '.git')]) {
      let rejected = false;
      try { validateFixtureParent(invalid); } catch { rejected = true; }
      if (!rejected) throw new Error('unsafe fixture parent accepted');
    }
  } finally {
    rmSync(selfTestRoot, {recursive: true, force: true});
  }
  const hookLines = BEFORE_TOOL_STAGE_PHASES.map((phase) => `NOMIFUN_LIVE_SMOKE_STAGE phase=${phase} status=pass`).join('\n');
  if (JSON.stringify(beforeToolStagesFromOutput(hookLines)) !== JSON.stringify(BEFORE_TOOL_STAGE_PHASES) ||
      beforeToolStagesFromOutput('NOMIFUN_LIVE_SMOKE_STAGE phase=before_tool.secret status=pass').length !== 0 ||
      beforeToolStagesFromOutput('NOMIFUN_LIVE_SMOKE_STAGE phase=before_tool.allow status=pass extra').length !== 0 ||
      !selectedTestPassed(`test ${BEFORE_TOOL_TEST_NAME} ... ok`, BEFORE_TOOL_TEST_NAME) ||
      selectedTestPassed('running 0 tests\ntest result: ok. 0 passed;', BEFORE_TOOL_TEST_NAME)) {
    throw new Error('before-tool evidence self-test failed');
  }
  if (!selectedTestPassed(`test ${MODEL_TEST_NAME} ... ok\r\n`, MODEL_TEST_NAME) ||
      selectedTestPassed('running 0 tests\ntest result: ok. 0 passed;', MODEL_TEST_NAME) ||
      selectedTestPassed(`test ${BEFORE_TOOL_TEST_NAME} ... ok`, MODEL_TEST_NAME)) {
    throw new Error('exact test execution proof self-test failed');
  }
  const readyLine = 'NOMIFUN_LIVE_REASONING_UI_READY port=41234 session=0190f5fe-7c00-7a00-8abc-012345678911';
  if (!navigationFixtureReady(readyLine) ||
      [readyLine.replace('41234', '0'), readyLine.replace('41234', '65536'),
        `test ${REASONING_TEST_NAME} ... ${readyLine}`, `${readyLine}\n`, `${readyLine} extra`]
        .some(line => navigationFixtureReady(line)) ||
      !selectedTestPassed(`test ${REASONING_TEST_NAME} ... ok\r\n`, REASONING_TEST_NAME) ||
      selectedTestPassed(`test ${REASONING_TEST_NAME} ... \n${readyLine}\nok`, REASONING_TEST_NAME)) {
    throw new Error('reasoning navigation signal and selected-test receipt self-test failed');
  }
  if (!ALLOWED_MODELS.has('step-3.7-flash') || !ALLOWED_MODELS.has('step-5-preview') ||
      ['step-3.77-flash', 'step-3.7-flash\n', 'step-3.7-flash ', 'step-5-preview\n', 'step-5-preview ', '', 'https://example.invalid/v1', 'arbitrary-model'].some((model) => ALLOWED_MODELS.has(model))) {
    throw new Error('model allowlist self-test failed');
  }
  if (!ALLOWED_MODELS.has('step-5-preview')) throw new Error('model allowlist self-test failed');
  const queueProof = INPUT_QUEUE_PHASES.map(phase => `NOMIFUN_INPUT_QUEUE_PHASE phase=${phase}`).join('\n') + '\n' + INPUT_QUEUE_PROOF;
  if (inputQueueEvidence(queueProof).host_restarts !== 1) throw new Error('input queue evidence self-test failed');
  for (const invalid of ['', INPUT_QUEUE_PROOF, queueProof + '\n' + INPUT_QUEUE_PROOF,
      queueProof.replace('queue_turns=2', 'queue_turns=1'), queueProof.replace('phase=stop_hold_verified', 'phase=resume_verified')]) {
    let rejected = false; try { inputQueueEvidence(invalid); } catch { rejected = true; }
    if (!rejected) throw new Error('incomplete input queue proof accepted');
  }
  if (selectedSmokeModel(undefined) !== (cronSmoke || componentSmoke || inputHandoffSmoke || resourceWaitSmoke || inputQueueSmoke ? 'step-5-preview' : DEFAULT_MODEL)
      || selectedSmokeModel(DEFAULT_MODEL) !== DEFAULT_MODEL || selectedSmokeModel('step-5-preview') !== 'step-5-preview'
      || inputHandoffSmoke && (selectedSmokeTest !== INPUT_HANDOFF_TEST_NAME || selectedSmokeMode !== 'user_input_handoff'
        || !selectedTestPassed(`test ${INPUT_HANDOFF_TEST_NAME} ... ok\n`, selectedSmokeTest))
      || resourceWaitSmoke && (selectedSmokeTest !== RESOURCE_WAIT_TEST_NAME || selectedSmokeMode !== 'resource_wait'
        || !selectedTestPassed(`test ${RESOURCE_WAIT_TEST_NAME} ... ok\n`, selectedSmokeTest))
      || inputQueueSmoke && (selectedSmokeTest !== INPUT_QUEUE_TEST_NAME || selectedSmokeMode !== 'input_queue'
        || !selectedTestPassed(`test ${INPUT_QUEUE_TEST_NAME} ... ok\n`, selectedSmokeTest))) {
    throw new Error('input handoff selection and model default self-test failed');
  }
  const scrubbed = environmentWithoutCredential({
    PATH: 'safe',
    [API_KEY_ENVIRONMENT_NAME.toLowerCase()]: 'must-not-copy',
  });
  if (
    scrubbed.PATH !== 'safe' ||
    Object.keys(scrubbed).some(isCredentialEnvironmentName)
  ) {
    throw new Error('credential environment scrub self-test failed');
  }

  const typed = typedFailureFromOutput(
    'noise\nNOMIFUN_LIVE_SMOKE_FAILURE phase=coding.exec code=COMMAND_FAILED status=422\nnoise',
  );
  if (
    typed?.phase !== 'coding.exec' ||
    typed.code !== 'COMMAND_FAILED' ||
    typed.status !== '422' ||
    typedFailureFromOutput('phase=coding.exec code=COMMAND_FAILED status=422')
  ) {
    throw new Error('failure sentinel parser self-test failed');
  }

  const executable = testExecutableFromCargoJson(
    JSON.stringify({
      reason: 'compiler-artifact',
      target: { name: TEST_TARGET, kind: [TARGET_KIND] },
      executable: 'test-executable',
    }),
  );
  if (executable !== 'test-executable') {
    throw new Error('Cargo executable parser self-test failed');
  }
  console.log('live_smoke_runner_self_test_status=pass code=OK status=200');
}

if (selfTest) runSelfTest();
else await main();
