import { normalizeDeliveryIntent, QUEUED_INPUT, type SessionInputDeliveryIntent } from '@/common/chat/inputSubmission';
import { conversationTarget, type ConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';

type Attempt = { fingerprint: string; key: string; content: string; files: string[]; injectSkills: string[]; delivery?: SessionInputDeliveryIntent };
const attempts = new Map<string, Attempt[]>();
const storageKey = (conversationId: ConversationId) => sessionStorageKey('nomi-input-delivery', conversationTarget(conversationId));

function read(key: string): Attempt[] {
  if (attempts.has(key)) return attempts.get(key)!;
  try {
    const stored: unknown = JSON.parse(sessionStorage.getItem(key) || '[]');
    if (Array.isArray(stored)) return stored.flatMap(value => {
      return typeof value?.fingerprint === 'string' && typeof value?.key === 'string'
        && typeof value?.content === 'string' && Array.isArray(value?.files) && value.files.every((file: unknown) => typeof file === 'string')
        && (value?.delivery === undefined || normalizeDeliveryIntent(value.delivery) !== undefined)
        && Array.isArray(value?.injectSkills) && value.injectSkills.every((skill: unknown) => typeof skill === 'string')
        ? [{ fingerprint: value.fingerprint, key: value.key, content: value.content, files: value.files, injectSkills: value.injectSkills, ...(value.delivery === undefined ? {} : { delivery: normalizeDeliveryIntent(value.delivery)! }) }] : [];
    });
  } catch { /* A blocked browser store still retains the mounted attempt. */ }
  return [];
}

const fingerprint = (input: string, files: string[]) => JSON.stringify({ input, files });

export function pendingInputDelivery(conversationId: ConversationId, input: string, files: string[]): Attempt | undefined {
  return read(storageKey(conversationId)).find(attempt => attempt.fingerprint === fingerprint(input, files));
}

/** A retry resubmits the original payload and key. Only the canonical intake
 * owns its execution claim; the composer never chooses a Turn or checkpoint. */
export function inputDeliveryAttempt(conversationId: ConversationId, input: string, files: string[], injectSkills: string[] = [], content = input, delivery: SessionInputDeliveryIntent = QUEUED_INPUT): Attempt {
  const key = storageKey(conversationId);
  const previous = read(key);
  const existing = previous.find(attempt => attempt.fingerprint === fingerprint(input, files));
  if (existing) return existing;
  if (previous.length >= 16) {
    throw new Error('Too many messages have unconfirmed delivery. Retry an earlier message before sending another.');
  }
  const attempt = { fingerprint: fingerprint(input, files), key: uuidv7(), content, files: [...files], injectSkills: [...injectSkills], delivery: Object.freeze({ ...delivery }) };
  const next = [...previous, attempt];
  attempts.set(key, next);
  try { sessionStorage.setItem(key, JSON.stringify(next)); } catch { /* Memory fallback. */ }
  return attempt;
}

/** Only an accepted receipt or explicit host proof of non-admission can end
 * this retry identity. A transport error or generic conflict is still unknown. */
export function settleInputDelivery(conversationId: ConversationId, acceptedKey: string): void {
  const key = storageKey(conversationId);
  const next = read(key).filter(attempt => attempt.key !== acceptedKey);
  attempts.set(key, next);
  try {
    if (next.length) sessionStorage.setItem(key, JSON.stringify(next));
    else sessionStorage.removeItem(key);
  } catch { /* The acknowledged disposition remains authoritative in memory. */ }
}
