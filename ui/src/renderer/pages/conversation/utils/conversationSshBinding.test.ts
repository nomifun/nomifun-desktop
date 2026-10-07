import { describe, expect, test } from 'bun:test';
import { firstSshHostId, secondSshHostId, makeSshConversation } from '../../../../../test/fixtures/sshConversation';
import { conversationSshHostId } from './conversationSshBinding';
import { isOrdinaryWorkConversation } from '../SessionList/hooks/conversationListFilter';
import { isSameConversationList } from '../SessionList/hooks/useConversationListSync';

describe('canonical SSH conversation identity', () => {
  test('a host-bound session uses the canonical resource even when extra contains only workspace', () => {
    const conversation = makeSshConversation();
    expect(conversation.extra).toEqual({ workspace: '' });
    expect(conversationSshHostId(conversation)).toBe(firstSshHostId);
    expect(isOrdinaryWorkConversation(conversation)).toBe(false);
  });

  test('a local session remains ordinary despite its preset advertising the SSH capability', () => {
    const local = makeSshConversation(null, 2);
    expect(conversationSshHostId(local)).toBeUndefined();
    expect(isOrdinaryWorkConversation(local)).toBe(true);
    expect(conversationSshHostId({})).toBeUndefined();
    expect(conversationSshHostId(null)).toBeUndefined();
  });

  test('host changes and binding removal are visible even when modified_at is unchanged', () => {
    const first = makeSshConversation();
    const second = makeSshConversation(secondSshHostId, 2);
    const removed = makeSshConversation(null, 3);
    expect(first.modified_at).toBe(second.modified_at);
    expect(isSameConversationList([first], [makeSshConversation(firstSshHostId, 2)])).toBe(false);
    expect(conversationSshHostId(second)).toBe(secondSshHostId);
    expect(isSameConversationList([first], [second])).toBe(false);
    expect(isSameConversationList([second], [removed])).toBe(false);
    expect(isOrdinaryWorkConversation(removed)).toBe(true);
    expect(isSameConversationList([second], [makeSshConversation(secondSshHostId, 2)])).toBe(true);
  });

  test('other resource kinds never become SSH identity', () => {
    const conversation = makeSshConversation();
    conversation.agent_snapshot!.canonical_binding!.typed_resource_bindings[0].resource_kind = 'workspace';
    expect(conversationSshHostId(conversation)).toBeUndefined();
    expect(isOrdinaryWorkConversation(conversation)).toBe(true);
  });
});
