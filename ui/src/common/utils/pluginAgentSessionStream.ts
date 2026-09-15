import type { PluginAgentSessionStream, PluginRuntimeSurfaceLaunchDescriptor } from '../types/pluginRuntimePlatform';

const PREFIX = 'nomifun-plugin-agent-session-';
const MAX_PENDING = 8;
const MAX_EVENT_BYTES = 64 * 1024;

/** Bounded view delivery, not a Session, event store or authorization cache. */
export function createPluginAgentSessionStreamRelay(
  port: { postMessage(message: unknown): void },
  scope: Pick<PluginRuntimeSurfaceLaunchDescriptor, 'plugin_id' | 'surface_session_id' | 'surface_generation'>
) {
  let active = false;
  let closed = false;
  let sequence = 0;
  let subscriptionId = 0;
  let gap: string | null = null;
  const pending = new Set<number>();

  function send(payload: Record<string, unknown>) {
    const seq = ++sequence;
    pending.add(seq);
    try { port.postMessage({ type: `${PREFIX}event-v1`, subscription_id: subscriptionId, seq, ...payload }); }
    catch { close(); }
  }

  function flushGap() {
    if (!active || closed || pending.size || !gap) return;
    const reason = gap;
    gap = null;
    send({ kind: 'resync_required', reason });
  }

  function resync(reason = 'transport_gap') {
    if (!active || closed) return;
    gap = reason;
    flushGap();
  }

  function close() {
    active = false;
    closed = true;
    pending.clear();
    gap = null;
  }

  return {
    close,
    resync,
    receive(value: unknown): boolean {
      if (!value || typeof value !== 'object') return false;
      const message = value as Record<string, unknown>;
      if (message.type === `${PREFIX}subscribe-v1`) {
        if (!closed && Number.isSafeInteger(message.subscription_id) && Number(message.subscription_id) > subscriptionId) {
          subscriptionId = Number(message.subscription_id);
          pending.clear(); gap = null;
          active = true; resync('initial');
        }
        return true;
      }
      if (message.type === `${PREFIX}unsubscribe-v1`) {
        if (message.subscription_id === subscriptionId) { active = false; pending.clear(); gap = null; }
        return true;
      }
      if (message.type === `${PREFIX}ack-v1`) {
        if (message.subscription_id === subscriptionId && Number.isSafeInteger(message.seq)) pending.delete(Number(message.seq));
        flushGap();
        return true;
      }
      return false;
    },
    stream(value: PluginAgentSessionStream) {
      if (!active || closed || !value || value.plugin_id !== scope.plugin_id ||
          value.surface_session_id !== scope.surface_session_id ||
          value.surface_generation !== scope.surface_generation) return;
      if (gap || pending.size >= MAX_PENDING) { resync('slow_consumer'); return; }
      try {
        if (new TextEncoder().encode(JSON.stringify(value.event)).length > MAX_EVENT_BYTES) {
          resync('oversized_event'); return;
        }
        send({ kind: 'stream', event: value.event });
      } catch { resync('invalid_event'); }
    },
  };
}
