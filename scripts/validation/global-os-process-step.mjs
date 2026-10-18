import assert from 'node:assert/strict';

// A finite native verification starts exactly once. Observation windows can
// end while its child is still running; later proposals only poll that original
// handle and cursor. Intentionally long-lived fixtures opt into their existing
// running/poll/cancel business steps with awaitExit:false.
export function nativeProcessStep(step) {
  assert.notEqual(step.name, 'exec_command', 'The retired synchronous process tool cannot enter current native validation');
  if (step.name !== 'start_process' || step.awaitExit === false) return step;
  let handle, pid, cursor = 0, text = '', closed = false, observation;
  const origin = typeof step.args === 'function' ? step.args : () => step.args;
  return {
    ...step,
    repeatable: true,
    maxAttempts: Math.max(step.maxAttempts ?? 1, 16),
    get name() { return handle ? 'poll_process' : 'start_process'; },
    processObservation: () => observation,
    args: async () => {
      assert(!closed, 'A terminal process verification cannot launch or poll again');
      return handle ? { process_id: handle, cursor, wait_ms: 30000 }
        : { ...await origin(), wait_ms: 30000 };
    },
    verify: async (value, context) => {
      assert(!closed, 'A terminal process receipt cannot be verified twice');
      assert(typeof value.process_id === 'string' && value.process_id.length > 0, 'A real managed process handle is required');
      // EngineProcessPoll carries pid on Running/Lost, while joined terminal
      // variants prove ownership through the original process_id and cleanup.
      // A fast first start may already be Exited without ever exposing a PID.
      if (value.state === 'running' || value.pid !== undefined) {
        assert(Number.isSafeInteger(value.pid) && value.pid > 1, 'A real native process PID is required');
        if (pid !== undefined) assert.equal(value.pid, pid, 'Polling cannot adopt a replacement PID');
        else pid = value.pid;
      }
      assert(typeof value.output?.text === 'string' && Number.isSafeInteger(value.output.next_cursor)
        && value.output.next_cursor >= cursor, 'Native process output requires its monotonic cursor');
      assert.equal(value.output.dropped_bytes, 0, 'Truncated native output cannot prove verification');
      assert.equal(value.output.decode_errors, 0, 'Undecodable native output cannot prove verification');
      if (handle) {
        assert.equal(value.process_id, handle, 'Polling cannot adopt a replacement process');
      } else { handle = value.process_id; }
      cursor = value.output.next_cursor; text += value.output.text;
      if (value.state === 'running') return false;
      closed = true;
      assert.equal(value.state, 'exited');
      assert.equal(value.exit_code, 0);
      assert.equal(value.cleanup?.reaped, true);
      assert.deepEqual(value.cleanup.errors ?? [], []);
      // This is a derived observation for the original fixture assertion;
      // raw per-call receipts remain unchanged in the evidence report.
      observation = { ...value, output: { ...value.output, text } };
      const complete = await step.verify(observation, context);
      assert(complete !== false, 'An exited process cannot be relaunched to satisfy an incomplete verification');
      return true;
    },
  };
}
