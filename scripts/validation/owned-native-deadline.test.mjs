import assert from 'node:assert/strict';
import test from 'node:test';
import { superviseNativeChild } from './owned-native-deadline.mjs';

test('a normal owned exit stays zero and clears the observer before the deadline', async () => {
  const states = [];
  const result = await superviseNativeChild(process.execPath, ['-e', 'process.exit(0)'], {
    deadlineMs: 5000, terminateGraceMs: 100, stdio: 'ignore', onState: state => states.push(state),
  });
  assert.equal(result.exitCode, 0);
  assert.equal(result.expired, false);
  assert.equal(result.terminateSent, false);
  assert.equal(result.forceKillSent, false);
  assert.equal(states[0].running, true);
  assert.equal(states.at(-1).running, false);
});

test('an expired owned child is killed with a bounded grace and never claims success', async () => {
  const states = [];
  const result = await superviseNativeChild(process.execPath,
    ['-e', "process.on('SIGTERM',()=>{});setInterval(()=>{},1000)"], {
      deadlineMs: 300, terminateGraceMs: 100, stdio: 'ignore', onState: state => states.push(state),
    });
  assert.equal(result.expired, true);
  assert.equal(result.terminateSent, true);
  assert.equal(result.forceKillSent, true);
  assert.equal(result.signal, 'SIGKILL');
  assert.ok(states.some(state => state.expired && !state.running));
  assert.equal(states.at(-1).running, false);
  assert.ok(result.elapsedMs < 5000);
});

test('zero after deadline termination is still an expired observation', async () => {
  const result = await superviseNativeChild(process.execPath,
    ['-e', "process.on('SIGTERM',()=>process.exit(0));setInterval(()=>{},1000)"], {
      deadlineMs: 300, terminateGraceMs: 500, stdio: 'ignore',
    });
  assert.equal(result.exitCode, 0);
  assert.equal(result.expired, true);
  assert.equal(result.terminateSent, true);
  assert.equal(result.forceKillSent, false);
});
