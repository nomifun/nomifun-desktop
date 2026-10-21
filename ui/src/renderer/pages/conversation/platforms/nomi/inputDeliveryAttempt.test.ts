import '../../../../../../test/setup-dom.ts';
import { expect, test } from 'bun:test';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { settleInputDelivery, inputDeliveryAttempt, pendingInputDelivery } from './inputDeliveryAttempt';
import { setBrowserStorageGeneration, sessionStorageKey } from '@/common/utils/browserStorageKey';
import { conversationTarget } from '@/common/types/ids';

const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000995');

test('an unknown intake retains its original input payload and key across later configuration changes', () => {
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000994');
  const original = inputDeliveryAttempt(session, 'Dependency ready', ['/work/brief.txt'], ['original-skill'], 'Original formatted payload');
  expect(inputDeliveryAttempt(session, 'Dependency ready', ['/work/brief.txt'], ['other-skill'], 'New formatting after workspace hydration')).toEqual(original);
  expect(pendingInputDelivery(session, 'Dependency ready', ['/work/brief.txt'])).toEqual(original);
  expect(pendingInputDelivery(session, 'Different instruction', ['/work/brief.txt'])).toBeUndefined();
  expect(original.content).toBe('Original formatted payload');
  expect(original).not.toHaveProperty('target');
  settleInputDelivery(session, original.key);
  const next = inputDeliveryAttempt(session, 'Dependency ready', ['/work/brief.txt'], ['other-skill']);
  expect(next.key).not.toBe(original.key);
  expect(next.injectSkills).toEqual(['other-skill']);
  settleInputDelivery(session, next.key);
});

test('capacity refuses new inputs without evicting any unknown delivery', () => {
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000993');
  const pending = Array.from({ length: 16 }, (_, index) => inputDeliveryAttempt(session, `Draft ${index}`, [], ['original']));
  expect(() => inputDeliveryAttempt(session, 'New draft', [], ['new'])).toThrow('unconfirmed delivery');
  for (let index = 0; index < pending.length; index++) {
    expect(inputDeliveryAttempt(session, `Draft ${index}`, [], ['new'])).toEqual(pending[index]);
  }
  settleInputDelivery(session, pending[1].key);
  expect(inputDeliveryAttempt(session, 'New draft', [], ['new']).injectSkills).toEqual(['new']);
  expect(pendingInputDelivery(session, 'Draft 0', [])).toEqual(pending[0]);
});

test('unknown acknowledgement freezes mode and observed Turn across preference changes', () => {
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000992');
  const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000991');
  const original = inputDeliveryAttempt(session, 'Keep this supplement', [], [], 'Keep this supplement', { kind: 'prefer_current', turn_id: turn });
  const retry = inputDeliveryAttempt(session, 'Keep this supplement', [], [], 'new formatting', { kind: 'queue' });
  expect(retry).toEqual(original);
  expect(retry.delivery).toEqual({ kind: 'prefer_current', turn_id: turn });
  settleInputDelivery(session, original.key);
});

test('a current-generation unconfirmed envelope without delivery keeps its original identity', () => {
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000990');
  const key = sessionStorageKey('nomi-input-delivery', conversationTarget(session));
  const original = { fingerprint: JSON.stringify({ input: 'Original pending input', files: [] }), key: 'original-unknown-key',
    content: 'Original canonical envelope', files: [], injectSkills: ['original'] };
  sessionStorage.setItem(key, JSON.stringify([original]));
  expect(pendingInputDelivery(session, 'Original pending input', [])).toEqual(original);
  expect(inputDeliveryAttempt(session, 'Original pending input', [], [], 'changed', { kind: 'queue' })).toEqual(original);
  settleInputDelivery(session, original.key);
});
