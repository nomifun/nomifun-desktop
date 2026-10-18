import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { execFile } from 'node:child_process';
import { lstat, readFile, readdir, readlink } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { promisify } from 'node:util';

const execute = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const absent = path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const gone = pid => { try { process.kill(pid, 0); return false; } catch (error) { if (error.code === 'ESRCH') return true; throw error; } };
const textOf = value => value.text ?? value.content;
const exited = value => { assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup.reaped, true); };
const observe = async (command, args) => {
  try { const value = await execute(command, args, { encoding: 'utf8', timeout: 10000, maxBuffer: 256 * 1024 }); return { command, args, exit_code: 0, ...value }; }
  catch (error) { if (!Number.isInteger(error.code)) throw error; return { command, args, exit_code: error.code, stdout: error.stdout ?? '', stderr: error.stderr ?? '' }; }
};
const descriptorTargets = async pid => {
  const root = `/proc/${pid}/fd`;
  return (await Promise.all((await readdir(root)).map(async name => {
    try { return await readlink(join(root, name)); } catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
  }))).filter(Boolean);
};

export async function observeLinuxListener({ pid, port }) {
  assert(Number.isSafeInteger(pid) && pid > 1 && Number.isInteger(port) && port > 0 && port < 65536);
  const descriptors = await descriptorTargets(pid);
  const expected = `0100007F:${port.toString(16).toUpperCase().padStart(4, '0')}`;
  const rows = (await readFile(`/proc/${pid}/net/tcp`, 'utf8')).trim().split('\n').slice(1);
  return rows.some(line => {
    const columns = line.trim().split(/\s+/);
    return columns[1] === expected && columns[3] === '0A' && descriptors.includes(`socket:[${columns[9]}]`);
  });
}

export async function verifyLinuxNativeApp({ pid, port, data, uid }) {
  assert.equal(process.platform, 'linux'); assert(Number.isSafeInteger(uid) && uid > 0);
  const executable = await readlink(`/proc/${pid}/exe`);
  assert.equal(basename(executable), 'nomifun-desktop', 'Announcement must identify the real native App executable');
  assert.equal((await lstat(`/proc/${pid}`)).uid, uid, 'Native App must run as the ordinary fixture user');
  assert((await descriptorTargets(pid)).includes(join(data, 'nomifun-backend.db')), 'The exact native fixture database must be open in the App');
  assert(await observeLinuxListener({ pid, port }), 'The exact loopback announcement must be an App-owned socket');
  return { pid, uid, executable, database_owned: true, loopback_listener_owned: true };
}

