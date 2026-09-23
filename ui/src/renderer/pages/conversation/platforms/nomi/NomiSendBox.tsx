/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { conversationTarget, type ConversationId, type MessageId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { ipcBridge } from '@/common';
import { uuid, uuidv7 } from '@/common/utils';
import CommandQueuePanel from '@/renderer/components/chat/CommandQueuePanel';
import contentStyles from '../../components/ConversationContentColumn.module.css';
import { ComposerToolRail } from '@/renderer/components/chat/SessionCapabilityPicker';
import SendBox from '@/renderer/components/chat/SendBox';
import FileAttachButton from '@/renderer/components/media/FileAttachButton';
import ComposerAttachments from '@/renderer/components/chat/ComposerAttachments';
import { useAutoTitle } from '@/renderer/hooks/chat/useAutoTitle';
import { getSendBoxDraftHook, type FileOrFolderItem } from '@/renderer/hooks/chat/useSendBoxDraft';
import { createSetUploadFile, useSendBoxFiles } from '@/renderer/hooks/chat/useSendBoxFiles';
import { useSlashCommands } from '@/renderer/hooks/chat/useSlashCommands';
import { useOpenFileSelector } from '@/renderer/hooks/file/useOpenFileSelector';
import { useLatestRef } from '@/renderer/hooks/ui/useLatestRef';
import {
  useAddOrUpdateMessage,
  useMessageList,
  useRemoveMessageByMsgId,
} from '@/renderer/pages/conversation/Messages/hooks';
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
import { warmupConversationForPassiveMount } from '@/renderer/pages/conversation/utils/warmupConversation';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { allSupportedExts } from '@/renderer/services/FileService';
import { emitter, useAddEventListener } from '@/renderer/utils/emitter';
import { mergeFileSelectionItems } from '@/renderer/utils/file/fileSelection';
import { buildDisplayMessage, collectSelectedFiles } from '@/renderer/utils/file/messageFiles';
import { Message, Tooltip } from '@arco-design/web-react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { NomiMessageRuntime } from './useNomiMessage';
import NomiModelSelector from './NomiModelSelector';
import { ContextUsageRing } from './ContextUsageRing';
import type { NomiModelSelection } from './useNomiModelSelection';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { evaluateNomiVisionSend } from './nomiVisionSendGuard';
import { steerOrQueue } from './steerOrQueue';
import CreationControls, { CreationModelSelector } from '@/renderer/creation/CreationControls';
import { ComposerSceneHeader } from '@/renderer/creation/ComposerSceneSelector';
import { useCreationComposer } from '@/renderer/creation/CreationComposerContext';
import { useGenerationModel } from '@/renderer/creation/useGenerationModel';
import { buildCreationRequest, creationAttempt, acknowledgeCreationAttempt } from '@/renderer/creation/submission';
import { creationTasksKey, submitCreation } from '@/renderer/creation/client';
import { mutate as mutateSWR } from 'swr';
import { useConfig } from '@/renderer/hooks/config/useConfig';

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
  agentSelectorNode?: React.ReactNode;
  agent_name?: string;
  turnActivity: NomiMessageRuntime;
  /** Product-owned controls may occupy the rail; Agent/resource authority stays frozen. */
  capabilityControls?: React.ReactNode;
  modelSelectionHint?: string;
  modelSelectionDisabled?: boolean;
  /** Existing collaboration control, rendered in the composer side rail. */
  collaboratorSelectorNode?: React.ReactNode;
  /**
   * Extra node(s) rendered in the bottom right-tools group. A projected task
   * uses this to surface its task-requirement control inside the participant conversation.
   */
  extraRightTools?: React.ReactNode;
  /** False for product surfaces that currently support chat only. */
  creationEnabled?: boolean;
  /** Hide work-session configuration chrome on a dedicated product surface. */
  compactProductComposer?: boolean;
}> = ({
  conversation_id,
  modelSelection,
  agentSelectorNode,
  agent_name,
  turnActivity,
  capabilityControls,
  modelSelectionHint,
  modelSelectionDisabled,
  collaboratorSelectorNode,
  extraRightTools,
  creationEnabled = true,
  compactProductComposer = false,
}) => {
  const [workspacePath, setWorkspacePath] = useState('');
  const { t } = useTranslation();
  const { checkAndUpdateTitle } = useAutoTitle();
  const { current_model } = modelSelection;
  const [defaultVisionModel] = useConfig('models.default.vision');

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
        visionModel: defaultVisionModel,
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
      defaultVisionModel,
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
  const modelPickerDisabled = Boolean(modelSelectionDisabled || running);
  const modelPickerHint = running
    ? t('conversation.chat.modelSwitchAfterTurn')
    : modelSelectionHint;
  const hasContextUsage =
    typeof tokenUsage?.context_window === 'number' &&
    tokenUsage.context_window > 0 &&
    typeof tokenUsage?.context_tokens === 'number';

  const { atPath, uploadFile, setAtPath, setUploadFile, content, setContent } = useSendBoxDraft(conversation_id);
  const creationContext = useCreationComposer();
  const creation = creationEnabled ? creationContext : null;
  const generation = useGenerationModel(creation, collectSelectedFiles(uploadFile, atPath));
  const [creationSubmitting, setCreationSubmitting] = useState(false);
  const creationSubmittingRef = useRef(false);
  const isCreating = Boolean(creation?.draft.mode);
  useEffect(() => {
    if (creation?.draft.pendingPrompt === undefined) return;
    setContent(creation.draft.pendingPrompt);
    creation.update(draft => ({ ...draft, pendingPrompt: undefined }));
  }, [creation?.draft.pendingPrompt, creation?.update, setContent]);

  useEffect(() => {
    if (!creation?.draft.pendingFiles) return;
    setUploadFile(previous => Array.from(new Set([...previous, ...creation.draft.pendingFiles!])));
    creation.update(draft => ({ ...draft, pendingFiles: undefined }));
  }, [creation?.draft.pendingFiles, creation?.update, setUploadFile]);

  const handleContentChange = useCallback(
    (val: string) => {
      setContent(val);
    },
    [setContent]
  );

  const [agentWarmed, setAgentWarmed] = useState(false);
  const [initialDeliveryReady, setInitialDeliveryReady] = useState(false);
  useEffect(() => {
    void getConversationOrNull(conversation_id).then((res) => {
      if (!res?.extra?.workspace) return;
      setWorkspacePath(res.extra.workspace);
    });
  }, [conversation_id]);

  useEffect(() => {
    if (!conversation_id || isCreating) return;
    let cancelled = false;
    setAgentWarmed(false);
    setInitialDeliveryReady(false);
    void warmupConversationForPassiveMount(conversation_id)
      .then((warmed) => {
        // Finished sessions hydrate without creating a runtime. Do not query
        // runtime-only slash commands merely because hydration completed.
        if (!cancelled) {
          setAgentWarmed(warmed);
          // `false` means an already-Ready canonical Session needed no passive
          // warmup, not that its guarded initial handoff must stay blocked.
          setInitialDeliveryReady(true);
        }
      })
      .catch((error) => {
        if (!cancelled) Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
      });
    return () => { cancelled = true; };
  }, [conversation_id, isCreating, t]);

  const slash_commands = useSlashCommands(conversation_id, {
    conversation_type: 'nomi',
    agentStatus: agentWarmed ? 'active' : null,
  });

  const addOrUpdateMessage = useAddOrUpdateMessage();
  const removeMessageByMsgId = useRemoveMessageByMsgId();
  const messageList = useMessageList();
  const messageListRef = useLatestRef(messageList);
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

  // Handle the Guid handoff only after passive warmup has settled.
  // This sequences the UI requests; runtime admission remains backend-owned.
  useEffect(() => {
    if (!conversation_id || !current_model?.use_model || !initialDeliveryReady) return;

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
  }, [conversation_id, current_model?.use_model, executeCommand, initialDeliveryReady, setContent]);

  const onSendHandler = async (message: string) => {
    const filesToSend = collectSelectedFiles(uploadFile, atPath);
    if (creation?.draft.mode) {
      if (creationSubmittingRef.current) throw new Error('任务正在提交');
      if (!generation.ready || creation.preparing) throw new Error('请先选择可用的生成模型');
      creationSubmittingRef.current = true;
      setCreationSubmitting(true);
      const submittedReferences = creation.draft.references;
      try {
        const presetId = await creation.resolvePreset?.() ?? creation.presetId;
        if (!presetId) throw new Error('请选择可用的创意 Agent');
        const request = buildCreationRequest(creation.draft, message, presetId, filesToSend, generation.selected);
        const key = creationAttempt(conversation_id, request);
        const receipt = await submitCreation(conversation_id, request, key);
        addOrUpdateMessage({ id: uuid(), msg_id: receipt.message_id, type: 'text', position: 'right', conversation_id, content: { content: message }, created_at: Date.now() });
        acknowledgeCreationAttempt(conversation_id, key);
        // A failed status refresh cannot turn a successful admission into a retry.
        void mutateSWR(creationTasksKey(conversation_id), (previous: typeof receipt.tasks | undefined) => [...(previous || []).filter(task => !receipt.tasks.some(next => next.creation_task_id === task.creation_task_id)), ...receipt.tasks], { revalidate: true }).catch(() => {});
        if (request.files?.length && contentRef.current === message) {
          setUploadFile(previous => previous.filter(file => !request.files!.includes(file)));
          setAtPath(atPathRef.current.filter(item => !request.files!.includes(typeof item === 'string' ? item : item.path)));
        }
        creation.update(draft => request.inputs.length && draft.references === submittedReferences ? { ...draft, references: draft.references.filter(ref => !request.inputs.some(input => input.asset_id === ref.asset_id)) } : draft);
        emitter.emit('chat.history.refresh');
      } catch (error) {
        Message.error(error instanceof Error ? error.message : String(error));
        throw error;
      } finally { creationSubmittingRef.current = false; setCreationSubmitting(false); }
      return;
    }
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

    await executeCommand({
      input: message,
      files: filesToSend,
    });
  };

  // Canonical history is immutable. Editing a previous prompt explicitly
  // submits the revised text as a new Turn; it never truncates or rewrites the
  // accepted Session event chain.
  const handleEditResubmit = useCallback(
    async (_msgId: MessageId, _createdAt: number, message: string) => {
      const filesToSend = collectSelectedFiles(uploadFile, atPath);
      if (!canSendFiles(filesToSend)) return;
      setWaitingResponse(true);
      const displayMessage = buildDisplayMessage(message, filesToSend, workspacePath);
      try {
        const res = await ipcBridge.conversation.sendMessage.invoke({
          conversation_id,
          input: displayMessage,
          files: filesToSend,
          idempotency_key: uuidv7(),
        });
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
        // Retain a held draft for explicit review. This error may follow
        // successful delivery, so it must never automatically start a turn.
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
    if (
      !(await steerOrQueue(
        {
          input: message,
          files: filesToSend,
        },
        executeSteer,
        enqueue
      ))
    ) {
        Message.warning(t('conversation.steer.fallbackQueued'));
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
    <div className={`${contentStyles.column} ${contentStyles.composer} flex flex-col mt-auto ${compactProductComposer ? 'mb-12px' : 'mb-16px'}`}>
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
        sideTools={compactProductComposer
          ? undefined
          : capabilityControls !== undefined
            ? capabilityControls
            : collaboratorSelectorNode
              ? <ComposerToolRail ariaLabel={t('guid.collaboration.models.label')}>
                  {collaboratorSelectorNode}
                </ComposerToolRail>
              : undefined}
        prefix={compactProductComposer ? undefined : <ComposerSceneHeader agent={agentSelectorNode} sceneSelectionEnabled={creationEnabled} />}
        data-testid='nomi-sendbox'
        showPinnedPlan={!compactProductComposer}
        value={content}
        onChange={handleContentChange}
        selectedWorkspaceItems={atPath}
        onSelectedWorkspaceItemsChange={(items) => {
          emitter.emit('nomi.selected.file', items);
          setAtPath(items);
        }}
        loading={isCreating ? creationSubmitting : isBusy}
        disabled={isCreating ? !generation.ready || creation?.preparing : !current_model?.use_model || modelSelectionDisabled || creation?.preparing}
        preserveDraftUntilAccepted={Boolean(creation)}
        skipChatWarmup={isCreating}
        placeholder={
          compactProductComposer
            ? t('nomi.cohabit.composerPlaceholder', {
                name: agent_name || 'Nomi',
                defaultValue: '和{{name}}说点什么…',
              })
            : isCreating ? '描述你想创作的内容，可添加参考素材…' : current_model?.use_model
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
        defaultMultiLine={!compactProductComposer}
        lockMultiLine={!compactProductComposer}
        compactActions={compactProductComposer}
        bottomHint={compactProductComposer ? ' ' : undefined}
        tools={
          <FileAttachButton
            openFileSelector={openFileSelector}
            onLocalFilesAdded={handleFilesAdded}
            showLoadedCapabilities={false}
          />
        }
        creationTools={creation ? <CreationControls prompt={content} onPromptChange={setContent} files={collectSelectedFiles(uploadFile, atPath)} /> : undefined}
        rightTools={
          (
            <div
              className='sendbox-responsive-config-group flex flex-1 items-center justify-end gap-2 min-w-0'
              data-composer-group
              data-testid='nomi-sendbox-config-group'
            >
              {!compactProductComposer && hasContextUsage && (
                <ContextUsageRing
                  used={tokenUsage?.context_tokens}
                  max={tokenUsage?.context_window}
                  inputTokens={tokenUsage?.input_tokens}
                  outputTokens={tokenUsage?.output_tokens}
                  reasoningTokens={tokenUsage?.reasoning_tokens}
                />
              )}
              {!compactProductComposer && isCreating && <CreationModelSelector files={collectSelectedFiles(uploadFile, atPath)} />}
              {!compactProductComposer && !isCreating && (
                <Tooltip content={modelPickerHint} disabled={!modelPickerHint}>
                  <span className='inline-flex min-w-0'>
                    <NomiModelSelector
                      selection={modelSelection}
                      disabled={modelPickerDisabled}
                      className='nomi-sendbox-model-btn'
                    />
                  </span>
                </Tooltip>
              )}
              {!compactProductComposer && extraRightTools}
            </div>
          )
        }
        renderAttachments={(workspaceItems, onRemoveWorkspaceItem) => <ComposerAttachments
          files={uploadFile}
          onRemoveFile={(path) => setUploadFile(previous => previous.filter(file => file !== path))}
          workspaceItems={workspaceItems}
          onRemoveWorkspaceItem={onRemoveWorkspaceItem}
        />}
        onSend={onSendHandler}
        onSteer={onSteerHandler}
        steerAvailable={!isCreating}
        onEditResubmit={isCreating ? undefined : handleEditResubmit}
        slash_commands={slash_commands}
        onSlashBuiltinCommand={onSlashBuiltinCommand}
        allowSendWhileLoading={!isCreating}
      />
    </div>
  );
};

export default NomiSendBox;
