/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { conversationTarget, type ConversationId, type MessageId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { ipcBridge } from '@/common';
import { uuid, uuidv7 } from '@/common/utils';
import AgentModeSelector from '@/renderer/components/agent/AgentModeSelector';
import CommandQueuePanel from '@/renderer/components/chat/CommandQueuePanel';
import MobileActionSheet, {
  type MobileActionSheetEntry,
  type MobileActionSheetOption,
  useAttachEntry,
} from '@/renderer/components/chat/MobileActionSheet';
import SendBox from '@/renderer/components/chat/SendBox';
import FileAttachButton from '@/renderer/components/media/FileAttachButton';
import FilePreview from '@/renderer/components/media/FilePreview';
import HorizontalFileList from '@/renderer/components/media/HorizontalFileList';
import SummonControl from '@/renderer/pages/conversation/components/SummonPanel';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { useAutoTitle } from '@/renderer/hooks/chat/useAutoTitle';
import { getSendBoxDraftHook, type FileOrFolderItem } from '@/renderer/hooks/chat/useSendBoxDraft';
import { createSetUploadFile, useSendBoxFiles } from '@/renderer/hooks/chat/useSendBoxFiles';
import { useSlashCommands } from '@/renderer/hooks/chat/useSlashCommands';
import { useOpenFileSelector } from '@/renderer/hooks/file/useOpenFileSelector';
import { useLatestRef } from '@/renderer/hooks/ui/useLatestRef';
import {
  snapshotEditSuffixLocalIds,
  useAddOrUpdateMessage,
  useMessageList,
  useRemoveMessageByMsgId,
  useRemoveMessagesByLocalIds,
} from '@/renderer/pages/conversation/Messages/hooks';
import { savePreferredMode } from '@/renderer/pages/guid/hooks/agentSelectionUtils';
import {
  shouldEnqueueConversationCommand,
  useConversationCommandQueue,
  type ConversationCommandQueueExecution,
  type ConversationCommandQueueItem,
} from '@/renderer/pages/conversation/platforms/useConversationCommandQueue';
import {
  claimInitialMessageDelivery,
  completeInitialMessageDelivery,
  handleInitialMessageDeliveryFailure,
  readAuthorizedInitialMessageDelivery,
  releaseInitialMessageDelivery,
} from '@/renderer/pages/conversation/platforms/initialMessageDelivery';
import { classifyPublicMessageDelivery } from '@/renderer/pages/conversation/platforms/publicMessageDelivery';
import {
  stopConversationAndConfirmRelease,
  waitForConversationTurnReleaseUntilSettled,
} from '@/renderer/pages/conversation/platforms/requestConversationStop';
import {
  shouldReleaseStopInteraction,
  useConversationStopAttemptGuard,
} from '@/renderer/pages/conversation/platforms/useConversationStopAttemptGuard';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { getConversationRuntimeWorkspaceErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import {
  warmupConversation,
  warmupConversationForPassiveMount,
} from '@/renderer/pages/conversation/utils/warmupConversation';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { allSupportedExts } from '@/renderer/services/FileService';
import { iconColors } from '@/renderer/styles/colors';
import { emitter, useAddEventListener } from '@/renderer/utils/emitter';
import { mergeFileSelectionItems } from '@/renderer/utils/file/fileSelection';
import { buildDisplayMessage, collectSelectedFiles } from '@/renderer/utils/file/messageFiles';
import type { AgentModeOption } from '@/renderer/utils/model/agentModes';
import { Message, Tag } from '@arco-design/web-react';
import { Brain, MagicHat, Shield } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { NomiMessageRuntime } from './useNomiMessage';
import NomiModelSelector from './NomiModelSelector';
import { ContextUsageRing } from './ContextUsageRing';
import type { NomiModelSelection } from './useNomiModelSelection';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { evaluateNomiVisionSend } from './nomiVisionSendGuard';

const useNomiSendBoxDraft = getSendBoxDraftHook('nomi', {
  _type: 'nomi',
  atPath: [],
  content: '',
  uploadFile: [],
});

const EMPTY_AT_PATH: Array<string | FileOrFolderItem> = [];
const EMPTY_UPLOAD_FILES: string[] = [];

const useSendBoxDraft = (conversation_id: ConversationId) => {
  const { data, mutate } = useNomiSendBoxDraft(conversation_id);

  const atPath = data?.atPath ?? EMPTY_AT_PATH;
  const uploadFile = data?.uploadFile ?? EMPTY_UPLOAD_FILES;
  const content = data?.content ?? '';

  const setAtPath = useCallback(
    (nextAtPath: Array<string | FileOrFolderItem>) => {
      mutate((prev) => ({ ...prev, atPath: nextAtPath }));
    },
    [data, mutate]
  );

  const setUploadFile = createSetUploadFile(mutate, data);

  const setContent = useCallback(
    (nextContent: string) => {
      mutate((prev) => ({ ...prev, content: nextContent }));
    },
    [data, mutate]
  );

  return {
    atPath,
    uploadFile,
    setAtPath,
    setUploadFile,
    content,
    setContent,
  };
};

const NomiSendBox: React.FC<{
  conversation_id: ConversationId;
  modelSelection: NomiModelSelection;
  session_mode?: string;
  agent_name?: string;
  dynamicModes: AgentModeOption[];
  turnActivity: NomiMessageRuntime;
  /**
   * Hide the permission/agent-mode selector (and the mobile action-sheet
   * model + permission entries). Used by locked surfaces like the desktop
   * companion chat, which runs in a fixed yolo mode with a locked model.
   */
  hideModeSelector?: boolean;
  /** Conversation collaborator-model control, rendered after the main model. */
  collaboratorSelectorNode?: React.ReactNode;
  /**
   * Extra node(s) rendered in the right-tools group, after the collaborator
   * selector and before the permission selector. A projected task uses this to
   * surface its task-requirement control inside the participant conversation.
   */
  extraRightTools?: React.ReactNode;
}> = ({
  conversation_id,
  modelSelection,
  session_mode,
  agent_name,
  dynamicModes,
  turnActivity,
  hideModeSelector,
  collaboratorSelectorNode,
  extraRightTools,
}) => {
  const [workspacePath, setWorkspacePath] = useState('');
  const [currentMode, setCurrentMode] = useState<string | undefined>(session_mode);
  const [isMobileSheetOpen, setIsMobileSheetOpen] = useState(false);
  const layout = useLayoutContext();
  const isMobile = Boolean(layout?.isMobile);
  const conversationContext = useConversationContextSafe();
  const loadedSkills = conversationContext?.loadedSkills ?? [];
  const loadedMcpStatuses = conversationContext?.loadedMcpStatuses ?? [];
  const { t } = useTranslation();
  const providerLabel = useModelSelectorProviderLabel();
  const { checkAndUpdateTitle } = useAutoTitle();
  const { current_model } = modelSelection;

  const {
    data: providerGraph,
    isLoading: isProviderGraphLoading,
    error: providerGraphError,
  } = useProvidersQuery();
  const canSendFiles = useCallback(
    (files: string[]) => {
      const decision = evaluateNomiVisionSend({
        files,
        providers: providerGraph ?? [],
        providerGraphResolved:
          !isProviderGraphLoading && !providerGraphError && Array.isArray(providerGraph),
        providerId: current_model?.id,
        model: current_model?.use_model,
      });
      if (decision.allowed) return true;
      Message.warning(
        decision.reason === 'capability_unavailable'
          ? t('conversation.chat.visionCapabilityUnavailable')
          : t('conversation.chat.visionModelBlocked', {
              model: current_model?.use_model ?? '',
            })
      );
      return false;
    },
    [
      current_model?.id,
      current_model?.use_model,
      isProviderGraphLoading,
      providerGraph,
      providerGraphError,
      t,
    ]
  );

  const {
    running,
    hasHydratedRunningState,
    tokenUsage,
    setActiveMsgId,
    markTurnAccepted,
    reconcilePublicDeliveryReplay,
    reconcileAfterStreamTerminal,
    setWaitingResponse,
    resetState,
    confirmStopped,
    getTurnStartGeneration,
    getTurnCompletionGeneration,
  } = turnActivity;
  const hasContextUsage =
    typeof tokenUsage?.context_window === 'number' &&
    tokenUsage.context_window > 0 &&
    typeof tokenUsage?.context_tokens === 'number';

  const { atPath, uploadFile, setAtPath, setUploadFile, content, setContent } = useSendBoxDraft(conversation_id);

  const handleContentChange = useCallback(
    (val: string) => {
      setContent(val);
    },
    [setContent]
  );

  const [agentWarmed, setAgentWarmed] = useState(false);
  const prepareRuntimeSync = useCallback(async () => {
    await warmupConversation(conversation_id);
  }, [conversation_id]);
  const prepareRuntimeForRead = useCallback(async () => {
    await warmupConversationForPassiveMount(conversation_id);
  }, [conversation_id]);

  useEffect(() => {
    void getConversationOrNull(conversation_id).then((res) => {
      if (!res?.extra?.workspace) return;
      setWorkspacePath(res.extra.workspace);
    });
  }, [conversation_id]);

  useEffect(() => {
    if (!conversation_id) return;
    setAgentWarmed(false);
    void warmupConversationForPassiveMount(conversation_id)
      .then(() => {
        setAgentWarmed(true);
      })
      .catch((error) => {
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
      });
  }, [conversation_id, t]);

  const slash_commands = useSlashCommands(conversation_id, {
    conversation_type: 'nomi',
    agentStatus: agentWarmed ? 'active' : null,
  });

  const addOrUpdateMessage = useAddOrUpdateMessage();
  const removeMessageByMsgId = useRemoveMessageByMsgId();
  const messageList = useMessageList();
  const messageListRef = useLatestRef(messageList);
  const removeMessagesByLocalIds = useRemoveMessagesByLocalIds();
  const { setSendBoxHandler } = usePreviewContext();
  const [isStopping, setIsStopping] = useState(false);
  const isBusy = running || isStopping;
  const { beginStopAttempt, getStopAttemptStatus } = useConversationStopAttemptGuard(
    conversation_id,
    getTurnStartGeneration,
    getTurnCompletionGeneration
  );

  useEffect(() => {
    setIsStopping(false);
  }, [conversation_id]);

  const setContentRef = useLatestRef(setContent);
  const contentRef = useLatestRef(content);
  const atPathRef = useLatestRef(atPath);

  // Register handler for adding text from preview panel to sendbox
  useEffect(() => {
    const handler = (text: string) => {
      const new_content = content ? `${content}\n${text}` : text;
      setContentRef.current(new_content);
    };
    setSendBoxHandler(handler);
  }, [setSendBoxHandler, content]);

  // Listen for sendbox.fill event to append text to sendbox
  useAddEventListener(
    'sendbox.fill',
    (text: string) => {
      const prev = contentRef.current;
      setContentRef.current(prev ? `${prev}${text}` : text);
    },
    []
  );

  // Shared file handling logic
  const { handleFilesAdded, clearFiles } = useSendBoxFiles({
    atPath,
    uploadFile,
    setAtPath,
    setUploadFile,
  });

  const executeCommand = useCallback(
    async (
      {
        id = uuidv7(),
        input,
        files,
        initialOnly = false,
      }: Pick<ConversationCommandQueueItem, 'input' | 'files'> &
        Partial<Pick<ConversationCommandQueueItem, 'id'>> & {
          initialOnly?: boolean;
        },
      execution?: ConversationCommandQueueExecution,
      deferLocalTurnUntilFresh = execution !== undefined
    ) => {
      if (!current_model?.use_model) {
        Message.warning(t('conversation.chat.noModelSelected'));
        throw new Error('No model selected');
      }
      if (!canSendFiles(files)) {
        throw new Error('Image send blocked by the selected chat capability');
      }

      // Persisted queue/recovery deliveries start behind an idle fence. Only
      // the atomic first-delivery winner may open a new local turn.
      if (!deferLocalTurnUntilFresh) setWaitingResponse(true);

      const displayMessage = buildDisplayMessage(input, files, workspacePath);
      let msg_id: MessageId | null = null;
      try {
        if (!deferLocalTurnUntilFresh) {
          void checkAndUpdateTitle(conversation_id, input);
        }
        // Wait for the server-assigned msg_id before rendering the optimistic
        // user bubble so the local row uses the same id as the DB row and
        // subsequent WebSocket stream events — avoids duplicate bubbles when
        // useMessageLstCache reloads.
        const res = await ipcBridge.conversation.sendMessage.invoke({
          input: displayMessage,
          conversation_id,
          files,
          idempotency_key: id,
          initial_only: initialOnly,
        });
        if (execution && !execution.isCurrent()) return;
        msg_id = res.msg_id;
        const disposition = classifyPublicMessageDelivery(res);
        if (disposition === 'fresh') {
          if (deferLocalTurnUntilFresh) {
            setWaitingResponse(true);
            void checkAndUpdateTitle(conversation_id, input);
          }
          markTurnAccepted();
          setActiveMsgId(msg_id);
        // Use add=false (compose mode) so composeMessageWithIndex can de-dup
        // by msg_id — this prevents a duplicate bubble if useMessageLstCache
        // already inserted the DB row for this same msg_id.
        addOrUpdateMessage({
          id: uuid(),
          msg_id,
          type: 'text',
          position: 'right',
          conversation_id,
          content: {
            content: displayMessage,
          },
          created_at: Date.now(),
        });
        } else {
          setActiveMsgId(null);
          reconcilePublicDeliveryReplay(res.completed);
        }
        emitter.emit('chat.history.refresh');
        if (files.length > 0) {
          emitter.emit('nomi.workspace.refresh');
        }
        return disposition;
      } catch (error) {
        if (execution && !execution.isCurrent()) return;
        if (msg_id) removeMessageByMsgId(msg_id);
        setActiveMsgId(null);
        setWaitingResponse(false);
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
        throw error;
      }
    },
    [
      addOrUpdateMessage,
      checkAndUpdateTitle,
      canSendFiles,
      conversation_id,
      current_model?.use_model,
      markTurnAccepted,
      reconcilePublicDeliveryReplay,
      setActiveMsgId,
      removeMessageByMsgId,
      setWaitingResponse,
      t,
      workspacePath,
    ]
  );

  const {
    items: queuedCommands,
    isPaused: isQueuePaused,
    isInteractionLocked: isQueueInteractionLocked,
    hasPendingCommands,
    enqueue,
    remove,
    clear,
    reorder,
    pause,
    resume,
    lockInteraction,
    unlockInteraction,
    resetActiveExecution,
  } = useConversationCommandQueue({
    conversation_id: conversation_id,
    enabled: true,
    isBusy,
    isHydrated: hasHydratedRunningState,
    onExecute: executeCommand,
  });

  // Handle initial message from Guid page — wait until model is ready
  useEffect(() => {
    if (!conversation_id || !current_model?.use_model) return;

    const target = conversationTarget(conversation_id);
    const draftStorageKey = sessionStorageKey('draft', target);
    const draftProcessedKey = sessionStorageKey('initial-message-processed-draft', target);
    if (!sessionStorage.getItem(draftProcessedKey)) {
      const storedDraft = sessionStorage.getItem(draftStorageKey);
      if (storedDraft) {
        sessionStorage.setItem(draftProcessedKey, '1');
        sessionStorage.removeItem(draftStorageKey);
        try {
          const { input } = JSON.parse(storedDraft) as { input?: unknown };
          if (typeof input === 'string') {
            setContent(input.slice(0, 6000));
          }
        } catch (error) {
          console.error('[NomiSendBox] Failed to fill draft message:', error);
          sessionStorage.removeItem(draftProcessedKey);
        }
        return;
      }
    }

    const storageKey = sessionStorageKey('initial-message-nomi', target);
    const processedKey = sessionStorageKey('initial-message-processed-nomi', target);

    const processInitialMessage = async () => {
      if (!sessionStorage.getItem(storageKey) || !claimInitialMessageDelivery(storageKey)) return;

      let attemptedIdempotencyKey: string | null = null;
      try {
        sessionStorage.removeItem(processedKey);
        const initialMessage = await readAuthorizedInitialMessageDelivery(
          sessionStorage,
          storageKey,
          conversation_id
        );
        if (!initialMessage) {
          releaseInitialMessageDelivery(storageKey);
          return;
        }
        const { input, files, idempotency_key } = initialMessage;
        attemptedIdempotencyKey = idempotency_key;
        await executeCommand(
          { id: idempotency_key, input, files, initialOnly: true },
          undefined,
          true
        );
        completeInitialMessageDelivery(sessionStorage, storageKey, idempotency_key);
      } catch (error) {
        handleInitialMessageDeliveryFailure(
          sessionStorage,
          storageKey,
          attemptedIdempotencyKey,
          error
        );
        console.error('[NomiSendBox] Failed to send initial message:', error);
        sessionStorage.removeItem(processedKey);
      }
    };

    void processInitialMessage();
  }, [conversation_id, current_model?.use_model, executeCommand, setContent]);

  const onSendHandler = async (message: string) => {
    const filesToSend = collectSelectedFiles(uploadFile, atPath);
    if (!canSendFiles(filesToSend)) return;
    clearFiles();
    emitter.emit('nomi.selected.file.clear');

    if (
      shouldEnqueueConversationCommand({
        enabled: true,
        isBusy,
        hasPendingCommands,
      })
    ) {
      enqueue({ input: message, files: filesToSend });
      return;
    }

    await executeCommand({ input: message, files: filesToSend });
  };

  // 编辑最近一条用户消息并截断重跑。请求成功前保留旧消息和附件；成功后只移除
  // 请求发出时捕获的旧本地行，避免误删 HTTP 返回前已到达的 replacement stream。
  const handleEditResubmit = useCallback(
    async (msgId: MessageId, createdAt: number, message: string) => {
      const filesToSend = collectSelectedFiles(uploadFile, atPath);
      if (!canSendFiles(filesToSend)) return;
      const oldSuffixLocalIds = snapshotEditSuffixLocalIds(
        messageListRef.current,
        msgId,
        createdAt
      );
      setWaitingResponse(true);
      const displayMessage = buildDisplayMessage(message, filesToSend, workspacePath);
      try {
        const res = await ipcBridge.conversation.editResubmit.invoke({
          conversation_id,
          msg_id: msgId,
          input: displayMessage,
          files: filesToSend,
          idempotency_key: uuidv7(),
        });
        removeMessagesByLocalIds(oldSuffixLocalIds);
        clearFiles();
        emitter.emit('nomi.selected.file.clear');
        const disposition = classifyPublicMessageDelivery(res);
        if (disposition === 'fresh') {
          markTurnAccepted();
          // 乐观插入新用户气泡（compose 模式按 msg_id 去重，避免 DB 行重复）。
          addOrUpdateMessage({
            id: uuid(),
            msg_id: res.msg_id,
            type: 'text',
            position: 'right',
            conversation_id,
            content: {
              content: displayMessage,
            },
            created_at: Date.now(),
          });
          setActiveMsgId(res.msg_id);
        } else {
          setActiveMsgId(null);
          reconcilePublicDeliveryReplay(res.completed);
        }
        emitter.emit('chat.history.refresh');
        if (filesToSend.length > 0) emitter.emit('nomi.workspace.refresh');
      } catch (error) {
        setWaitingResponse(false);
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
        throw error;
      }
    },
    [
      atPath,
      conversation_id,
      uploadFile,
      workspacePath,
      clearFiles,
      markTurnAccepted,
      canSendFiles,
      reconcilePublicDeliveryReplay,
      messageListRef,
      removeMessagesByLocalIds,
      addOrUpdateMessage,
      setActiveMsgId,
      setWaitingResponse,
      t,
    ]
  );

  // Steering injects into the turn that is ALREADY running — it does NOT start a
  // new turn, so we deliberately skip setWaitingResponse(true) (unlike
  // executeCommand). Renders the optimistic user bubble the same way so the
  // interjection shows immediately.
  const executeSteer = useCallback(
    async ({ input, files }: Pick<ConversationCommandQueueItem, 'input' | 'files'>) => {
      const displayMessage = buildDisplayMessage(input, files, workspacePath);
      let msg_id: MessageId | null = null;
      try {
        const res = await ipcBridge.conversation.steer.invoke({
          input: displayMessage,
          conversation_id,
          files,
          idempotency_key: uuidv7(),
        });
        msg_id = res.msg_id;
        const disposition = classifyPublicMessageDelivery(res);
        if (disposition === 'fresh') {
          setActiveMsgId(msg_id);
          addOrUpdateMessage({
            id: uuid(),
            msg_id,
            type: 'text',
            position: 'right',
            conversation_id,
            content: {
              content: displayMessage,
            },
            created_at: Date.now(),
          });
        } else if (disposition === 'replayed_in_flight') {
          // The steer delivery itself never starts a turn. An ambiguous
          // accepted replay may only learn whether its parent turn is still
          // running from the authoritative runtime GET.
          reconcilePublicDeliveryReplay(false);
        } else {
          // `completed` belongs to the steer receipt, not to the parent model
          // turn. Keep the parent's existing lifecycle intact while closing
          // this already-delivered interjection; only a Conversation GET (or
          // a turn event) may later settle the parent.
          setActiveMsgId(null);
          reconcileAfterStreamTerminal();
        }
        emitter.emit('chat.history.refresh');
        if (files.length > 0) {
          emitter.emit('nomi.workspace.refresh');
        }
      } catch (error) {
        if (msg_id) removeMessageByMsgId(msg_id);
        // Rethrow so the caller can divert the interjection into the persisted
        // command queue. Swallowing here (as this used to) stranded the draft:
        // the box had already been cleared, so the text was unrecoverable.
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
        throw error;
      }
    },
    [
      addOrUpdateMessage,
      conversation_id,
      reconcileAfterStreamTerminal,
      reconcilePublicDeliveryReplay,
      removeMessageByMsgId,
      setActiveMsgId,
      t,
      workspacePath,
    ]
  );

  const onSteerHandler = async (message: string) => {
    const filesToSend = collectSelectedFiles(uploadFile, atPath);
    if (!canSendFiles(filesToSend)) return;
    clearFiles();
    emitter.emit('nomi.selected.file.clear');
    try {
      await executeSteer({ input: message, files: filesToSend });
    } catch {
      // Steering has no durable channel of its own: a failed delivery is simply
      // gone. Divert into the same persisted command queue the normal send path
      // uses when busy, so an offline click keeps both the text and the
      // attachments instead of losing them to an error toast. This is the
      // fallback the catch in executeSteer has always claimed to perform, and
      // conversation.steer.fallbackQueued is the message written for it.
      enqueue({ input: message, files: filesToSend });
      Message.info(t('conversation.steer.fallbackQueued'));
    }
  };

  const handleEditQueuedCommand = useCallback(
    (item: ConversationCommandQueueItem) => {
      remove(item.id);
      setContent(item.input);
      setUploadFile(Array.from(new Set(item.files)));
      setAtPath([]);
      emitter.emit('nomi.selected.file.clear');
    },
    [remove, setAtPath, setContent, setUploadFile]
  );

  const appendSelectedFiles = useCallback(
    (files: string[]) => {
      setUploadFile((prev) => [...prev, ...files]);
    },
    [setUploadFile]
  );
  const { openFileSelector, onSlashBuiltinCommand } = useOpenFileSelector({
    onFilesSelected: appendSelectedFiles,
  });

  const { entries: attachEntries, hiddenFileInput: attachHiddenInput } = useAttachEntry({
    openFileSelector,
    onLocalFilesAdded: handleFilesAdded,
    dividerBefore: true,
  });

  // Mode switching for the mobile action sheet — mirrors AgentModeSelector's
  // setMode call so the bottom-sheet path stays in lockstep with the desktop dropdown.
  const handleSheetModeChange = useCallback(
    async (mode: string) => {
      if (mode === currentMode) return;
      try {
        await prepareRuntimeSync();
        await ipcBridge.agentConversation.setMode.invoke({ conversation_id, mode });
        setCurrentMode(mode);
        void savePreferredMode('nomi', mode);
        Message.success(t('agentMode.switchSuccess'));
      } catch (error) {
        console.error('[NomiSendBox] Failed to switch mode via sheet:', error);
        Message.error(t('agentMode.switchFailed'));
      }
    },
    [conversation_id, currentMode, prepareRuntimeSync, t]
  );

  // Sync currentMode from backend when the sheet first opens / conversation switches
  useEffect(() => {
    if (!isMobile || !isMobileSheetOpen) return;
    if (!conversation_id) return;
    let cancelled = false;
    void prepareRuntimeSync()
      .then(() => ipcBridge.agentConversation.getMode.invoke({ conversation_id }))
      .then((result) => {
        if (cancelled || !result) return;
        if (result.initialized !== false) {
          setCurrentMode(result.mode);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [conversation_id, isMobile, isMobileSheetOpen, prepareRuntimeSync]);

  const handleSheetModelSelect = useCallback(
    (value: string) => {
      // value format: `${providerId}::${modelName}`
      const [providerId, modelName] = value.split('::');
      const provider = modelSelection.providers.find((p) => p.id === providerId);
      if (!provider || !modelName) return;
      void modelSelection.handleSelectModel(provider, modelName);
    },
    [modelSelection]
  );

  const sheetEntries = useMemo<MobileActionSheetEntry[]>(() => {
    if (!isMobile) return [];

    const availableModes: AgentModeOption[] =
      dynamicModes.length > 0
        ? dynamicModes
        : [
            { value: 'default', label: 'Default' },
            { value: 'auto_edit', label: 'Auto-Accept Edits' },
            { value: 'yolo', label: 'YOLO' },
          ];
    const modeOptions: MobileActionSheetOption[] = availableModes.map((mode) => ({
      key: mode.value,
      label: t(`agentMode.${mode.value}`, { defaultValue: mode.label }),
      description: mode.description,
      active: currentMode === mode.value,
    }));

    const modelOptions: MobileActionSheetOption[] = modelSelection.providers.flatMap((provider) =>
      modelSelection.getAvailableModels(provider).map((modelName) => ({
        key: `${provider.id}::${modelName}`,
        label: modelName,
        description: providerLabel(provider),
        active:
          modelSelection.current_model?.id === provider.id && modelSelection.current_model?.use_model === modelName,
      }))
    );

    const currentModeLabel =
      modeOptions.find((opt) => opt.active)?.label ?? t('agentMode.default', { defaultValue: 'Default' });
    const currentModelLabel = modelSelection.current_model?.use_model || t('conversation.welcome.selectModel');

    const entries: MobileActionSheetEntry[] = [
      // Locked surfaces (companion) hide the model + permission entries: model is
      // pinned to the companion profile and permission is fixed to yolo.
      ...(hideModeSelector
        ? []
        : [
            {
              key: 'model',
              icon: <Brain theme='outline' size='16' />,
              label: t('common.model', { defaultValue: 'Model' }),
              meta: currentModelLabel,
              submenu: {
                title: t('common.model', { defaultValue: 'Model' }),
                options: modelOptions,
                onSelect: handleSheetModelSelect,
                emptyText: t('conversation.welcome.selectModel'),
              },
            },
            {
              key: 'permission',
              icon: <Shield theme='outline' size='16' />,
              label: t('agentMode.permission', { defaultValue: 'Permission' }),
              meta: currentModeLabel,
              submenu: {
                title: t('agentMode.permission', { defaultValue: 'Permission' }),
                options: modeOptions,
                onSelect: (key: string) => void handleSheetModeChange(key),
              },
            },
          ]),
      ...attachEntries,
    ];

    if (loadedSkills.length > 0) {
      const skillOptions: MobileActionSheetOption[] = loadedSkills.map((name) => ({
        key: name,
        label: `/${name}`,
      }));
      entries.push({
        key: 'skills',
        icon: <MagicHat theme='outline' size='16' />,
        label: t('common.skills', { defaultValue: 'Skills' }),
        variant: 'muted',
        submenu: {
          title: t('common.skills', { defaultValue: 'Skills' }),
          selectable: false,
          options: skillOptions,
          onSelect: (name) => {
            setContent(`/${name} `);
          },
        },
      });
    }

    if (loadedMcpStatuses.length > 0) {
      const mcpOptions: MobileActionSheetOption[] = loadedMcpStatuses.map((item) => ({
        key: item.name,
        label: item.name,
        description:
          item.status === 'loaded'
            ? undefined
            : item.reason
              ? `${t(`conversation.mcp.status.${item.status}` as const)} · ${item.reason}`
              : t(`conversation.mcp.status.${item.status}` as const),
      }));
      entries.push({
        key: 'mcp',
        icon: <Shield theme='outline' size='16' />,
        label: t('conversation.mcp.loaded', { defaultValue: 'Loaded MCP' }),
        variant: 'muted',
        submenu: {
          title: t('conversation.mcp.loaded', { defaultValue: 'Loaded MCP' }),
          selectable: false,
          options: mcpOptions,
          onSelect: () => undefined,
        },
      });
    }

    return entries;
  }, [
    attachEntries,
    currentMode,
    dynamicModes,
    handleSheetModeChange,
    handleSheetModelSelect,
    hideModeSelector,
    isMobile,
    loadedMcpStatuses,
    loadedSkills,
    modelSelection,
    providerLabel,
    setContent,
    t,
  ]);

  useAddEventListener('nomi.selected.file', setAtPath);
  useAddEventListener('nomi.selected.file.append', (selectedItems: Array<string | FileOrFolderItem>) => {
    const merged = mergeFileSelectionItems(atPathRef.current, selectedItems);
    if (merged !== atPathRef.current) {
      setAtPath(merged as Array<string | FileOrFolderItem>);
    }
  });

  // Stop conversation handler
  const handleStop = async (): Promise<void> => {
    if (isStopping) return;
    const stopAttempt = beginStopAttempt();
    setIsStopping(true);
    resetState();
    pause();
    resetActiveExecution('stop');

    const result = await stopConversationAndConfirmRelease(conversation_id);
    const stopAttemptStatus = getStopAttemptStatus(stopAttempt);
    if (stopAttemptStatus !== 'current') {
      if (shouldReleaseStopInteraction(stopAttemptStatus)) setIsStopping(false);
      return;
    }
    if (result.status === 'released' || result.status === 'deleted') {
      confirmStopped();
      setIsStopping(false);
      resetActiveExecution('external-reset');
      return;
    }

    // A timeout/unknown result is not idle authority. Keep the stop lock and
    // queue pause until a later GET proves the runtime is idle or deleted.
    console.warn('[NomiSendBox] stop request needs continued authoritative confirmation', result);
    Message.warning({
      content: t('conversation.stop.confirming', {
        defaultValue: 'Stop requested. Waiting for the task to finish stopping...',
      }),
      closable: true,
    });
    const settled = await waitForConversationTurnReleaseUntilSettled(conversation_id, {
      isCurrent: () => getStopAttemptStatus(stopAttempt) === 'current',
    });
    const settledAttemptStatus = getStopAttemptStatus(stopAttempt);
    if (settledAttemptStatus !== 'current') {
      if (shouldReleaseStopInteraction(settledAttemptStatus)) setIsStopping(false);
      return;
    }
    if (settled === 'released' || settled === 'deleted') {
      confirmStopped();
      setIsStopping(false);
      resetActiveExecution('external-reset');
      return;
    }

    console.warn('[NomiSendBox] stop confirmation became stale', result);
  };

  // Clear conversation context (release model context); keeps message records.
  const handleClearContext = async (): Promise<void> => {
    try {
      await ipcBridge.conversation.clearContext.invoke({ conversation_id });
      Message.success({
        content: t('conversation.clearContext.success', { defaultValue: 'Context cleared' }),
        duration: 2000,
        closable: true,
      });
    } catch (error) {
      console.warn('[NomiSendBox] clear context failed', error);
      Message.error({
        content: t('conversation.clearContext.failed', { defaultValue: 'Failed to clear context' }),
        closable: true,
      });
    }
  };

  return (
    <div className='max-w-800px w-full mx-auto flex flex-col mt-auto mb-16px'>
      <CommandQueuePanel
        items={queuedCommands}
        paused={isQueuePaused}
        interactionLocked={isQueueInteractionLocked}
        onPause={pause}
        onResume={resume}
        onInteractionLock={lockInteraction}
        onInteractionUnlock={unlockInteraction}
        onEdit={handleEditQueuedCommand}
        onReorder={reorder}
        onRemove={remove}
        onClear={clear}
      />
      <SendBox
        key={conversation_id}
        data-testid='nomi-sendbox'
        showPinnedPlan
        onMobilePlusClick={isMobile ? () => setIsMobileSheetOpen(true) : undefined}
        value={content}
        onChange={handleContentChange}
        selectedWorkspaceItems={atPath}
        onSelectedWorkspaceItemsChange={(items) => {
          emitter.emit('nomi.selected.file', items);
          setAtPath(items);
        }}
        loading={isBusy}
        disabled={!current_model?.use_model}
        placeholder={
          current_model?.use_model
            ? t('agent.sendbox.placeholder', {
                backend: agent_name || 'Nomi',
                defaultValue: `Send message to {{backend}}...`,
              })
            : t('conversation.chat.noModelSelected')
        }
        onStop={handleStop}
        onClearContext={handleClearContext}
        className='z-10'
        onFilesAdded={handleFilesAdded}
        hasPendingAttachments={uploadFile.length > 0 || atPath.length > 0}
        supportedExts={allSupportedExts}
        defaultMultiLine={!isMobile}
        lockMultiLine={!isMobile}
        tools={
          <FileAttachButton
            openFileSelector={openFileSelector}
            onLocalFilesAdded={handleFilesAdded}
            loadedMcpStatuses={loadedMcpStatuses}
          />
        }
        rightTools={
          hideModeSelector ? undefined : (
            <div
              className='sendbox-responsive-config-group flex flex-1 items-center justify-end gap-2 min-w-0'
              data-testid='nomi-sendbox-config-group'
            >
              {hasContextUsage && (
                <ContextUsageRing
                  used={tokenUsage?.context_tokens}
                  max={tokenUsage?.context_window}
                  inputTokens={tokenUsage?.input_tokens}
                  outputTokens={tokenUsage?.output_tokens}
                  reasoningTokens={tokenUsage?.reasoning_tokens}
                />
              )}
              <NomiModelSelector selection={modelSelection} className='nomi-sendbox-model-btn' />
              {/* 召唤伙伴（设计 B5）：仅普通工作会话可见 —— 伙伴/客服等锁定面
                  通过 hideModeSelector 隐藏整个配置组，天然不渲染。 */}
              <SummonControl conversationId={conversation_id} />
              {collaboratorSelectorNode}
              {extraRightTools}
              <AgentModeSelector
                backend='nomi'
                conversation_id={conversation_id}
                compact
                initialMode={session_mode}
                dynamicModes={dynamicModes}
                compactLeadingIcon={<Shield theme='outline' size='14' fill={iconColors.secondary} />}
                modeLabelFormatter={(mode) => t(`agentMode.${mode.value}`, { defaultValue: mode.label })}
                compactLabelPrefix={t('agentMode.permission')}
                hideCompactLabelPrefixOnMobile
                beforeRuntimeSync={prepareRuntimeForRead}
                beforeRuntimeMutation={prepareRuntimeSync}
              />
            </div>
          )
        }
        prefix={
          <>
            {uploadFile.length > 0 && (
              <HorizontalFileList>
                {uploadFile.map((path) => (
                  <FilePreview
                    key={path}
                    data-testid={`nomi-file-tag-${uploadFile.indexOf(path)}`}
                    path={path}
                    onRemove={() => setUploadFile(uploadFile.filter((v) => v !== path))}
                  />
                ))}
              </HorizontalFileList>
            )}
            {atPath.some((item) => (typeof item === 'string' ? false : !item.isFile)) && (
              <div className='flex flex-wrap items-center gap-8px mb-8px'>
                {atPath.map((item) => {
                  if (typeof item === 'string') return null;
                  if (!item.isFile) {
                    const folderIndex = atPath.filter((v) => typeof v !== 'string' && !v.isFile).indexOf(item);
                    return (
                      <Tag
                        key={item.path}
                        data-testid={`nomi-folder-tag-${folderIndex}`}
                        bordered={false}
                        className='!bg-primary-1 !text-primary-6'
                        closable
                        onClose={() => {
                          const newAtPath = atPath.filter((v) => (typeof v === 'string' ? true : v.path !== item.path));
                          emitter.emit('nomi.selected.file', newAtPath);
                          setAtPath(newAtPath);
                        }}
                      >
                        {item.name}
                      </Tag>
                    );
                  }
                  return null;
                })}
              </div>
            )}
          </>
        }
        onSend={onSendHandler}
        onSteer={onSteerHandler}
        steerAvailable
        onEditResubmit={handleEditResubmit}
        slash_commands={slash_commands}
        onSlashBuiltinCommand={onSlashBuiltinCommand}
        allowSendWhileLoading
      />
      {isMobile && (
        <>
          <MobileActionSheet
            open={isMobileSheetOpen}
            onClose={() => setIsMobileSheetOpen(false)}
            title={t('common.more', { defaultValue: 'More' })}
            entries={sheetEntries}
          />
          {attachHiddenInput}
        </>
      )}
    </div>
  );
};

export default NomiSendBox;
