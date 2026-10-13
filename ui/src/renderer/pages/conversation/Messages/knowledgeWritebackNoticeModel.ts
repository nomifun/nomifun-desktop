import type { TMessage } from '@/common/chat/chatLib';
import { normalizeToolCall } from '@/common/chat/normalizeToolCall';
import { parseKnowledgeBaseId, type KnowledgeBaseId } from '@/common/types/ids';

export type WrittenKnowledgeFile = { kb_id: KnowledgeBaseId; rel_path: string };
export type KnowledgeWritebackNotice = { files: WrittenKnowledgeFile[]; partial: boolean };

function file(value: unknown): WrittenKnowledgeFile | undefined {
  if (!value || typeof value !== 'object') return;
  const row = value as Record<string, unknown>;
  const id = row.kb_id ?? row.resource_id;
  const path = row.rel_path ?? row.relative_path;
  if (typeof id !== 'string' || typeof path !== 'string' || !path || path.length > 4096 ||
    path.startsWith('/') || path.includes('\\') || /^[A-Za-z]:/.test(path) || /[\u0000-\u001f\u007f]/.test(path) ||
    path.split('/').some(part => !part || part === '.' || part === '..')) return;
  try { return { kb_id: parseKnowledgeBaseId(id), rel_path: path }; } catch { return; }
}

/** Presentation uses settled owner results, never arguments or assistant claims. */
export function knowledgeWritebackFromMessage(message: TMessage): KnowledgeWritebackNotice | undefined {
  if (message.type === 'agent_status' && message.content.turn_summary) {
    const report = message.content.knowledge_writeback;
    if (report?.status !== 'written' && report?.status !== 'partial') return;
    const files = (report.written ?? []).flatMap(target => { const parsed = file(target); return parsed ? [parsed] : []; });
    return files.length ? { files, partial: report.status === 'partial' } : undefined;
  }
  if (message.type !== 'tool_call' || message.content.capability_id !== 'knowledge' ||
    message.content.action_id !== 'knowledge/write') return;
  const result = normalizeToolCall(message);
  if (result?.status !== 'completed' || !result.output) return;
  try {
    const receipt: unknown = JSON.parse(result.output);
    if (!receipt || typeof receipt !== 'object' || !('created' in receipt) || typeof receipt.created !== 'boolean') return;
    const parsed = file(receipt);
    return parsed ? { files: [parsed], partial: false } : undefined;
  } catch { return; }
}

export function mergeKnowledgeWritebackNotices(
  previous: KnowledgeWritebackNotice | undefined,
  incoming: KnowledgeWritebackNotice,
): KnowledgeWritebackNotice {
  const files = new Map<string, WrittenKnowledgeFile>();
  for (const target of [...(previous?.files ?? []), ...incoming.files]) {
    files.set(`${target.kb_id}:${target.rel_path}`, target);
  }
  return { files: [...files.values()], partial: previous?.partial === true || incoming.partial };
}
