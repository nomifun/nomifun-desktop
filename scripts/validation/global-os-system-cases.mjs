// macOS-only, reversible acceptance cases for the existing native-Agent driver.
// This module describes ordinary file/process tool calls. It neither starts an
// Agent backend nor performs system mutations from the driver process. Native
// helper commands below are read-only observations of those actual tool calls.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { lstat, readFile, realpath } from 'node:fs/promises';
import { createServer } from 'node:net';
import { join, relative } from 'node:path';
import { promisify } from 'node:util';

const execute = promisify(execFile);
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const fixturePattern = /^\/private\/tmp\/nomifun-global-os-product-[0-9a-f-]{36}$/;
const requireExited = value => {
  assert.equal(value.state, 'exited');
  assert.equal(value.exit_code, 0);
  assert.equal(value.cleanup.reaped, true);
};
const textOf = value => {
  assert.equal(typeof value.content, 'string', 'A real file read must return its content');
  return value.content;
};
const xmlText = value => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;')
  .replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll("'", '&apos;');
const notPresent = async path => lstat(path).then(() => false, error => {
  if (error.code === 'ENOENT') return true;
  throw error;
});

async function nativeObservation(program, args) {
  try {
    const result = await execute(program, args, { encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024 });
    return { program, args, exit_code: 0, stdout: result.stdout, stderr: result.stderr };
  } catch (error) {
    if (Number.isInteger(error.code)) return {
      program, args, exit_code: error.code, stdout: error.stdout ?? '', stderr: error.stderr ?? '',
    };
    throw error;
  }
}

function processIsGone(pid) {
  try { process.kill(pid, 0); return false; }
  catch (error) { if (error.code === 'ESRCH') return true; throw error; }
}

/** Allocate a currently unused IPv4 loopback port without starting the service.
 * The reservation ends on close; the later nc bind is authoritative. A bind
 * collision must fail that run, never connect to or kill an unrelated service.
 */
export async function allocateLoopbackPort() {
  const reservation = createServer();
  await new Promise((resolve, reject) => {
    reservation.once('error', reject);
    reservation.listen({ host: '127.0.0.1', port: 0, exclusive: true }, resolve);
  });
  const port = reservation.address().port;
  await new Promise((resolve, reject) => reservation.close(error => error ? reject(error) : resolve()));
  return port;
}

// launchctl print is deliberately queried for this one unique label only. Do
// not capture the user's whole GUI domain/service inventory in acceptance logs.
const absentServiceScript = [
  'set +e',
  'output=$(/bin/launchctl print "$1" 2>&1)',
  'status=$?',
  'printf "%s\\n" "$output"',
  'if [ "$status" -eq 0 ]; then printf "%s\\n" "Unexpected registered test service" >&2; exit 72; fi',
  'case "$output" in',
  '  *"Could not find service \\\"$2\\\""*) printf "NOMIFUN_EXPECTED_SERVICE_ABSENT:%s\\n" "$status"; exit 0 ;;',
  'esac',
  'exit "$status"',
].join('\n');

const cleanupServiceScript = [
  'set +e',
  'output=$(/bin/launchctl print "$1" 2>&1)',
  'status=$?',
  'printf "NOMIFUN_CLEANUP_TARGET:%s\\n" "$1"',
  'printf "%s\\n" "$output"',
  'if [ "$status" -eq 0 ]; then /bin/launchctl bootout "$1"; exit $?; fi',
  'case "$output" in',
  '  *"Could not find service \\\"$2\\\""*) printf "NOMIFUN_EXPECTED_SERVICE_ABSENT:%s\\n" "$status"; exit 0 ;;',
  'esac',
  'exit "$status"',
].join('\n');

// This command can only delete this scenario's UUID domain after the driver
// observed that exact domain absent before any write. A negative read is an
// expected absence observation only when defaults names that exact domain.
const cleanupPreferencesScript = [
  'set +e',
  'output=$(/usr/bin/defaults read "$1" 2>&1)',
  'status=$?',
  'printf "NOMIFUN_PREFERENCES_TARGET:%s\\n" "$1"',
  'printf "%s\\n" "$output"',
  'if [ "$status" -eq 0 ]; then',
  '  if [ "$2" != 1 ]; then printf "%s\\n" "Initial preferences absence was not proven; no domain was deleted" >&2; exit 73; fi',
  '  /usr/bin/defaults delete "$1"; exit $?',
  'fi',
  'case "$output" in',
  '  *"Domain $1 does not exist"*) printf "NOMIFUN_EXPECTED_PREFERENCES_ABSENT:%s\\n" "$status"; exit 0 ;;',
  'esac',
  'exit "$status"',
].join('\n');

