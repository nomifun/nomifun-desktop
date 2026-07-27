import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const sessionListSource = readFileSync(new URL('./SessionList/index.tsx', import.meta.url), 'utf8');
const conversationActionsSource = readFileSync(
  new URL('./SessionList/hooks/useConversationActions.ts', import.meta.url),
  'utf8'
);
const bridgeSource = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');

describe('authoritative conversation deletion route handling', () => {
  test('clears the matching active conversation and replaces its deleted route', () => {
    expect(source.includes("if (event.conversation_id !== conversationId)")).toBe(true);
    expect(source.includes("if (event.action === 'deleted')")).toBe(true);
    expect(source.includes("emitter.emit('conversation.deleted', conversationId)")).toBe(true);
    expect(source.includes('void mutate(undefined, { revalidate: false })')).toBe(true);
    expect(source.includes("void navigate('/guid', { replace: true })")).toBe(true);
  });

  test('does not turn non-deleted list changes into local deletion success', () => {
    const deletedBranch = source.indexOf("if (event.action === 'deleted')");
    const localDelete = source.indexOf("emitter.emit('conversation.deleted', conversationId)");
    const ordinaryBranch = source.indexOf("if (event.action !== 'updated' && event.action !== 'created')");

    expect(deletedBranch).toBeGreaterThan(-1);
    expect(localDelete).toBeGreaterThan(deletedBranch);
    expect(localDelete).toBeLessThan(ordinaryBranch);
  });

  test('treats every resolved no-content DELETE as success', () => {
    expect(
      bridgeSource.includes("remove: httpDelete<void, { conversation_id: ConversationId }>")
    ).toBe(true);
    expect(
      conversationActionsSource.includes(
        'await ipcBridge.conversation.remove.invoke({ conversation_id: conversation_id });'
      )
    ).toBe(true);
    expect(
      conversationActionsSource.includes(
        'const success = await ipcBridge.conversation.remove.invoke'
      )
    ).toBe(false);
    expect(
      sessionListSource.includes('const success = await ipcBridge.conversation.remove.invoke')
    ).toBe(false);
  });
});
