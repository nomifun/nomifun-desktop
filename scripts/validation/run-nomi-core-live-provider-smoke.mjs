#!/usr/bin/env bun

/**
 * Compile and run an opt-in Nomi-core or native-browser live-provider smoke without exposing
 * its credential to Cargo, build scripts, argv, files, logs, or tool children.
 *
 * The smoke is intentionally pinned to StepFun Coding Plan
 * (`stepfun-plan` / `step-3.7-flash`) in the Rust fixture.
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
 *   bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --model-smoke
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
import { existsSync, realpathSync, statSync, mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute, basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

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
const ALLOWED_MODELS = new Set([DEFAULT_MODEL]);
const MODEL_TEST_NAME = 'nomi_core_selected_model_reaches_live_stepfun';
const BEFORE_TOOL_TEST_NAME = 'nomi_core_product_before_tool_reaches_live_stepfun';
const BEFORE_TOOL_STAGE_PHASES = ['before_tool.publish_select', 'before_tool.allow', 'before_tool.deny', 'before_tool.continuation'];
const COMPACTION_TEST_NAME = 'unified_runtime_compaction_reaches_live_stepfun_without_discarding_history';
const PRODUCT_TEST_NAME = 'nomi_core_product_chain_reaches_live_stepfun_and_remote_binding';
const GLOBAL_TIMEOUT_MS = 30 * 60 * 1000;
const CARGO_OUTPUT_LIMIT_BYTES = 32 * 1024 * 1024;
const TEST_OUTPUT_LIMIT_BYTES = 8 * 1024 * 1024;
const FAILURE_SENTINEL =
  /^NOMIFUN_LIVE_SMOKE_FAILURE phase=([a-z0-9_.-]+) code=([A-Z0-9_]+) status=([0-9]{3})$/;
const compileOnly = process.argv.includes('--compile-only');
const selfTest = process.argv.includes('--self-test');
const modelSmoke = process.argv.includes('--model-smoke');
const compactionSmoke = process.argv.includes('--compaction-smoke');
const beforeToolSmoke = process.argv.includes('--before-tool-smoke');
const retainNativeFixture = process.argv.includes('--retain-native-fixture');
const globalDeadline = Date.now() + GLOBAL_TIMEOUT_MS;

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
  { environment, input = null, outputLimitBytes, onStdoutLine = null },
) {
  const remainingMs = globalDeadline - Date.now();
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
    child.stderr.on('data', collect(stderr));
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
      })) : [],
    };
  } catch { return null; }
}
function browserProofFromOutput(output) {
  const prefix = 'BROWSER_WORKSPACE_SMOKE_PASS ';
  const line = output.split(/\r?\n/).find(line => line.startsWith(prefix) && line.length < 16000);
  try {
    const proof = JSON.parse(line?.slice(prefix.length) ?? 'null');
    return proof?.scope === 'live-agent-only' && proof.native_fixture_profile_cleanup === true && proof.agent?.real_provider === true && proof.agent?.reproduced_bug_with_trusted_click === true && proof.agent?.workspace_code_changed_and_reloaded === true && proof.agent?.terminal_before_unlock === true && JSON.stringify(proof.agent?.trusted_retest_values) === '[1,2,3]';
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

async function main() {
  const userArgs = process.argv.slice(2);
  const allowedFlags = ['--compile-only', '--self-test', '--browser', '--browser-gui', '--model-smoke', '--compaction-smoke', '--before-tool-smoke', '--retain-native-fixture'];
  if (userArgs.some((arg, index) => {
    if (arg === '--data-dir') return !browserGui || !userArgs[index + 1] || userArgs[index + 1].startsWith('--');
    if (index > 0 && userArgs[index - 1] === '--data-dir') return false;
    return !allowedFlags.includes(arg);
  }) || userArgs.filter(arg => arg === '--data-dir').length > 1) {
    emitFailure('live_smoke_status=not_run', 'RUNNER_ARGUMENT_INVALID', 400);
    process.exitCode = 2;
    return;
  }
  if (([browser, browserGui, modelSmoke, compactionSmoke, beforeToolSmoke].filter(Boolean).length > 1) || (retainNativeFixture && !beforeToolSmoke)) {
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
  const model = process.env[MODEL_ENVIRONMENT_NAME] ?? DEFAULT_MODEL;
  if (!ALLOWED_MODELS.has(model)) {
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
    if ([RETAIN_FIXTURE_ENVIRONMENT_NAME, FIXTURE_PARENT_ENVIRONMENT_NAME].includes(name.toUpperCase())) delete environment[name];
  }
  if (retainNativeFixture) {
    environment[RETAIN_FIXTURE_ENVIRONMENT_NAME] = '1';
    environment[FIXTURE_PARENT_ENVIRONMENT_NAME] = fixtureParent;
  }

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
      'CARGO_BUILD_FAILED',
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
  try {
    console.log(`live_smoke_phase=execute mode=${browserGui ? 'browser_gui' : browser ? 'browser_frontend' : modelSmoke ? 'selected_model' : beforeToolSmoke ? 'before_tool' : compactionSmoke ? 'runtime_compaction' : 'product_chain'} model=${model}`);
    test = await runCaptured(
      executable,
      browserGui ? [guiDataDir, '--live-frontend'] : browser ? ['--live-agent-only'] : [
        modelSmoke ? MODEL_TEST_NAME : beforeToolSmoke ? BEFORE_TOOL_TEST_NAME : compactionSmoke ? COMPACTION_TEST_NAME : PRODUCT_TEST_NAME,
        '--exact',
        '--ignored',
        '--test-threads=1',
        '--nocapture',
      ],
      {
        environment,
        input: credentialInput,
        outputLimitBytes: TEST_OUTPUT_LIMIT_BYTES,
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
      console.log('browser_live_frontend_status=pass native_click=true workspace_fix=true retest=1,2,3 terminal_unlock=true');
      console.log('live_smoke_status=pass code=OK status=200');
      process.exitCode = 0;
      return;
    }
    // libtest exits successfully even when an exact filter matches zero tests.
    const selected = modelSmoke ? MODEL_TEST_NAME : beforeToolSmoke ? BEFORE_TOOL_TEST_NAME : compactionSmoke ? COMPACTION_TEST_NAME : PRODUCT_TEST_NAME;
    if (!selectedTestPassed(test.stdout, selected)) {
      emitFailure('live_smoke_status=not_run', 'SELECTED_TEST_DID_NOT_PASS', 503);
      process.exitCode = 2;
      return;
    }
    if (beforeToolSmoke && (stages.length !== BEFORE_TOOL_STAGE_PHASES.length ||
        BEFORE_TOOL_STAGE_PHASES.some((phase, index) => stages[index] !== phase))) {
      emitFailure('live_smoke_status=fail', 'BEFORE_TOOL_STAGE_EVIDENCE_INCOMPLETE', 503);
      process.exitCode = 2;
      return;
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
    console.log(`live_smoke_mode=${modelSmoke ? 'selected_model' : beforeToolSmoke ? 'before_tool' : compactionSmoke ? 'runtime_compaction' : 'product_chain'} model=${model}`);
    console.log('live_smoke_status=pass code=OK status=200');
    process.exitCode = 0;
    return;
  }

  const typed = typedFailureFromOutput(`${test.stdout}\n${test.stderr}`);
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

function runSelfTest() {
  const proof = 'BROWSER_WORKSPACE_SMOKE_PASS ' + JSON.stringify({ scope: 'live-agent-only', native_fixture_profile_cleanup: true, agent: { real_provider: true, reproduced_bug_with_trusted_click: true, workspace_code_changed_and_reloaded: true, terminal_before_unlock: true, trusted_retest_values: [1,2,3] } });
  if (!browserProofFromOutput(proof) || browserProofFromOutput('') || browserProofFromOutput(proof.replace('[1,2,3]', '[2,4,6]'))) throw new Error('browser proof parsing failed');
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
      !selectedTestPassed(`test ${COMPACTION_TEST_NAME} ... ok\r\n`, COMPACTION_TEST_NAME) ||
      selectedTestPassed('running 0 tests\ntest result: ok. 0 passed;', MODEL_TEST_NAME) ||
      selectedTestPassed(`test ${PRODUCT_TEST_NAME} ... ok`, MODEL_TEST_NAME)) {
    throw new Error('exact test execution proof self-test failed');
  }
  if (!ALLOWED_MODELS.has('step-3.7-flash') ||
      ['step-3.77-flash', 'step-3.7-flash\n', 'step-3.7-flash ', '', 'https://example.invalid/v1', 'arbitrary-model'].some((model) => ALLOWED_MODELS.has(model))) {
    throw new Error('model allowlist self-test failed');
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
