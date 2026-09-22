import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');
const modelSelectorSource = readFileSync(
  new URL('../platforms/nomi/NomiModelSelector.tsx', import.meta.url),
  'utf8',
);

describe('Versioned Conversation model authority', () => {
  test('switches only the model binding while collaboration remains frozen', () => {
    expect(source.includes('readOnly: true')).toBe(false);
    expect(modelSelectorSource.includes('selection.pickerDisabled')).toBe(true);
    expect(source.includes('disabledReason={frozenSessionConfigHint}')).toBe(true);
    expect(source.includes('ipcBridge.conversation.switchModel.invoke')).toBe(true);
    expect(source.includes('ipcBridge.conversation.update.invoke')).toBe(false);
    expect(source.includes('ipcBridge.conversation.stop.invoke')).toBe(false);
    expect(source.includes('modelSelectionDisabled={modelSwitching}')).toBe(true);
    expect(source.includes("enabled_capabilities.includes('agent.collaboration')")).toBe(true);
    expect(source.includes('const collaborationControlNode = collaborationAvailable ?')).toBe(true);
  });
});
