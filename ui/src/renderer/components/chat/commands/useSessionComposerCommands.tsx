import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';
import { useConversationExport } from '@/renderer/hooks/file/useConversationExport';
import { getLastAssistantText } from '@/renderer/utils/chat/getLastAssistantText';
import { fetchLatestConversationReply } from '@/renderer/utils/chat/conversationExportData';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { emitter } from '@/renderer/utils/emitter';
import { Message } from '@arco-design/web-react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { ConversationExportPanel } from './ConversationExportPanel';
import type { ComposerCommand, ComposerCommandExecution, ComposerCommandPanel } from './types';
import { useSessionCommandActivity, type SessionCommandActivity } from './useSessionCommandActivity';

/** Session actions consume the existing authenticated APIs. No runtime warmup,
 * Session creation or authorization is performed by opening this catalog. */
export function useSessionComposerCommands({ conversationId, workspace, openFiles, activity: suppliedActivity, disabledReason }: {
  conversationId?: ConversationId;
  workspace?: string;
  openFiles?: (execution: ComposerCommandExecution) => void | Promise<void>;
  activity?: SessionCommandActivity;
  disabledReason?: string;
}) {
  const { t } = useTranslation();
  const activity = useSessionCommandActivity(conversationId, suppliedActivity);
  const exportFlow = useConversationExport({ conversation_id: conversationId, workspace, t, messageApi: Message });
  const commands = useMemo<ComposerCommand[]>(() => {
    const result: ComposerCommand[] = [];
    if (openFiles) result.push({ id: 'files.open', name: 'open', description: t('conversation.workspace.addFile'),
      ...(disabledReason ? { disabledReason } : { execute: openFiles }) });
    const noSession = !conversationId ? t('messages.commands.sessionRequired') : undefined;
    result.push({ id: 'conversation.copy', name: 'copy', description: t('messages.copy'),
      ...(noSession || activity !== 'idle'
        ? { disabledReason: noSession || t(activity === 'unknown' ? 'messages.commands.stateUnconfirmed' : 'messages.commands.replyAfterTurn') }
        : { execute: async execution => {
          const messages = await fetchLatestConversationReply(conversationId!, execution.isCurrent);
          if (!execution.isCurrent()) return;
          const text = getLastAssistantText(messages, false);
          if (!text) { Message.warning(t('messages.copyLastOutput.empty')); return; }
          await copyText(text);
          if (execution.isCurrent()) Message.success(t('messages.copySuccess'));
        } }) });
    result.push({ id: 'conversation.export', name: 'export', description: t('messages.export.commandDescription'),
      ...(noSession ? { disabledReason: noSession } : { execute: exportFlow.openExportFlow }) });
    const clearBlocked = noSession || disabledReason || (activity !== 'idle'
      ? t(activity === 'unknown' ? 'messages.commands.stateUnconfirmed' : 'messages.commands.clearAfterTurn') : undefined);
    result.push({ id: 'session.clear-context', name: 'clear', description: t('conversation.clearContext.description'),
      ...(clearBlocked ? { disabledReason: clearBlocked } : { execute: async execution => {
        await ipcBridge.conversation.clearContext.invoke({ conversation_id: conversationId!, idempotency_key: execution.idempotencyKey });
        if (execution.isCurrent()) {
          emitter.emit('chat.history.refresh');
          Message.success(t('conversation.clearContext.success'));
        }
      } }) });
    return result;
  }, [activity, conversationId, disabledReason, exportFlow.openExportFlow, openFiles, t]);
  const panel: ComposerCommandPanel = {
    isOpen: exportFlow.isOpen,
    node: <ConversationExportPanel flow={exportFlow} />,
    onKeyDown: exportFlow.handleKeyDown,
    close: exportFlow.closeExportFlow,
  };
  return { commands, panel };
}
