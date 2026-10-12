import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { parseConversationId, parseProviderId } from '@/common/types/ids';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { CompanionHandle } from '../workspace/types';
import { saveCompanionChatModel } from './saveCompanionChatModel';

const conversation = { id: parseConversationId('019b0000-0000-7000-8000-000000000011') };
const provider = parseProviderId('019b0000-0000-7000-8000-000000000012');
const previous = { provider_id: provider, model: 'model-a' };
const next = { provider_id: provider, model: 'model-b' };
const restores: Array<() => void> = [];
afterEach(() => restores.splice(0).reverse().forEach((restore) => restore()));
const patcher = (fn: (...args: unknown[]) => Promise<unknown>) => fn as CompanionHandle['patchCompanion'];

test('a companion model edit switches the same canonical Session before saving its default', async () => {
  const calls: string[] = [];
  const switchModel = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockImplementation(async () => { calls.push('session'); return true; });
  restores.push(() => switchModel.mockRestore());
  const result = await saveCompanionChatModel({ conversation, sessionModel: previous, profileModel: previous, next,
    onSessionChanged: () => calls.push('committed'),
    patchCompanion: patcher(async (patch, options) => { calls.push('default'); expect(patch).toEqual({ model: next }); expect(options).toEqual({ optimistic: false }); return {}; }),
  });
  expect(result).toEqual({ status: 'saved' });
  expect(calls).toEqual(['session', 'committed', 'default']);
  expect(switchModel.mock.calls[0][0]).toEqual({ conversation_id: conversation.id, provider_id: provider, model: 'model-b' });
});

test.each(['AGENT_SESSION_TURN_ACTIVE', 'AGENT_SESSION_EFFECTS_UNSETTLED'])('a refused canonical transition does not change profile defaults (%s)', async (code) => {
  const refusal = new BackendHttpError({ method: 'PUT', path: '/api/agent-sessions/test/model', status: 409,
    body: { code, error: 'native Session is not idle and settled' } });
  const switchModel = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockRejectedValue(refusal);
  restores.push(() => switchModel.mockRestore());
  let profileSaved = false;
  await expect(saveCompanionChatModel({ conversation, sessionModel: previous, profileModel: previous, next,
    patchCompanion: patcher(async () => { profileSaved = true; return {}; }),
  })).rejects.toBe(refusal);
  expect(profileSaved).toBe(false);
});

test('a failed default save reports the already committed Session and its retry does not switch again', async () => {
  const switchModel = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockResolvedValue(true);
  restores.push(() => switchModel.mockRestore());
  let committed = previous;
  const failed = await saveCompanionChatModel({ conversation, sessionModel: previous, profileModel: previous, next,
    onSessionChanged: (model) => { committed = model; }, patchCompanion: patcher(async () => { throw new Error('disk unavailable'); }),
  });
  expect(failed).toEqual({ status: 'default-save-failed' });
  expect(committed).toEqual(next);
  const retry = await saveCompanionChatModel({ conversation, sessionModel: committed, profileModel: previous, next,
    patchCompanion: patcher(async () => ({})),
  });
  expect(retry).toEqual({ status: 'saved' });
  expect(switchModel).toHaveBeenCalledTimes(1);
});

test('an unconfigured companion saves its first default without inventing a Session binding', async () => {
  const switchModel = spyOn(ipcBridge.conversation.switchModel, 'invoke');
  restores.push(() => switchModel.mockRestore());
  const result = await saveCompanionChatModel({ next, profileModel: null, patchCompanion: patcher(async () => ({})) });
  expect(result).toEqual({ status: 'saved' });
  expect(switchModel).not.toHaveBeenCalled();
  const failure = new Error('profile unavailable');
  await expect(saveCompanionChatModel({ next, patchCompanion: patcher(async () => { throw failure; }) })).rejects.toBe(failure);
});
