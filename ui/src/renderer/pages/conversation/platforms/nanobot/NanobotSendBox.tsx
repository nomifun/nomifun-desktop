/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import { getSendBoxDraftHook } from '@/renderer/hooks/chat/useSendBoxDraft';
import { useSlashCommands } from '@/renderer/hooks/chat/useSlashCommands';
import BasicRuntimeSendBox, {
  type BasicRuntimeDraftHook,
  type BasicRuntimeSendBoxConfig,
} from '@/renderer/pages/conversation/platforms/BasicRuntimeSendBox';
import React from 'react';

const useNanobotSendBoxDraft = getSendBoxDraftHook('nanobot', {
  _type: 'nanobot',
  atPath: [],
  content: '',
  uploadFile: [],
});

const NANOBOT_SENDBOX_CONFIG: BasicRuntimeSendBoxConfig = {
  logTag: '[NanobotSendBox]',
  selectedFileEvents: {
    set: 'nanobot.selected.file',
    append: 'nanobot.selected.file.append',
    clear: 'nanobot.selected.file.clear',
  },
  initialMessageFeature: 'initial-message-nanobot',
  initialMessageProcessedFeature: 'initial-message-processed-nanobot',
  sendMessage: ipcBridge.conversation.sendMessage,
  responseStream: ipcBridge.conversation.responseStream,
  // The concrete draft store carries the 'nanobot' `_type` discriminant, which
  // the shared send box neither reads nor rewrites (mutations spread the
  // previous draft), so widening to the structural hook type is safe.
  useDraft: useNanobotSendBoxDraft as unknown as BasicRuntimeDraftHook,
  useSlashCommandList: useSlashCommands,
  // Nanobot is stateless: the guid-page initial message is sent immediately,
  // and direct sends before any initial delivery format against an empty
  // workspace path (resolved only during initial-message delivery).
  workspaceResolution: 'at-initial-message',
  // Nanobot has no resumable session history, so clear-context is intentionally
  // not offered here (the backend reports it as unsupported).
  backendName: 'Nanobot',
  emitSelectedFileOnChange: true,
  showFolderTags: true,
  reportPendingAttachments: true,
};

const NanobotSendBox: React.FC<{ conversation_id: ConversationId }> = ({ conversation_id }) => (
  <BasicRuntimeSendBox conversation_id={conversation_id} config={NANOBOT_SENDBOX_CONFIG} />
);

export default NanobotSendBox;
