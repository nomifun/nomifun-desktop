/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ConversationId, TerminalId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import { Message, Modal } from '@arco-design/web-react';
import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

type Params = {
  selectedConversationIds: Set<ConversationId>;
  selectedTerminalIds: Set<TerminalId>;
  setSelectedConversationIds: Dispatch<SetStateAction<Set<ConversationId>>>;
  setSelectedTerminalIds: Dispatch<SetStateAction<Set<TerminalId>>>;
  activeConversationId: ConversationId | null;
  activeTerminalId: TerminalId | null;
  onBatchModeChange?: (value: boolean) => void;
};

type SelectionReceipts = {
  conversationIds: Set<ConversationId>;
  terminalIds: Set<TerminalId>;
  deletedConversations: Set<ConversationId>;
  deletedTerminals: Set<TerminalId>;
};

export const useSessionBatchDelete = ({
  selectedConversationIds,
  selectedTerminalIds,
  setSelectedConversationIds,
  setSelectedTerminalIds,
  activeConversationId,
  activeTerminalId,
  onBatchModeChange,
}: Params) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pending = useRef(false);
  const mounted = useRef(true);
  const closeConfirmation = useRef<(() => void) | null>(null);
  const activeSession = useRef({ activeConversationId, activeTerminalId });
  activeSession.current = { activeConversationId, activeTerminalId };
  const [isDeleting, setIsDeleting] = useState(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      closeConfirmation.current?.();
    };
  }, []);

  const handleBatchDelete = useCallback(() => {
    // Keep one confirmation/submission in flight, even before React re-renders.
    if (pending.current) return;
    // The confirmation owns this exact selection; list updates cannot shorten it.
    const conversationIds = Array.from(selectedConversationIds);
    const terminalIds = Array.from(selectedTerminalIds);
    const total = conversationIds.length + terminalIds.length;
    if (total === 0) {
      Message.warning(t('conversation.history.batchNoSelection'));
      return;
    }
    pending.current = true;
    const receipts: SelectionReceipts = {
      conversationIds: new Set(conversationIds), terminalIds: new Set(terminalIds),
      deletedConversations: new Set(), deletedTerminals: new Set(),
    };
    let submission: Promise<void> | undefined;
    // These listeners belong to the confirmed batch, not the sidebar mount.
    // Closing/collapsing that UI cannot discard a later authoritative receipt.
    const recordConversationDeletion = (id: ConversationId) => {
      if (receipts.conversationIds.has(id)) receipts.deletedConversations.add(id);
    };
    emitter.on('conversation.deleted', recordConversationDeletion);
    const offConversation = ipcBridge.conversation.listChanged.on((event) => {
      if (event.action === 'deleted') recordConversationDeletion(event.conversation_id);
    });
    const offTerminal = ipcBridge.terminal.onRemoved.on((event) => {
      if (receipts.terminalIds.has(event.terminal_id)) receipts.deletedTerminals.add(event.terminal_id);
    });
    const release = () => {
      emitter.off('conversation.deleted', recordConversationDeletion);
      offConversation();
      offTerminal();
      closeConfirmation.current = null;
      pending.current = false;
    };

    const removeSelected = async () => {
      setIsDeleting(true);
      const { deletedConversations, deletedTerminals } = receipts;
      try {
        // Session deletion includes runtime/resource cleanup and database writes.
        // Await each receipt instead of flooding those owners with a full selection.
        for (const conversation_id of conversationIds) {
          if (deletedConversations.has(conversation_id)) continue;
          try {
            await ipcBridge.conversation.remove.invoke({ conversation_id });
          } catch (error) {
            console.error('Failed to batch delete conversation:', conversation_id, error);
            continue;
          }
          deletedConversations.add(conversation_id);
          emitter.emit('conversation.deleted', conversation_id);
        }
        for (const terminal_id of terminalIds) {
          if (deletedTerminals.has(terminal_id)) continue;
          try {
            await ipcBridge.terminal.remove.invoke({ terminal_id });
          } catch (error) {
            console.error('Failed to batch delete terminal:', terminal_id, error);
            continue;
          }
          deletedTerminals.add(terminal_id);
        }

        const active = activeSession.current;
        if (mounted.current && (
          (active.activeConversationId && deletedConversations.has(active.activeConversationId)) ||
          (active.activeTerminalId && deletedTerminals.has(active.activeTerminalId))
        )) {
          void navigate('/guid', { replace: true });
        }
        emitter.emit('chat.history.refresh');
        if (terminalIds.length > 0) emitter.emit('terminal.list.refresh');
        // Remove only confirmed successes. Failed items stay selected for retry.
        if (mounted.current) {
          setSelectedConversationIds((previous) => new Set(
            Array.from(previous).filter((id) => !deletedConversations.has(id))
          ));
          setSelectedTerminalIds((previous) => new Set(
            Array.from(previous).filter((id) => !deletedTerminals.has(id))
          ));
        }
        const deleted = deletedConversations.size + deletedTerminals.size;
        const failed = total - deleted;
        if (failed === 0) {
          Message.success(t('conversation.history.batchDeleteSuccess', { count: deleted }));
          if (mounted.current) onBatchModeChange?.(false);
        } else if (deleted > 0) {
          Message.warning(t('conversation.history.batchDeletePartial', { deleted, failed }));
        } else {
          Message.error(t('conversation.history.batchDeleteFailed', { count: failed }));
        }
      } finally {
        release();
        if (mounted.current) setIsDeleting(false);
      }
    };

    const modal = Modal.confirm({
      title: t('conversation.history.batchDelete', { count: total }),
      content: t('conversation.history.batchDeleteConfirm', { count: total }),
      okText: t('conversation.history.confirmDelete'),
      cancelText: t('conversation.history.cancelDelete'),
      okButtonProps: { status: 'warning' },
      onOk: () => (submission ??= removeSelected()),
      onCancel: () => {
        if (!submission) release();
      },
      style: { borderRadius: '12px' },
      alignCenter: true,
      getPopupContainer: () => document.body,
    });
    closeConfirmation.current = () => {
      modal.close();
      if (!submission) release();
    };
  }, [
    selectedConversationIds, selectedTerminalIds, setSelectedConversationIds,
    setSelectedTerminalIds, onBatchModeChange,
    navigate, t,
  ]);

  return { handleBatchDelete, isDeleting };
};
