import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');
const modelSelectorSource = readFileSync(
  new URL('../platforms/nomi/NomiModelSelector.tsx', import.meta.url),
  'utf8',
);

describe('Versioned Conversation model authority', () => {
  test('switches only the model binding and passes the frozen collaboration capability to the shared composer', () => {
    expect(source.includes('readOnly: true')).toBe(false);
    expect(modelSelectorSource.includes('selection.pickerDisabled')).toBe(true);
    expect(source.includes('ipcBridge.conversation.switchModel.invoke')).toBe(true);
    expect(source.includes('ipcBridge.conversation.update.invoke')).toBe(false);
    expect(source.includes('ipcBridge.conversation.stop.invoke')).toBe(false);
    expect(source.includes('modelSelectionDisabled={modelSwitching || agentSwitching}')).toBe(true);
    expect(source.includes('CollaborationComposerControl')).toBe(false);
    expect(source.includes('collaborationControlNode')).toBe(false);
    expect(source.includes("collaborationEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('agent.collaboration') === true}")).toBe(true);
  });
});
