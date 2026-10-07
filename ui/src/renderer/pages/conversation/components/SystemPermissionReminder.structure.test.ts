import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const conversation = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');
const companion = readFileSync(
  new URL('../../nomi/companion/CompanionChatPanel.tsx', import.meta.url),
  'utf8'
);

describe('conversation system-permission reminder placement', () => {
  test('mounts beside existing header controls for ordinary and companion conversations', () => {
    expect(conversation.includes('<SystemPermissionReminder')).toBe(true);
    expect(conversation.includes('snapshot={conversation.agent_snapshot}')).toBe(true);
    expect(companion.includes('<SystemPermissionReminder')).toBe(true);
    expect(companion.includes('snapshot={conversation.agent_snapshot}')).toBe(true);
  });

  test('does not grow the shared ChatLayout contract for one permission capability', () => {
    const layout = readFileSync(new URL('./ChatLayout/index.tsx', import.meta.url), 'utf8');
    expect(layout.includes('SystemPermissionReminder')).toBe(false);
    expect(layout.includes('systemPermissions')).toBe(false);
  });
});
