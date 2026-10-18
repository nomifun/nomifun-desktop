import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { execFile } from 'node:child_process';
import { lstat, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { promisify } from 'node:util';
const execute = promisify(execFile), delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const absent = path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const gone = pid => { try { process.kill(pid, 0); return false; } catch (error) { if (error.code === 'ESRCH') return true; throw error; } };

// System manager acceptance is separate from the existing user-manager case.
// The native administrator tool performs effects; observations never signal,
// stop, reconfigure, or directly read protected service fixture bytes.
export function createLinuxSystemRootScenario({ fixture, workspace, uid }) {
  assert.equal(process.platform, 'linux'); assert(uid > 0); assert(/^\/tmp\/nomifun-global-os-product-[0-9a-f-]{36}$/.test(fixture)); assert.equal(workspace, join(fixture, 'workspace'));
  const suffix = randomUUID().replaceAll('-', ''), target = `nomifun-global-os-system-${suffix}.service`, root = join(fixture, 'system-root'), marker = join(root, 'marker.txt');
  const nativeEvidence = []; let servicePid, unitInitiallyAbsent = false;
  const show = async present => {
    const value = await execute('/usr/bin/systemctl', ['--system', 'show', '--property=LoadState,ActiveState,SubState,MainPID,Transient,ControlGroup,WorkingDirectory,User', '--', target], { encoding: 'utf8', timeout: 10000, maxBuffer: 16384 });
    const props = Object.fromEntries(value.stdout.trim().split('\n').map(line => { const at = line.indexOf('='); return [line.slice(0, at), line.slice(at + 1)]; }));
    if (!present) { assert.equal(props.LoadState, 'not-found'); assert.equal(props.MainPID, '0'); nativeEvidence.push({ phase: 'system_unit_absent', target, ...props }); return; }
    assert.equal(props.LoadState, 'loaded'); assert.equal(props.ActiveState, 'active'); assert.equal(props.Transient, 'yes'); assert.equal(props.WorkingDirectory, root);
    const pid = Number(props.MainPID); assert(Number.isSafeInteger(pid) && pid > 1); if (servicePid) assert.equal(pid, servicePid); servicePid = pid;
    const status = await readFile(`/proc/${pid}/status`, 'utf8'); assert(/^Uid:\s+0\s+0\s+0\s+0$/m.test(status)); const cgroup = await readFile(`/proc/${pid}/cgroup`, 'utf8'); assert(cgroup.includes(`/system.slice/${target}`));
    nativeEvidence.push({ phase: 'real_system_service_running', target, service_pid: pid, uid: 0, cgroup: cgroup.trim(), ...props });
  };
  const waitPresent = async () => { let last; for (let i = 0; i < 60; i++) { try { await show(true); return; } catch (error) { last = error; await delay(50); } } throw last; };
  const waitGone = async () => { let last; for (let i = 0; i < 60; i++) { try { await show(false); assert(!servicePid || gone(servicePid)); return; } catch (error) { last = error; await delay(50); } } throw last; };
  const admin = (id, command, args, verify) => ({ id, name: 'start_process', args: { command, args, execution_identity: 'administrator', timeout_ms: 15000 }, verify: async value => { assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup.reaped, true); await verify?.(value); } });
  const stop = admin('system-root-stop', '/bin/sh', ['-c', 'set -eu; state=$(/usr/bin/systemctl --system show --property=LoadState --value -- "$1"); if [ "$state" != not-found ]; then /usr/bin/systemctl --system stop -- "$1"; fi', 'nomifun-exact-system-stop', target], waitGone);
  const clean = admin('system-root-clean', '/usr/bin/python3', ['-c', 'import os,sys; r,p=sys.argv[1:];s=os.lstat(r);assert s.st_uid==0 and s.st_mode&0o777==0o700;os.unlink(p) if os.path.lexists(p) else None;os.rmdir(r)', root, marker], async () => { await waitGone(); assert(await absent(root)); nativeEvidence.push({ phase: 'system_root_fixture_absent', root, service_pid_gone: !servicePid || gone(servicePid) }); });
  const stopArguments = stop.args; stop.args = () => { assert(unitInitiallyAbsent, 'Never stop a system unit whose initial absence was not proven'); return stopArguments; };
  const script = 'set -eu; umask 077; set -C; { printf "%s\\n" "$2"; /usr/bin/id -u; /bin/pwd; printf "%s\\n" "$$"; } > "$1.pending"; /usr/bin/mv -T -- "$1.pending" "$1"; exec /bin/sleep 600';
  const steps = [
    admin('system-root-prepare', '/usr/bin/python3', ['-c', 'import os,sys;os.mkdir(sys.argv[1],0o700);print(str(os.getuid())+":"+oct(os.stat(sys.argv[1]).st_mode&0o777))', root], async value => { assert.equal(value.output.text, '0:0o700\n'); const info = await lstat(root); assert(info.isDirectory() && !info.isSymbolicLink()); assert.equal(info.uid, 0); assert.equal(info.mode & 0o777, 0o700); await show(false); unitInitiallyAbsent = true; }),
    admin('system-root-start', '/usr/bin/systemd-run', ['--system', '--collect', '--expand-environment=no', `--unit=${target}`, '--property=Type=exec', `--working-directory=${root}`, '/bin/sh', '-c', script, 'nomifun-system-root-fixture', marker, suffix], waitPresent),
    admin('system-root-status', '/usr/bin/systemctl', ['--system', 'show', '--property=LoadState,ActiveState,MainPID,Transient,ControlGroup,WorkingDirectory', '--', target], async value => { assert(value.output.text.includes(`MainPID=${servicePid}\n`)); await show(true); }),
    { id: 'system-root-scope', name: 'read_file', args: { path: root, format: 'instruction_scope', execution_identity: 'administrator' }, verify: value => { assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true); assert.equal(value.canonical_path, root); } },
    { id: 'system-root-marker', name: 'read_file', repeatable: true, maxAttempts: 8, args: { path: marker, execution_identity: 'administrator', missing_ok: true }, verify: value => { if (value.kind === 'workspace_file_absent') return false; assert.equal(value.content, `${suffix}\n0\n${root}\n${servicePid}\n`); assert.equal(value.workspace_path, undefined); nativeEvidence.push({ phase: 'actual_system_service_uid_cwd_pid_marker', target, pid: servicePid, uid: 0, cwd: root, marker_sha256: value.sha256 }); } },
    stop, clean,
  ];
  return { title: 'Native Linux system manager service acceptance', resources: { root, marker, target }, steps, nativeEvidence,
    prompt: `Only operate on ${root} and unique transient system unit ${target}. Use native administrator exec to create the root:0700 fixture, prove this unit initially absent, then systemd-run --system --collect --expand-environment=no with Type=exec and this exact cwd. The bounded root service writes only its own marker and sleeps at most 600 seconds. Independently observe real system-manager MainPID, system.slice cgroup, UID0 and WorkingDirectory; use administrator read_file instruction_scope and the exact marker to verify actual service pwd/PID. Stop and collect only this unit, prove the exact PID gone and unit not-found, then remove only this marker and fixture. No existing services, system configuration, daily files or helper-owned process substitutes.`,
    completionSummary: role => role === 'cleanup' ? `The original system-root failure remains failed. This fresh settled cleanup Turn only stopped/collected exact transient system unit ${target}, proved its observed PID absent and removed its unique fixture ${root}.` : `The default General Agent created one unique root fixture and registered actual transient system unit ${target} through the native administrator path. Native system-manager properties, system.slice cgroup, UID0, WorkingDirectory and root-written marker matched the same PID. The exact unit was stopped and collected, its PID disappeared, and the fixture was deleted. User-manager evidence remains separate; this bounded local test does not prove distro-wide service compatibility or persistence across OS reboot.`,
    cleanupPrompt: `The failed original Turn must be settled with proven cleanup and zero pending/unknown effects first. This new Turn only queries and stops exact transient system unit ${target}, proves it absent and its observed PID gone, then removes ${marker} and exact directory ${root}. Do not replay start/configuration effects or touch any other services or files.`, cleanupSteps: [stop, clean], cleanupNeeded: () => absent(root).then(value => !value),
    cleanupIntent: { prerequisite: 'Exact original Turn settled with proven cleanup and zero pending/unknown effects.', temporary_process: () => ({ observed_pid: servicePid, action: 'Only stop/collect the exact registered transient system unit and independently prove its MainPID gone.' }), persistent_service: { service_target: target } },
  };
}
