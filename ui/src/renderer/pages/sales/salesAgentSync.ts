import { ipcBridge } from '@/common';
import type { IMessageText, TMessage } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import { getLastAssistantText } from '@/renderer/utils/chat/getLastAssistantText';
import { extractSalesDashboardEvents, type SalesAgentSnapshot } from './salesAgentReport';
import type { SalesTask } from './salesWorkspace';

const isAssistantText = (message: TMessage): message is IMessageText =>
  message.type === 'text' &&
  message.position === 'left' &&
  !message.hidden &&
  typeof message.content?.content === 'string';

const assistantTranscript = (messages: TMessage[]) =>
  messages
    .filter(isAssistantText)
    .map((message) => message.content.content)
    .join('\n');

export const loadSalesAgentSnapshot = async (task: SalesTask): Promise<SalesAgentSnapshot | null> => {
  if (!task.conversationId) return null;
  const conversationId = parseConversationId(task.conversationId);
  const [conversation, messagePage] = await Promise.all([
    ipcBridge.conversation.get.invoke({ conversation_id: conversationId }),
    ipcBridge.database.getConversationMessages.invoke({
      conversation_id: conversationId,
      page: 0,
      page_size: 10000,
      content_mode: 'compact',
    }),
  ]);
  if (!conversation) return null;

  const messages = messagePage?.items ?? [];
  const processing = Boolean(
    conversation.runtime?.is_processing ||
      conversation.status === 'pending' ||
      conversation.status === 'running'
  );
  const latest = getLastAssistantText(messages, processing) ?? task.agentSummary ?? '';

  return {
    taskId: task.id,
    taskStatus: processing ? 'running' : conversation.status === 'finished' ? 'completed' : task.status,
    agentSummary: latest.slice(0, 4000),
    syncedAt: new Date().toISOString(),
    events: extractSalesDashboardEvents(assistantTranscript(messages)),
  };
};