/**
 * Steps use the driver's existing {id,name,args,verify} interface. Repeated
 * readiness checks require a fresh tool-call ID; they never repeat a mutation.
 *
 * preserveServiceForHostExit is an explicit two-phase test: this Turn leaves
 * only its unique launchd job registered, the driver proves App exit separately,
 * observeAfterAppExit checks native survival, and a new clean canonical Turn
 * executes cleanupSteps after the App restarts. The default single-run case
 * cleans everything and explicitly leaves App-exit persistence unverified.
 */
export function createMacSystemScenario({ fixture, workspace, uid, loopbackPort, preserveServiceForHostExit = false }) {
  assert.equal(process.platform, 'darwin', 'This scenario uses real macOS launchd and Apple nc');
  assert.notEqual(process.getuid(), 0, 'The acceptance driver must be the ordinary login user');
  assert(fixturePattern.test(fixture), 'Only the driver-allocated unique fixture namespace is accepted');
  assert.equal(workspace, join(fixture, 'workspace'));
  assert.equal(uid, process.getuid(), 'gui domain must belong to the actual login driver');
  assert(Number.isInteger(loopbackPort) && loopbackPort > 1023 && loopbackPort <= 65535);
  const nonce = randomUUID();
  const root = join(fixture, `system-cases-${nonce}`);
  const config = join(root, 'config.json');
  const plist = join(root, 'service.plist');
  const serviceMarker = join(root, 'service-marker.txt');
  const label = `com.nomifun.acceptance.global-os.${nonce}`;
  const domain = `gui/${uid}`;
  const target = `${domain}/${label}`;
  const preferencesDomain = `com.nomifun.validation.globalos.${nonce}`;
  assert(/^com\.nomifun\.validation\.globalos\.[0-9a-f-]{36}$/.test(preferencesDomain));
  const userToolSource = join(root, 'source', 'tool.sh');
  const userToolBin = join(root, 'bin');
  const userToolName = `nomifun-global-os-tool-${nonce}`;
  const userToolInstalled = join(userToolBin, userToolName);
  const userToolMarker = `NOMIFUN_USER_TOOL_${nonce}`;
  const userToolScript = `#!/bin/sh\nset -eu\nprintf '%s\\n' '${userToolMarker}'\nprintf '<%s>\\n' "$@"\n`;
  const userToolArgs = ['', '中文 空格', "single' and \"double\" quote", '$HOME;$(printf NOMIFUN_SHOULD_STAY_LITERAL)&|<>*?%!', 'back\\slash'];
  const userToolExpected = `${userToolMarker}\n${userToolArgs.map(argument => `<${argument}>\n`).join('')}`;
  const markerValue = `NOMIFUN_LAUNCHD_${nonce}`;
  const networkBody = `NOMIFUN_LOOPBACK_${nonce}\n`;
  const networkPath = `/nomifun-${nonce}`;
  const originalConfig = `${JSON.stringify({ fixture: nonce, mode: 'before', loopback: '127.0.0.1', port: loopbackPort })}\n`;
  const changedConfig = originalConfig.replace('"mode":"before"', '"mode":"after"');
  const response = `HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: ${Buffer.byteLength(networkBody)}\r\nConnection: close\r\n\r\n${networkBody}`;
  const nativeEvidence = [];
  let servicePid, networkPid, networkProcessId, networkCursor = 0, networkOutput = '', preferencesAbsentBefore = false;

  const requireFixture = async () => {
    assert.equal(await realpath(fixture), fixture);
    const info = await lstat(fixture);
    assert(info.isDirectory() && !info.isSymbolicLink());
    assert.equal(info.uid, uid); assert.equal(info.mode & 0o777, 0o700);
    assert.equal(relative(workspace, root).startsWith('..'), true, 'System fixtures must be outside the Session workspace');
  };
  const observeService = async ({ absent = false } = {}) => {
    const observation = await nativeObservation('/bin/launchctl', ['print', target]);
    nativeEvidence.push({ phase: absent ? 'service_absence' : 'service_registered', ...observation });
    const combined = `${observation.stdout}${observation.stderr}`;
    if (absent) {
      assert.notEqual(observation.exit_code, 0);
      assert(combined.includes(`Could not find service "${label}"`), 'Failure must prove exact service absence, not a missing GUI session or access denial');
    } else {
      assert.equal(observation.exit_code, 0);
      assert.match(observation.stdout, /state = running/);
      assert(observation.stdout.includes(`pid = ${servicePid}`));
      process.kill(servicePid, 0);
    }
    return observation;
  };
  const observePreferences = async ({ absent = false, phase } = {}) => {
    const observation = await nativeObservation('/usr/bin/defaults', absent ? ['read', preferencesDomain] : ['read', preferencesDomain, 'FixtureEnabled']);
    nativeEvidence.push({ phase: phase ?? (absent ? 'preferences_absent' : 'preferences_present'), preferences_domain: preferencesDomain, ...observation });
    if (absent) {
      assert.notEqual(observation.exit_code, 0);
      assert(`${observation.stdout}${observation.stderr}`.includes(`Domain ${preferencesDomain} does not exist`), 'Expected negative defaults read must identify this exact absent domain, not authentication, I/O or tool failure');
    } else {
      assert.equal(observation.exit_code, 0);
      assert.equal(observation.stdout.trim(), '1');
    }
    return observation;
  };
  const observeListener = async () => {
    const observation = await nativeObservation('/usr/sbin/lsof', ['-nP', '-a', '-p', String(networkPid), `-iTCP@127.0.0.1:${loopbackPort}`, '-sTCP:LISTEN', '-Fpn']);
    nativeEvidence.push({ phase: 'network_listener', ...observation });
    if (observation.exit_code === 1 && !observation.stdout.trim() && !observation.stderr.trim()) return false;
    assert.equal(observation.exit_code, 0, 'The exact nc listener must be independently observable');
    assert(observation.stdout.split('\n').includes(`p${networkPid}`));
    assert(observation.stdout.split('\n').includes(`n127.0.0.1:${loopbackPort}`), 'Do not accept wildcard/LAN listeners or another process');
    return true;
  };
  const serviceScript = 'set -eu; umask 077; set -C; { printf "%s\\n" "$2"; /usr/bin/id -u; /bin/pwd; printf "%s\\n" "$$"; } > "$1"; exec /bin/sleep 900';
  const programArguments = ['/bin/sh', '-c', serviceScript, 'nomifun-global-os-service', serviceMarker, markerValue];
  const plistContent = `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>\n<key>Label</key><string>${xmlText(label)}</string>\n<key>ProgramArguments</key><array>${programArguments.map(value => `<string>${xmlText(value)}</string>`).join('')}</array>\n<key>WorkingDirectory</key><string>${xmlText(root)}</string>\n<key>RunAtLoad</key><false/>\n<key>KeepAlive</key><false/>\n<key>AbandonProcessGroup</key><false/>\n<key>ProcessType</key><string>Background</string>\n<key>Umask</key><integer>63</integer>\n<key>StandardOutPath</key><string>${xmlText(join(root, 'service.stdout'))}</string>\n<key>StandardErrorPath</key><string>${xmlText(join(root, 'service.stderr'))}</string>\n</dict></plist>\n`;
  const execStep = (id, command, args, verify, extra = {}) => ({ id: `system-${id}`, name: 'exec_command', args: { command, args, timeout_ms: 15000 }, verify: async value => { requireExited(value); await verify?.(value); }, ...extra });
  const absentStep = id => execStep(id, '/bin/sh', ['-c', absentServiceScript, 'nomifun-exact-service-absence', target, label], async value => {
    assert.match(value.output.text, /NOMIFUN_EXPECTED_SERVICE_ABSENT:[1-9]\d*/);
    await observeService({ absent: true });
  });
  const awaitServiceExit = async () => {
    await observeService({ absent: true });
    if (!servicePid) return;
    for (let attempt = 0; attempt < 50; attempt++) {
      if (processIsGone(servicePid)) return;
      await delay(100);
    }
    throw new Error('launchd service registration is absent but its exact observed PID has not exited; retain evidence');
  };
  const cleanupSteps = [
    execStep('cleanup-preferences', '/bin/sh', [], async value => {
      assert(value.output.text.startsWith(`NOMIFUN_PREFERENCES_TARGET:${preferencesDomain}\n`));
      nativeEvidence.push({ phase: 'preferences_failure_cleanup', preferences_domain: preferencesDomain, stdout: value.output.text });
      await observePreferences({ absent: true, phase: 'preferences_absent_after_failure_cleanup' });
    }, { args: () => ({ command: '/bin/sh', args: ['-c', cleanupPreferencesScript, 'nomifun-exact-preferences-cleanup', preferencesDomain, preferencesAbsentBefore ? '1' : '0'], timeout_ms: 15000 }) }),
    execStep('cleanup-service', '/bin/sh', ['-c', cleanupServiceScript, 'nomifun-exact-service-cleanup', target, label], async value => {
      assert(value.output.text.startsWith(`NOMIFUN_CLEANUP_TARGET:${target}\n`));
      // Kickstart's receipt may have been lost before its PID reached this
      // driver. Only the exact unique label's native print can supply it now.
      const observedPid = value.output.text.match(/(?:^|\n)\s*pid = ([1-9]\d*)\s*(?:\n|$)/)?.[1];
      if (observedPid) {
        if (servicePid) assert.equal(Number(observedPid), servicePid, 'The observed service identity changed; preserve evidence instead of guessing');
        servicePid = Number(observedPid);
      } else if (!servicePid && !value.output.text.includes('NOMIFUN_EXPECTED_SERVICE_ABSENT:')) {
        assert.match(value.output.text, /state = not running/, 'Without a captured PID or native not-running state, independent service cleanup is unproven');
      }
      nativeEvidence.push({ phase: 'failure_cleanup_service_identity', service_target: target, observed_pid: servicePid, stdout: value.output.text });
      await awaitServiceExit();
    }),
    absentStep('cleanup-service-absence'),
    { id: 'system-cleanup-files', name: 'delete_path', args: { path: root }, verify: async () => assert(await notPresent(root)) },
  ];
  const steps = [
    { id: 'system-instruction-scope', name: 'read_file', args: { path: fixture, format: 'instruction_scope', recursive: true }, verify: value => {
      assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true);
    } },
    execStep('preflight', '/bin/sh', ['-c', 'set -eu; for tool in /bin/launchctl /usr/bin/defaults /usr/bin/install /usr/bin/nc /usr/bin/curl /usr/sbin/lsof /bin/mkdir /bin/sleep /bin/pwd /usr/bin/id; do if [ ! -x "$tool" ]; then printf "Required native executable unavailable: %s\\n" "$tool" >&2; exit 69; fi; done; /usr/bin/id -u', 'nomifun-system-prerequisites'], async value => {
      assert.equal(value.output.text, `${uid}\n`); await requireFixture(); assert(await notPresent(root));
      await observePreferences({ absent: true, phase: 'preferences_absent_before_write' }); preferencesAbsentBefore = true;
    }),
    absentStep('service-absent-before'),
    { id: 'system-config-target-instruction-scope', name: 'read_file', args: { path: config, format: 'instruction_scope' }, verify: value => {
      assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true);
    } },
    { id: 'system-config-write', name: 'write_file', args: { path: config, content: originalConfig }, verify: async () => assert.equal(await readFile(config, 'utf8'), originalConfig) },
    { id: 'system-config-change', name: 'apply_patch', args: { files: [{ path: config, expected_source: { kind: 'existing', sha256: createHash('sha256').update(originalConfig).digest('hex') }, hunks: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, lines: [{ kind: 'remove', text: originalConfig.trimEnd() }, { kind: 'add', text: changedConfig.trimEnd() }] }] }] }, verify: async () => assert.equal(await readFile(config, 'utf8'), changedConfig) },
    { id: 'system-config-read', name: 'read_file', args: { path: config }, verify: value => assert.equal(textOf(value), changedConfig) },
    { id: 'system-user-tool-instruction-scope', name: 'read_file', args: { path: userToolSource, format: 'instruction_scope' }, verify: value => {
      assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true);
    } },
    { id: 'system-user-tool-source-write', name: 'write_file', args: { path: userToolSource, content: userToolScript }, verify: async () => {
      assert.equal(await readFile(userToolSource, 'utf8'), userToolScript); assert(await notPresent(userToolBin));
    } },
    execStep('user-tool-bin-create', '/bin/mkdir', ['-m', '755', userToolBin]),
    execStep('user-tool-install', '/usr/bin/install', ['-m', '755', userToolSource, userToolInstalled], async () => {
      const info = await lstat(userToolInstalled);
      assert(info.isFile() && !info.isSymbolicLink()); assert.equal(info.uid, uid); assert.equal(info.mode & 0o7777, 0o755);
      const installed = await readFile(userToolInstalled, 'utf8'); assert.equal(installed, userToolScript);
      nativeEvidence.push({ phase: 'user_tool_installed', source: userToolSource, path: userToolInstalled, uid: info.uid, mode: (info.mode & 0o7777).toString(8), sha256: createHash('sha256').update(installed).digest('hex') });
    }),
    execStep('user-tool-bare-command', userToolName, userToolArgs, value => assert.equal(value.output.text, userToolExpected), { args: { command: userToolName, args: userToolArgs, env: { PATH: `${userToolBin}:/usr/bin:/bin` }, timeout_ms: 15000 } }),
    { id: 'system-user-tool-uninstall', name: 'delete_path', args: { path: userToolInstalled }, verify: async () => {
      assert(await notPresent(userToolInstalled)); nativeEvidence.push({ phase: 'user_tool_uninstalled', path: userToolInstalled, absent: true });
    } },
    execStep('preferences-write', '/usr/bin/defaults', [], async () => {
      await observePreferences({ phase: 'preferences_present_after_write' });
    }, { args: () => {
      assert(preferencesAbsentBefore, 'The exact random preference domain must be confirmed absent before a write');
      return { command: '/usr/bin/defaults', args: ['write', preferencesDomain, 'FixtureEnabled', '-bool', 'true'], timeout_ms: 15000 };
    } }),
    execStep('preferences-read', '/usr/bin/defaults', ['read', preferencesDomain, 'FixtureEnabled'], value => assert.equal(value.output.text.trim(), '1')),
    execStep('preferences-delete', '/usr/bin/defaults', ['delete', preferencesDomain], async () => {
      await observePreferences({ absent: true, phase: 'preferences_absent_after_delete' });
    }),
    { id: 'system-plist-write', name: 'write_file', args: { path: plist, content: plistContent }, verify: async () => assert.equal(await readFile(plist, 'utf8'), plistContent) },
    execStep('service-bootstrap', '/bin/launchctl', ['bootstrap', domain, plist]),
    execStep('service-kickstart', '/bin/launchctl', ['kickstart', '-p', target], async value => {
      assert.match(value.output.text.trim(), /^[1-9]\d*$/); servicePid = Number(value.output.text.trim());
      let marker;
      const expectedMarker = `${markerValue}\n${uid}\n${root}\n${servicePid}\n`;
      for (let attempt = 0; attempt < 30; attempt++) {
        if (!await notPresent(serviceMarker)) marker = await readFile(serviceMarker, 'utf8');
        if (marker === expectedMarker) break;
        await delay(100);
      }
      assert.equal(marker, expectedMarker);
      await observeService();
      let parent;
      const expectedIdentity = new RegExp(`^\\s*1\\s+${uid}\\s+sleep\\s*$`);
      for (let attempt = 0; attempt < 30; attempt++) {
        parent = await nativeObservation('/bin/ps', ['-p', String(servicePid), '-o', 'ppid=', '-o', 'uid=', '-o', 'ucomm=']);
        nativeEvidence.push({ phase: 'launchd_process_identity', ...parent }); assert.equal(parent.exit_code, 0);
        if (expectedIdentity.test(parent.stdout)) break;
        await delay(100);
      }
      assert.match(parent.stdout, expectedIdentity, 'The service must be launchd-owned, not a detached Agent descendant');
    }),
    execStep('service-print', '/bin/launchctl', ['print', target], value => { assert.match(value.output.text, /state = running/); assert(value.output.text.includes(`pid = ${servicePid}`)); }),
    { id: 'system-service-marker-read', name: 'read_file', args: { path: serviceMarker }, verify: value => assert.equal(textOf(value), `${markerValue}\n${uid}\n${root}\n${servicePid}\n`) },
    { id: 'system-network-start', name: 'start_process', args: { command: '/usr/bin/nc', args: ['-4', '-l', '127.0.0.1', String(loopbackPort)], cwd: root, wait_ms: 0, timeout_ms: 60000 }, verify: value => {
      assert.equal(value.state, 'running'); networkProcessId = value.process_id; networkPid = value.pid; networkCursor = value.output.next_cursor; networkOutput = value.output.text;
      assert(networkProcessId); assert(Number.isInteger(networkPid)); process.kill(networkPid, 0);
    } },
    { id: 'system-network-poll-running', name: 'poll_process', repeatable: true, maxAttempts: 12, args: () => ({ process_id: networkProcessId, cursor: networkCursor, wait_ms: 100 }), verify: async value => {
      assert.equal(value.state, 'running'); networkCursor = value.output.next_cursor; networkOutput += value.output.text;
      const listening = await observeListener(); if (!listening) await delay(100); return listening;
    } },
    { id: 'system-network-response', name: 'write_process_stdin', args: () => ({ process_id: networkProcessId, input: response }), verify: value => {
      assert.equal(value.state, 'running'); networkCursor = value.output.next_cursor; networkOutput += value.output.text;
    } },
    execStep('network-request', '/usr/bin/curl', ['--noproxy', '*', '--fail', '--silent', '--show-error', '--max-time', '10', `http://127.0.0.1:${loopbackPort}${networkPath}`], value => assert.equal(value.output.text, networkBody)),
    { id: 'system-network-stop', name: 'cancel_process', args: () => ({ process_id: networkProcessId }), verify: value => {
      assert(['cancelled', 'exited'].includes(value.state)); assert.equal(value.cleanup.reaped, true);
      if (value.state === 'exited') assert.equal(value.exit_code, 0);
      networkCursor = value.output.next_cursor; networkOutput += value.output.text; assert(networkOutput.includes(`GET ${networkPath} HTTP/1.1`));
      assert(networkOutput.includes(`Host: 127.0.0.1:${loopbackPort}`)); assert(processIsGone(networkPid));
    } },
    { id: 'system-network-confirm-exit', name: 'poll_process', allowCancelledObservation: true, args: () => ({ process_id: networkProcessId, cursor: networkCursor, wait_ms: 0 }), verify: async value => {
      assert(['cancelled', 'exited'].includes(value.state)); assert.equal(value.cleanup.reaped, true); assert(processIsGone(networkPid));
      const observation = await nativeObservation('/usr/sbin/lsof', ['-nP', '-a', '-p', String(networkPid), `-iTCP@127.0.0.1:${loopbackPort}`, '-sTCP:LISTEN', '-Fpn']);
      nativeEvidence.push({ phase: 'network_cleanup', ...observation }); assert.equal(observation.exit_code, 1); assert.equal(observation.stdout, ''); assert.equal(observation.stderr, '');
    } },
    { id: 'system-config-restore', name: 'write_file', args: { path: config, content: originalConfig }, verify: async () => assert.equal(await readFile(config, 'utf8'), originalConfig) },
    { id: 'system-config-restored-read', name: 'read_file', args: { path: config }, verify: value => assert.equal(textOf(value), originalConfig) },
    ...preserveServiceForHostExit ? [] : [
      execStep('service-bootout', '/bin/launchctl', ['bootout', target], awaitServiceExit),
      absentStep('service-absent-after'),
      { id: 'system-files-delete', name: 'delete_path', args: { path: root }, verify: async () => assert(await notPresent(root)) },
    ],
  ];
  const pendingAcceptance = [{
    id: 'launchd-survives-native-App-exit', status: 'not_run',
    reason: preserveServiceForHostExit ? 'This phase intentionally retains only the unique launchd fixture; the driver must prove App exit, independently observe survival, restart, then use cleanupSteps in a new settled Turn.' : 'The normal scenario keeps the native App running and cleans its launchd fixture. App-exit survival is not tested by registration, PPID 1, or a completed Agent Turn.',
  }, {
    id: 'native-network-security-settings', status: 'not_run',
    reason: 'The scenario changes an isolated configuration file and creates/reads/deletes one unique native CFPreferences test domain. It does not change daily preferences, network or security configuration and does not prove all OS setting categories.',
  }];
  return {
    title: preserveServiceForHostExit ? 'macOS isolated preferences/system/network acceptance, retained service phase' : 'macOS isolated native preferences/launchd/configuration/loopback acceptance',
    prompt: `只在宿主本次分配的 ${root} 创建测试配置和 launchd plist。用文件工具写入、patch、读回并恢复配置。以文件工具写入唯一测试工具脚本 ${userToolSource}，通过原生 /usr/bin/install -m 755 安装到本次 ${userToolInstalled}，用裸随机工具名 ${userToolName} 和本次 bin PATH 运行，核对空参数、中文、引号及元字符均保持literal，再用文件工具卸载并确认不存在；不下载或安装第三方工具，不更改日常PATH。宿主已分配唯一原生偏好测试域 ${preferencesDomain}，先独立确认不存在后，通过现役进程执行 /usr/bin/defaults write 该域 FixtureEnabled -bool true，defaults read 该键核对1，再 defaults delete 该唯一域并独立只读确认不存在；不改写任何既有偏好域。以普通登录用户 ${uid} 在 ${domain} 注册并启动唯一测试服务 ${label}，读回其标记和原生状态。通过受管进程在 127.0.0.1:${loopbackPort} 启动本次 nc 监听，用本机 curl --noproxy '*' 请求唯一测试路径，核对响应和请求输出，停止并确认清理监听。${preserveServiceForHostExit ? '按本测试的明确意图保留唯一 launchd 服务用于下一阶段 App 退出后的独立存续验证；不要声称该验证已经完成。' : '最后 bootout 唯一测试服务，核对原生状态不存在并删除全部本次测试文件。'}不修改任何既有服务、日常系统配置、代理、防火墙或其他网络监听，不联网下载。原生偏好测试域的成功不代表真实网络或安全配置已全面验收。`,
    steps, cleanupSteps, pendingAcceptance, nativeEvidence,
    resources: { root, config, plist, label, domain, target, serviceMarker, preferencesDomain, userToolSource, userToolInstalled, userToolName, loopbackPort, networkPath, retainedService: preserveServiceForHostExit },
    cleanupIntent: {
      prerequisite: 'Cancel the exact original canonical Turn and prove native cleanup, no active producer and zero pending/unknown effects before a new cleanup Turn. Never reuse a process_id from an earlier Turn.',
      temporary_process: () => ({ process_id: networkProcessId, observed_pid: networkPid, action: 'The original Turn owner performs cancellation and proven process-tree cleanup; the host script must not kill a raw PID.' }),
      native_preferences: { domain: preferencesDomain, action: 'Only after exact initial absence was proven, a new settled cleanup Turn may delete this UUID test domain if it exists; host observations remain read-only.' },
      persistent_service: { service_target: target, action: 'Execute cleanupSteps through the same default Agent/product process and file tools after the canonical prerequisite; preserve unknown state rather than deleting files underneath a possibly running service.' },
    },
    async observeAfterAppExit({ appPid }) {
      assert(preserveServiceForHostExit, 'Use the explicit retained-service phase to test App-exit persistence');
      assert(Number.isInteger(appPid) && appPid > 1 && appPid !== process.pid);
      assert(processIsGone(appPid), 'The real native App PID must have exited before survival can be claimed');
      assert(Number.isInteger(servicePid)); await observeService();
      assert.equal(await readFile(serviceMarker, 'utf8'), `${markerValue}\n${uid}\n${root}\n${servicePid}\n`);
      return { status: 'passed', app_pid: appPid, service_pid: servicePid, service_target: target, cleanup_required: true };
    },
  };
}
