/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { conversationTarget, type ConversationId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { uuid } from '@/common/utils';
import { getSendBoxDraftHook } from '@/renderer/hooks/chat/useSendBoxDraft';
import { useSlashCommands } from '@/renderer/hooks/chat/useSlashCommands';
import {
  claimInitialMessageDelivery,
  completeInitialMessageDelivery,
  persistInitialMessageDelivery,
  quarantineInitialMessageDelivery,
  readAuthorizedInitialMessageDelivery,
  readInitialMessageDelivery,
  releaseInitialMessageDelivery,
  type PersistedInitialMessage,
} from '@/renderer/pages/conversation/platforms/initialMessageDelivery';
import { classifyPublicMessageDelivery } from '@/renderer/pages/conversation/platforms/publicMessageDelivery';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { emitter, useAddEventListener } from '@/renderer/utils/emitter';
import BasicRuntimeSendBox, {
  type BasicRuntimeDraftHook,
  type BasicRuntimeSendBoxConfig,
  type BasicRuntimeSendBoxController,
  type BasicRuntimeStreamHooks,
} from '@/renderer/pages/conversation/platforms/BasicRuntimeSendBox';
import React, { useCallback, useEffect, useMemo, useRef } from 'react';

const useOpenClawSendBoxDraft = getSendBoxDraftHook('openclaw-gateway', {
  _type: 'openclaw-gateway',
  atPath: [],
  content: '',
  uploadFile: [],
});

/**
 * OpenClaw-only Star Office install flow, mounted as a platform extension of
 * the shared BasicRuntimeSendBox. Delivers explicit install requests as new
 * turns, recovers persisted requests across remounts with replay-or-initial-
 * only semantics, and reports turn completion via the stream 'finish' hook.
 */
function useStarOfficeInstallFlow(controller: BasicRuntimeSendBoxController): BasicRuntimeStreamHooks {
  const {
    conversation_id,
    setAiProcessing,
    beginLocalTurn,
    markLocalTurnAccepted,
    reconcilePublicDeliveryReplay,
    cancelLocalTurn,
    checkAndUpdateTitle,
    addOrUpdateMessage,
  } = controller;

  // Track whether the current turn was triggered by a Star Office install request
  const starOfficeInstallInFlightRef = useRef(false);

  const deliverStarOfficeRequest = useCallback(
    async (
      delivery: PersistedInitialMessage,
      storageKey: string,
      initialOnly = false
    ) => {
      if (!claimInitialMessageDelivery(storageKey)) return;
      const {
        conversation_id: deliveryConversationId,
        input: text,
        idempotency_key,
      } = delivery;

      starOfficeInstallInFlightRef.current = true;
      try {
        const result = await ipcBridge.openclawConversation.sendMessage.invoke({
          input: text,
          conversation_id: deliveryConversationId,
          idempotency_key,
          initial_only: initialOnly,
        });
        completeInitialMessageDelivery(sessionStorage, storageKey, idempotency_key);
        const disposition = classifyPublicMessageDelivery(result);
        if (disposition === 'fresh') {
          beginLocalTurn();
          setAiProcessing(true);
          void checkAndUpdateTitle(conversation_id, text);
          markLocalTurnAccepted();
          addOrUpdateMessage({
            id: uuid(),
            msg_id: result.msg_id,
            conversation_id,
            type: 'text',
            position: 'right',
            content: { content: text },
            created_at: Date.now(),
          });
        } else {
          if (result.completed) starOfficeInstallInFlightRef.current = false;
          reconcilePublicDeliveryReplay(result.completed);
        }
        emitter.emit('chat.history.refresh');
      } catch (error) {
        if (
          initialOnly &&
          isBackendHttpError(error) &&
          error.status === 409 &&
          error.code === 'CONFLICT'
        ) {
          quarantineInitialMessageDelivery(
            sessionStorage,
            storageKey,
            idempotency_key
          );
        } else {
          releaseInitialMessageDelivery(storageKey);
        }
        cancelLocalTurn();
        setAiProcessing(false);
        starOfficeInstallInFlightRef.current = false;
      }
    },
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

  useAddEventListener(
    'staroffice.install.request',
    ({ conversation_id: eventConversationId, text }) => {
      if (eventConversationId !== conversation_id) return;
      const storageKey = sessionStorageKey(
        'staroffice-turn',
        conversationTarget(conversation_id)
      );
      const delivery = persistInitialMessageDelivery(
        sessionStorage,
        storageKey,
        conversation_id,
        text,
        []
      );
      // This event comes directly from a user's click, so it is an explicit
      // new turn even on a Finished Conversation with existing history.
      void deliverStarOfficeRequest(delivery, storageKey);
    },
    [conversation_id, deliverStarOfficeRequest]
  );

  useEffect(() => {
    const storageKey = sessionStorageKey(
      'staroffice-turn',
      conversationTarget(conversation_id)
    );
    const pending = readInitialMessageDelivery(sessionStorage, storageKey);
    if (pending) {
      void getConversationOrNull(conversation_id)
        .then(async (conversation) => {
          if (
            pending.conversation_id !== conversation_id ||
            !conversation ||
            conversation.id !== conversation_id ||
            (conversation.status !== 'pending' && conversation.status !== 'running')
          ) {
            quarantineInitialMessageDelivery(
              sessionStorage,
              storageKey,
              pending.idempotency_key
            );
            return;
          }

          if (conversation.status === 'running') {
            // Automatic recovery is replay-or-initial-only even while a
            // snapshot says Running. The unrelated turn could complete before
            // this POST; a normal send would then fresh-start this stale key
            // from Finished. The backend checks an existing exact receipt
            // first, otherwise refuses anything except creation generation 0.
            void deliverStarOfficeRequest(pending, storageKey, true);
            return;
          }

          // A Pending remount is safe only for the immutable creation
          // generation. The backend's initial-only transaction rejects a
          // same-id reset generation even when its transcript is empty.
          const authorized = await readAuthorizedInitialMessageDelivery(
            sessionStorage,
            storageKey,
            conversation_id
          );
          if (authorized) {
            void deliverStarOfficeRequest(authorized, storageKey, true);
          }
        })
        .catch(() => {
          quarantineInitialMessageDelivery(
            sessionStorage,
            storageKey,
            pending.idempotency_key
          );
        });
    }
    return () => releaseInitialMessageDelivery(storageKey);
  }, [conversation_id, deliverStarOfficeRequest]);

  const onStreamFinish = useCallback(() => {
    if (starOfficeInstallInFlightRef.current) {
      starOfficeInstallInFlightRef.current = false;
      emitter.emit('staroffice.install.finished', { conversation_id });
    }
  }, [conversation_id]);

  return useMemo(() => ({ onStreamFinish }), [onStreamFinish]);
}

const OPENCLAW_SENDBOX_CONFIG: BasicRuntimeSendBoxConfig = {
  logTag: '[OpenClawSendBox]',
  selectedFileEvents: {
    set: 'openclaw-gateway.selected.file',
    append: 'openclaw-gateway.selected.file.append',
    clear: 'openclaw-gateway.selected.file.clear',
  },
  // Historical suffix: 'openclaw', not the 'openclaw-gateway' conversation type.
  initialMessageFeature: 'initial-message-openclaw',
  initialMessageProcessedFeature: 'initial-message-processed-openclaw',
  sendMessage: ipcBridge.openclawConversation.sendMessage,
  responseStream: ipcBridge.openclawConversation.responseStream,
  // The concrete draft store carries the 'openclaw-gateway' `_type`
  // discriminant, which the shared send box neither reads nor rewrites
  // (mutations spread the previous draft), so widening to the structural hook
  // type is safe.
  useDraft: useOpenClawSendBoxDraft as unknown as BasicRuntimeDraftHook,
  useSlashCommandList: useSlashCommands,
  usePlatformExtension: useStarOfficeInstallFlow,
  // In backend-proxy mode, warmup happens on the backend when send_message is
  // called, so the initial message only waits for mount + runtime hydration.
  initialMessageDelayMs: 200,
  initialMessageAfterHydration: true,
  workspaceResolution: 'on-mount',
  enableClearContext: true,
  backendName: 'OpenClaw',
  emitSelectedFileOnChange: true,
  showFolderTags: true,
  reportPendingAttachments: true,
  defaultMultiLine: true,
  lockMultiLine: true,
};

const OpenClawSendBox: React.FC<{ conversation_id: ConversationId }> = ({ conversation_id }) => (
  <BasicRuntimeSendBox conversation_id={conversation_id} config={OPENCLAW_SENDBOX_CONFIG} />
);

export default OpenClawSendBox;
