import { expect, test } from 'bun:test';
import type { IMessageAgentStatus, IMessageToolCall } from '@/common/chat/chatLib';
import { normalizeAgentStatusContent } from '@/common/chat/chatLib';
import { parseConversationId, parseKnowledgeBaseId } from '@/common/types/ids';
import { knowledgeWritebackFromMessage, mergeKnowledgeWritebackNotices } from './knowledgeWritebackNoticeModel';

const kb = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000001');
const conversation = parseConversationId('0190f5fe-7c00-7a00-8000-000000000002');
const status = (state: string, event_seq?: number): IMessageAgentStatus => ({
  id: 'summary', conversation_id: conversation, type: 'agent_status',
  content: normalizeAgentStatusContent({ backend: 'nomi', status: 'prepared', turn_summary: true,
    knowledge_writeback: { status: state, event_seq, written: [{ kb_id: kb, rel_path: 'notes/loops.md' }] } }),
});
const tool = (state: 'completed' | 'error' | 'running'): IMessageToolCall => ({
  id: 'tool', conversation_id: conversation, type: 'tool_call', content: {
    call_id: 'call', name: 'knowledge_write', capability_id: 'knowledge', action_id: 'knowledge/write', status: state,
    args: { base: kb, rel_path: 'notes/loops.md' },
    output: JSON.stringify({ resource_id: kb, relative_path: 'notes/loops.md', created: true }),
  },
});

test('canonical receipts distinguish progress, actual success and unconfirmed/cancelled results', () => {
  expect(knowledgeWritebackFromMessage(status('written'))?.files).toEqual([{ kb_id: kb, rel_path: 'notes/loops.md' }]);
  expect(knowledgeWritebackFromMessage(status('partial'))?.status).toBe('partial');
  for (const state of ['running', 'failed', 'cancelled', 'interrupted', 'no_completer']) {
    expect(knowledgeWritebackFromMessage(status(state, 12))?.status).toBe(state);
  }
  for (const state of ['disabled', 'no_candidate', 'writing']) expect(knowledgeWritebackFromMessage(status(state, 12))).toBeUndefined();
});
test('only the successful first-party owner result proves a tool write', () => {
  expect(knowledgeWritebackFromMessage(tool('completed'))?.files).toHaveLength(1);
  expect(knowledgeWritebackFromMessage(tool('error'))).toBeUndefined();
  expect(knowledgeWritebackFromMessage(tool('running'))).toBeUndefined();
  const plugin = tool('completed'); plugin.content.capability_id = 'plugin.knowledge';
  expect(knowledgeWritebackFromMessage(plugin)).toBeUndefined();
  const guessed = tool('completed'); guessed.content.output = '已写入知识库';
  expect(knowledgeWritebackFromMessage(guessed)).toBeUndefined();
  guessed.content.output = JSON.stringify({ resource_id: kb, relative_path: 'notes/loops.md' });
  expect(knowledgeWritebackFromMessage(guessed)).toBeUndefined();
});
test('the same tool and finalizer target appears once, while different libraries stay distinct', () => {
  const explicit = knowledgeWritebackFromMessage(tool('completed'))!;
  const automatic = knowledgeWritebackFromMessage(status('written'))!;
  expect(mergeKnowledgeWritebackNotices(explicit, automatic).files).toHaveLength(1);
  automatic.files[0]!.kb_id = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000003');
  expect(mergeKnowledgeWritebackNotices(explicit, automatic).files).toHaveLength(2);
});
test('a cancelled or interrupted write-back retains settled tool files and its canonical result state', () => {
  const explicit = knowledgeWritebackFromMessage(tool('completed'))!;
  for (const state of ['cancelled', 'interrupted']) {
    const asynchronous = knowledgeWritebackFromMessage(status(state, 13))!;
    asynchronous.files = [];
    const merged = mergeKnowledgeWritebackNotices(asynchronous, explicit);
    expect(merged.status).toBe(state);
    expect(merged.event_seq).toBe(13);
    expect(merged.files).toEqual(explicit.files);
  }
  const completed = knowledgeWritebackFromMessage(status('written', 14))!;
  expect(mergeKnowledgeWritebackNotices(completed, knowledgeWritebackFromMessage(status('running', 12))!).status).toBe('written');
});
test('canonical structured diagnostic follows its original Turn and wins over a stale receipt', () => {
  const settled = status('failed', 13);
  settled.content.knowledge_writeback!.diagnostic = { stage: 'decode', code: 'USER_LLM_PROVIDER_INVALID_RESPONSE',
    elapsed_ms: 1200, received_frames: 3, received_bytes: 400, output_bytes: 200 };
  const notice = knowledgeWritebackFromMessage(settled)!;
  expect(notice.diagnostic).toEqual(settled.content.knowledge_writeback!.diagnostic);
  expect(mergeKnowledgeWritebackNotices(notice, knowledgeWritebackFromMessage(status('running', 12))!).diagnostic).toEqual(notice.diagnostic);
});
