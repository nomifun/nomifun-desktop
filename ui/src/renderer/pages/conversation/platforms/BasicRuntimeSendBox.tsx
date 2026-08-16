/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { conversationTarget, type ConversationId, type MessageId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { ipcBridge } from '@/common';
import { uuid, uuidv7 } from '@/common/utils';
import type { TMessage } from '@/common/chat/chatLib';
import type { SlashCommandItem } from '@/common/chat/slash/types';
import CommandQueuePanel from '@/renderer/components/chat/CommandQueuePanel';
import SendBox from '@/renderer/components/chat/SendBox';
import FileAttachButton from '@/renderer/components/media/FileAttachButton';
import FilePreview from '@/renderer/components/media/FilePreview';
import HorizontalFileList from '@/renderer/components/media/HorizontalFileList';
import { useAutoTitle } from '@/renderer/hooks/chat/useAutoTitle';
import type { FileOrFolderItem } from '@/renderer/hooks/chat/useSendBoxDraft';
import { createSetUploadFile } from '@/renderer/hooks/chat/useSendBoxFiles';
import { useOpenFileSelector } from '@/renderer/hooks/file/useOpenFileSelector';
import { useLatestRef } from '@/renderer/hooks/ui/useLatestRef';
import { useAddOrUpdateMessage, useRemoveMessageByMsgId } from '@/renderer/pages/conversation/Messages/hooks';
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
import { useAuthoritativeTurnLifecycle } from '@/renderer/pages/conversation/platforms/useAuthoritativeTurnLifecycle';
import {
  shouldReleaseStopInteraction,
  useConversationStopAttemptGuard,
} from '@/renderer/pages/conversation/platforms/useConversationStopAttemptGuard';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { getConversationRuntimeWorkspaceErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { allSupportedExts, type FileMetadata } from '@/renderer/services/FileService';
import { emitter, useAddEventListener } from '@/renderer/utils/emitter';
import { mergeFileSelectionItems } from '@/renderer/utils/file/fileSelection';
import { buildDisplayMessage } from '@/renderer/utils/file/messageFiles';
import { Message, Tag } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * Draft shape shared by all basic-runtime platforms. The concrete draft stores
 * (created with getSendBoxDraftHook) additionally carry a platform `_type`
 * discriminant that this component neither reads nor rewrites — mutations
 * always spread the previous draft.
 */
export type BasicRuntimeDraftData = {
  atPath: Array<string | FileOrFolderItem>;
  content: string;
  uploadFile: string[];
};

export type BasicRuntimeDraftHook = (conversation_id: ConversationId) => {
  data: BasicRuntimeDraftData | undefined;
  mutate: (mutator: (prev: BasicRuntimeDraftData) => BasicRuntimeDraftData) => void;
};

/**
 * Capabilities handed to a platform extension hook so platform-only turn flows
 * (e.g. the OpenClaw Star Office install flow) can open/close local turns and
 * render optimistic bubbles exactly like the shared send paths do.
 */
export interface BasicRuntimeSendBoxController {
  conversation_id: ConversationId;
  /** Sets the processing indicator; also syncs the internal aiProcessing ref. */
  setAiProcessing: (value: boolean) => void;
  beginLocalTurn: () => void;
  markLocalTurnAccepted: () => void;
  reconcilePublicDeliveryReplay: (completed: boolean) => void;
  cancelLocalTurn: () => void;
  checkAndUpdateTitle: ReturnType<typeof useAutoTitle>['checkAndUpdateTitle'];
  addOrUpdateMessage: ReturnType<typeof useAddOrUpdateMessage>;
}

/** Stream interceptors a platform extension can return. */
export interface BasicRuntimeStreamHooks {
  /** Invoked when the response stream reports 'finish', before terminal reconciliation. */
  onStreamFinish?: () => void;
}

/**
 * Per-platform parameterization of the shared basic-runtime send box.
 *
 * IMPORTANT: pass a module-level constant. The config supplies hooks
 * (useDraft / useSlashCommandList / usePlatformExtension), so its identity must
 * be stable for the lifetime of a mounted component to respect the rules of
 * hooks.
 */
export interface BasicRuntimeSendBoxConfig {
  /** Tag used in console warnings, e.g. '[RemoteSendBox]'. */
  logTag: string;
  /** Workspace-rail selection events mirrored by this send box. */
  selectedFileEvents: {
    set: 'remote.selected.file' | 'openclaw-gateway.selected.file';
    append: 'remote.selected.file.append' | 'openclaw-gateway.selected.file.append';
    clear: 'remote.selected.file.clear' | 'openclaw-gateway.selected.file.clear';
  };
  /**
   * sessionStorage feature names for the guid-page initial-message delivery.
   * Note the historical suffixes: openclaw uses 'initial-message-openclaw',
   * not the 'openclaw-gateway' conversation type.
   */
  initialMessageFeature: 'initial-message-remote' | 'initial-message-openclaw';
  initialMessageProcessedFeature: 'initial-message-processed-remote' | 'initial-message-processed-openclaw';
  /** Message send channel (openclaw routes through ipcBridge.openclawConversation). */
  sendMessage: typeof ipcBridge.conversation.sendMessage;
  /** Turn stream channel (openclaw routes through ipcBridge.openclawConversation). */
  responseStream: typeof ipcBridge.conversation.responseStream;
  /** Draft store hook created once at module scope via getSendBoxDraftHook. */
  useDraft: BasicRuntimeDraftHook;
  /** Slash command list hook; omit to disable slash commands (remote). */
  useSlashCommandList?: (conversation_id: ConversationId) => SlashCommandItem[];
  /**
   * Platform extension hook mounted inside the send box (e.g. the OpenClaw
   * Star Office install flow). May return stream interceptors.
   */
  usePlatformExtension?: (controller: BasicRuntimeSendBoxController) => BasicRuntimeStreamHooks | undefined;
  /**
   * Delay before processing a pending guid-page initial message, giving the
   * component time to mount and the stream listener to attach. Omit to send
   * immediately when a runtime needs no warmup.
   */
  initialMessageDelayMs?: number;
  /** Gate initial-message processing on runtime hydration (openclaw). */
  initialMessageAfterHydration?: boolean;
  /**
   * How the workspace path used by buildDisplayMessage is resolved:
   * - 'on-mount': read from the conversation as soon as the box mounts
   *   (remote / openclaw).
   * - 'at-initial-message': resolved only while delivering a guid-page initial
   *   message (direct sends before any initial delivery format against an
   *   empty workspace path).
   */
  workspaceResolution: 'on-mount' | 'at-initial-message';
  /**
   * Offer the clear-context action. A runtime omits it when it has no
   * resumable session history and the backend reports clear-context as
   * unsupported.
   */
  enableClearContext?: boolean;
  /** Backend display name for the placeholder (and fallback while resolving). */
  backendName: string;
  /** Optional async backend name resolver (remote agent name). */
  resolveBackendName?: (conversation_id: ConversationId) => Promise<string | undefined>;
  /** Re-emit the selection event when items change inside the send box (openclaw). */
  emitSelectedFileOnChange?: boolean;
  /** Render closable tags for selected workspace folders (openclaw). */
  showFolderTags?: boolean;
  /** Report pending attachments to the SendBox (openclaw). */
  reportPendingAttachments?: boolean;
  /** SendBox multiline behavior (remote / openclaw force multiline). */
  defaultMultiLine?: boolean;
  lockMultiLine?: boolean;
}

const useNoSlashCommands = (_conversation_id: ConversationId): SlashCommandItem[] | undefined => undefined;
const useNoPlatformExtension = (_controller: BasicRuntimeSendBoxController): BasicRuntimeStreamHooks | undefined =>
  undefined;

const EMPTY_AT_PATH: Array<string | FileOrFolderItem> = [];
const EMPTY_UPLOAD_FILES: string[] = [];

/**
 * Shared send box for the "basic runtime" platforms (remote /
 * openclaw-gateway): identical turn-lifecycle hydration, response-stream
 * subscription, draft persistence, command queue and stop wiring, with the
 * platform differences captured by {@link BasicRuntimeSendBoxConfig}.
 *
 * The stateful ACP / Nomi send boxes have materially different flows and are
 * intentionally not built on this component.
 */
const BasicRuntimeSendBox: React.FC<{
  conversation_id: ConversationId;
  config: BasicRuntimeSendBoxConfig;
}> = ({ conversation_id, config }) => {
  const [workspacePath, setWorkspacePath] = useState('');
  const { t } = useTranslation();
  const { checkAndUpdateTitle } = useAutoTitle();
  const useSlashCommandList = config.useSlashCommandList ?? useNoSlashCommands;
  const slash_commands = useSlashCommandList(conversation_id);
  const addOrUpdateMessage = useAddOrUpdateMessage();
  const removeMessageByMsgId = useRemoveMessageByMsgId();
  const { setSendBoxHandler } = usePreviewContext();

  const [backendName, setBackendName] = useState(config.backendName);
  const [aiProcessing, setAiProcessingState] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [hasHydratedRunningState, setHasHydratedRunningState] = useState(false);
  const isBusy = aiProcessing || isStopping;

  // Ref mirror of aiProcessing for immediate access in stream handlers.
  // 使用 ref 同步状态，以便在事件处理程序中立即访问
  const aiProcessingRef = useRef(false);
  const setAiProcessing = useCallback((value: boolean) => {
    aiProcessingRef.current = value;
    setAiProcessingState(value);
  }, []);

  const {
    beginLocalTurn,
    markLocalTurnAccepted,
    reconcilePublicDeliveryReplay,
    cancelLocalTurn,
    stopOptimistically,
    confirmStopped,
    resyncAuthoritativeRuntime,
    acceptsStreamActivity,
    reconcileAfterStreamTerminal,
    getTurnStartGeneration,
    getTurnCompletionGeneration,
  } = useAuthoritativeTurnLifecycle(conversation_id, {
    onTurnStarted: () => setAiProcessing(true),
    onTurnCompleted: () => setAiProcessing(false),
    onAuthoritativeRuntime: (isRunning) => {
      setAiProcessing(isRunning);
      setHasHydratedRunningState(true);
    },
  });
  const { beginStopAttempt, getStopAttemptStatus } = useConversationStopAttemptGuard(
    conversation_id,
    getTurnStartGeneration,
    getTurnCompletionGeneration
  );

  const useDraft = config.useDraft;
  const { data: draftData, mutate: mutateDraft } = useDraft(conversation_id);
  const atPath = draftData?.atPath ?? EMPTY_AT_PATH;
  const uploadFile = draftData?.uploadFile ?? EMPTY_UPLOAD_FILES;
  const content = draftData?.content ?? '';

  const setAtPath = useCallback(
    (val: Array<string | FileOrFolderItem>) => {
      mutateDraft((prev) => ({ ...prev, atPath: val }));
    },
    [mutateDraft]
  );

  const setUploadFile = createSetUploadFile(mutateDraft, draftData);

  const setContent = useCallback(
    (val: string) => {
      mutateDraft((prev) => ({ ...prev, content: val }));
    },
    [mutateDraft]
  );

  const handleContentChange = useCallback(
    (val: string) => {
      setContent(val);
    },
    [setContent]
  );

  const setContentRef = useLatestRef(setContent);
  const contentRef = useLatestRef(content);
  const atPathRef = useLatestRef(atPath);

  // Platform extension (e.g. OpenClaw Star Office flow) mounts here and may
  // intercept stream events. The config is module-constant, so the hook slot
  // is stable across renders.
  const controller = useMemo<BasicRuntimeSendBoxController>(
    () => ({
      conversation_id,
      setAiProcessing,
      beginLocalTurn,
      markLocalTurnAccepted,
      reconcilePublicDeliveryReplay,
      cancelLocalTurn,
      checkAndUpdateTitle,
      addOrUpdateMessage,
    }),
    [
      addOrUpdateMessage,
      beginLocalTurn,
      cancelLocalTurn,
      checkAndUpdateTitle,
      conversation_id,
      markLocalTurnAccepted,
      reconcilePublicDeliveryReplay,
      setAiProcessing,
    ]
  );
  const usePlatformExtension = config.usePlatformExtension ?? useNoPlatformExtension;
  const streamHooks = usePlatformExtension(controller);
  const streamHooksRef = useLatestRef(streamHooks);

  // Reset state when the conversation changes and restore the actual running
  // status from the backend before trusting local state, to avoid flicker when
  // switching to a running conversation.
  // 先获取后端状态再重置 aiProcessing，避免切换到运行中的会话时闪烁
  useEffect(() => {
    setAiProcessing(false);
    setIsStopping(false);
    setHasHydratedRunningState(false);
    // This starts with an immediate read, retries BackendRequestError/unknown
    // snapshots with capped backoff, adopts active_turn_id when processing, and
    // keeps polling until the runtime is authoritatively idle.
    resyncAuthoritativeRuntime({ immediate: true });
  }, [conversation_id, resyncAuthoritativeRuntime, setAiProcessing]);

  useEffect(() => {
    const handler = (text: string) => {
      const new_content = content ? `${content}\n${text}` : text;
      setContentRef.current(new_content);
    };
    setSendBoxHandler(handler);
  }, [setSendBoxHandler, content, setContentRef]);

  useAddEventListener(
    'sendbox.fill',
    (text: string) => {
      const prev = contentRef.current;
      setContentRef.current(prev ? `${prev}${text}` : text);
    },
    []
  );

  useEffect(() => {
    return config.responseStream.on((message) => {
      if (conversation_id !== message.conversation_id) {
        return;
      }
      switch (message.type) {
        case 'thought':
          if (acceptsStreamActivity(message.turn_id) && !aiProcessingRef.current) {
            setAiProcessing(true);
          }
          break;
        case 'finish':
          // Stream completion can precede release of the backend turn handle.
          streamHooksRef.current?.onStreamFinish?.();
          reconcileAfterStreamTerminal();
          break;
        case 'error':
          reconcileAfterStreamTerminal();
          break;
        case 'content':
        case 'acp_permission': {
          if (!acceptsStreamActivity(message.turn_id)) break;
          // Auto-recover the processing state if content arrives after finish.
          if (!aiProcessingRef.current) {
            setAiProcessing(true);
          }
          break;
        }
        default:
          break;
      }
    });
  }, [
    acceptsStreamActivity,
    config.responseStream,
    conversation_id,
    reconcileAfterStreamTerminal,
    setAiProcessing,
    streamHooksRef,
  ]);

  useEffect(() => {
    if (config.workspaceResolution !== 'on-mount') return;
    void getConversationOrNull(conversation_id).then((res) => {
      if (!res?.extra?.workspace) return;
      setWorkspacePath(res.extra.workspace);
    });
  }, [config.workspaceResolution, conversation_id]);

  const resolveBackendName = config.resolveBackendName;
  useEffect(() => {
    if (!resolveBackendName) return;
    let cancelled = false;
    void resolveBackendName(conversation_id).then((name) => {
      if (!cancelled && name) setBackendName(name);
    });
    return () => {
      cancelled = true;
    };
  }, [conversation_id, resolveBackendName]);

  const handleFilesAdded = useCallback(
    (pastedFiles: FileMetadata[]) => {
      const file_paths = pastedFiles.map((file) => file.path);
      setUploadFile((prev) => [...prev, ...file_paths]);
    },
    [setUploadFile]
  );

  useAddEventListener(config.selectedFileEvents.set, (items: Array<string | FileOrFolderItem>) => {
    setTimeout(() => {
      setAtPath(items);
    }, 10);
  });

  useAddEventListener(config.selectedFileEvents.append, (items: Array<string | FileOrFolderItem>) => {
    setTimeout(() => {
      const merged = mergeFileSelectionItems(atPathRef.current, items);
      if (merged !== atPathRef.current) {
        setAtPath(merged as Array<string | FileOrFolderItem>);
      }
    }, 10);
  });

  const executeCommand = useCallback(
    async (
      {
        id = uuidv7(),
        input,
        files,
      }: Pick<ConversationCommandQueueItem, 'input' | 'files'> &
        Partial<Pick<ConversationCommandQueueItem, 'id'>>,
      execution?: ConversationCommandQueueExecution,
      deferLocalTurnUntilFresh = execution !== undefined
    ) => {
      const displayMessage = buildDisplayMessage(input, files, workspacePath);

      if (!deferLocalTurnUntilFresh) {
        beginLocalTurn();
        setAiProcessing(true);
      }
      let msg_id: MessageId | null = null;
      try {
        if (!deferLocalTurnUntilFresh) {
          void checkAndUpdateTitle(conversation_id, input);
        }
        // Wait for the server-assigned msg_id before rendering the optimistic
        // user bubble so the local row uses the same id as the DB row and
        // subsequent WebSocket stream events — avoids duplicate bubbles when
        // useMessageLstCache reloads.
        const res = await config.sendMessage.invoke({
          input: displayMessage,
          conversation_id,
          files,
          idempotency_key: id,
        });
        if (execution && !execution.isCurrent()) return;
        msg_id = res.msg_id;
        const disposition = classifyPublicMessageDelivery(res);
        if (disposition === 'fresh') {
          if (deferLocalTurnUntilFresh) {
            beginLocalTurn();
            setAiProcessing(true);
            void checkAndUpdateTitle(conversation_id, input);
          }
          markLocalTurnAccepted();
          const userMessage: TMessage = {
            id: uuid(),
            msg_id,
            conversation_id,
            type: 'text',
            position: 'right',
            content: { content: displayMessage },
            created_at: Date.now(),
          };
          // Use add=false (compose mode) so composeMessageWithIndex can de-dup
          // by msg_id against the DB row that useMessageLstCache may insert.
          addOrUpdateMessage(userMessage);
        } else {
          reconcilePublicDeliveryReplay(res.completed);
        }
        emitter.emit('chat.history.refresh');
        return disposition;
      } catch (error) {
        if (execution && !execution.isCurrent()) return;
        if (msg_id) removeMessageByMsgId(msg_id);
        cancelLocalTurn();
        setAiProcessing(false);
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
        throw error;
      }
    },
    [
      addOrUpdateMessage,
      beginLocalTurn,
      cancelLocalTurn,
      checkAndUpdateTitle,
      config.sendMessage,
      conversation_id,
      markLocalTurnAccepted,
      reconcilePublicDeliveryReplay,
      removeMessageByMsgId,
      setAiProcessing,
      t,
      workspacePath,
    ]
  );

  const {
    items,
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

  const onSendHandler = async (message: string) => {
    emitter.emit(config.selectedFileEvents.clear);
    const file_paths = [...uploadFile, ...atPath.map((item) => (typeof item === 'string' ? item : item.path))];
    setAtPath([]);
    setUploadFile([]);

    if (
      shouldEnqueueConversationCommand({
        enabled: true,
        isBusy,
        hasPendingCommands,
      })
    ) {
      enqueue({ input: message, files: file_paths });
      return;
    }

    await executeCommand({ input: message, files: file_paths });
  };

  const handleEditQueuedCommand = useCallback(
    (item: ConversationCommandQueueItem) => {
      remove(item.id);
      setContent(item.input);
      setUploadFile(Array.from(new Set(item.files)));
      setAtPath([]);
      emitter.emit(config.selectedFileEvents.clear);
    },
    [config.selectedFileEvents.clear, remove, setAtPath, setContent, setUploadFile]
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

  // Handle initial message from the guid page.
  useEffect(() => {
    if (!conversation_id) return;
    if (config.initialMessageAfterHydration && !hasHydratedRunningState) return;

    const target = conversationTarget(conversation_id);
    const storageKey = sessionStorageKey(config.initialMessageFeature, target);
    const processedKey = sessionStorageKey(config.initialMessageProcessedFeature, target);

    const processInitialMessage = async () => {
      if (!sessionStorage.getItem(storageKey) || !claimInitialMessageDelivery(storageKey)) return;

      let attemptedIdempotencyKey: string | null = null;
      try {
        // Remove the legacy consume-before-POST marker. The payload itself
        // remains durable until an accepted response.
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
        let resolvedWorkspace = workspacePath;
        if (config.workspaceResolution === 'at-initial-message') {
          const res = await getConversationOrNull(conversation_id);
          resolvedWorkspace = res?.extra?.workspace ?? '';
          setWorkspacePath(resolvedWorkspace);
        }
        const initialDisplayMessage = buildDisplayMessage(input, files, resolvedWorkspace);

        // Fetch the server-assigned msg_id before rendering the optimistic
        // bubble so the local row uses the same id as the persisted DB row.
        const sendResult = await config.sendMessage.invoke({
          input: initialDisplayMessage,
          conversation_id,
          files,
          idempotency_key,
          initial_only: true,
        });
        completeInitialMessageDelivery(sessionStorage, storageKey, idempotency_key);
        const { msg_id } = sendResult;
        const disposition = classifyPublicMessageDelivery(sendResult);
        if (disposition === 'fresh') {
          beginLocalTurn();
          setAiProcessing(true);
          void checkAndUpdateTitle(conversation_id, input);
          markLocalTurnAccepted();

          const userMessage: TMessage = {
            id: uuid(),
            msg_id,
            conversation_id,
            type: 'text',
            position: 'right',
            content: { content: initialDisplayMessage },
            created_at: Date.now(),
          };
          // Use add=false (compose mode) so composeMessageWithIndex can de-dup
          // by msg_id against the DB row that useMessageLstCache may insert.
          addOrUpdateMessage(userMessage);
        } else {
          reconcilePublicDeliveryReplay(sendResult.completed);
        }

        emitter.emit('chat.history.refresh');
      } catch (error) {
        handleInitialMessageDeliveryFailure(
          sessionStorage,
          storageKey,
          attemptedIdempotencyKey,
          error
        );
        sessionStorage.removeItem(processedKey);
        cancelLocalTurn();
        setAiProcessing(false);
        Message.error(getConversationRuntimeWorkspaceErrorMessage(error, t));
      }
    };

    if (config.initialMessageDelayMs === undefined) {
      processInitialMessage().catch(console.error);
      return;
    }

    // Small delay to let the component mount and the stream listener attach.
    const timer = setTimeout(() => {
      processInitialMessage().catch(console.error);
    }, config.initialMessageDelayMs);
    return () => {
      clearTimeout(timer);
    };
  }, [
    addOrUpdateMessage,
    beginLocalTurn,
    cancelLocalTurn,
    checkAndUpdateTitle,
    config,
    conversation_id,
    hasHydratedRunningState,
    markLocalTurnAccepted,
    reconcilePublicDeliveryReplay,
    setAiProcessing,
    t,
    workspacePath,
  ]);

  const handleStop = async (): Promise<void> => {
    if (isStopping) return;
    const stopAttempt = beginStopAttempt();
    setIsStopping(true);
    stopOptimistically();
    setAiProcessing(false);
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

    // A timeout, transport error, or still-processing snapshot is not proof
    // that the runtime is idle. Keep both the local stop lock and the queue
    // paused while an independent authoritative poll waits for idle.
    console.warn(`${config.logTag} stop request needs continued authoritative confirmation`, result);
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

    // The poll only returns `stale` when this stop attempt was superseded.
    // Keep the conservative lock if that cannot be proven otherwise.
    console.warn(`${config.logTag} stop confirmation became stale`, result);
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
      console.warn(`${config.logTag} clear context failed`, error);
      Message.error({
        content: t('conversation.clearContext.failed', { defaultValue: 'Failed to clear context' }),
        closable: true,
      });
    }
  };

  const uploadPreviews = uploadFile.length > 0 && (
    <HorizontalFileList>
      {uploadFile.map((path) => (
        <FilePreview key={path} path={path} onRemove={() => setUploadFile(uploadFile.filter((v) => v !== path))} />
      ))}
    </HorizontalFileList>
  );

  const folderTags = config.showFolderTags &&
    atPath.some((item) => (typeof item === 'string' ? false : !item.isFile)) && (
      <div className='flex flex-wrap items-center gap-8px mb-8px'>
        {atPath.map((item) => {
          if (typeof item === 'string') return null;
          if (!item.isFile) {
            return (
              <Tag
                key={item.path}
                bordered={false}
                className='!bg-primary-1 !text-primary-6'
                closable
                onClose={() => {
                  const newAtPath = atPath.filter((v) => (typeof v === 'string' ? true : v.path !== item.path));
                  emitter.emit(config.selectedFileEvents.set, newAtPath);
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
    );

  return (
    <div className='max-w-800px w-full mx-auto flex flex-col mt-auto mb-16px'>
      <CommandQueuePanel
        items={items}
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
        showPinnedPlan
        value={content}
        onChange={handleContentChange}
        selectedWorkspaceItems={atPath}
        onSelectedWorkspaceItemsChange={(nextSelectedItems) => {
          if (config.emitSelectedFileOnChange) {
            emitter.emit(config.selectedFileEvents.set, nextSelectedItems);
          }
          setAtPath(nextSelectedItems);
        }}
        loading={isBusy}
        disabled={false}
        className='z-10'
        placeholder={
          isBusy
            ? t('conversation.chat.processing')
            : t('acp.sendbox.placeholder', {
                backend: backendName,
                defaultValue: `Send message to ${backendName}...`,
              })
        }
        onStop={handleStop}
        onClearContext={config.enableClearContext ? handleClearContext : undefined}
        onFilesAdded={handleFilesAdded}
        hasPendingAttachments={
          config.reportPendingAttachments ? uploadFile.length > 0 || atPath.length > 0 : undefined
        }
        supportedExts={allSupportedExts}
        defaultMultiLine={config.defaultMultiLine}
        lockMultiLine={config.lockMultiLine}
        tools={<FileAttachButton openFileSelector={openFileSelector} onLocalFilesAdded={handleFilesAdded} />}
        prefix={
          <>
            {uploadPreviews}
            {folderTags}
          </>
        }
        onSend={onSendHandler}
        slash_commands={slash_commands}
        onSlashBuiltinCommand={config.useSlashCommandList ? onSlashBuiltinCommand : undefined}
        allowSendWhileLoading
      ></SendBox>
    </div>
  );
};

export default BasicRuntimeSendBox;
