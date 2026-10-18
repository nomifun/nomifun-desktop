import { expect, test } from 'bun:test';
import { nativeProcessStep } from './global-os-process-step.mjs';

// Mirror the actual EngineProcessPoll variants: only Running/Lost expose pid.
const processReceipt = (state, text, next_cursor, changes = {}) => ({ process_id: 'original-process',
  ...(['running', 'lost'].includes(state) ? { pid: 321 } : {}),
  state, exit_code: state === 'exited' ? 0 : null, cleanup: { reaped: state === 'exited', errors: [] },
  output: { text, next_cursor, dropped_bytes: 0, decode_errors: 0 }, ...changes });

test('finite native steps start once and poll the exact original handle before verifying all output', async () => {
  let argsReads = 0, verified;
  const source = { id: 'finite', name: 'start_process', args: () => {
    argsReads++; return { command: 'C:\\fixture\\literal.ps1', args: ['', ' 中文 ', 'a&b'], execution_identity: 'administrator', timeout_ms: 90000 };
  }, verify: value => { verified = value; } };
  const step = nativeProcessStep(source);
  expect(step.name).toBe('start_process');
  expect(await step.args()).toEqual({ command: 'C:\\fixture\\literal.ps1', args: ['', ' 中文 ', 'a&b'], execution_identity: 'administrator', timeout_ms: 90000, wait_ms: 30000 });
  const first = processReceipt('running', 'first ', 6), second = processReceipt('running', 'second ', 13), last = processReceipt('exited', 'last', 17);
  expect(last.pid).toBeUndefined();
  expect(await step.verify(first, {})).toBe(false); expect(verified).toBeUndefined();
  expect(step.name).toBe('poll_process');
  expect(await step.args()).toEqual({ process_id: 'original-process', cursor: 6, wait_ms: 30000 });
  expect(await step.verify(second, {})).toBe(false);
  expect(await step.args()).toEqual({ process_id: 'original-process', cursor: 13, wait_ms: 30000 });
  expect(await step.verify(last, {})).toBe(true);
  expect(verified.output.text).toBe('first second last'); expect(step.processObservation()).toBe(verified);
  expect(first.output.text).toBe('first '); expect(last.output.text).toBe('last'); expect(argsReads).toBe(1);
  await expect(step.args()).rejects.toThrow('cannot launch or poll again');
  await expect(step.verify(last, {})).rejects.toThrow('cannot be verified twice');
});

test('a fast start already exited uses the real terminal schema without fabricating a PID', async () => {
  let verified;
  const step = nativeProcessStep({ name: 'start_process', args: { command: 'fixture' }, verify: value => { verified = value; } });
  const receipt = processReceipt('exited', 'complete', 8);
  expect(receipt.pid).toBeUndefined(); expect(await step.verify(receipt, {})).toBe(true);
  expect(verified).toEqual(receipt); expect(verified.pid).toBeUndefined();
  await expect(step.args()).rejects.toThrow('cannot launch or poll again');
});

test('running receipts require a real PID and any subsequently supplied PID must match', async () => {
  for (const pid of [undefined, null, 0, 1, -1, 3.5, '321']) {
    const step = nativeProcessStep({ name: 'start_process', args: { command: 'fixture' }, verify: () => {} });
    await expect(step.verify(processReceipt('running', '', 0, { pid }), {})).rejects.toThrow('real native process PID');
  }
  const step = nativeProcessStep({ name: 'start_process', args: { command: 'fixture' }, verify: () => {} });
  await step.verify(processReceipt('running', 'first', 5), {});
  await expect(step.verify(processReceipt('running', 'replacement', 16, { pid: 999 }), {})).rejects.toThrow('replacement PID');
  expect(await step.verify(processReceipt('exited', 'done', 9, { pid: 321 }), {})).toBe(true);
});

test('finite observation refuses replacement identity, lost/truncated output and nonzero or unproven exits', async () => {
  for (const invalid of [
    processReceipt('exited', '', 0, { process_id: 'replacement' }),
    processReceipt('exited', '', 0, { pid: 999 }),
    processReceipt('exited', '', 0),
    processReceipt('lost', '', 7),
    processReceipt('exited', '', 7, { exit_code: 1 }),
    processReceipt('exited', '', 7, { cleanup: { reaped: false, errors: [] } }),
    processReceipt('exited', '', 7, { cleanup: { reaped: true, errors: ['unknown child'] } }),
    processReceipt('exited', '', 7, { output: { text: '', next_cursor: 7, dropped_bytes: 1, decode_errors: 0 } }),
    processReceipt('exited', '', 7, { output: { text: '', next_cursor: 7, dropped_bytes: 0, decode_errors: 1 } }),
  ]) {
    let verifications = 0;
    const step = nativeProcessStep({ name: 'start_process', args: { command: 'fixture' }, verify: () => { verifications++; } });
    await step.verify(processReceipt('running', 'prefix', 6), {});
    await expect(step.verify(invalid, {})).rejects.toThrow();
    expect(verifications).toBe(0);
  }
});

test('intentional background start/poll/cancel plans keep their own running verification', () => {
  const background = { name: 'start_process', awaitExit: false, args: { command: 'fixture', wait_ms: 0 }, verify: () => false };
  expect(nativeProcessStep(background)).toBe(background);
  const read = { name: 'read_file' }; expect(nativeProcessStep(read)).toBe(read);
  expect(() => nativeProcessStep({ name: 'exec_command' })).toThrow('retired synchronous process tool');
});

test('an incomplete verification after exit cannot restart the native command', async () => {
  const step = nativeProcessStep({ name: 'start_process', args: { command: 'fixture' }, verify: () => false });
  await expect(step.verify(processReceipt('exited', 'done', 4), {})).rejects.toThrow('cannot be relaunched');
  await expect(step.args()).rejects.toThrow('cannot launch or poll again');
});
