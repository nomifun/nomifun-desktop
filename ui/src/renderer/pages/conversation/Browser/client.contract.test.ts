import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
const source = readFileSync(new URL('./client.ts', import.meta.url), 'utf8');
describe('User side browser client contract', () => {
  test('scopes the domain browser to the current canonical Session', () => {
    expect(source).toContain('/api/agent-sessions/${encodeURIComponent(id)}/browser');
    expect(source).toContain('agent_session_id: string');
    expect(source).toContain('{ agentSessionId: id, bounds, events }');
    expect(source).not.toContain('/api/conversations/');
  });
  test('does not import Agent grants, resource bindings or attached Chrome state', () => {
    expect(source).toContain('browser_id: string');
    expect(source).toContain('current.browser_id !== incoming.browser_id');
    for (const agentField of ['allowed_actions', 'resource_binding_id', 'attachedProvider', 'BrowserActionGrant']) {
      expect(source).not.toContain(agentField);
    }
  });
});
