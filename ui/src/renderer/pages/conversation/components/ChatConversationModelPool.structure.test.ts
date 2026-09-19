import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');
const modelSelectorSource = readFileSync(
  new URL('../platforms/nomi/NomiModelSelector.tsx', import.meta.url),
  'utf8',
);

describe('Frozen Conversation model authority', () => {
  test('renders the bound model and collaboration defaults without issuing rejected Session mutations', () => {
    expect(source.includes('readOnly: true')).toBe(true);
    expect(modelSelectorSource.includes('selection.pickerDisabled')).toBe(true);
    expect(source.includes('disabledReason={frozenSessionConfigHint}')).toBe(true);
    expect(source.includes('ipcBridge.conversation.update.invoke')).toBe(false);
    expect(source.includes('ipcBridge.conversation.stop.invoke')).toBe(false);
    expect(source.includes("enabled_capabilities.includes('agent.collaboration')")).toBe(true);
    expect(source.includes('const collaborationControlNode = collaborationAvailable ?')).toBe(true);
  });
});
