import '../../../../../test/setup-dom.ts';
import { describe, expect, test } from 'bun:test';
import { parseBridgeRequest } from './PluginRuntimeSurfacePanel';

describe('Agent Session Surface bridge', () => {
  test('parser accepts only scoped command data and bounded history', () => {
    const wrap = (command: unknown) => ({ call_id: 'call', target: { target: 'agent_session', request: command } });
    for (const command of [
      { operation: 'observe', after_seq: 0, limit: 200 },
      { operation: 'turn', input: { content: 'hello' }, idempotency_key: 'stable' },
      { operation: 'cancel' },
    ]) {
      expect(parseBridgeRequest(wrap(command))).toEqual(wrap(command));
      for (const field of ['agent_session_id', 'owner_user_id', 'conversation_id', 'extra']) {
        expect(parseBridgeRequest(wrap({ ...command, [field]: 'forged' }))).toBeNull();
        const request = wrap(command);
        expect(parseBridgeRequest({ ...request, [field]: 'forged' })).toBeNull();
        expect(parseBridgeRequest({ ...request, target: { ...request.target, [field]: 'forged' } })).toBeNull();
      }
    }
    for (const command of [
      { operation: 'observe', after_seq: -1, limit: 100 },
      { operation: 'observe', after_seq: 0, limit: 201 },
      { operation: 'observe', after_seq: 0, limit: 0 },
      { operation: 'observe', after_seq: Number.MAX_SAFE_INTEGER + 1, limit: 1 },
      { operation: 'turn', input: [], idempotency_key: 'stable' },
      { operation: 'turn', input: {}, idempotency_key: '字'.repeat(86) },
    ]) expect(parseBridgeRequest(wrap(command))).toBeNull();
  });

});
