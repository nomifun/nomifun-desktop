// Evidence checks shared by the desktop lifecycle fixture and its native
// macOS driver. These checks never start a process or dispatch desktop input.
import assert from 'node:assert/strict';

export function computerGuideForLifecycle(manifest, platform, mode) {
  assert((platform === 'win32' && mode === 'quit')
    || (platform === 'darwin' && ['appkit', 'cmd-q'].includes(mode)),
  'Computer resource observation requires normal Windows Quit or macOS AppKit/Cmd-Q');
  const tools = manifest.tools.filter(tool => tool.driver_tool === 'get_guide');
  assert.equal(tools.length, 1, 'The guide must have one frozen definition');
  const guide = tools[0];
  const schema = guide.platforms[platform === 'win32' ? 'windows' : 'macos']?.input_schema;
  assert(schema, 'The guide must have an actual frozen schema for this platform');
  assert.equal(schema.type, 'object');
  assert.equal((schema.required ?? []).length, 0, 'The guide must accept an empty object');
  assert.equal(guide.effect_class, 'Pure');
  assert.equal(guide.system_permissions.accessibility, false);
  assert.equal(guide.system_permissions.screen_recording, 'never');
  return guide;
}

export function macosComputerProcessProof(events, hostPid, workerPid, { exited = false } = {}) {
  assert(Number.isSafeInteger(hostPid) && hostPid > 1);
  assert(Number.isSafeInteger(workerPid) && workerPid > 1 && workerPid !== hostPid);
  const registration = pid => {
    const matches = events.filter(event => event.event === 'registered' && event.pid === pid);
    assert.equal(matches.length, 1, 'Each original Computer owner must have one native registration');
    const value = matches[0];
    assert(Number.isSafeInteger(value.index) && value.index >= 0);
    assert(typeof value.unique_id === 'string' && /^[1-9][0-9]*$/.test(value.unique_id));
    assert(Number.isSafeInteger(value.pid_version) && value.pid_version > 0);
    return value;
  };
  const host = registration(hostPid), worker = registration(workerPid);
  assert.equal(host.index, 0, 'The first native owner must remain the exact Desktop host');
  assert.notEqual(worker.index, host.index);
  assert.equal(worker.parent_pid, host.pid, 'The Computer worker must be a direct App child');
  assert.equal(worker.parent_unique_id, host.unique_id, 'The worker must name this exact App kernel generation');
  assert.equal(worker.original_parent_pid_version, host.pid_version);
  const exits = events.filter(event => event.event === 'exit' && (event.index === worker.index || event.pid === worker.pid));
  if (!exited) {
    assert.equal(exits.length, 0, 'An already exited Computer worker cannot prove live Quit cleanup');
    return { host, worker };
  }
  assert.equal(exits.length, 1, 'The original Computer worker needs one exact native exit receipt');
  const receipt = exits[0];
  assert.equal(receipt.index, worker.index);
  assert.equal(receipt.pid, worker.pid);
  assert.equal(receipt.note_exit, true);
  assert.equal(receipt.note_exit_status, true, 'NOTE_EXIT alone does not prove the worker exit code');
  assert.equal(receipt.native_status, 0, 'The original Computer worker must exit normally with code zero');
  return { host, worker, exit: receipt, worker_exit_code: 0 };
}
