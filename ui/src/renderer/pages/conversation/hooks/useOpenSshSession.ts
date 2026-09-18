/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { IApiSshHost } from '@/common/adapter/ipcBridge';
import { conversationTarget, parseConversationId } from '@/common/types/ids';
import { emitter } from '@renderer/utils/emitter';
import { seedConversationCache } from '@renderer/pages/conversation/utils/conversationCache';
import { useGuidModelSelection } from '@renderer/pages/guid/hooks/useGuidModelSelection';
import type { AgentPresetDraft } from '@/common/types/agentPlatform';

/**
 * Open a nomi conversation bound to a saved SSH host, then jump to it.
 *
 * One implementation for both entry points — the host book in settings and the
 * sidebar's remote-session popover — so a host always starts a session the same
 * way. The conversation only carries `extra.ssh_host_id`; the session factory is
 * what connects the host and hands the agent its remote tools, so nothing here
 * touches the transport.
 *
 * Resolves `true` once the conversation exists and navigation was issued, so a
 * caller can close its own surface only on success and leave it open (with the
 * error toast still on screen) otherwise.
 */
export const useOpenSshSession = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { current_model } = useGuidModelSelection('nomi');

  return useCallback(
    async (host: IApiSshHost): Promise<boolean> => {
      if (!current_model) {
        Message.warning(t('conversation.noModelConfigured'));
        return false;
      }
      try {
        const editor = await ipcBridge.agentPlatform.createFromTemplate.invoke({
          template_id: 'chat.minimal',
          request: {
            display_name: 'SSH Agent',
            reuse_existing: true,
            model_route_refs: {},
            chat_route_records: {},
            model: { provider_id: current_model.id, model: current_model.use_model },
          },
        });
        let preset = editor.preset;
        const hasSsh = editor.draft.document.enabled_capabilities.some(
          (selection) => selection.capability.id === 'ssh'
        );
        if (!hasSsh) {
          const draft: AgentPresetDraft = {
            ...editor.draft,
            document: {
              ...editor.draft.document,
              enabled_capabilities: [
                ...editor.draft.document.enabled_capabilities,
                {
                  capability: {
                    id: 'ssh' as AgentPresetDraft['document']['enabled_capabilities'][number]['capability']['id'],
                    version: '1.0.0',
                  },
                  action_allowlist: ['ssh/exec'],
                },
              ],
            },
          };
          const saved = await ipcBridge.agentPlatform.saveRevision.invoke({
            preset_id: editor.preset.preset_id,
            request: {
              expected_current_revision: editor.draft.current_revision,
              draft,
              reason: 'Enable the SSH Module for saved-host sessions',
            },
          });
          preset = saved.preset;
        }
        const session = await ipcBridge.agentPlatform.sessions.create.invoke({
          preset_id: preset.preset_id,
          title: host.name,
          model: { provider_id: current_model.id, model: current_model.use_model },
          resource_selections: [{ resource_kind: 'ssh_host', resource_id: host.sshHostId }],
        });
        const conversation = await ipcBridge.conversation.get.invoke({
          conversation_id: parseConversationId(session.agent_session_id),
        });
        if (!conversation || !conversation.id) {
          Message.error(t('conversation.createFailed'));
          return false;
        }
        emitter.emit('chat.history.refresh');
        seedConversationCache(conversation);
        void conversationTarget(conversation.id);
        await navigate(`/conversation/${conversation.id}`);
        return true;
      } catch {
        Message.error(t('conversation.createFailed'));
        return false;
      }
    },
    [current_model, navigate, t]
  );
};

export default useOpenSshSession;
