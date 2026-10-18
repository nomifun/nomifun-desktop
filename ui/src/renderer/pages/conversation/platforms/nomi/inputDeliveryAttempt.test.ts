import { expect, test } from 'bun:test';
import { parseConversationId } from '@/common/types/ids';
import { settleInputDelivery, inputDeliveryAttempt, pendingInputDelivery } from './inputDeliveryAttempt';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';

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
