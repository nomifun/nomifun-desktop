/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId, RemoteAgentId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import { getSendBoxDraftHook } from '@/renderer/hooks/chat/useSendBoxDraft';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import BasicRuntimeSendBox, {
  type BasicRuntimeDraftHook,
  type BasicRuntimeSendBoxConfig,
} from '@/renderer/pages/conversation/platforms/BasicRuntimeSendBox';
import React from 'react';

const useRemoteSendBoxDraft = getSendBoxDraftHook('remote', {
  _type: 'remote',
  atPath: [],
  content: '',
  uploadFile: [],
});

/** Resolve the remote agent's display name for the send box placeholder. */
const resolveRemoteAgentName = async (conversation_id: ConversationId): Promise<string | undefined> => {
  const res = await getConversationOrNull(conversation_id);
  const extra = res?.extra as { remote_agent_id?: RemoteAgentId } | undefined;
  if (extra?.remote_agent_id == null) return undefined;
  const agent = await ipcBridge.remoteAgent.get.invoke({ remote_agent_id: extra.remote_agent_id });
  return agent?.name || undefined;
};

const REMOTE_SENDBOX_CONFIG: BasicRuntimeSendBoxConfig = {
  logTag: '[RemoteSendBox]',
  selectedFileEvents: {
    set: 'remote.selected.file',
    append: 'remote.selected.file.append',
    clear: 'remote.selected.file.clear',
  },
  initialMessageFeature: 'initial-message-remote',
  initialMessageProcessedFeature: 'initial-message-processed-remote',
  sendMessage: ipcBridge.conversation.sendMessage,
  responseStream: ipcBridge.conversation.responseStream,
  // The concrete draft store carries the 'remote' `_type` discriminant, which
  // the shared send box neither reads nor rewrites (mutations spread the
  // previous draft), so widening to the structural hook type is safe.
  useDraft: useRemoteSendBoxDraft as unknown as BasicRuntimeDraftHook,
  // Small delay to let the component mount and the stream listener attach.
  initialMessageDelayMs: 300,
  workspaceResolution: 'on-mount',
  enableClearContext: true,
  backendName: 'Remote Agent',
  resolveBackendName: resolveRemoteAgentName,
  defaultMultiLine: true,
  lockMultiLine: true,
};

const RemoteSendBox: React.FC<{ conversation_id: ConversationId }> = ({ conversation_id }) => (
  <BasicRuntimeSendBox conversation_id={conversation_id} config={REMOTE_SENDBOX_CONFIG} />
);

export default RemoteSendBox;
