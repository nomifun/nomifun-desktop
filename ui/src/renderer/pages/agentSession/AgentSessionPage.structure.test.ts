import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const page = readFileSync(new URL('./AgentSessionPage.tsx', import.meta.url), 'utf8');
const bridge = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');
const card = readFileSync(new URL('./SessionProjectionCard.tsx', import.meta.url), 'utf8');
const inspector = readFileSync(new URL('./SessionInspector.tsx', import.meta.url), 'utf8');

describe('canonical AgentSession UI', () => {
  test('uses only AgentSession APIs and renders canonical projections', () => {
    expect(page.includes('agentPlatform.sessions.get')).toBe(true);
    expect(page.includes('agentPlatform.sessions.capabilities')).toBe(true);
    expect(page.includes('<SessionProjectionCard')).toBe(true);
    expect(page.includes('<SessionInspector')).toBe(true);
    expect(page.includes('/api/' + 'conver' + 'sations')).toBe(false);
  });

  test('covers fork, delete, active generation, and SESSION_DELETED', () => {
    expect(page.includes('active_set_generation')).toBe(true);
    expect(page.includes('sessions.fork')).toBe(true);
    expect(page.includes('sessions.delete')).toBe(true);
    expect(page.includes('SESSION_DELETED')).toBe(true);
    expect(bridge.includes('/api/agent-sessions/${encodeURIComponent(params.agent_session_id)}/messages')).toBe(true);
  });

  test('uses ASCII-safe sequence and fallback labels', () => {
    expect(card.includes(String.fromCharCode(0x2013))).toBe(false);
    expect(inspector.includes(String.fromCharCode(0x2014))).toBe(false);
    expect(card.includes('#{card.firstSeq}-{card.lastSeq}')).toBe(true);
    expect(inspector.includes("'n/a'")).toBe(true);
  });
});
