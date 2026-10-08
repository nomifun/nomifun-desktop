import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const conversation = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');

describe('conversation system-permission reminder placement', () => {
  test('mounts beside existing conversation header controls', () => {
    expect(conversation.includes('<SystemPermissionReminder')).toBe(true);
    expect(conversation.includes('snapshot={conversation.agent_snapshot}')).toBe(true);
  });

  test('does not grow the shared ChatLayout contract for one permission capability', () => {
    const layout = readFileSync(new URL('./ChatLayout/index.tsx', import.meta.url), 'utf8');
    expect(layout.includes('SystemPermissionReminder')).toBe(false);
    expect(layout.includes('systemPermissions')).toBe(false);
  });
});
