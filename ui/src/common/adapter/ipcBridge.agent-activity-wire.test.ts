import { expect, test } from 'bun:test';
import { agentActivities } from './ipcBridge';

test('activity discovery encodes its read cursor and preserves original continuation facts', async () => {
  const original = globalThis.fetch;
  const calls: string[] = [];
  try {
    globalThis.fetch = (async (url) => {
      calls.push(String(url));
      return Response.json({ success: true, data: { items: [{ continuation: 'answer_queued' }], next_cursor: 'session:next:~' } });
    }) as typeof fetch;
    const result = await agentActivities.list.invoke({ after_cursor: 'session:a:input:12&next=bad', limit: 50 });
    const url = new URL(calls[0], 'http://localhost');
    expect(url.searchParams.get('after_cursor')).toBe('session:a:input:12&next=bad');
    expect(url.searchParams.get('limit')).toBe('50');
    expect(result.items[0].continuation).toBe('answer_queued');
    expect(calls).toHaveLength(1);
  } finally { globalThis.fetch = original; }
});

test('all-stop captures read-only targets then forwards an exact accepted input without inventing a Turn', async () => {
  const original = globalThis.fetch;
  const calls: Array<{ url: string; method: string; body?: unknown }> = [];
  const target = { kind: 'session_input' as const, agent_session_id: '0190f5fe-7c00-7a00-8000-000000000001', message_id: '0190f5fe-7c00-7a00-8000-000000000002' };
  try {
    globalThis.fetch = (async (url, init) => {
      calls.push({ url: String(url), method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : undefined });
      return Response.json({ success: true, data: String(url).endsWith('/stop-targets')
        ? { captured_at_ms: 123, targets: [target] } : { accepted: true, execution_stopped: false, resources: [] } });
    }) as typeof fetch;
    const snapshot = await agentActivities.stopTargets.invoke();
    const command = { target: snapshot.targets[0], idempotency_key: 'original-stop', retry_cleanup: false };
    const receipt = await agentActivities.stop.invoke(command);
    expect(calls[0].method).toBe('GET');
    expect(calls[0].body).toBeUndefined();
    expect(calls[1].method).toBe('POST');
    expect(calls[1].body).toEqual(command);
    expect(receipt.accepted).toBe(true);
    expect(receipt.execution_stopped).toBe(false);
  } finally { globalThis.fetch = original; }
});

test('AutoWork discovery preserves distinct original run and delegated-only Execution targets', async () => {
  const original = globalThis.fetch;
  const root = { kind: 'auto_work_run' as const, target_kind: 'conversation' as const, target_id: 'lead',
    requirement_id: 'requirement', claim_generation: 7 };
  const delegated = { kind: 'execution_step' as const, execution_id: 'execution', step_id: 'root-step',
    expected_execution_version: 12, expected_step_version: 4 };
  const commands: unknown[] = [];
  try {
    globalThis.fetch = (async (_url, init) => {
      if (init?.method === 'POST') {
        commands.push(JSON.parse(String(init.body)));
        return Response.json({ success: true, data: { accepted: true, execution_stopped: false, resources: [] } });
      }
      return Response.json({ success: true, data: { items: [{ target: root, delegation_target: delegated }], next_cursor: null } });
    }) as typeof fetch;
    const result = await agentActivities.list.invoke({});
    expect(result.items[0].target).toEqual(root);
    expect(result.items[0].delegation_target).toEqual(delegated);
    await agentActivities.stop.invoke({ target: result.items[0].delegation_target!, idempotency_key: 'delegation-stop', scope: 'delegated_work' });
    await agentActivities.stop.invoke({ target: result.items[0].target, idempotency_key: 'run-stop' });
    expect(commands).toEqual([{ target: delegated, idempotency_key: 'delegation-stop', scope: 'delegated_work' },
      { target: root, idempotency_key: 'run-stop' }]);
  } finally { globalThis.fetch = original; }
});
