import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { IKnowledgeConsumer } from '@/common/adapter/ipcBridge';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import { sessionConsumers, unmountConsumer, unmountSessionConsumers } from './knowledgeUnmount';

const KB_A = parseKnowledgeBaseId('019b0000-0000-7000-8000-000000000001');
const KB_B = parseKnowledgeBaseId('019b0000-0000-7000-8000-000000000002');

const sessionConsumer = (targetId: string | null, enabled = true): IKnowledgeConsumer => ({
  target_kind: 'conversation',
  target_id: targetId,
  enabled,
});

const sessionBinding = (kbIds = [KB_A, KB_B]) => ({
  enabled: true,
  writeback: false,
  writeback_eagerness: 'manual' as const,
  kb_ids: kbIds,
});

const spies: { mockRestore(): void }[] = [];
afterEach(() => {
  while (spies.length > 0) spies.pop()?.mockRestore();
});
const track = <T extends { mockRestore(): void }>(spy: T): T => {
  spies.push(spy);
  return spy;
};

describe('knowledge unmount helpers', () => {
  test('sessionConsumers keeps only live session selections that fence deletion', () => {
    const consumers: IKnowledgeConsumer[] = [
      sessionConsumer('session-1'),
      sessionConsumer('session-2', false),
      { target_kind: 'workpath', target_id: 'C:/ws', enabled: true },
      { target_kind: 'terminal', target_id: 'term-1', enabled: true },
      { target_kind: 'conversation', target_id: null, enabled: true },
    ];
    expect(sessionConsumers(consumers).map((c) => c.target_id)).toEqual(['session-1', 'session-2']);
  });

  test('conversation consumers are detached through the canonical session update', async () => {
    const getKnowledge = track(
      spyOn(ipcBridge.agentPlatform.sessions.getKnowledge, 'invoke').mockResolvedValue(sessionBinding())
    );
    const updateKnowledge = track(
      spyOn(ipcBridge.agentPlatform.sessions.updateKnowledge, 'invoke').mockResolvedValue(sessionBinding([KB_B]))
    );
    await unmountConsumer(sessionConsumer('session-1'), KB_A);
    expect(getKnowledge).toHaveBeenCalledWith({ agent_session_id: 'session-1' });
    expect(updateKnowledge).toHaveBeenCalledWith({
      agent_session_id: 'session-1',
      binding: { enabled: true, writeback: false, writeback_eagerness: 'manual', kb_ids: [KB_B] },
    });
  });

  test('a session binding already missing the base is not rewritten', async () => {
    track(spyOn(ipcBridge.agentPlatform.sessions.getKnowledge, 'invoke').mockResolvedValue(sessionBinding([KB_B])));
    const updateKnowledge = track(
      spyOn(ipcBridge.agentPlatform.sessions.updateKnowledge, 'invoke').mockResolvedValue(sessionBinding([KB_B]))
    );
    await unmountConsumer(sessionConsumer('session-1'), KB_A);
    expect(updateKnowledge).not.toHaveBeenCalled();
  });

  test('non-session consumers go through the knowledge binding API', async () => {
    track(
      spyOn(ipcBridge.knowledge.getBinding, 'invoke').mockResolvedValue({
        enabled: true,
        writeback: true,
        writeback_eagerness: 'auto',
        channel_write_enabled: true,
        kb_ids: [KB_A],
      })
    );
    const setBinding = track(
      spyOn(ipcBridge.knowledge.setBinding, 'invoke').mockResolvedValue({
        enabled: false,
        writeback: false,
        writeback_eagerness: 'manual',
        channel_write_enabled: true,
        kb_ids: [],
      })
    );
    await unmountConsumer({ target_kind: 'workpath', target_id: 'C:/ws', enabled: true }, KB_A);
    expect(setBinding).toHaveBeenCalledWith({
      kind: 'workpath',
      target_id: 'C:/ws',
      enabled: false,
      writeback: false,
      writeback_eagerness: 'manual',
      channel_write_enabled: true,
      kb_ids: [],
    });
  });

  test('unmountSessionConsumers skips non-session rows and aborts on the first failure', async () => {
    track(spyOn(ipcBridge.agentPlatform.sessions.getKnowledge, 'invoke').mockResolvedValue(sessionBinding()));
    const updateKnowledge = track(
      spyOn(ipcBridge.agentPlatform.sessions.updateKnowledge, 'invoke')
        .mockResolvedValueOnce(sessionBinding([KB_B]))
        .mockRejectedValueOnce(new Error('turn active'))
    );
    const consumers: IKnowledgeConsumer[] = [
      { target_kind: 'workpath', target_id: 'C:/ws', enabled: true },
      sessionConsumer('session-1'),
      sessionConsumer('session-2'),
      sessionConsumer('session-3'),
    ];
    await expect(unmountSessionConsumers(consumers, KB_A)).rejects.toThrow('turn active');
    expect(updateKnowledge).toHaveBeenCalledTimes(2);
  });
});
