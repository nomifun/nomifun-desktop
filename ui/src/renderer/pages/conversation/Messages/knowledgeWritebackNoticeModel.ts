import type { KnowledgeWritebackState, TMessage } from '@/common/chat/chatLib';
import { parseKnowledgeBaseId, type KnowledgeBaseId } from '@/common/types/ids';

export type WrittenKnowledgeFile = { kb_id: KnowledgeBaseId; rel_path: string };
export type KnowledgeWritebackNotice = {
  status: KnowledgeWritebackState['status'];
  files: WrittenKnowledgeFile[];
  detail?: string;
  diagnostic?: KnowledgeWritebackState['diagnostic'];
  event_seq?: number;
};

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

/** Presentation uses canonical progress and settled owner results, never assistant claims. */
export function knowledgeWritebackFromMessage(message: TMessage): KnowledgeWritebackNotice | undefined {
  if (message.type === 'agent_status' && message.content.turn_summary) {
    const report = message.content.knowledge_writeback;
    if (!report || report.status === 'no_candidate') return;
    const files = (report.written ?? []).flatMap(target => { const parsed = file(target); return parsed ? [parsed] : []; });
    if ((report.status === 'written' || report.status === 'partial') && !files.length) return;
    return { status: report.status, files, detail: report.detail, diagnostic: report.diagnostic, event_seq: report.event_seq };
  }
  if (message.type !== 'tool_call' || message.content.capability_id !== 'knowledge' ||
    message.content.action_id !== 'knowledge/write') return;
  if (message.content.status !== 'completed') return;
  const parsed = file(message.content.written_knowledge_file);
  return parsed ? { files: [parsed], status: 'written' } : undefined;
}

export function mergeKnowledgeWritebackNotices(
  previous: KnowledgeWritebackNotice | undefined,
  incoming: KnowledgeWritebackNotice,
): KnowledgeWritebackNotice {
  const files = new Map<string, WrittenKnowledgeFile>();
  for (const target of [...(previous?.files ?? []), ...incoming.files]) {
    files.set(`${target.kb_id}:${target.rel_path}`, target);
  }
  const preferred = previous && (previous.event_seq !== undefined || incoming.event_seq !== undefined)
    ? (incoming.event_seq ?? 0) > (previous.event_seq ?? 0) ? incoming : previous
    : incoming;
  const status = previous?.event_seq === undefined && incoming.event_seq === undefined
    && (previous?.status === 'partial' || incoming.status === 'partial') ? 'partial' : preferred.status;
  return { ...preferred, status, files: [...files.values()] };
}
