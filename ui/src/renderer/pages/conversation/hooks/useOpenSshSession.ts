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
import { conversationTarget } from '@/common/types/ids';
import { emitter } from '@renderer/utils/emitter';
import { seedConversationCache } from '@renderer/pages/conversation/utils/conversationCache';
import { useGuidModelSelection } from '@renderer/pages/guid/hooks/useGuidModelSelection';

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
        const conversation = await ipcBridge.conversation.create.invoke({
          type: 'nomi',
          name: host.name,
          model: current_model,
          extra: {
            workspace: '',
            custom_workspace: false,
            default_files: [],
            ssh_host_id: host.sshHostId,
          },
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
