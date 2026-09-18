import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./client.ts', import.meta.url), 'utf8');

describe('Browser AgentSession client contract', () => {
  test('uses only canonical AgentSession resource routes and identities', () => {
    expect(source).toContain('/api/agent-sessions/${encodeURIComponent(id)}/browser');
    expect(source).toContain('agent_session_id: string');
    expect(source).toContain('{ agentSessionId: id, bounds, events }');
    expect(source).not.toContain('/api/conversations/');
    expect(source).not.toContain('conversation_id: string');
    expect(source).toContain("httpRequest('GET', '/api/browser-providers/attached-chrome')");
    expect(source).toContain("'connected' | 'connection_lost' | 'disconnecting' | 'cleanup_failed'");
  });

  test('keeps provider identity in every snapshot merge boundary', () => {
    expect(source).toContain("export type BrowserProviderKind = 'managed' | 'attached_chrome'");
    expect(source).toContain('current.resource_binding_id !== incoming.resource_binding_id');
    expect(source).toContain('current.provider_id !== incoming.provider_id');
    expect(source).toContain('current.provider_kind !== incoming.provider_kind');
    expect(source).toContain('allowed_actions: BrowserActionGrant[]');
  });

  test('maps every human command to the exact server Action gate', () => {
    expect(source).toContain("return 'browser/navigate'");
    expect(source).toContain("return 'browser/download'");
    expect(source).toContain("return 'browser/act'");
  });
});