// Effects are exclusively submitted by the public canonical Agent driver.
// The host only allocates names and observes exact fixture identities read-only.
export function createLinuxSystemScenario({ fixture, workspace, uid, loopbackPort, preserveServiceForHostExit = false }) {
  assert.equal(process.platform, 'linux'); assert(uid > 0);
  assert(/^\/tmp\/nomifun-global-os-product-[0-9a-f-]{36}$/.test(fixture), 'Only a host-allocated unique temporary fixture is accepted');
  assert.equal(workspace, join(fixture, 'workspace'));
  const root = join(fixture, 'linux-system');
  const config = join(root, 'config', 'fixture.conf');
  const source = join(root, 'tool-source.sh'), bin = join(root, 'bin');
  const suffix = randomUUID().replaceAll('-', '');
  const userToolName = `nomifun-fixture-${suffix}`, installed = join(bin, userToolName);
  const target = `nomifun-global-os-${suffix}.service`, serviceMarker = join(root, 'service-marker');
  const managerKey = `NOMIFUN_GLOBAL_OS_${suffix.toUpperCase()}`, managerValue = `fixture_${suffix}`;
  const networkPath = `/nomifun-${suffix}`, networkBody = `NOMIFUN_LOOPBACK_${suffix}\n`;
  const original = 'fixture_enabled=false\n', changed = 'fixture_enabled=true\n';
  const literalArgs = ['', '中文', 'two words', 'quote"value', '$HOME;touch nope', "single'quote"];
  const toolScript = '#!/bin/sh\nset -eu\nprintf "NOMIFUN_USER_TOOL\\n"\nfor arg do printf "<%s>\\n" "$arg"; done\n';
  const toolExpected = `NOMIFUN_USER_TOOL\n${literalArgs.map(arg => `<${arg}>\n`).join('')}`;
  const nativeEvidence = [];
  let servicePid, managerAbsentBefore = false, networkPid, networkProcessId, cursor = 0, networkOutput = '';
  const managerObservation = async expected => {
    const value = await observe('/usr/bin/systemctl', ['--user', 'show-environment']);
    assert.equal(value.exit_code, 0, 'A real user systemd manager/bus is required');
    const matching = value.stdout.split('\n').filter(line => line.startsWith(`${managerKey}=`));
    assert.deepEqual(matching, expected ? [`${managerKey}=${managerValue}`] : []);
    nativeEvidence.push({ phase: expected ? 'manager_configuration_present' : 'manager_configuration_absent', key: managerKey, matching, exit_code: value.exit_code });
  };
  const serviceObservation = async expected => {
    const value = await observe('/usr/bin/systemctl', ['--user', 'show', '--property=LoadState,ActiveState,SubState,MainPID,Transient,ControlGroup', '--', target]);
    assert.equal(value.exit_code, 0);
    const properties = Object.fromEntries(value.stdout.trim().split('\n').map(line => { const i = line.indexOf('='); return [line.slice(0, i), line.slice(i + 1)]; }));
    nativeEvidence.push({ phase: expected ? 'systemd_service_running' : 'systemd_service_absent', target, ...value });
    if (!expected) { assert.equal(properties.LoadState, 'not-found'); assert.equal(properties.MainPID, '0'); return; }
    assert.equal(properties.LoadState, 'loaded'); assert.equal(properties.ActiveState, 'active'); assert.equal(properties.Transient, 'yes');
    const pid = Number(properties.MainPID); assert(Number.isSafeInteger(pid) && pid > 1);
    if (servicePid) assert.equal(pid, servicePid, 'A changed service identity is not the original fixture');
    servicePid = pid;
    assert.equal((await lstat(`/proc/${pid}`)).uid, uid);
    assert((await readFile(`/proc/${pid}/cgroup`, 'utf8')).includes(target), 'Service must belong to its registered systemd cgroup');
    assert.equal(await readlink(`/proc/${pid}/cwd`), root);
    assert.equal(await readFile(serviceMarker, 'utf8'), `${suffix}\n${uid}\n${root}\n${pid}\n`);
  };
  const serviceGone = async () => {
    for (let i = 0; i < 50; i++) {
      try { await serviceObservation(false); if (!servicePid || gone(servicePid)) return; } catch (error) { if (i === 49) throw error; }
      await delay(100);
    }
    throw new Error('The exact systemd service/PID cleanup remains unproven');
  };
  const exec = (id, command, args, verify, extra = {}) => ({ id: `linux-${id}`, name: 'start_process', args: { command, args, timeout_ms: 15000 }, verify: async value => { exited(value); await verify?.(value); }, ...extra });
  const scope = (id, path) => ({ id: `linux-${id}`, name: 'read_file', args: { path, format: 'instruction_scope', recursive: true }, verify: value => { assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true); } });
  const serviceCleanupScript = 'set -eu; state=$(/usr/bin/systemctl --user show --property=LoadState --value -- "$1"); if [ "$state" != not-found ]; then /usr/bin/systemctl --user show --property=MainPID --value -- "$1"; /usr/bin/systemctl --user stop -- "$1"; else printf "NOMIFUN_ALREADY_ABSENT\\n"; fi';
  const cleanupService = () => exec('service-stop', '/bin/sh', ['-c', serviceCleanupScript, 'nomifun-exact-service-cleanup', target], async value => {
    const observed = value.output.text.trim();
    if (/^[1-9]\d*$/.test(observed)) { if (servicePid) assert.equal(Number(observed), servicePid); servicePid = Number(observed); }
    else assert(['0', 'NOMIFUN_ALREADY_ABSENT'].includes(observed), 'Cleanup must retain the exact native service identity');
    await serviceGone();
  });
  const cleanupManager = () => exec('manager-restore', '/usr/bin/systemctl', ['--user', 'unset-environment', managerKey], () => managerObservation(false), { args: () => {
    assert(managerAbsentBefore, 'Do not change a manager key whose initial absence was never proven');
    return { command: '/usr/bin/systemctl', args: ['--user', 'unset-environment', managerKey], timeout_ms: 15000 };
  } });
  const deleteFiles = () => ({ id: 'linux-files-delete', name: 'delete_path', args: { path: root }, verify: async () => assert(await absent(root)) });
  const installation = [
    scope('fixture-scope', fixture),
    exec('prerequisites', '/bin/sh', ['-c', 'set -eu; for p in /usr/bin/systemctl /usr/bin/systemd-run /usr/bin/python3 /usr/bin/install /usr/bin/curl; do test -x "$p"; done; /usr/bin/id -u'], async value => {
      assert.equal(value.output.text, `${uid}\n`); assert(await absent(root)); await managerObservation(false); managerAbsentBefore = true; await serviceObservation(false);
    }),
    scope('config-scope', config),
    { id: 'linux-config-write', name: 'write_file', args: { path: config, content: original }, verify: async () => assert.equal(await readFile(config, 'utf8'), original) },
    { id: 'linux-config-patch', name: 'apply_patch', args: { files: [{ path: config, expected_source: { kind: 'existing', sha256: createHash('sha256').update(original).digest('hex') }, hunks: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, lines: [{ kind: 'remove', text: original.trimEnd() }, { kind: 'add', text: changed.trimEnd() }] }] }] }, verify: async () => assert.equal(await readFile(config, 'utf8'), changed) },
    { id: 'linux-config-read', name: 'read_file', args: { path: config }, verify: value => assert.equal(textOf(value), changed) },
    scope('tool-source-scope', source),
    { id: 'linux-tool-source-write', name: 'write_file', args: { path: source, content: toolScript }, verify: async () => assert.equal(await readFile(source, 'utf8'), toolScript) },
    exec('tool-bin-create', '/bin/mkdir', ['-m', '755', bin]),
    exec('tool-install', '/usr/bin/install', ['-m', '755', source, installed], async () => {
      const metadata = await lstat(installed); assert(metadata.isFile() && !metadata.isSymbolicLink()); assert.equal(metadata.uid, uid); assert.equal(metadata.mode & 0o7777, 0o755); assert.equal(await readFile(installed, 'utf8'), toolScript);
      nativeEvidence.push({ phase: 'user_software_installed', path: installed, uid, mode: '755', sha256: createHash('sha256').update(toolScript).digest('hex') });
    }),
    exec('tool-bare-name', userToolName, literalArgs, value => assert.equal(value.output.text, toolExpected), { args: { command: userToolName, args: literalArgs, env: { PATH: `${bin}:/usr/bin:/bin` }, timeout_ms: 15000 } }),
    { id: 'linux-tool-uninstall', name: 'delete_path', args: { path: installed }, verify: async () => assert(await absent(installed)) },
  ];
  const serviceScript = 'set -eu; umask 077; set -C; { printf "%s\\n" "$2"; /usr/bin/id -u; /bin/pwd; printf "%s\\n" "$$"; } > "$1.pending"; /usr/bin/mv -T -- "$1.pending" "$1"; exec /bin/sleep 900';
  const service = [
    exec('manager-configure', '/usr/bin/systemctl', ['--user', 'set-environment', `${managerKey}=${managerValue}`], () => managerObservation(true)),
    exec('manager-read', '/bin/sh', ['-c', 'set -eu; /usr/bin/systemctl --user show-environment | /usr/bin/grep -Fx -- "$1"', 'nomifun-exact-manager-key', `${managerKey}=${managerValue}`], value => assert.equal(value.output.text, `${managerKey}=${managerValue}\n`)),
    cleanupManager(),
    exec('service-register-start', '/usr/bin/systemd-run', ['--user', '--collect', '--expand-environment=no', `--unit=${target}`, '--property=Type=exec', `--working-directory=${root}`, '/bin/sh', '-c', serviceScript, 'nomifun-systemd-fixture', serviceMarker, suffix], async () => {
      for (let i = 0; i < 50 && await absent(serviceMarker); i++) await delay(100);
      await serviceObservation(true);
    }),
    exec('service-native-status', '/usr/bin/systemctl', ['--user', 'show', '--property=LoadState,ActiveState,MainPID,Transient', '--', target], () => serviceObservation(true)),
    { id: 'linux-service-marker-read', name: 'read_file', args: { path: serviceMarker }, verify: value => assert.equal(textOf(value), `${suffix}\n${uid}\n${root}\n${servicePid}\n`) },
  ];
  const serverProgram = 'import http.server,sys\nport=int(sys.argv[1]); body=sys.argv[2].encode(); path=sys.argv[3]\nclass Handler(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  assert self.path==path\n  print("NOMIFUN_REQUEST:"+self.path,flush=True)\n  self.send_response(200); self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body)\n def log_message(self,*args): pass\nserver=http.server.HTTPServer(("127.0.0.1",port),Handler)\nprint("NOMIFUN_LISTENING",flush=True)\nserver.serve_forever()\n';
  const network = [
    { id: 'linux-network-start', name: 'start_process', awaitExit: false, args: { command: '/usr/bin/python3', args: ['-u', '-c', serverProgram, String(loopbackPort), networkBody, networkPath], cwd: root, wait_ms: 0, timeout_ms: 60000 }, verify: value => { assert.equal(value.state, 'running'); networkProcessId = value.process_id; networkPid = value.pid; cursor = value.output.next_cursor; networkOutput = value.output.text; assert(Number.isInteger(networkPid)); } },
    { id: 'linux-network-running', name: 'poll_process', repeatable: true, maxAttempts: 20, args: () => ({ process_id: networkProcessId, cursor, wait_ms: 100 }), verify: async value => {
      assert.equal(value.state, 'running'); cursor = value.output.next_cursor; networkOutput += value.output.text;
      const listening = await observeLinuxListener({ pid: networkPid, port: loopbackPort });
      if (listening) nativeEvidence.push({ phase: 'loopback_listener', pid: networkPid, port: loopbackPort, app_owned_socket: true, address: '127.0.0.1' });
      return listening && networkOutput.includes('NOMIFUN_LISTENING');
    } },
    exec('network-curl', '/usr/bin/curl', ['--noproxy', '*', '--fail', '--silent', '--show-error', '--max-time', '10', `http://127.0.0.1:${loopbackPort}${networkPath}`], value => assert.equal(value.output.text, networkBody)),
    { id: 'linux-network-cancel', name: 'cancel_process', args: () => ({ process_id: networkProcessId }), verify: value => { assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); cursor = value.output.next_cursor; networkOutput += value.output.text; assert(networkOutput.includes(`NOMIFUN_REQUEST:${networkPath}`)); } },
    { id: 'linux-network-terminal', name: 'poll_process', allowCancelledObservation: true, args: () => ({ process_id: networkProcessId, cursor, wait_ms: 0 }), verify: value => { assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); assert(gone(networkPid)); nativeEvidence.push({ phase: 'loopback_cleanup', pid: networkPid, process_gone: true, reaped: true }); } },
  ];
  const finalization = [
    { id: 'linux-config-restore', name: 'write_file', args: { path: config, content: original }, verify: async () => assert.equal(await readFile(config, 'utf8'), original) },
    { id: 'linux-config-restored-read', name: 'read_file', args: { path: config }, verify: value => assert.equal(textOf(value), original) },
    ...preserveServiceForHostExit ? [] : [cleanupService(), deleteFiles()],
  ];
  const cleanupPrompt = `Only clean this run's unique systemd user service ${target}, manager key ${managerKey}, and fixture ${root}. The previous canonical Turn must first be fully settled with zero pending/unknown effects and proven temporary-process cleanup. Query the exact service MainPID before stopping it; independently confirm service absence and that PID exit, restore this unique initially absent manager key to absence, then delete this fixture. Do not retry any original effects or operate on existing services/files.`;
  const phases = [
    { role: 'system-install-config', steps: installation, prompt: `Only use ${root}: create/patch/read the isolated configuration, write the unique source and install it with native install -m755 into ${bin}; execute its bare name with literal argv and task PATH, then uninstall it. Confirm the unique service ${target} and manager key ${managerKey} are absent. Preserve the remaining fixture for later requested phases.` },
    { role: 'system-service', steps: service, prompt: `Set/read/unset only the initially absent unique systemd user manager environment key ${managerKey}. Register and start only transient user service ${target} with systemd-run --user, collect mode and the supplied fixture cwd. Verify its real systemd status, cgroup, UID, PID and marker. Explicitly retain this service for the later requested phases.` },
    { role: 'system-network', steps: network, prompt: `Complete one bounded managed-process lifecycle: serve HTTP on only 127.0.0.1:${loopbackPort}${networkPath}, separately poll it running, make the exact curl --noproxy '*' request and verify the response and request output, cancel and terminal-poll with proven process cleanup. Preserve the explicitly retained service ${target}.` },
    { role: 'system-finalize', steps: finalization, prompt: `Restore/read the isolated configuration ${config}. ${preserveServiceForHostExit ? `Explicitly retain only the unique service ${target} and its fixture for actual App-exit survival verification.` : `Stop only service ${target}, verify its native absence and observed PID exit, then delete ${root}.`} Do not replay prior install/configuration/network effects.` },
  ];
  return {
    title: 'Linux native user installation/configuration/systemd/loopback acceptance', steps: phases.flatMap(phase => phase.steps), phases,
    resources: { root, config, userToolName, userToolInstalled: installed, target, managerKey, serviceMarker, loopbackPort, networkPath, retainedService: preserveServiceForHostExit },
    nativeEvidence, cleanupPrompt, cleanupSteps: [cleanupService(), cleanupManager(), deleteFiles()], cleanupNeeded: async () => !await absent(root),
    completionSummary: role => {
      const detail = {
        'system-install-config': 'The isolated configuration was written, patched and read back. A locally generated user executable was installed, run by bare name with literal argv and task PATH, then uninstalled. The unique service/manager key were independently absent before effects.',
        'system-service': 'The unique real systemd-manager configuration key was set/read/unset and independently restored to absence. The unique transient user service was started and its actual PID, UID, cwd, cgroup and marker verified; it is explicitly retained for the next phases.',
        'system-network': 'The managed HTTP server listened only on IPv4 loopback, a separate running poll proved its exact socket ownership, curl exchanged the exact body/path, and cancellation plus terminal poll proved process cleanup. The explicitly retained service remains for finalization.',
        'system-finalize': preserveServiceForHostExit ? 'The isolated configuration was restored/read. Only the explicitly requested unique service and fixture remain for actual App-exit survival verification.' : 'The isolated configuration was restored/read, the unique service stopped and collected, its exact observed PID exited, and fixture files were deleted.',
        cleanup: 'The earlier failure remains failed. This fresh settled Turn only stopped/collected this unique service, confirmed its exact observed PID exited, restored the unique manager key to absence and removed the fixture.',
        'retained-cleanup': 'The actual old native App exited and the same unique service/PID/marker survived; the restarted App authenticated the same owner JWT and Session. This fresh Turn stopped/collected the service, restored its unique manager key and removed the fixture.',
      }[role];
      assert(detail, `Unknown Linux completion role ${role}`);
      return `${detail} All evidence belongs to this run's fixture ${root}, service ${target} and manager key ${managerKey}; only this phase's actual observed calls support completion. This does not claim third-party package installation or broad daily-setting coverage.`;
    },
    survivalAcceptanceId: 'systemd-survives-native-App-exit',
    pendingAcceptance: [{ id: 'systemd-survives-native-App-exit', status: 'not_run', reason: 'Registration/cgroup evidence does not prove survival after the actual native App exits; use the retained-service scenario.' }, { id: 'distribution-package-and-daily-settings', status: 'not_run', reason: 'This installs one isolated user executable and changes one unique real systemd-manager setting. It does not install distro packages or change existing daily/system-wide configuration.' }],
    cleanupIntent: { prerequisite: 'Exact canonical cancellation and zero pending/unknown effects before any fresh cleanup Turn; no host-side effects.', temporary_process: () => ({ process_id: networkProcessId, observed_pid: networkPid, action: 'The original canonical Turn owner cancels and reaps its temporary process tree.' }), persistent_service: { service_target: target, action: cleanupPrompt } },
    async observeAfterAppExit({ appPid }) { assert(preserveServiceForHostExit && gone(appPid)); await serviceObservation(true); return { status: 'passed', app_pid: appPid, service_pid: servicePid, service_target: target, cleanup_required: true }; },
  };
}
