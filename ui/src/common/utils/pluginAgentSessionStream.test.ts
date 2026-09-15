import { describe, expect, test } from 'bun:test';
import { createPluginAgentSessionStreamRelay } from './pluginAgentSessionStream';
import type { PluginRuntimeId } from '../types/pluginRuntimePlatform';

const prefix = 'nomifun-plugin-agent-session-';
const scope = {
  plugin_id: '0190f5fe-7c00-7a00-8000-000000000003' as PluginRuntimeId,
  surface_session_id: '0190f5fe-7c00-7a00-8000-000000000004', surface_generation: 1,
};

function fixture() {
  const messages: Array<Record<string, unknown>> = [];
  const relay = createPluginAgentSessionStreamRelay({ postMessage: message => messages.push(message as Record<string, unknown>) }, scope);
  const receive = (type: string, fields = {}) => relay.receive({ type: prefix + type + '-v1', subscription_id: 1, ...fields });
  const stream = () => relay.stream({ ...scope, event: { type: 'text', data: 'hello' } });
  return { messages, relay, receive, stream };
}

describe('plugin Session live stream relay', () => {
  test('requires subscription and exact audience; strips routing fields', () => {
    const { messages, relay, receive, stream } = fixture();
    stream();
    expect(messages).toHaveLength(0);
    receive('subscribe');
    expect(messages[0]?.kind).toBe('resync_required');
    receive('ack', { seq: 1 });
    for (const mismatch of [{ plugin_id: 'other' }, { surface_session_id: 'other' }, { surface_generation: 2 }]) {
      relay.stream({ ...scope, ...mismatch, event: {} } as Parameters<typeof relay.stream>[0]);
    }
    expect(messages).toHaveLength(1);
    stream();
    expect(messages[1]).toEqual({ type: prefix + 'event-v1', subscription_id: 1, seq: 2, kind: 'stream', event: { type: 'text', data: 'hello' } });
    receive('unsubscribe'); stream();
    expect(messages).toHaveLength(2);
    receive('subscribe', { subscription_id: 2 });
    relay.close(); stream(); receive('subscribe', { subscription_id: 3 });
    expect(messages).toHaveLength(3);
  });

  test('bounds a burst at eight credits then reports one gap after all ACKs', () => {
    const { messages, receive, stream } = fixture();
    receive('subscribe'); receive('ack', { seq: 1 });
    for (let i = 0; i < 1000; i++) stream();
    expect(messages).toHaveLength(9);
    receive('ack', { seq: 900 });
    receive('ack', { seq: 2, subscription_id: 9 });
    expect(messages).toHaveLength(9);
    for (let seq = 2; seq < 9; seq++) receive('ack', { seq });
    receive('ack', { seq: 2 });
    expect(messages).toHaveLength(9);
    receive('ack', { seq: 9 });
    expect(messages).toHaveLength(10);
    expect(messages[9]?.reason).toBe('slow_consumer');
    receive('ack', { seq: 10 }); stream();
    expect(messages[10]?.kind).toBe('stream');
  });

  test('oversize and reconnect require resync; stale subscription cannot stop a new one', () => {
    const { messages, relay, receive, stream } = fixture();
    receive('subscribe'); receive('ack', { seq: 1 });
    relay.stream({ ...scope, event: { data: '字'.repeat(24000) } });
    expect(messages[1]?.reason).toBe('oversized_event');
    relay.resync();
    expect(messages).toHaveLength(2);
    receive('ack', { seq: 2 });
    expect(messages[2]?.reason).toBe('transport_gap');
    receive('unsubscribe'); receive('subscribe', { subscription_id: 2 });
    receive('unsubscribe');
    stream();
    expect(messages.at(-1)?.kind).toBe('stream');
    expect(messages.at(-1)?.subscription_id).toBe(2);
  });

  test('failed delivery closes the relay', () => {
    let calls = 0;
    const relay = createPluginAgentSessionStreamRelay({ postMessage() { calls++; throw new Error('closed'); } }, scope);
    relay.receive({ type: prefix + 'subscribe-v1', subscription_id: 1 });
    relay.resync(); relay.stream({ ...scope, event: {} });
    expect(calls).toBe(1);
  });
});
