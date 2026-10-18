import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const source = read('./nomi/NomiSendBox.tsx');
const queue = read('./useConversationCommandQueue.ts');

describe('ordinary input delivery ownership', () => {
  test('uses one intake without client execution or recovery routing', () => {
    expect(source.match(/conversation\.sendMessage\.invoke\(/g)).toHaveLength(1);
    expect(source.includes('conversation.steer.invoke')).toBe(false);
    expect(source.includes('resumeInput')).toBe(false);
    expect(source.includes('inputAction')).toBe(false);
    expect(source.includes('markTurnAccepted(')).toBe(false);
    expect(source.includes('reconcileAfterStreamTerminal();')).toBe(true);
  });
  test('retries unknown input with its stored key and payload', () => {
    expect(source.includes('pendingInputDelivery(conversation_id, input, files)')).toBe(true);
    expect(source.includes('idempotency_key: id ?? attempt!.key')).toBe(true);
    expect(source.includes('inject_skills: attempt?.injectSkills ?? skills')).toBe(true);
    expect(source.includes("error.code === 'INPUT_NOT_ACCEPTED'")).toBe(true);
  });
  test('preserves explicit queued and initial identities until canonical acceptance', () => {
    expect(queue.includes('id: uuidv7(),')).toBe(true);
    expect(queue.includes('items: restoreQueuedCommand(state.items, nextCommand)')).toBe(true);
    expect(source.includes('{ id: idempotency_key, input, files, initialOnly: true }')).toBe(true);
    expect(source.includes('completeInitialMessageDelivery(sessionStorage, storageKey, idempotency_key)')).toBe(true);
  });
});
