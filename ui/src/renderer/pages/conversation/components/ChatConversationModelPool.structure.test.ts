import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');
const modelSelectorSource = readFileSync(
  new URL('../platforms/nomi/NomiModelSelector.tsx', import.meta.url),
  'utf8',
);

describe('Versioned Conversation model authority', () => {
  test('switches only the model binding without restoring the redundant collaboration trigger', () => {
    expect(source.includes('readOnly: true')).toBe(false);
    expect(modelSelectorSource.includes('selection.pickerDisabled')).toBe(true);
    expect(source.includes('ipcBridge.conversation.switchModel.invoke')).toBe(true);
    expect(source.includes('ipcBridge.conversation.update.invoke')).toBe(false);
    expect(source.includes('ipcBridge.conversation.stop.invoke')).toBe(false);
    expect(source.includes('modelSelectionDisabled={modelSwitching || agentSwitch?.applying === true}')).toBe(true);
    expect(source.includes('CollaborationComposerControl')).toBe(false);
    expect(source.includes('collaborationControlNode')).toBe(false);
  });
});
